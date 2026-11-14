use crate::api::{self, Ask, Pending, Request};
use crate::db::{self, Db};
use crate::profile::ProfileId;
use crate::progress::Tracks;
use crate::sidecar::Sidecar;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

pub const OBSERVED: [&str; 5] = ["track-list", "sid", "sub-visibility", "aid", "sub-delay"];
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
const NO_SUBTITLES: &str = "No subtitles in this file";
const OFF: &str = "Subtitles off";
const MOST_DELAY: f64 = 60.0;

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

#[derive(Clone, Debug, PartialEq, Serialize)]
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

impl Track {
    fn parse(track: &Value, kind: &str) -> Option<Self> {
        if track["type"] != kind {
            return None;
        }
        let id = track["id"].as_i64()?;
        let text = |key: &str| {
            track[key]
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        };
        let language = text("lang");
        let title = text("title");
        let flag = |key: &str| track[key].as_bool().unwrap_or(false);
        let titled = |word: &str| {
            title
                .as_deref()
                .is_some_and(|title| title.split_whitespace().any(|part| part.eq_ignore_ascii_case(word)))
        };
        let name = language.as_deref().and_then(language_name).map(str::to_string);
        let label = name
            .or_else(|| title.clone())
            .or_else(|| language.clone())
            .unwrap_or_else(|| format!("Track {id}"));
        let forced = flag("forced") || titled("forced");
        let hearing_impaired = flag("hearing-impaired") || titled("SDH");
        Some(Self {
            id,
            language,
            title,
            label,
            forced,
            hearing_impaired,
            external: flag("external"),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
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

impl Playing {
    fn label(&self, id: i64) -> String {
        self.tracks
            .iter()
            .find(|track| track.id == id)
            .map_or_else(|| format!("track {id}"), |track| track.label.clone())
    }

    fn preferred(&self, languages: &[String]) -> Option<i64> {
        let full = || self.tracks.iter().filter(|track| !track.forced);
        languages
            .iter()
            .find_map(|wanted| {
                let speaking = || {
                    full().filter(|track| {
                        track
                            .language
                            .as_deref()
                            .is_some_and(|code| same_language(code, wanted))
                    })
                };
                speaking()
                    .find(|track| !track.hearing_impaired)
                    .or_else(|| speaking().next())
            })
            .or_else(|| full().next())
            .or_else(|| self.tracks.first())
            .map(|track| track.id)
    }

    fn show(&self, id: i64) -> (Vec<Value>, String) {
        let mut commands = Vec::new();
        if self.track != Some(id) {
            commands.push(json!(["set_property", "sid", id]));
        }
        commands.push(json!(["set_property", "sub-visibility", true]));
        (commands, format!("Subtitles: {}", self.label(id)))
    }

    fn hide() -> (Vec<Value>, String) {
        (vec![json!(["set_property", "sub-visibility", false])], OFF.to_string())
    }

    fn toggle(&self, languages: &[String]) -> Answer {
        match (self.visible, self.track) {
            _ if self.tracks.is_empty() => Answer::refused(NO_SUBTITLES.to_string()),
            (true, Some(_)) => Answer::done(Self::hide()),
            (_, Some(id)) => Answer::done(self.show(id)),
            (_, None) => Answer::done(self.preferred(languages).map_or_else(Self::hide, |id| self.show(id))),
        }
    }

    fn choose_audio(&self, id: i64) -> Result<(Vec<Value>, String), String> {
        let track = self
            .audio_tracks
            .iter()
            .find(|track| track.id == id)
            .ok_or_else(|| format!("There is no audio track {id}"))?;
        Ok((
            vec![json!(["set_property", "aid", id])],
            format!("Audio: {}", track.label),
        ))
    }

    fn choose(&self, track: Option<i64>) -> Result<(Vec<Value>, String), String> {
        match track {
            None => Ok(Self::hide()),
            Some(id) if self.tracks.iter().any(|track| track.id == id) => Ok(self.show(id)),
            Some(id) => Err(format!("There is no subtitle track {id}")),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Observed {
    Tracks { subtitles: Vec<Track>, audio: Vec<Track> },
    Track(Option<i64>),
    Audio(Option<i64>),
    Visible(bool),
    Delay(f64),
}

impl Observed {
    pub fn parse(message: &Value) -> Option<Self> {
        let data = &message["data"];
        match message["name"].as_str()? {
            "track-list" => {
                let list = data.as_array()?;
                let of = |kind: &str| list.iter().filter_map(|track| Track::parse(track, kind)).collect();
                Some(Self::Tracks {
                    subtitles: of("sub"),
                    audio: of("audio"),
                })
            }
            "sid" => Some(Self::Track(data.as_i64())),
            "aid" => Some(Self::Audio(data.as_i64())),
            "sub-visibility" => data.as_bool().map(Self::Visible),
            "sub-delay" => data.as_f64().map(Self::Delay),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Answer {
    pub commands: Vec<Value>,
    pub message: String,
    pub ok: bool,
}

impl Answer {
    fn done((commands, message): (Vec<Value>, String)) -> Self {
        Self {
            commands,
            message,
            ok: true,
        }
    }

    fn refused(message: String) -> Self {
        Self {
            commands: Vec::new(),
            message,
            ok: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Restyle,
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
struct Inner {
    settings: Settings,
    playing: Option<Playing>,
    file: Option<PathBuf>,
    sidecars: Vec<Sidecar>,
    awaited: usize,
    files: BTreeMap<PathBuf, Remembered>,
}

impl Inner {
    fn remember(&mut self, change: impl FnOnce(&mut Remembered)) -> Option<(PathBuf, Remembered)> {
        let file = self.file.clone()?;
        let remembered = self.files.entry(file.clone()).or_default();
        let before = *remembered;
        change(remembered);
        let after = *remembered;
        if after.is_empty() {
            self.files.remove(&file);
        }
        (after != before).then_some((file, after))
    }
}

fn delay_label(delay: f64) -> String {
    if delay == 0.0 {
        "Subtitle delay off".to_string()
    } else {
        format!("Subtitle delay {delay:+.1} s")
    }
}

#[derive(Clone, Default)]
pub struct Subtitles {
    store: Option<(Db, ProfileId)>,
    state: Arc<Mutex<Inner>>,
}

impl Subtitles {
    pub fn load(db: &Db, profile: ProfileId) -> Self {
        let settings = db
            .subtitle_settings(profile)
            .unwrap_or_else(|error| {
                eprintln!("subtitles: the stored settings: {error}");
                None
            })
            .and_then(|settings| settings.checked().ok())
            .unwrap_or_default();
        let files = db.subtitle_files().unwrap_or_else(|error| {
            eprintln!("subtitles: the stored delays: {error}");
            BTreeMap::new()
        });
        Self {
            store: Some((db.clone(), profile)),
            state: Arc::new(Mutex::new(Inner {
                settings,
                files,
                ..Inner::default()
            })),
        }
    }

    pub fn settings(&self) -> Settings {
        self.state.lock().unwrap().settings.clone()
    }

    pub fn mpv_args(&self, file: &Path) -> Vec<String> {
        let state = self.state.lock().unwrap();
        let mut args = state.settings.mpv_args();
        let delay = state.files.get(file).map_or(0.0, |remembered| remembered.delay);
        args.push(format!("--sub-delay={delay}"));
        args
    }

    pub fn file_delay(&self, file: &Path) -> f64 {
        self.state.lock().unwrap().files.get(file).map_or(0.0, |remembered| remembered.delay)
    }

    pub fn plays(&self, file: &Path) -> bool {
        self.state.lock().unwrap().file.as_deref() == Some(file)
    }

    pub fn embedded(&self, file: &Path) -> Option<usize> {
        self.state
            .lock()
            .unwrap()
            .files
            .get(file)
            .and_then(|remembered| remembered.embedded)
    }

    fn playing(&self) -> Option<Playing> {
        self.state.lock().unwrap().playing.clone()
    }

    pub fn chosen(&self) -> (Option<i64>, Option<i64>) {
        self.playing().map_or((None, None), |playing| {
            (playing.track.filter(|_| playing.visible), playing.audio)
        })
    }

    pub fn audio_to_restore(&self) -> Option<i64> {
        self.playing().and_then(|playing| {
            playing
                .last_audio
                .or_else(|| playing.audio_tracks.first().map(|track| track.id))
        })
    }

    pub fn delay(&self) -> f64 {
        self.playing().map_or(0.0, |playing| playing.delay)
    }

    fn save(&self, settings: Settings) -> db::Result<()> {
        if let Some((db, profile)) = &self.store {
            db.save_subtitle_settings(*profile, &settings)?;
        }
        self.state.lock().unwrap().settings = settings;
        Ok(())
    }

    fn save_file(&self, (file, remembered): (PathBuf, Remembered)) {
        if let Some((db, _)) = &self.store
            && let Err(error) = db.save_subtitle_file(&file, remembered)
        {
            eprintln!("subtitles: {}: {error}", file.display());
        }
    }

    pub fn tracks(&self) -> Option<Tracks> {
        self.playing().map(|playing| Tracks {
            audio: playing.audio,
            subtitle: playing.track,
            subtitles_shown: Some(playing.visible),
        })
    }

    pub fn started(&self, file: &Path, sidecars: Vec<Sidecar>) {
        let mut state = self.state.lock().unwrap();
        state.playing = Some(Playing::default());
        state.file = Some(file.to_path_buf());
        state.awaited = sidecars.len();
        state.sidecars = sidecars;
    }

    pub fn stopped(&self) {
        let mut state = self.state.lock().unwrap();
        state.playing = None;
        state.file = None;
        state.sidecars.clear();
        state.awaited = 0;
    }

    pub fn loaded(&self) -> Vec<Value> {
        self.state
            .lock()
            .unwrap()
            .sidecars
            .iter()
            .map(|sidecar| {
                json!([
                    "sub-add",
                    sidecar.path,
                    "auto",
                    sidecar.title,
                    sidecar.language.as_deref().unwrap_or_default()
                ])
            })
            .collect()
    }

    pub fn observe(&self, observed: Observed) -> Vec<Value> {
        let mut state = self.state.lock().unwrap();
        let mut commands = Vec::new();
        let mut remembered = None;
        let shown = state.settings.shown;
        let languages = state.settings.languages.clone();
        let awaited = state.awaited;
        let playing = state.playing.get_or_insert_with(Playing::default);
        match observed {
            Observed::Tracks { subtitles, audio } => {
                playing.tracks = subtitles;
                playing.audio_tracks = audio;
                let external = playing.tracks.iter().filter(|track| track.external).count();
                let embedded = playing.tracks.len() - external;
                let settled = awaited > 0 && external >= awaited;
                if settled
                    && shown
                    && let Some(id) = playing.preferred(&languages)
                {
                    commands = playing.show(id).0;
                }
                if settled {
                    state.awaited = 0;
                }
                remembered = state.remember(|remembered| remembered.embedded = Some(embedded));
            }
            Observed::Track(track) => playing.track = track,
            Observed::Audio(track) => {
                playing.audio = track;
                playing.last_audio = track.or(playing.last_audio);
            }
            Observed::Visible(visible) => playing.visible = visible,
            Observed::Delay(delay) => playing.delay = delay,
        }
        drop(state);
        if let Some(remembered) = remembered {
            self.save_file(remembered);
        }
        commands
    }

    pub fn change(&self, change: Change) -> Answer {
        let mut state = self.state.lock().unwrap();
        let playing = state.playing.clone().unwrap_or_default();
        match change {
            Change::Restyle => {
                let commands = state
                    .settings
                    .style()
                    .into_iter()
                    .map(|(name, value)| json!(["set_property", name, value]))
                    .collect();
                Answer::done((commands, "Subtitle style changed".to_string()))
            }
            Change::Toggle => playing.toggle(&state.settings.languages),
            Change::Track(track) => playing.choose(track).map_or_else(Answer::refused, Answer::done),
            Change::Audio(track) => playing.choose_audio(track).map_or_else(Answer::refused, Answer::done),
            Change::Delay(delay) => {
                if state.file.is_none() {
                    return Answer::refused("Nothing is playing".to_string());
                }
                if !delay.is_finite() {
                    return Answer::refused("The delay must be a number of seconds".to_string());
                }
                let delay = ((delay * 10.0).round() / 10.0).clamp(-MOST_DELAY, MOST_DELAY);
                if let Some(remembered) = state.remember(|remembered| remembered.delay = delay) {
                    drop(state);
                    self.save_file(remembered);
                }
                Answer::done((vec![json!(["set_property", "sub-delay", delay])], delay_label(delay)))
            }
            Change::Add { video, sidecar } => {
                if state.file.as_deref() != Some(video.as_path()) {
                    return Answer::refused("That is not what plays now".to_string());
                }
                state.sidecars.push(sidecar.clone());
                let commands = vec![
                    json!([
                        "sub-add",
                        sidecar.path,
                        "select",
                        sidecar.title,
                        sidecar.language.as_deref().unwrap_or_default()
                    ]),
                    json!(["set_property", "sub-visibility", true]),
                ];
                Answer::done((commands, format!("Subtitles: {}", sidecar.title)))
            }
        }
    }
}

#[derive(Serialize)]
struct View {
    settings: Settings,
    playing: Option<Playing>,
}

#[derive(Clone)]
struct Endpoint {
    subtitles: Subtitles,
    app: Ask,
}

pub fn routes<S: Clone + Send + Sync + 'static>(subtitles: Subtitles, app: Ask) -> Router<S> {
    Router::new()
        .route("/api/subtitles", get(show).put(update))
        .route("/api/subtitles/track", post(track))
        .route("/api/subtitles/toggle", post(toggle))
        .route("/api/subtitles/delay", post(delay))
        .route("/api/audio/track", post(audio))
        .with_state(Endpoint { subtitles, app })
}

fn view(subtitles: &Subtitles) -> Json<View> {
    Json(View {
        settings: subtitles.settings(),
        playing: subtitles.playing(),
    })
}

async fn show(State(endpoint): State<Endpoint>) -> Json<View> {
    view(&endpoint.subtitles)
}

async fn update(State(endpoint): State<Endpoint>, Json(changes): Json<Map<String, Value>>) -> Response {
    let settings = match endpoint.subtitles.settings().merged(changes) {
        Ok(settings) => settings,
        Err(error) => return api::problem(StatusCode::BAD_REQUEST, error),
    };
    let saving = endpoint.subtitles.clone();
    let saved = tokio::task::spawn_blocking(move || saving.save(settings)).await;
    match saved {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return api::problem(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
        Err(error) => return api::problem(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
    let (reply, _) = oneshot::channel();
    (endpoint.app)(Pending {
        request: Request::Subtitles(Change::Restyle),
        reply,
    });
    view(&endpoint.subtitles).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackBody {
    track: Option<i64>,
}

async fn track(State(endpoint): State<Endpoint>, Json(body): Json<TrackBody>) -> Response {
    api::ask_app(&endpoint.app, Request::Subtitles(Change::Track(body.track))).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioBody {
    track: i64,
}

async fn audio(State(endpoint): State<Endpoint>, Json(body): Json<AudioBody>) -> Response {
    api::ask_app(&endpoint.app, Request::Subtitles(Change::Audio(body.track))).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DelayBody {
    delay: f64,
}

async fn delay(State(endpoint): State<Endpoint>, Json(body): Json<DelayBody>) -> Response {
    api::ask_app(&endpoint.app, Request::Subtitles(Change::Delay(body.delay))).await
}

async fn toggle(State(endpoint): State<Endpoint>) -> Response {
    api::ask_app(&endpoint.app, Request::Subtitles(Change::Toggle)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Outcome;
    use std::sync::mpsc;
    use tokio::net::TcpListener;

    fn sub(id: i64, lang: &str, title: &str) -> Value {
        json!({ "id": id, "type": "sub", "lang": lang, "title": title, "selected": false })
    }

    fn endless_summer() -> Value {
        json!([
            { "id": 1, "type": "video", "title": "The.Endless.Summer" },
            { "id": 1, "type": "audio", "lang": "en" },
            sub(1, "en", "English SRT"),
            { "id": 2, "type": "sub", "lang": "en", "title": "English SRT-SDH", "hearing-impaired": true },
            sub(3, "fr", "French SRT"),
        ])
    }

    fn playing(tracks: Value, track: Option<i64>, visible: bool) -> Playing {
        let Some(Observed::Tracks { subtitles, audio }) =
            Observed::parse(&json!({ "name": "track-list", "data": tracks }))
        else {
            panic!("a track list");
        };
        Playing {
            visible,
            track,
            tracks: subtitles,
            audio_tracks: audio,
            ..Playing::default()
        }
    }

    #[test]
    fn only_subtitle_tracks_are_listed_and_named_after_their_language() {
        let playing = playing(endless_summer(), Some(1), true);
        let labels: Vec<_> = playing.tracks.iter().map(|track| track.label.as_str()).collect();
        assert_eq!(labels, ["English", "English", "French"]);
        assert!(playing.tracks[1].hearing_impaired);
        assert_eq!(playing.tracks[2].title.as_deref(), Some("French SRT"));
        let unnamed = Track::parse(&json!({ "id": 4, "type": "sub" }), "sub").unwrap();
        assert_eq!(unnamed.label, "Track 4");
        let titled = Track::parse(&json!({ "id": 5, "type": "sub", "lang": "xx", "title": "Commentary" }), "sub").unwrap();
        assert_eq!(titled.label, "Commentary");
        assert_eq!(
            Track::parse(&json!({ "id": 6, "type": "sub", "lang": "ger" }), "sub")
                .unwrap()
                .label,
            "German"
        );
    }

    #[test]
    fn region_tags_name_and_match_their_language() {
        let maul = json!([
            { "id": 1, "type": "sub", "lang": "en-US" },
            { "id": 2, "type": "sub", "lang": "en-US", "title": "SDH", "hearing-impaired": true },
            { "id": 3, "type": "sub", "lang": "zh-Hant-HK", "title": "Hong Kong" },
            { "id": 7, "type": "sub", "lang": "de-DE" },
            { "id": 10, "type": "sub", "lang": "es-419", "title": "Latin America" },
            { "id": 13, "type": "sub", "lang": "hu-HU" },
        ]);
        let playing = playing(maul, None, false);
        let labels: Vec<_> = playing.tracks.iter().map(|track| track.label.as_str()).collect();
        assert_eq!(
            labels,
            ["English", "English", "Chinese", "German", "Spanish", "Hungarian"]
        );
        assert_eq!(playing.preferred(&["de".to_string(), "en".to_string()]), Some(7));
        assert_eq!(playing.preferred(&["spa".to_string()]), Some(10));
        assert_eq!(playing.preferred(&["en".to_string()]), Some(1));
    }

    #[test]
    fn mpv_reports_the_track_and_whether_it_shows() {
        let sid = json!({ "event": "property-change", "name": "sid", "data": 2 });
        let off = json!({ "event": "property-change", "name": "sid", "data": false });
        let visible = json!({ "event": "property-change", "name": "sub-visibility", "data": false });
        assert_eq!(Observed::parse(&sid), Some(Observed::Track(Some(2))));
        assert_eq!(Observed::parse(&off), Some(Observed::Track(None)));
        assert_eq!(Observed::parse(&visible), Some(Observed::Visible(false)));
        assert_eq!(Observed::parse(&json!({ "name": "pause", "data": true })), None);
    }

    #[test]
    fn the_key_hides_shown_subtitles() {
        let Answer { commands, message, .. } = playing(endless_summer(), Some(1), true).toggle(&[]);
        assert_eq!(commands, [json!(["set_property", "sub-visibility", false])]);
        assert_eq!(message, "Subtitles off");
    }

    #[test]
    fn the_key_shows_the_selected_track_again() {
        let Answer { commands, message, .. } = playing(endless_summer(), Some(3), false).toggle(&["en".to_string()]);
        assert_eq!(commands, [json!(["set_property", "sub-visibility", true])]);
        assert_eq!(message, "Subtitles: French");
    }

    #[test]
    fn without_a_selected_track_the_key_picks_the_first_preferred_language() {
        let languages = ["de".to_string(), "fr".to_string(), "en".to_string()];
        let Answer { commands, message, .. } = playing(endless_summer(), None, false).toggle(&languages);
        assert_eq!(
            commands,
            [
                json!(["set_property", "sid", 3]),
                json!(["set_property", "sub-visibility", true])
            ]
        );
        assert_eq!(message, "Subtitles: French");
        let english = playing(endless_summer(), None, true).preferred(&["eng".to_string()]);
        assert_eq!(english, Some(1));
        let forced = playing(
            json!([{ "id": 1, "type": "sub", "forced": true }, sub(2, "it", "")]),
            None,
            false,
        );
        assert_eq!(forced.preferred(&["en".to_string()]), Some(2));
    }

    #[test]
    fn a_file_without_subtitles_says_so() {
        let tracks = json!([{ "id": 1, "type": "video" }, { "id": 1, "type": "audio" }]);
        let answer = playing(tracks, None, true).toggle(&["en".to_string()]);
        assert!(answer.commands.is_empty());
        assert!(!answer.ok);
        assert_eq!(answer.message, "No subtitles in this file");
    }

    #[test]
    fn a_chosen_track_must_exist() {
        let playing = playing(endless_summer(), Some(1), false);
        assert_eq!(playing.choose(Some(2)).unwrap().1, "Subtitles: English");
        assert_eq!(playing.choose(None).unwrap().1, "Subtitles off");
        assert!(playing.choose(Some(9)).is_err());
    }

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

    #[test]
    fn settings_survive_a_restart_and_belong_to_one_profile() {
        let db = Db::memory(Path::new("/lib"));
        let subtitles = Subtitles::load(&db, ProfileId::FIRST);
        assert_eq!(subtitles.settings(), Settings::default());
        let bigger = Settings {
            size: 1.5,
            ..Settings::default()
        };
        subtitles.save(bigger.clone()).unwrap();
        assert_eq!(Subtitles::load(&db, ProfileId::FIRST).settings(), bigger);
        assert_eq!(Subtitles::load(&db, ProfileId(2)).settings(), Settings::default());
    }

    #[test]
    fn a_restyle_sets_every_style_property() {
        let subtitles = Subtitles::default();
        let commands = subtitles.change(Change::Restyle).commands;
        assert_eq!(commands.len(), Settings::default().style().len());
        assert!(commands.contains(&json!(["set_property", "sub-scale", 1.0])));
    }

    #[test]
    fn the_state_follows_what_mpv_reports_until_playback_stops() {
        let subtitles = Subtitles::default();
        subtitles.started(Path::new("/lib/Movies/Up/Up.mkv"), Vec::new());
        let tracks = Observed::parse(&json!({ "name": "track-list", "data": endless_summer() })).unwrap();
        subtitles.observe(tracks);
        subtitles.observe(Observed::Track(Some(1)));
        subtitles.observe(Observed::Visible(true));
        assert_eq!(subtitles.change(Change::Toggle).message, "Subtitles off");
        assert_eq!(subtitles.chosen(), (Some(1), None));
        subtitles.observe(Observed::Visible(false));
        subtitles.observe(Observed::Audio(Some(1)));
        assert_eq!(subtitles.chosen(), (None, Some(1)));
        subtitles.stopped();
        assert_eq!(subtitles.playing(), None);
    }

    struct Server {
        base: String,
        asked: mpsc::Receiver<String>,
        _runtime: tokio::runtime::Runtime,
        db: Db,
    }

    fn serve() -> Server {
        let db = Db::memory(Path::new("/lib"));
        let subtitles = Subtitles::load(&db, ProfileId::FIRST);
        subtitles.started(Path::new("/lib/Movies/Up/Up.mkv"), Vec::new());
        subtitles.observe(Observed::parse(&json!({ "name": "track-list", "data": endless_summer() })).unwrap());
        let (asked, received) = mpsc::channel();
        let app: Ask = Arc::new(move |pending: Pending| {
            let _ = asked.send(format!("{:?}", pending.request));
            let _ = pending.reply.send(Outcome {
                message: "Subtitles: English".to_string(),
                ok: true,
            });
        });
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let router: Router = routes(subtitles, app);
        runtime.spawn(async move { axum::serve(listener, router).await });
        Server {
            base,
            asked: received,
            _runtime: runtime,
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
    fn the_api_shows_settings_and_the_tracks_of_what_plays() {
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
    fn a_put_saves_the_change_and_restyles_what_plays() {
        let server = serve();
        let (status, body) = send(&server, "PUT", "/api/subtitles", json!({ "size": 1.4, "shown": true }));
        assert_eq!(status, 200);
        assert_eq!(body["settings"]["size"], 1.4);
        assert_eq!(body["settings"]["shown"], true);
        assert_eq!(body["settings"]["color"], "#FFFFFF");
        assert_eq!(server.asked.recv().unwrap(), "Subtitles(Restyle)");
        let saved = server.db.subtitle_settings(ProfileId::FIRST).unwrap().unwrap();
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
    fn audio_tracks_are_named_after_their_language_and_chosen_by_id() {
        let tracks = json!([
            { "id": 1, "type": "audio", "lang": "eng", "title": "Surround 5.1" },
            { "id": 2, "type": "audio", "lang": "ger" },
            sub(1, "en", "English SRT"),
        ]);
        let mut playing = playing(tracks, None, false);
        let labels: Vec<_> = playing.audio_tracks.iter().map(|track| track.label.as_str()).collect();
        assert_eq!(labels, ["English", "German"]);
        assert_eq!(playing.tracks.len(), 1);
        let aid = json!({ "event": "property-change", "name": "aid", "data": 2 });
        assert_eq!(Observed::parse(&aid), Some(Observed::Audio(Some(2))));
        playing.audio = Some(1);
        let (commands, message) = playing.choose_audio(2).unwrap();
        assert_eq!(commands, [json!(["set_property", "aid", 2])]);
        assert_eq!(message, "Audio: German");
        assert_eq!(playing.choose_audio(7), Err("There is no audio track 7".to_string()));
    }

    fn english(path: &str) -> Sidecar {
        Sidecar {
            path: PathBuf::from(path),
            language: Some("en".to_string()),
            title: "English".to_string(),
            forced: false,
            hearing_impaired: false,
        }
    }

    #[test]
    fn library_subtitles_are_added_once_the_file_loads_and_shown_when_wanted() {
        let subtitles = Subtitles::default();
        subtitles
            .save(Settings {
                shown: true,
                ..Settings::default()
            })
            .unwrap();
        let film = Path::new("/lib/Movies/Airplane 1980/Airplane.mp4");
        subtitles.started(film, vec![english("/lib/Movies/Airplane 1980/Subs/13_English.srt")]);
        assert!(subtitles.mpv_args(film).contains(&"--sub-auto=no".to_string()));
        assert_eq!(
            subtitles.loaded(),
            [json!([
                "sub-add",
                "/lib/Movies/Airplane 1980/Subs/13_English.srt",
                "auto",
                "English",
                "en"
            ])]
        );
        let none = json!([{ "id": 1, "type": "audio", "lang": "en" }]);
        let tracks = |data: Value| Observed::parse(&json!({ "name": "track-list", "data": data })).unwrap();
        assert!(subtitles.observe(tracks(none)).is_empty());
        let added = json!([
            { "id": 1, "type": "audio", "lang": "en" },
            { "id": 1, "type": "sub", "lang": "en", "title": "English", "external": true },
        ]);
        assert_eq!(
            subtitles.observe(tracks(added.clone())),
            [
                json!(["set_property", "sid", 1]),
                json!(["set_property", "sub-visibility", true])
            ]
        );
        assert!(subtitles.observe(tracks(added)).is_empty());
        assert_eq!(subtitles.embedded(film), Some(0));
    }

    #[test]
    fn the_delay_applies_live_and_is_remembered_for_the_file() {
        let db = Db::memory(Path::new("/lib"));
        let film = Path::new("/lib/Movies/Up/Up.mkv");
        let subtitles = Subtitles::load(&db, ProfileId::FIRST);
        assert!(!subtitles.change(Change::Delay(0.5)).ok);
        subtitles.started(film, Vec::new());
        let answer = subtitles.change(Change::Delay(-1.04));
        assert_eq!(answer.commands, [json!(["set_property", "sub-delay", -1.0])]);
        assert_eq!(answer.message, "Subtitle delay -1.0 s");
        subtitles.observe(Observed::Delay(-1.0));
        assert_eq!(subtitles.delay(), -1.0);
        subtitles.stopped();
        let again = Subtitles::load(&db, ProfileId::FIRST);
        assert!(again.mpv_args(film).contains(&"--sub-delay=-1".to_string()));
        assert!(
            again
                .mpv_args(Path::new("/lib/other.mkv"))
                .contains(&"--sub-delay=0".to_string())
        );
        again.started(film, Vec::new());
        assert_eq!(again.change(Change::Delay(0.0)).message, "Subtitle delay off");
        assert!(!db.subtitle_files().unwrap().contains_key(film));
        assert_eq!(
            again.change(Change::Delay(500.0)).commands,
            [json!(["set_property", "sub-delay", 60.0])]
        );
    }

    #[test]
    fn fetched_subtitles_are_added_only_to_the_file_that_plays() {
        let subtitles = Subtitles::default();
        let film = PathBuf::from("/lib/Movies/Up/Up.mkv");
        subtitles.started(&film, Vec::new());
        let added = subtitles.change(Change::Add {
            video: film.clone(),
            sidecar: english("/lib/Movies/Up/Up.en.srt"),
        });
        assert!(added.ok);
        assert_eq!(
            added.commands,
            [
                json!(["sub-add", "/lib/Movies/Up/Up.en.srt", "select", "English", "en"]),
                json!(["set_property", "sub-visibility", true]),
            ]
        );
        let elsewhere = subtitles.change(Change::Add {
            video: PathBuf::from("/lib/Movies/Cars/Cars.mkv"),
            sidecar: english("/lib/Movies/Cars/Cars.en.srt"),
        });
        assert!(!elsewhere.ok);
        assert!(subtitles.plays(&film));
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
