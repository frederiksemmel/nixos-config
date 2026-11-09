use crate::api::Outcome;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const SERVICE: &str = "urn:schemas-upnp-org:service:RenderingControl:1";
const STEP: i64 = 2;
const TIMEOUT: Duration = Duration::from_secs(3);
const NOT_SET_UP: &str = "This TV has no volume control set up";

#[derive(Debug, PartialEq)]
pub enum Failure {
    Unreachable(String),
    Refused(String),
    Garbled(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreachable(error) => write!(f, "The TV is off or not on the network ({error})"),
            Self::Refused(_) => write!(f, "The TV refused the volume change"),
            Self::Garbled(what) => write!(f, "The TV answered something unexpected: {what}"),
        }
    }
}

#[derive(Clone)]
pub struct Renderer {
    description: String,
    agent: ureq::Agent,
    control: Arc<Mutex<Option<String>>>,
}

impl Renderer {
    pub fn new(description: String) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(TIMEOUT))
            .build()
            .into();
        Self {
            description,
            agent,
            control: Arc::new(Mutex::new(None)),
        }
    }

    fn control(&self) -> Result<String, Failure> {
        if let Some(url) = self.control.lock().unwrap().clone() {
            return Ok(url);
        }
        let text = self
            .agent
            .get(&self.description)
            .call()
            .and_then(|mut response| response.body_mut().read_to_string())
            .map_err(|error| Failure::Unreachable(error.to_string()))?;
        let path = control_path(&text).ok_or_else(|| Failure::Garbled("no RenderingControl service".to_string()))?;
        let url = resolve(&self.description, path);
        *self.control.lock().unwrap() = Some(url.clone());
        Ok(url)
    }

    fn call(&self, action: &str, arguments: &str) -> Result<String, Failure> {
        let url = self.control()?;
        let body = envelope(action, arguments);
        let (status, text) = self
            .agent
            .post(&url)
            .header("SOAPAction", format!("\"{SERVICE}#{action}\""))
            .header("Content-Type", "text/xml; charset=\"utf-8\"")
            .header("Connection", "close")
            .send(body.as_bytes())
            .and_then(|mut response| {
                let status = response.status().as_u16();
                Ok((status, response.body_mut().read_to_string()?))
            })
            .map_err(|error| Failure::Unreachable(error.to_string()))?;
        match (element(&text, "errorCode"), status) {
            (Some(code), _) => {
                let description = element(&text, "errorDescription").unwrap_or("no reason given");
                Err(Failure::Refused(format!("{code} {description}")))
            }
            (None, 200..=299) => Ok(text),
            (None, status) => Err(Failure::Refused(format!("HTTP {status}"))),
        }
    }

    fn number(&self, action: &str, name: &str) -> Result<i64, Failure> {
        let text = self.call(action, "<Channel>Master</Channel>")?;
        element(&text, name)
            .and_then(|value| value.trim().parse().ok())
            .ok_or_else(|| Failure::Garbled(format!("no {name}")))
    }

    pub fn step(&self, by: i64) -> Result<i64, Failure> {
        let volume = (self.number("GetVolume", "CurrentVolume")? + by).clamp(0, 100);
        self.call(
            "SetVolume",
            &format!("<Channel>Master</Channel><DesiredVolume>{volume}</DesiredVolume>"),
        )?;
        Ok(volume)
    }

    pub fn toggle_mute(&self) -> Result<bool, Failure> {
        let muted = self.number("GetMute", "CurrentMute")? == 0;
        self.call(
            "SetMute",
            &format!(
                "<Channel>Master</Channel><DesiredMute>{}</DesiredMute>",
                u8::from(muted)
            ),
        )?;
        Ok(muted)
    }
}

fn envelope(action: &str, arguments: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{SERVICE}\">\
         <InstanceID>0</InstanceID>{arguments}</u:{action}></s:Body></s:Envelope>"
    )
}

fn element<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let start = xml.find(&open)? + open.len();
    let end = start + xml[start..].find(&format!("</{name}>"))?;
    Some(&xml[start..end])
}

fn control_path(description: &str) -> Option<&str> {
    let service = description.find(SERVICE)?;
    element(&description[service..], "controlURL").map(str::trim)
}

