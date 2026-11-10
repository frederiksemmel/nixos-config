mod samsung;
mod upnp;

use crate::api::Outcome;
use crate::cec::{self, Screen};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const STEP: i64 = 2;
const SHOWING_POLL: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Generic,
    Samsung,
}

impl Kind {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "generic" => Ok(Self::Generic),
            "samsung" => Ok(Self::Samsung),
            other => Err(format!("unknown TV kind {other}, expected generic or samsung")),
        }
    }

    fn of_manufacturer(manufacturer: &str) -> Self {
        if manufacturer.to_lowercase().contains("samsung") {
            Self::Samsung
        } else {
            Self::Generic
        }
    }
}

pub struct Settings {
    pub kind: Option<Kind>,
    pub renderer: Option<String>,
    pub token: PathBuf,
}

#[derive(Debug, PartialEq)]
pub enum Failure {
    Unreachable(String),
    Refused(String),
    Garbled(String),
    Unsupported(&'static str),
    NeedsPairing,
    Denied,
    NoAnswer,
    Unsaved(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable(_) => write!(f, "The TV is off or not on the network"),
            Self::Refused(_) => write!(f, "The TV refused the change"),
            Self::Garbled(what) => write!(f, "The TV answered something unexpected: {what}"),
            Self::Unsupported(what) => write!(f, "{what} is not supported by this TV"),
            Self::NeedsPairing => write!(f, "The TV is not paired with SemmelTV yet"),
            Self::Denied => write!(f, "The TV denied the pairing"),
            Self::NoAnswer => write!(f, "Nobody pressed Allow on the TV in time"),
            Self::Unsaved(error) => write!(f, "The TV allowed SemmelTV, but saving that failed ({error})"),
        }
    }
}

pub trait Power: Send + Sync {
    fn screen(&self) -> Screen;
    fn show_semmeltv(&self) -> Result<(), Failure>;
    fn turn_off(&self) -> Result<(), Failure>;
}

pub trait Volume: Send + Sync {
    fn step(&self, by: i64) -> Result<i64, Failure>;
    fn toggle_mute(&self) -> Result<bool, Failure>;
}

pub trait Pairing: Send + Sync {
    fn pair(&self) -> Result<(), Failure>;
}

struct Cec;

impl Power for Cec {
    fn screen(&self) -> Screen {
        cec::screen()
    }

    fn show_semmeltv(&self) -> Result<(), Failure> {
        cec::show_semmeltv().map_err(|error| Failure::Unreachable(error.to_string()))
    }

