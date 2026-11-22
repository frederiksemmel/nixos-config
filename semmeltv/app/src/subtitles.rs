use crate::api;
use crate::auth::gate::Who;
use crate::core::{Core, Request};
use crate::db::{self, Db};
use crate::link::messages::PlaySubtitles;
use crate::profile::ProfileId;
use crate::sidecar::Sidecar;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

mod tracks;
pub use tracks::{Answer, OBSERVED, Observed, SubtitleTracks};

const LANGUAGES: &[(&[&str], &str)] = &[
    (&["de", "ger", "deu"], "German"),
    (&["en", "eng"], "English"),
    (&["fr", "fre", "fra"], "French"),
    (&["es", "spa"], "Spanish"),
    (&["it", "ita"], "Italian"),
    (&["nl", "dut", "nld"], "Dutch"),
    (&["pt", "por"], "Portuguese"),
    (&["sv", "swe"], "Swedish"),
    (&["da", "dan"], "Danish"),
    (&["no", "nor", "nob", "nb", "nn", "nno"], "Norwegian"),
    (&["fi", "fin"], "Finnish"),
    (&["is", "ice", "isl"], "Icelandic"),
    (&["pl", "pol"], "Polish"),
    (&["cs", "cze", "ces"], "Czech"),
    (&["sk", "slo", "slk"], "Slovak"),
    (&["hu", "hun"], "Hungarian"),
    (&["ro", "rum", "ron"], "Romanian"),
    (&["bg", "bul"], "Bulgarian"),
    (&["hr", "hrv"], "Croatian"),
    (&["sr", "srp"], "Serbian"),
    (&["sl", "slv"], "Slovenian"),
    (&["el", "gre", "ell"], "Greek"),
    (&["tr", "tur"], "Turkish"),
    (&["ru", "rus"], "Russian"),
    (&["uk", "ukr"], "Ukrainian"),
    (&["he", "heb"], "Hebrew"),
    (&["ar", "ara"], "Arabic"),
    (&["hi", "hin"], "Hindi"),
    (&["th", "tha"], "Thai"),
    (&["vi", "vie"], "Vietnamese"),
    (&["id", "ind"], "Indonesian"),
    (&["ms", "may", "msa"], "Malay"),
    (&["ja", "jpn"], "Japanese"),
    (&["ko", "kor"], "Korean"),
    (&["zh", "chi", "zho"], "Chinese"),
    (&["ca", "cat"], "Catalan"),
    (&["et", "est"], "Estonian"),
    (&["lv", "lav"], "Latvian"),
    (&["lt", "lit"], "Lithuanian"),
    (&["mk", "mac", "mkd"], "Macedonian"),
];
const ALIASES: &[(&str, &str)] = &[
    ("bokmal", "no"),
    ("bokmål", "no"),
    ("nynorsk", "no"),
    ("brazilian", "pt"),
];
const MOST_LANGUAGES: usize = 8;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub shown: bool,
    pub languages: Vec<String>,
    pub size: f64,
    pub position: f64,
    pub color: String,
    pub outline: f64,
    pub outline_color: String,
    pub shadow: f64,
    pub background: f64,
    pub restyle: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            shown: false,
            languages: vec!["en".to_string(), "de".to_string()],
            size: 1.0,
            position: 100.0,
            color: "#FFFFFF".to_string(),
            outline: 1.65,
            outline_color: "#000000".to_string(),
            shadow: 0.0,
            background: 0.0,
            restyle: false,
        }
    }
}