fn resolve(description: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_string();
    }
    let after_scheme = description.find("://").map_or(0, |at| at + 3);
    let origin = description[after_scheme..]
        .find('/')
        .map_or(description, |at| &description[..after_scheme + at]);
    format!("{origin}/{}", path.trim_start_matches('/'))
}

pub fn routes<S: Clone + Send + Sync + 'static>(renderer: Option<Renderer>) -> Router<S> {
    Router::new()
        .route("/api/tv/volume-up", post(volume_up))
        .route("/api/tv/volume-down", post(volume_down))
        .route("/api/tv/mute", post(mute))
        .with_state(renderer)
}

async fn volume_up(State(renderer): State<Option<Renderer>>) -> Response {
    act(renderer, |renderer| {
        renderer.step(STEP).map(|volume| format!("TV volume {volume}"))
    })
    .await
}

async fn volume_down(State(renderer): State<Option<Renderer>>) -> Response {
    act(renderer, |renderer| {
        renderer.step(-STEP).map(|volume| format!("TV volume {volume}"))
    })
    .await
}

async fn mute(State(renderer): State<Option<Renderer>>) -> Response {
    act(renderer, |renderer| {
        renderer
            .toggle_mute()
            .map(|muted| if muted { "TV muted" } else { "TV sound on" }.to_string())
    })
    .await
}

async fn act(
    renderer: Option<Renderer>,
    change: impl FnOnce(&Renderer) -> Result<String, Failure> + Send + 'static,
) -> Response {
    let Some(renderer) = renderer else {
        return answer(Err(NOT_SET_UP.to_string()));
    };
    let done = tokio::task::spawn_blocking(move || change(&renderer)).await;
    answer(match done {
        Ok(result) => result.map_err(|failure| {
            eprintln!("TV volume: {failure:?}");
            failure.to_string()
        }),
        Err(error) => Err(error.to_string()),
    })
}

