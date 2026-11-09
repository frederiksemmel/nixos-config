use crate::api::{self, Ask, Pending, Request};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

pub const OBSERVED: [&str; 4] = ["track-list", "sid", "sub-visibility", "aid"];
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
];
const MOST_LANGUAGES: usize = 8;
const PER_FILE_STYLE: [&str; 3] = ["sub-pos", "sub-scale", "sub-ass-override"];
const NO_SUBTITLES: &str = "No subtitles in this file";
const OFF: &str = "Subtitles off";

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
            "--sub-auto=fuzzy".to_string(),
        ];
        for name in PER_FILE_STYLE {
            args.push(format!("--watch-later-options-remove={name}"));
        }
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
        let name = language.as_deref().and_then(language_name).map(str::to_string);
        let label = name
            .or_else(|| title.clone())
            .or_else(|| language.clone())
            .unwrap_or_else(|| format!("Track {id}"));
        Some(Self {
            id,
            language,
            title,
            label,
            forced: flag("forced"),
            hearing_impaired: flag("hearing-impaired"),
            external: flag("external"),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Playing {
    pub visible: bool,
    pub track: Option<i64>,
    pub tracks: Vec<Track>,
    pub audio: Option<i64>,
    pub audio_tracks: Vec<Track>,
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Change {
    Restyle,
    Toggle,
    Track(Option<i64>),
    Audio(i64),
}

#[derive(Default)]
struct Inner {
    settings: Settings,
    playing: Option<Playing>,
}

#[derive(Clone, Default)]
pub struct Subtitles {
    path: Option<PathBuf>,
    state: Arc<Mutex<Inner>>,
}

impl Subtitles {
    pub fn load(path: PathBuf) -> Self {
        let settings = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok())
            .and_then(|settings| settings.checked().ok())
            .unwrap_or_default();
        Self {
            path: Some(path),
            state: Arc::new(Mutex::new(Inner {
                settings,
                playing: None,
            })),
        }
    }

    pub fn settings(&self) -> Settings {
        self.state.lock().unwrap().settings.clone()
    }

    fn playing(&self) -> Option<Playing> {
        self.state.lock().unwrap().playing.clone()
    }

    pub fn chosen(&self) -> (Option<i64>, Option<i64>) {
        self.playing().map_or((None, None), |playing| {
            (playing.track.filter(|_| playing.visible), playing.audio)
        })
    }

    fn save(&self, settings: Settings) -> io::Result<()> {
        if let Some(path) = &self.path {
            let temporary = path.with_extension("json.tmp");
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir)?;
            }
            fs::write(&temporary, serde_json::to_vec_pretty(&settings)?)?;
            fs::rename(temporary, path)?;
        }
        self.state.lock().unwrap().settings = settings;
        Ok(())
    }

    pub fn started(&self) {
        self.state.lock().unwrap().playing = Some(Playing::default());
    }

    pub fn stopped(&self) {
        self.state.lock().unwrap().playing = None;
    }

    pub fn observe(&self, observed: Observed) {
        let mut state = self.state.lock().unwrap();
        let playing = state.playing.get_or_insert_with(Playing::default);
        match observed {
            Observed::Tracks { subtitles, audio } => {
                playing.tracks = subtitles;
                playing.audio_tracks = audio;
            }
            Observed::Track(track) => playing.track = track,
            Observed::Audio(track) => playing.audio = track,
            Observed::Visible(visible) => playing.visible = visible,
        }
    }

    pub fn change(&self, change: Change) -> Answer {
        let state = self.state.lock().unwrap();
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
            "--watch-later-options-remove=sub-scale",
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
    fn settings_survive_a_restart_and_a_broken_file_falls_back_to_defaults() {
        let dir = std::env::temp_dir().join(format!("semmeltv-subtitles-{}", std::process::id()));
        let path = dir.join("subtitles.json");
        let subtitles = Subtitles::load(path.clone());
        assert_eq!(subtitles.settings(), Settings::default());
        let bigger = Settings {
            size: 1.5,
            ..Settings::default()
        };
        subtitles.save(bigger.clone()).unwrap();
        assert_eq!(Subtitles::load(path.clone()).settings(), bigger);
        fs::write(&path, "{ not json").unwrap();
        assert_eq!(Subtitles::load(path).settings(), Settings::default());
        fs::remove_dir_all(dir).unwrap();
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
        subtitles.started();
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
        dir: PathBuf,
    }

    impl Drop for Server {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn serve(name: &str) -> Server {
        let dir = std::env::temp_dir().join(format!("semmeltv-subtitles-api-{name}-{}", std::process::id()));
        let subtitles = Subtitles::load(dir.join("subtitles.json"));
        subtitles.started();
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
            dir,
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
        let server = serve("show");
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
        let server = serve("put");
        let (status, body) = send(&server, "PUT", "/api/subtitles", json!({ "size": 1.4, "shown": true }));
        assert_eq!(status, 200);
        assert_eq!(body["settings"]["size"], 1.4);
        assert_eq!(body["settings"]["shown"], true);
        assert_eq!(body["settings"]["color"], "#FFFFFF");
        assert_eq!(server.asked.recv().unwrap(), "Subtitles(Restyle)");
        let saved: Settings = serde_json::from_slice(&fs::read(server.dir.join("subtitles.json")).unwrap()).unwrap();
        assert_eq!(saved.size, 1.4);
        let (status, body) = send(&server, "PUT", "/api/subtitles", json!({ "color": "pink" }));
        assert_eq!(status, 400);
        assert!(body["error"].as_str().unwrap().contains("colour"));
        assert_eq!(send(&server, "PUT", "/api/subtitles", json!({ "size": "big" })).0, 400);
    }

    #[test]
    fn tracks_and_the_toggle_go_to_the_tv() {
        let server = serve("track");
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
}