    fn turn_off(&self) -> Result<(), Failure> {
        cec::turn_tv_off().map_err(|error| Failure::Unreachable(error.to_string()))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Woken {
    AlreadyShowing,
    Switched,
    TurnedOn,
}

struct Backends {
    kind: Kind,
    volume: Option<Box<dyn Volume>>,
    pairing: Option<Box<dyn Pairing>>,
}

fn backends(kind: Kind, renderer: Option<&upnp::Renderer>, token: &Path) -> Backends {
    let volume = renderer.map(|renderer| Box::new(renderer.clone()) as Box<dyn Volume>);
    let pairing = match kind {
        Kind::Generic => None,
        Kind::Samsung => renderer
            .map(|renderer| Box::new(samsung::Remote::new(renderer.host(), token.to_path_buf())) as Box<dyn Pairing>),
    };
    Backends { kind, volume, pairing }
}

#[derive(Clone)]
pub struct Television {
    inner: Arc<Inner>,
}

struct Inner {
    power: Box<dyn Power>,
    kind: Option<Kind>,
    renderer: Option<upnp::Renderer>,
    token: PathBuf,
    chosen: Mutex<Option<Arc<Backends>>>,
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Abilities {
    kind: Option<Kind>,
    volume: bool,
    pairing: bool,
    paired: Option<bool>,
}

impl Television {
    pub fn new(settings: Settings) -> Self {
        Self {
            inner: Arc::new(Inner {
                power: Box::new(Cec),
                kind: settings.kind,
                renderer: settings.renderer.map(upnp::Renderer::new),
                token: settings.token,
                chosen: Mutex::new(None),
            }),
        }
    }

    fn chosen(&self) -> Result<Arc<Backends>, Failure> {
        let inner = &self.inner;
        if let Some(backends) = inner.chosen.lock().unwrap().clone() {
            return Ok(backends);
        }
        let kind = match (inner.kind, &inner.renderer) {
            (Some(kind), _) => kind,
            (None, Some(renderer)) => Kind::of_manufacturer(&renderer.manufacturer()?),
            (None, None) => Kind::Generic,
        };
        let backends = Arc::new(backends(kind, inner.renderer.as_ref(), &inner.token));
        *inner.chosen.lock().unwrap() = Some(backends.clone());
        Ok(backends)
    }

    pub fn abilities(&self) -> Abilities {
        match self.chosen() {
            Ok(backends) => Abilities {
                kind: Some(backends.kind),
                volume: backends.volume.is_some(),
                pairing: backends.pairing.is_some(),
                paired: backends.pairing.is_some().then(|| self.paired()),
            },
            Err(_) => Abilities {
                kind: None,
                volume: self.inner.renderer.is_some(),
                pairing: false,
                paired: None,
            },
        }
    }

    fn paired(&self) -> bool {
        std::fs::read_to_string(&self.inner.token).is_ok_and(|token| !token.trim().is_empty())
    }

    pub fn show_semmeltv(&self) -> Result<(), Failure> {
        self.inner.power.show_semmeltv()
    }

    pub fn turn_off(&self) -> Result<(), Failure> {
        self.inner.power.turn_off()
    }

    pub fn wake(&self) -> Result<Woken, Failure> {
        let woken = match self.inner.power.screen() {
            Screen::Showing => return Ok(Woken::AlreadyShowing),
            Screen::Elsewhere => Woken::Switched,
            Screen::Off => Woken::TurnedOn,
        };
        self.inner.power.show_semmeltv().map(|()| woken)
    }

    pub fn wait_until_showing(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.inner.power.screen() == Screen::Showing {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(SHOWING_POLL);
        }
    }

    pub fn step_volume(&self, by: i64) -> Result<i64, Failure> {
        self.with_volume(|volume| volume.step(by))
    }

    pub fn toggle_mute(&self) -> Result<bool, Failure> {
        self.with_volume(|volume| volume.toggle_mute())
    }

    pub fn pair(&self) -> Result<(), Failure> {
        let backends = self.chosen()?;
        let pairing = backends.pairing.as_deref().ok_or(Failure::Unsupported("Pairing"))?;
        pairing.pair()
    }

    fn with_volume<T>(&self, change: impl FnOnce(&dyn Volume) -> Result<T, Failure>) -> Result<T, Failure> {
        let backends = self.chosen()?;
        let volume = backends
            .volume
            .as_deref()
            .ok_or(Failure::Unsupported("Volume control"))?;
        change(volume).map_err(|failure| match failure {
            Failure::Refused(_) if backends.pairing.is_some() => Failure::NeedsPairing,
            failure => failure,
        })
    }
}

pub fn routes<S: Clone + Send + Sync + 'static>(television: Television) -> Router<S> {
    Router::new()
        .route("/api/tv", get(abilities))
        .route("/api/tv/volume-up", post(volume_up))
        .route("/api/tv/volume-down", post(volume_down))
        .route("/api/tv/mute", post(mute))
        .route("/api/tv/pair", post(pair))
        .with_state(television)
}

async fn abilities(State(television): State<Television>) -> Response {
    match tokio::task::spawn_blocking(move || television.abilities()).await {
        Ok(abilities) => Json(abilities).into_response(),
        Err(error) => crate::api::problem(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn volume_up(State(television): State<Television>) -> Response {
    act(television, |television| {
        television.step_volume(STEP).map(|volume| format!("TV volume {volume}"))
    })
    .await
}

async fn volume_down(State(television): State<Television>) -> Response {
    act(television, |television| {
        television
            .step_volume(-STEP)
            .map(|volume| format!("TV volume {volume}"))
    })
    .await
}

async fn mute(State(television): State<Television>) -> Response {
    act(television, |television| {
        television
            .toggle_mute()
            .map(|muted| if muted { "TV muted" } else { "TV sound on" }.to_string())
    })
    .await
}

async fn pair(State(television): State<Television>) -> Response {
    act(television, |television| {
        television.pair().map(|()| "Paired with the TV".to_string())
    })
    .await
}

async fn act(
    television: Television,
    change: impl FnOnce(&Television) -> Result<String, Failure> + Send + 'static,
) -> Response {
    let done = tokio::task::spawn_blocking(move || change(&television)).await;
    let (status, outcome) = match done {
        Ok(Ok(message)) => (StatusCode::OK, Outcome { message, ok: true }),
        Ok(Err(failure)) => {
            eprintln!("TV: {failure:?}");
            let message = failure.to_string();
            (StatusCode::UNPROCESSABLE_ENTITY, Outcome { message, ok: false })
        }
        Err(error) => {
            let message = error.to_string();
            (StatusCode::UNPROCESSABLE_ENTITY, Outcome { message, ok: false })
        }
    };
    (status, Json(outcome)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use tokio::net::TcpListener;

    struct NoPower;

    impl Power for NoPower {
        fn screen(&self) -> Screen {
            Screen::Showing
        }

        fn show_semmeltv(&self) -> Result<(), Failure> {
            Ok(())
        }

        fn turn_off(&self) -> Result<(), Failure> {
            Ok(())
        }
    }

    struct Allowing;

    impl Pairing for Allowing {
        fn pair(&self) -> Result<(), Failure> {
            Ok(())
        }
    }

    fn television(kind: Option<Kind>, renderer: Option<upnp::Renderer>) -> Television {
        Television {
            inner: Arc::new(Inner {
                power: Box::new(NoPower),
                kind,
                renderer,
                token: std::env::temp_dir().join("semmeltv-tv-token-never-written"),
                chosen: Mutex::new(None),
            }),
        }
    }

    struct Fake {
        screen: Screen,
        shown: Mutex<u32>,
    }

    impl Power for Arc<Fake> {
        fn screen(&self) -> Screen {
            self.screen
        }

        fn show_semmeltv(&self) -> Result<(), Failure> {
            *self.shown.lock().unwrap() += 1;
            Ok(())
        }

        fn turn_off(&self) -> Result<(), Failure> {
            Ok(())
        }
    }

    fn woken(screen: Screen) -> (Result<Woken, Failure>, u32) {
        let screen = Arc::new(Fake {
            screen,
            shown: Mutex::new(0),
        });
        let television = Television {
            inner: Arc::new(Inner {
                power: Box::new(screen.clone()),
                kind: None,
                renderer: None,
                token: std::env::temp_dir().join("semmeltv-tv-token-never-written"),
                chosen: Mutex::new(None),
            }),
        };
        let woken = television.wake();
        let shown = *screen.shown.lock().unwrap();
        (woken, shown)
    }

    #[test]
    fn waking_turns_the_tv_on_only_when_it_is_not_showing_semmeltv() {
        assert_eq!(woken(Screen::Showing), (Ok(Woken::AlreadyShowing), 0));
        assert_eq!(woken(Screen::Elsewhere), (Ok(Woken::Switched), 1));
        assert_eq!(woken(Screen::Off), (Ok(Woken::TurnedOn), 1));
    }

    fn fixed(backends: Backends) -> Television {
        let television = television(Some(backends.kind), None);
        *television.inner.chosen.lock().unwrap() = Some(Arc::new(backends));
        television
    }

    #[test]
    fn the_kind_comes_from_config_or_the_manufacturer() {
        assert_eq!(Kind::parse("samsung"), Ok(Kind::Samsung));
        assert_eq!(Kind::parse("generic"), Ok(Kind::Generic));
        assert!(Kind::parse("lg").is_err());
        assert_eq!(Kind::of_manufacturer("Samsung Electronics"), Kind::Samsung);
        assert_eq!(Kind::of_manufacturer("LG Electronics."), Kind::Generic);
        let fake = upnp::tests::fake(upnp::tests::Tv::default());
        let detected = television(None, Some(fake.renderer())).abilities();
        assert_eq!(
            detected,
            Abilities {
                kind: Some(Kind::Samsung),
                volume: true,
                pairing: true,
                paired: Some(false)
            }
        );
        let configured = television(Some(Kind::Generic), Some(fake.renderer())).abilities();
        assert_eq!((configured.kind, configured.pairing), (Some(Kind::Generic), false));
    }

    #[test]
    fn only_a_samsung_with_a_renderer_pairs() {
        let renderer = upnp::Renderer::new("http://192.168.1.148:9197/dmr".to_string());
        let token = Path::new("/tmp/token");
        let samsung = backends(Kind::Samsung, Some(&renderer), token);
        assert!(samsung.volume.is_some() && samsung.pairing.is_some());
        let generic = backends(Kind::Generic, Some(&renderer), token);
        assert!(generic.volume.is_some() && generic.pairing.is_none());
        let bare = backends(Kind::Samsung, None, token);
        assert!(bare.volume.is_none() && bare.pairing.is_none());
    }

    #[test]
    fn a_refusal_means_pairing_only_on_a_tv_that_pairs() {
        let refusing = || {
            upnp::tests::fake(upnp::tests::Tv {
                refuse: true,
                ..upnp::tests::Tv::default()
            })
        };
        let fake = refusing();
        let samsung = fixed(Backends {
            kind: Kind::Samsung,
            volume: Some(Box::new(fake.renderer())),
            pairing: Some(Box::new(Allowing)),
        });
        assert_eq!(samsung.step_volume(STEP), Err(Failure::NeedsPairing));
        assert_eq!(samsung.toggle_mute(), Err(Failure::NeedsPairing));
        assert_eq!(
            Failure::NeedsPairing.to_string(),
            "The TV is not paired with SemmelTV yet"
        );
        let generic = fixed(Backends {
            kind: Kind::Generic,
            volume: Some(Box::new(fake.renderer())),
            pairing: None,
        });
        assert_eq!(
            generic.step_volume(STEP),
            Err(Failure::Refused("501 Action Failed".to_string()))
        );
    }

    #[test]
    fn a_missing_ability_is_not_supported() {
        let bare = television(Some(Kind::Generic), None);
        assert_eq!(bare.step_volume(STEP), Err(Failure::Unsupported("Volume control")));
        assert_eq!(bare.pair(), Err(Failure::Unsupported("Pairing")));
        assert_eq!(
            Failure::Unsupported("Pairing").to_string(),
            "Pairing is not supported by this TV"
        );
    }

    fn serve(television: Television) -> (String, tokio::runtime::Runtime) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let router: Router = routes(television);
        runtime.spawn(async move { axum::serve(listener, router).await });
        (base, runtime)
    }

    fn agent() -> ureq::Agent {
        ureq::Agent::config_builder().http_status_as_error(false).build().into()
    }

    fn post_json(url: String) -> (u16, Value) {
        let mut response = agent().post(url).send_empty().unwrap();
        (response.status().as_u16(), response.body_mut().read_json().unwrap())
    }

    #[test]
    fn the_api_answers_with_the_new_volume_or_the_reason() {
        let tv = upnp::tests::Tv::default();
        *tv.volume.lock().unwrap() = 20;
        let fake = upnp::tests::fake(tv);
        let (base, _runtime) = serve(television(Some(Kind::Samsung), Some(fake.renderer())));
        let (status, body) = post_json(format!("{base}/api/tv/volume-down"));
        assert_eq!((status, body["message"].as_str().unwrap()), (200, "TV volume 18"));
        let (_, body) = post_json(format!("{base}/api/tv/mute"));
        assert_eq!(body["message"], "TV muted");
        let abilities: Value = agent()
            .get(format!("{base}/api/tv"))
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();
        assert_eq!(
            abilities,
            serde_json::json!({ "kind": "samsung", "volume": true, "pairing": true, "paired": false })
        );
        let (base, _bare) = serve(television(Some(Kind::Generic), None));
        let (status, body) = post_json(format!("{base}/api/tv/pair"));
        assert_eq!((status, body["ok"].as_bool()), (422, Some(false)));
        assert_eq!(body["message"], "Pairing is not supported by this TV");
    }
}