fn answer(result: Result<String, String>) -> Response {
    let (status, outcome) = match result {
        Ok(message) => (StatusCode::OK, Outcome { message, ok: true }),
        Err(message) => (StatusCode::UNPROCESSABLE_ENTITY, Outcome { message, ok: false }),
    };
    (status, Json(outcome)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;
    use axum::routing::get;
    use serde_json::Value;
    use tokio::net::TcpListener;

    const DESCRIPTION: &str = "<root><device><serviceList>\
        <service><serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType>\
        <controlURL>/upnp/control/AVTransport1</controlURL></service>\
        <service><serviceType>urn:schemas-upnp-org:service:RenderingControl:1</serviceType>\
        <controlURL>/upnp/control/RenderingControl1</controlURL></service>\
        </serviceList></device></root>";

    #[derive(Clone, Default)]
    struct Tv {
        volume: Arc<Mutex<i64>>,
        muted: Arc<Mutex<bool>>,
        refuse: bool,
        received: Arc<Mutex<Vec<String>>>,
    }

    async fn rendering_control(State(tv): State<Tv>, headers: HeaderMap, body: String) -> Response {
        let action = headers["soapaction"]
            .to_str()
            .unwrap()
            .rsplit('#')
            .next()
            .unwrap()
            .trim_matches('"')
            .to_string();
        tv.received.lock().unwrap().push(body.clone());
        if tv.refuse && action.starts_with("Set") {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "<s:Envelope><s:Body><s:Fault><detail><UPnPError><errorCode>501</errorCode>\
                 <errorDescription>Action Failed</errorDescription></UPnPError></detail></s:Fault></s:Body></s:Envelope>",
            )
                .into_response();
        }
        let reply = match action.as_str() {
            "GetVolume" => format!("<CurrentVolume>{}</CurrentVolume>", tv.volume.lock().unwrap()),
            "GetMute" => format!("<CurrentMute>{}</CurrentMute>", u8::from(*tv.muted.lock().unwrap())),
            "SetVolume" => {
                *tv.volume.lock().unwrap() = element(&body, "DesiredVolume").unwrap().parse().unwrap();
                "<u:SetVolumeResponse/>".to_string()
            }
            "SetMute" => {
                *tv.muted.lock().unwrap() = element(&body, "DesiredMute") == Some("1");
                "<u:SetMuteResponse/>".to_string()
            }
            _ => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
        reply.into_response()
    }

    struct Fake {
        tv: Tv,
        base: String,
        _runtime: tokio::runtime::Runtime,
    }

    fn fake(tv: Tv) -> Fake {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/dmr", get(|| async { DESCRIPTION }))
            .route("/upnp/control/RenderingControl1", post(rendering_control))
            .with_state(tv.clone());
        runtime.spawn(async move { axum::serve(listener, router).await });
        Fake {
            tv,
            base,
            _runtime: runtime,
        }
    }

    fn renderer(fake: &Fake) -> Renderer {
        Renderer::new(format!("{}/dmr", fake.base))
    }

    #[test]
    fn the_control_url_comes_from_the_rendering_control_service() {
        assert_eq!(control_path(DESCRIPTION), Some("/upnp/control/RenderingControl1"));
        assert_eq!(
            resolve("http://192.168.1.148:9197/dmr", "/upnp/control/RenderingControl1"),
            "http://192.168.1.148:9197/upnp/control/RenderingControl1"
        );
        assert_eq!(resolve("http://tv:9197/dmr", "control"), "http://tv:9197/control");
        assert_eq!(resolve("http://tv:9197/dmr", "http://other/x"), "http://other/x");
    }

    #[test]
    fn volume_steps_by_two_within_its_range() {
        let tv = Tv::default();
        *tv.volume.lock().unwrap() = 11;
        let fake = fake(tv);
        let renderer = renderer(&fake);
        assert_eq!(renderer.step(STEP), Ok(13));
        assert_eq!(*fake.tv.volume.lock().unwrap(), 13);
        assert_eq!(renderer.step(-STEP), Ok(11));
        *fake.tv.volume.lock().unwrap() = 99;
        assert_eq!(renderer.step(STEP), Ok(100));
        *fake.tv.volume.lock().unwrap() = 1;
        assert_eq!(renderer.step(-STEP), Ok(0));
        let set = fake
            .tv
            .received
            .lock()
            .unwrap()
            .iter()
            .find(|body| body.contains("SetVolume"))
            .cloned()
            .unwrap();
        assert!(set.contains("<InstanceID>0</InstanceID><Channel>Master</Channel><DesiredVolume>13</DesiredVolume>"));
    }

    #[test]
    fn mute_toggles() {
        let fake = fake(Tv::default());
        let renderer = renderer(&fake);
        assert_eq!(renderer.toggle_mute(), Ok(true));
        assert!(*fake.tv.muted.lock().unwrap());
        assert_eq!(renderer.toggle_mute(), Ok(false));
    }

    #[test]
    fn a_refusal_is_reported_as_one() {
        let fake = fake(Tv {
            refuse: true,
            ..Tv::default()
        });
        let failure = renderer(&fake).step(STEP).unwrap_err();
        assert_eq!(failure, Failure::Refused("501 Action Failed".to_string()));
        assert_eq!(failure.to_string(), "The TV refused the volume change");
    }

    #[test]
    fn an_unreachable_tv_says_so() {
        let failure = Renderer::new("http://127.0.0.1:9/dmr".to_string())
            .step(STEP)
            .unwrap_err();
        assert!(matches!(failure, Failure::Unreachable(_)), "{failure:?}");
    }

    fn serve(renderer: Option<Renderer>) -> (String, tokio::runtime::Runtime) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let router: Router = routes(renderer);
        runtime.spawn(async move { axum::serve(listener, router).await });
        (base, runtime)
    }

    fn post_json(url: String) -> (u16, Value) {
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        let mut response = agent.post(url).send_empty().unwrap();
        (response.status().as_u16(), response.body_mut().read_json().unwrap())
    }

    #[test]
    fn the_api_answers_with_the_new_volume_or_the_reason() {
        let tv = Tv::default();
        *tv.volume.lock().unwrap() = 20;
        let fake = fake(tv);
        let (base, _runtime) = serve(Some(renderer(&fake)));
        let (status, body) = post_json(format!("{base}/api/tv/volume-down"));
        assert_eq!((status, body["message"].as_str().unwrap()), (200, "TV volume 18"));
        let (_, body) = post_json(format!("{base}/api/tv/mute"));
        assert_eq!(body["message"], "TV muted");
        let (base, _idle) = serve(None);
        let (status, body) = post_json(format!("{base}/api/tv/volume-up"));
        assert_eq!((status, body["ok"].as_bool()), (422, Some(false)));
        assert_eq!(body["message"], NOT_SET_UP);
    }
}