impl Settings {
    fn checked(mut self) -> Result<Self, String> {
        let color = |name: &str, value: &str| -> Result<String, String> {
            let hex = value.strip_prefix('#').unwrap_or(value);
            if hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                Ok(format!("#{}", hex.to_ascii_uppercase()))
            } else {
                Err(format!("{name} must be a colour like #FFFFFF, not {value}"))
            }
        };
        self.color = color("color", &self.color)?;
        self.outline_color = color("outline_color", &self.outline_color)?;
        let mut languages: Vec<String> = Vec::new();
        for language in &self.languages {
            let code = language.trim().to_ascii_lowercase();
            if !(2..=3).contains(&code.len()) || !code.bytes().all(|byte| byte.is_ascii_lowercase()) {
                return Err(format!("{language} is not a language code like en or ger"));
            }
            if !languages.contains(&code) {
                languages.push(code);
            }
        }
        languages.truncate(MOST_LANGUAGES);
        self.languages = languages;
        let number = |name: &str, value: f64, low: f64, high: f64| -> Result<f64, String> {
            if value.is_finite() {
                Ok(value.clamp(low, high))
            } else {
                Err(format!("{name} must be a number"))
            }
        };
        self.size = number("size", self.size, 0.5, 2.5)?;
        self.position = number("position", self.position, 50.0, 100.0)?;
        self.outline = number("outline", self.outline, 0.0, 6.0)?;
        self.shadow = number("shadow", self.shadow, 0.0, 6.0)?;
        self.background = number("background", self.background, 0.0, 1.0)?;
        Ok(self)
    }

    pub fn style(&self) -> Vec<(&'static str, Value)> {
        let (border, back) = if self.background > 0.0 {
            let alpha = (self.background * 255.0).round() as u8;
            ("background-box", format!("#{alpha:02X}000000"))
        } else {
            ("outline-and-shadow", "#C0000000".to_string())
        };
        vec![
            ("sub-scale", json!(self.size)),
            ("sub-pos", json!(self.position)),
            ("sub-color", json!(self.color)),
            ("sub-outline-size", json!(self.outline)),
            ("sub-outline-color", json!(self.outline_color)),
            ("sub-shadow-offset", json!(self.shadow)),
            ("sub-border-style", json!(border)),
            ("sub-back-color", json!(back)),
            ("sub-ass-override", json!(if self.restyle { "force" } else { "scale" })),
        ]
    }

    pub fn mpv_args(&self) -> Vec<String> {
        let mut args = vec![
            format!("--slang={}", self.languages.join(",")),
            format!("--sub-visibility={}", if self.shown { "yes" } else { "no" }),
            "--sub-auto=no".to_string(),
        ];
        for (name, value) in self.style() {
            let value = match value {
                Value::String(text) => text,
                other => other.to_string(),
            };
            args.push(format!("--{name}={value}"));
        }
        args
    }

    fn merged(&self, changes: Map<String, Value>) -> Result<Self, String> {
        let mut current = match serde_json::to_value(self) {
            Ok(Value::Object(fields)) => fields,
            _ => Map::new(),
        };
        current.extend(changes);
        serde_json::from_value::<Self>(Value::Object(current))
            .map_err(|error| error.to_string())?
            .checked()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: i64,
    pub language: Option<String>,
    pub title: Option<String>,
    pub label: String,
    pub forced: bool,
    pub hearing_impaired: bool,
    pub external: bool,
}

fn primary(code: &str) -> String {
    code.split(['-', '_']).next().unwrap_or_default().to_ascii_lowercase()
}

pub fn language_name(code: &str) -> Option<&'static str> {
    let code = primary(code);
    LANGUAGES
        .iter()
        .find(|(codes, _)| codes.contains(&code.as_str()))
        .map(|(_, name)| *name)
}

pub fn bibliographic(code: &str) -> Option<&'static str> {
    let code = primary(code);
    LANGUAGES
        .iter()
        .find(|(codes, _)| codes.contains(&code.as_str()))
        .and_then(|(codes, _)| codes.iter().find(|code| code.len() == 3).copied())
}

pub fn language_code(word: &str) -> Option<&'static str> {
    let word = word.trim().to_lowercase();
    if let Some((_, code)) = ALIASES.iter().find(|(alias, _)| *alias == word) {
        return Some(code);
    }
    LANGUAGES
        .iter()
        .find(|(codes, name)| codes.contains(&word.as_str()) || name.to_lowercase() == word)
        .map(|(codes, _)| codes[0])
}

fn same_language(code: &str, wanted: &str) -> bool {
    primary(code) == primary(wanted) || language_name(code).is_some_and(|name| language_name(wanted) == Some(name))
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Playing {
    pub visible: bool,
    pub delay: f64,
    pub track: Option<i64>,
    pub tracks: Vec<Track>,
    pub audio: Option<i64>,
    pub audio_tracks: Vec<Track>,
    #[serde(skip)]
    last_audio: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Change {
    Restyle(Settings),
    Toggle,
    Track(Option<i64>),
    Audio(i64),
    Delay(f64),
    Add { video: PathBuf, sidecar: Sidecar },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Remembered {
    pub delay: f64,
    pub embedded: Option<usize>,
}

impl Remembered {
    pub fn is_empty(&self) -> bool {
        self.delay == 0.0 && self.embedded.is_none()
    }
}

#[derive(Default)]
struct Kept {
    owner: Option<ProfileId>,
    stored: HashMap<ProfileId, Settings>,
    files: BTreeMap<PathBuf, Remembered>,
    playing: Option<Playing>,
}

async fn stored(db: &Db, profile: ProfileId) -> Option<Settings> {
    db.subtitle_settings(profile)
        .await
        .unwrap_or_else(|error| {
            eprintln!("subtitles: the stored settings of profile {}: {error}", profile.0);
            None
        })
        .and_then(|settings| settings.checked().ok())
}

/// The core's side of subtitles: each profile's settings, what is
/// remembered about each file, and the tracks the tv last reported.
#[derive(Clone, Default)]
pub struct SubtitleSettings {
    store: Option<Db>,
    files: Option<mpsc::UnboundedSender<(PathBuf, Remembered)>>,
    kept: Arc<Mutex<Kept>>,
}

impl SubtitleSettings {
    pub async fn load(db: &Db, profile: ProfileId) -> Self {
        let mut everyone = HashMap::new();
        let profiles = db.profiles().await.unwrap_or_else(|error| {
            eprintln!("subtitles: the profiles: {error}");
            Vec::new()
        });
        for id in profiles.iter().map(|profile| profile.id).chain([profile]) {
            if let Some(settings) = stored(db, id).await {
                everyone.insert(id, settings);
            }
        }
        let files = db.subtitle_files().await.unwrap_or_else(|error| {
            eprintln!("subtitles: the stored delays: {error}");
            BTreeMap::new()
        });
        let (changed, mut changes) = mpsc::unbounded_channel::<(PathBuf, Remembered)>();
        let keeping = db.clone();
        tokio::spawn(async move {
            while let Some((file, remembered)) = changes.recv().await {
                if let Err(error) = keeping.save_subtitle_file(&file, remembered).await {
                    eprintln!("subtitles: {}: {error}", file.display());
                }
            }
        });
        Self {
            store: Some(db.clone()),
            files: Some(changed),
            kept: Arc::new(Mutex::new(Kept {
                owner: Some(profile),
                stored: everyone,
                files,
                playing: None,
            })),
        }
    }

    pub fn owner(&self) -> ProfileId {
        self.kept.lock().unwrap().owner.unwrap_or(ProfileId::FIRST)
    }

    pub fn set_owner(&self, profile: ProfileId) {
        self.kept.lock().unwrap().owner = Some(profile);
    }

    pub fn settings_of(&self, profile: ProfileId) -> Settings {
        self.kept
            .lock()
            .unwrap()
            .stored
            .get(&profile)
            .cloned()
            .unwrap_or_default()
    }

    pub fn remembered(&self, file: &Path) -> Remembered {
        self.kept.lock().unwrap().files.get(file).copied().unwrap_or_default()
    }

    pub fn embedded(&self, file: &Path) -> Option<usize> {
        self.remembered(file).embedded
    }

    /// What the tv needs to show `file`'s subtitles the way `owner` likes them.
    pub fn for_play(&self, owner: ProfileId, file: &Path, sidecars: Vec<Sidecar>) -> PlaySubtitles {
        PlaySubtitles {
            settings: self.settings_of(owner),
            remembered: self.remembered(file),
            sidecars,
        }
    }

    /// Keeps what the tv found out about `file`, in memory and in the database.
    pub fn remember(&self, file: PathBuf, remembered: Remembered) {
        {
            let mut kept = self.kept.lock().unwrap();
            if kept.files.get(&file).copied().unwrap_or_default() == remembered {
                return;
            }
            if remembered.is_empty() {
                kept.files.remove(&file);
            } else {
                kept.files.insert(file.clone(), remembered);
            }
        }
        if let Some(files) = &self.files {
            let _ = files.send((file, remembered));
        }
    }

    /// The tracks of what plays, as the tv last reported them.
    pub fn publish(&self, playing: Option<Playing>) {
        self.kept.lock().unwrap().playing = playing;
    }

    pub fn playing(&self) -> Option<Playing> {
        self.kept.lock().unwrap().playing.clone()
    }

    async fn save(&self, profile: ProfileId, settings: Settings) -> db::Result<()> {
        if let Some(db) = &self.store {
            db.save_subtitle_settings(profile, &settings).await?;
        }
        self.kept.lock().unwrap().stored.insert(profile, settings);
        Ok(())
    }
}

#[derive(Serialize)]
struct View {
    settings: Settings,
    playing: Option<Playing>,
}

#[derive(Clone)]
struct Endpoint {
    subtitles: SubtitleSettings,
    core: Core,
}

pub fn routes<S: Clone + Send + Sync + 'static>(core: Core) -> Router<S> {
    let subtitles = core.subtitles().clone();
    Router::new()
        .route("/api/subtitles", get(show).put(update))
        .route("/api/subtitles/track", post(track))
        .route("/api/subtitles/toggle", post(toggle))
        .route("/api/subtitles/delay", post(delay))
        .route("/api/audio/track", post(audio))
        .with_state(Endpoint { subtitles, core })
}

fn view(subtitles: &SubtitleSettings, profile: ProfileId) -> Json<View> {
    Json(View {
        settings: subtitles.settings_of(profile),
        playing: subtitles.playing(),
    })
}

async fn show(State(endpoint): State<Endpoint>, Who(profile): Who) -> Json<View> {
    view(&endpoint.subtitles, profile)
}

async fn update(
    State(endpoint): State<Endpoint>,
    Who(profile): Who,
    Json(changes): Json<Map<String, Value>>,
) -> Response {
    let settings = match endpoint.subtitles.settings_of(profile).merged(changes) {
        Ok(settings) => settings,
        Err(error) => return api::problem(StatusCode::BAD_REQUEST, error),
    };
    if let Err(error) = endpoint.subtitles.save(profile, settings.clone()).await {
        return api::problem(StatusCode::INTERNAL_SERVER_ERROR, error.to_string());
    }
    if endpoint.subtitles.owner() == profile {
        endpoint.core.send(Request::Subtitles(Change::Restyle(settings)));
    }
    view(&endpoint.subtitles, profile).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackBody {
    track: Option<i64>,
}

async fn track(State(endpoint): State<Endpoint>, Json(body): Json<TrackBody>) -> Response {
    api::ask(&endpoint.core, Request::Subtitles(Change::Track(body.track))).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioBody {
    track: i64,
}

async fn audio(State(endpoint): State<Endpoint>, Json(body): Json<AudioBody>) -> Response {
    api::ask(&endpoint.core, Request::Subtitles(Change::Audio(body.track))).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DelayBody {
    delay: f64,
}

async fn delay(State(endpoint): State<Endpoint>, Json(body): Json<DelayBody>) -> Response {
    api::ask(&endpoint.core, Request::Subtitles(Change::Delay(body.delay))).await
}

async fn toggle(State(endpoint): State<Endpoint>) -> Response {
    api::ask(&endpoint.core, Request::Subtitles(Change::Toggle)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Outcome;
    use crate::library::Library;
    use std::sync::mpsc;
    use std::time::Duration;
    use tokio::net::TcpListener;

    #[test]
    fn settings_become_mpv_options() {
        let settings = Settings {
            shown: true,
            languages: vec!["de".to_string(), "en".to_string()],
            size: 1.25,
            background: 0.5,
            restyle: true,
            ..Settings::default()
        };
        let args = settings.mpv_args();
        for expected in [
            "--slang=de,en",
            "--sub-visibility=yes",
            "--sub-scale=1.25",
            "--sub-pos=100.0",
            "--sub-color=#FFFFFF",
            "--sub-border-style=background-box",
            "--sub-back-color=#80000000",
            "--sub-ass-override=force",
        ] {
            assert!(args.iter().any(|arg| arg == expected), "{expected} in {args:?}");
        }
        let plain = Settings::default().mpv_args();
        assert!(plain.contains(&"--sub-visibility=no".to_string()));
        assert!(plain.contains(&"--sub-border-style=outline-and-shadow".to_string()));
        assert!(plain.contains(&"--sub-ass-override=scale".to_string()));
    }

    #[test]
    fn changes_are_merged_checked_and_clamped() {
        let changes = |value: Value| value.as_object().unwrap().clone();
        let settings = Settings::default()
            .merged(changes(
                json!({ "size": 9.0, "color": "ffcc00", "languages": ["DE", "en", "de"] }),
            ))
            .unwrap();
        assert_eq!(settings.size, 2.5);
        assert_eq!(settings.color, "#FFCC00");
        assert_eq!(settings.languages, ["de", "en"]);
        assert_eq!(settings.position, 100.0);
        let bad = |value: Value| Settings::default().merged(changes(value)).is_err();
        assert!(bad(json!({ "color": "red" })));
        assert!(bad(json!({ "languages": ["english"] })));
        assert!(bad(json!({ "size": "big" })));
        assert!(bad(json!({ "font": "Comic Sans" })));
    }

    #[tokio::test]
    async fn settings_survive_a_restart_and_belong_to_one_profile() {
        let db = Db::memory().await;
        let subtitles = SubtitleSettings::load(&db, ProfileId::FIRST).await;
        assert_eq!(subtitles.settings_of(ProfileId::FIRST), Settings::default());
        let bigger = Settings {
            size: 1.5,
            ..Settings::default()
        };
        subtitles.save(ProfileId::FIRST, bigger.clone()).await.unwrap();
        let again = SubtitleSettings::load(&db, ProfileId::FIRST).await;
        assert_eq!(again.settings_of(ProfileId::FIRST), bigger);
        assert_eq!(again.settings_of(ProfileId(2)), Settings::default());
    }

    #[tokio::test]
    async fn a_play_carries_its_owners_settings_and_the_files_memory() {
        let db = Db::memory().await;
        let subtitles = SubtitleSettings::load(&db, ProfileId::FIRST).await;
        let big = Settings {
            size: 2.0,
            ..Settings::default()
        };
        subtitles.save(ProfileId(2), big.clone()).await.unwrap();
        let film = Path::new("/lib/Movies/Up/Up.mkv");
        let delayed = Remembered {
            delay: -1.0,
            embedded: Some(2),
        };
        subtitles.remember(film.to_path_buf(), delayed);
        let sidecars = vec![Sidecar {
            path: PathBuf::from("/lib/Movies/Up/Up.en.srt"),
            language: Some("en".to_string()),
            title: "English".to_string(),
            forced: false,
            hearing_impaired: false,
        }];
        assert_eq!(
            subtitles.for_play(ProfileId(2), film, sidecars.clone()),
            PlaySubtitles {
                settings: big,
                remembered: delayed,
                sidecars,
            }
        );
        assert_eq!(
            subtitles.for_play(ProfileId::FIRST, Path::new("/lib/other.mkv"), Vec::new()),
            PlaySubtitles::default()
        );
    }

    async fn kept(db: &Db, film: &Path, wanted: Option<Remembered>) -> bool {
        for _ in 0..500 {
            if db.subtitle_files().await.unwrap().get(film).copied() == wanted {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }

    #[tokio::test]
    async fn what_the_tv_remembers_about_a_file_survives_a_restart_until_it_is_undone() {
        let db = Db::memory().await;
        db.set_library(&crate::db::test_library());
        let film = Path::new("/lib/Movies/Up/Up.mkv");
        let subtitles = SubtitleSettings::load(&db, ProfileId::FIRST).await;
        let delayed = Remembered {
            delay: -1.0,
            embedded: None,
        };
        subtitles.remember(film.to_path_buf(), delayed);
        assert!(kept(&db, film, Some(delayed)).await);
        let again = SubtitleSettings::load(&db, ProfileId::FIRST).await;
        assert_eq!(again.remembered(film), delayed);
        again.remember(film.to_path_buf(), Remembered::default());
        assert_eq!(again.remembered(film), Remembered::default());
        assert!(kept(&db, film, None).await);
    }

    struct Server {
        base: String,
        asked: mpsc::Receiver<String>,
        runtime: tokio::runtime::Runtime,
        db: Db,
    }

    fn endless_summer() -> Playing {
        let list = json!([
            { "id": 1, "type": "video", "title": "The.Endless.Summer" },
            { "id": 1, "type": "audio", "lang": "en" },
            { "id": 1, "type": "sub", "lang": "en", "title": "English SRT" },
            { "id": 2, "type": "sub", "lang": "en", "title": "English SRT-SDH", "hearing-impaired": true },
            { "id": 3, "type": "sub", "lang": "fr", "title": "French SRT" },
        ]);
        let mut tracks = SubtitleTracks::new(Path::new("/lib/Movies/Up/Up.mkv"), PlaySubtitles::default());
        tracks.observe(Observed::parse(&json!({ "name": "track-list", "data": list })).unwrap());
        tracks.playing().unwrap()
    }

    fn serve() -> Server {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (core, asked) = runtime.block_on(async {
            let (core, inbox) = Core::detached(Library::default()).await;
            let asked = Core::answering(inbox, |_| Outcome::done("Subtitles: English"));
            (core, asked)
        });
        core.subtitles().publish(Some(endless_summer()));
        let listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let db = core.db().clone();
        let router: Router = routes(core);
        runtime.spawn(async move { axum::serve(listener, router).await });
        Server {
            base,
            asked,
            runtime,
            db,
        }
    }

    fn agent() -> ureq::Agent {
        ureq::Agent::config_builder().http_status_as_error(false).build().into()
    }

    fn send(server: &Server, method: &str, path: &str, body: Value) -> (u16, Value) {
        let url = format!("{}{path}", server.base);
        let mut response = match method {
            "GET" => agent().get(url).call(),
            "PUT" => agent().put(url).send_json(body),
            _ => agent().post(url).send_json(body),
        }
        .unwrap();
        let status = response.status().as_u16();
        (status, response.body_mut().read_json().unwrap_or(Value::Null))
    }

    #[test]
    fn the_api_shows_settings_and_the_tracks_the_tv_last_reported() {
        let server = serve();
        let (status, body) = send(&server, "GET", "/api/subtitles", Value::Null);
        assert_eq!(status, 200);
        assert_eq!(body["settings"]["languages"], json!(["en", "de"]));
        assert_eq!(body["settings"]["size"], 1.0);
        assert_eq!(body["playing"]["visible"], false);
        assert_eq!(body["playing"]["track"], Value::Null);
        assert_eq!(body["playing"]["tracks"][2]["label"], "French");
        assert_eq!(body["playing"]["tracks"][1]["hearing_impaired"], true);
        assert_eq!(body["playing"]["audio_tracks"][0]["label"], "English");
        assert_eq!(body["playing"]["audio"], Value::Null);
    }

    #[test]
    fn a_put_saves_the_change_and_restyles_what_plays_with_it() {
        let server = serve();
        let (status, body) = send(&server, "PUT", "/api/subtitles", json!({ "size": 1.4, "shown": true }));
        assert_eq!(status, 200);
        assert_eq!(body["settings"]["size"], 1.4);
        assert_eq!(body["settings"]["shown"], true);
        assert_eq!(body["settings"]["color"], "#FFFFFF");
        let asked = server.asked.recv().unwrap();
        assert!(
            asked.starts_with("Subtitles(Restyle(Settings { shown: true,"),
            "{asked}"
        );
        assert!(asked.contains("size: 1.4,"), "{asked}");
        let saved = server.runtime.block_on(server.db.subtitle_settings(ProfileId::FIRST));
        let saved = saved.unwrap().unwrap();
        assert_eq!(saved.size, 1.4);
        let (status, body) = send(&server, "PUT", "/api/subtitles", json!({ "color": "pink" }));
        assert_eq!(status, 400);
        assert!(body["error"].as_str().unwrap().contains("colour"));
        assert_eq!(send(&server, "PUT", "/api/subtitles", json!({ "size": "big" })).0, 400);
    }

    #[test]
    fn tracks_and_the_toggle_go_to_the_tv() {
        let server = serve();
        let (status, body) = send(&server, "POST", "/api/subtitles/track", json!({ "track": 2 }));
        assert_eq!((status, body["ok"].clone()), (200, json!(true)));
        assert_eq!(server.asked.recv().unwrap(), "Subtitles(Track(Some(2)))");
        send(&server, "POST", "/api/subtitles/track", json!({ "track": null }));
        assert_eq!(server.asked.recv().unwrap(), "Subtitles(Track(None))");
        send(&server, "POST", "/api/subtitles/toggle", json!({}));
        assert_eq!(server.asked.recv().unwrap(), "Subtitles(Toggle)");
        let (status, _) = send(&server, "POST", "/api/subtitles/track", json!({ "id": 2 }));
        assert_eq!(status, 422);
        let (status, _) = send(&server, "POST", "/api/audio/track", json!({ "track": 2 }));
        assert_eq!(status, 200);
        assert_eq!(server.asked.recv().unwrap(), "Subtitles(Audio(2))");
    }

    #[test]
    fn language_names_and_codes_find_their_code() {
        assert_eq!(language_code("English"), Some("en"));
        assert_eq!(language_code("eng"), Some("en"));
        assert_eq!(language_code("Bokmal"), Some("no"));
        assert_eq!(language_code("mac"), Some("mk"));
        assert_eq!(language_code("Klingon"), None);
        assert_eq!(bibliographic("en"), Some("eng"));
        assert_eq!(bibliographic("de-DE"), Some("ger"));
        assert_eq!(bibliographic("no"), Some("nor"));
    }
}
