mod codecs;
pub mod probe;
mod remux;
mod vtt;

use crate::api::{self, problem};
use crate::auth::gate::Who;
use crate::core::{Core, Request};
use crate::profile::ProfileId;
use crate::progress::Watched;
use crate::segments::{self, Kind as SegmentKind, Title};
use crate::sidecar;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use probe::{Client, Media, Verdict};
use remux::{Remux, Spec};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Write;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{OnceCell, watch};
use vtt::{Cue, Source};

const REST: Duration = Duration::from_secs(180);
const MOST_SESSIONS: usize = 3;
const CELLULAR_WARNING: u64 = 8_000_000;
const PLAYLIST: &str = "application/vnd.apple.mpegurl";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Moment {
    Start,
    Playing,
    Paused,
    Stopped,
    Ended,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub profile: ProfileId,
    pub device: Option<crate::screen::Device>,
    pub session: String,
    pub file: PathBuf,
    pub position: f64,
    pub duration: f64,
    pub credits: Option<f64>,
    pub played: f64,
    pub moment: Moment,
}

#[derive(Clone, Debug, Serialize)]
struct Subtitle {
    id: usize,
    name: String,
    language: Option<String>,
    forced: bool,
    default: bool,
    supported: bool,
    #[serde(skip)]
    source: Option<Source>,
}

impl Subtitle {
    /// The track inside the video this comes from, when it is a text track
    /// there, which the remux writes out as it goes.
    fn embedded(&self) -> Option<usize> {
        match &self.source {
            Some(Source::Embedded(track)) => Some(*track),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
struct Mark {
    kind: &'static str,
    label: &'static str,
    start: f64,
    end: f64,
}

#[derive(Clone, Debug, Default, Serialize)]
struct Marks {
    segments: Vec<Mark>,
    credits: Option<f64>,
}

struct Live {
    id: String,
    remux: Remux,
    dialogue: Arc<AtomicBool>,
    profile: ProfileId,
    device: Option<crate::screen::Device>,
    file: PathBuf,
    start: f64,
    duration: f64,
    bandwidth: u64,
    resolution: Option<(u32, u32)>,
    subtitles: Vec<Subtitle>,
    delay: f64,
    title: Option<Title>,
    chapter_credits: Option<f64>,
    marks: OnceCell<Marks>,
}

impl Live {
    fn credits(&self) -> Option<f64> {
        let found = self
            .chapter_credits
            .or_else(|| self.marks.get().and_then(|marks| marks.credits));
        Watched {
            duration: Some(self.duration),
            credits: found,
            ..Watched::default()
        }
        .credits_start()
    }

    /// Whether the profile switched dialogue mode since this stream started
    /// converting its sound, so it has to start again to follow.
    fn sound_changed(&self) -> bool {
        !self.remux.spec.copy_audio && self.dialogue.load(Ordering::SeqCst) != self.remux.spec.dialogue
    }
}

type Converted = Option<Result<Arc<String>, String>>;
type Captions = HashMap<PathBuf, watch::Receiver<Converted>>;

#[derive(Clone)]
struct Streams {
    core: Core,
    root: PathBuf,
    live: Arc<Mutex<HashMap<String, Arc<Live>>>>,
    probes: Arc<Mutex<HashMap<PathBuf, Media>>>,
    captions: Arc<Mutex<Captions>>,
}

pub fn routes<S: Clone + Send + Sync + 'static>(core: Core, root: PathBuf) -> Router<S> {
    remux::clear(&root);
    let streams = Streams {
        core,
        root,
        live: Arc::default(),
        probes: Arc::default(),
        captions: Arc::default(),
    };
    tokio::spawn(reap(streams.clone()));
    Router::new()
        .route("/api/stream/open", post(open))
        .route("/api/stream/{id}", delete(close))
        .route("/api/stream/{id}/main.m3u8", get(master))
        .route("/api/stream/{id}/video.m3u8", get(media_playlist))
        .route("/api/stream/{id}/subtitles/{number}", get(subtitles))
        .route("/api/stream/{id}/marks", get(marks))
        .route("/api/stream/{id}/progress", post(progress))
        .route("/api/stream/{id}/{name}", get(file))
        .with_state(streams)
}

async fn reap(streams: Streams) {
    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;
        let gone: Vec<Arc<Live>> = {
            let mut live = streams.live.lock().unwrap();
            let idle: Vec<String> = live
                .iter()
                .filter(|(_, live)| live.remux.idle() > REST)
                .map(|(id, _)| id.clone())
                .collect();
            idle.iter().filter_map(|id| live.remove(id)).collect()
        };
        for live in gone {
            eprintln!(
                "stream {}: ended after {} s without a request from {}",
                short(&live.id),
                live.remux.idle().as_secs(),
                device_name(live.device.as_ref())
            );
            live.remux.close();
        }
    }
}

fn short(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

fn device_name(device: Option<&crate::screen::Device>) -> &str {
    device.map_or("a browser", |device| device.name.as_str())
}

fn file_name(file: &std::path::Path) -> String {
    file.file_name().map_or_else(
        || file.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn new_id() -> String {
    let mut bytes = [0u8; 16];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut bytes))
        .is_err()
    {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |time| time.as_nanos());
        bytes = nanos.to_le_bytes();
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenBody {
    movie: Option<String>,
    show: Option<String>,
    season: Option<u32>,
    episode: Option<u32>,
    audio: Option<usize>,
    start: Option<f64>,
    #[serde(default)]
    client: Client,
    replaces: Option<String>,
    device: Option<crate::screen::Device>,
}

#[derive(Debug, Serialize)]
struct Next {
    season: u32,
    episode: u32,
    label: String,
    title: Option<String>,
    still: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Picked {
    Movie {
        key: String,
    },
    Show {
        key: String,
        season: u32,
        episode: u32,
        next: Option<Next>,
    },
}

impl Picked {
    fn art(&self) -> String {
        match self {
            Self::Movie { key } => format!("/api/movies/{key}"),
            Self::Show { key, .. } => format!("/api/shows/{key}"),
        }
    }

    fn episode(&self) -> Option<(u32, u32)> {
        match self {
            Self::Movie { .. } => None,
            Self::Show { season, episode, .. } => Some((*season, *episode)),
        }
    }
}

#[derive(Serialize)]
struct Opened {
    id: String,
    playlist: String,
    #[serde(flatten)]
    picked: Picked,
    title: String,
    caption: String,
    poster: String,
    backdrop: String,
    start: f64,
    /// The file time at which the stream's own timeline starts.
    offset: f64,
    duration: f64,
    audio: Vec<probe::Audio>,
    audio_index: Option<usize>,
    converted_audio: bool,
    subtitles: Vec<Subtitle>,
    subtitle_delay: f64,
    credits: Option<f64>,
    bitrate: Option<u64>,
    heavy: bool,
}

struct Chosen {
    file: PathBuf,
    picked: Picked,
    title: String,
    caption: String,
    tmdb: Option<u64>,
}

fn choose(streams: &Streams, body: &OpenBody, progress: &crate::progress::Progress) -> Result<Chosen, Response> {
    let library = streams.core.library();
    let missing = |key: &str| problem(StatusCode::NOT_FOUND, format!("{key} is not in the library"));
    match (&body.movie, &body.show) {
        (Some(key), None) => {
            let movie = library
                .movies
                .iter()
                .find(|movie| &movie.key == key)
                .ok_or_else(|| missing(key))?;
            Ok(Chosen {
                file: movie.path.clone(),
                picked: Picked::Movie { key: movie.key.clone() },
                title: movie.title.clone(),
                caption: movie.year.map(|year| year.to_string()).unwrap_or_default(),
                tmdb: movie.tmdb,
            })
        }
        (None, Some(key)) => {
            let show = library
                .shows
                .iter()
                .find(|show| &show.key == key)
                .ok_or_else(|| missing(key))?;
            let episode = match (body.season, body.episode) {
                (Some(season), Some(number)) => show.episode(season, number),
                (None, None) => progress.resume_episode(show),
                _ => {
                    return Err(problem(
                        StatusCode::BAD_REQUEST,
                        "give season and episode together".to_string(),
                    ));
                }
            }
            .ok_or_else(|| missing(&format!("{key} episode")))?;
            let still = |season: u32, number: u32| format!("/api/shows/{}/episodes/{season}/{number}/still", show.key);
            Ok(Chosen {
                file: episode.path.clone(),
                picked: Picked::Show {
                    key: show.key.clone(),
                    season: episode.season,
                    episode: episode.number,
                    next: show.episode_after(episode.season, episode.number).map(|next| Next {
                        season: next.season,
                        episode: next.number,
                        label: next.short_label(),
                        title: next.title.clone(),
                        still: still(next.season, next.number),
                    }),
                },
                title: show.title.clone(),
                caption: episode.label(),
                tmdb: show.tmdb,
            })
        }
        _ => Err(problem(
            StatusCode::BAD_REQUEST,
            "give either movie or show".to_string(),
        )),
    }
}

async fn examine(streams: &Streams, file: &PathBuf) -> Result<Media, String> {
    if let Some(found) = streams.probes.lock().unwrap().get(file) {
        return Ok(found.clone());
    }
    let media = probe::probe(file).await?;
    streams.probes.lock().unwrap().insert(file.clone(), media.clone());
    Ok(media)
}

fn subtitle_list(streams: &Streams, file: &std::path::Path, media: &Media, profile: ProfileId) -> Vec<Subtitle> {
    let settings = streams.core.subtitles().settings_of(profile);
    let mut found: Vec<Subtitle> = Vec::new();
    for sidecar in sidecar::find(file) {
        found.push(Subtitle {
            id: 0,
            name: sidecar.title.clone(),
            language: sidecar.language.clone(),
            forced: sidecar.forced,
            default: false,
            supported: true,
            source: Some(Source::Sidecar(sidecar.path.clone())),
        });
    }
    for embedded in &media.subtitles {
        let language = embedded.language.clone();
        let named = match language.as_deref() {
            Some(code) => crate::subtitles::language_name(code).map_or_else(|| code.to_uppercase(), str::to_string),
            None => "Subtitles".to_string(),
        };
        let mut name = embedded.title.clone().unwrap_or(named);
        if !embedded.is_text() {
            name.push_str(" (picture subtitles show only on the TV)");
        }
        found.push(Subtitle {
            id: 0,
            name,
            language,
            forced: embedded.forced,
            default: false,
            supported: embedded.is_text(),
            source: embedded.is_text().then_some(Source::Embedded(embedded.index)),
        });
    }
    if settings.shown {
        let preferred = settings.languages.iter().find_map(|wanted| {
            found.iter().position(|subtitle| {
                subtitle.supported
                    && !subtitle.forced
                    && subtitle.language.as_deref().is_some_and(|language| {
                        language == wanted.as_str()
                            || crate::subtitles::bibliographic(language) == crate::subtitles::bibliographic(wanted)
                    })
            })
        });
        if let Some(chosen) = preferred {
            found[chosen].default = true;
        }
    }
    for (id, subtitle) in found.iter_mut().enumerate() {
        subtitle.id = id;
    }
    found
}

fn resume_at(progress: &crate::progress::Progress, file: &PathBuf, duration: f64) -> f64 {
    let at = progress
        .files
        .get(file)
        .map_or(0.0, crate::progress::Watched::resume_at);
    if at < duration - 10.0 { at } else { 0.0 }
}

#[cfg(test)]
mod resume_tests {
    use super::resume_at;
    use crate::progress::{Place, Progress};
    use std::path::PathBuf;

    fn place(position: f64) -> Place {
        Place {
            position,
            duration: 4300.0,
            credits: None,
        }
    }

    #[test]
    fn a_phone_resumes_a_title_where_it_stopped_even_after_it_was_finished_once() {
        let file = PathBuf::from("/lib/The.White.Lotus.S02E07.mkv");
        let mut progress = Progress::default();
        assert_eq!(resume_at(&progress, &file, 4300.0), 0.0);
        progress.record(&file, place(977.0));
        assert_eq!(resume_at(&progress, &file, 4300.0), 977.0);
        progress.finish(&file);
        assert_eq!(resume_at(&progress, &file, 4300.0), 0.0);
        progress.record(&file, place(3657.0));
        assert_eq!(resume_at(&progress, &file, 4300.0), 3657.0);
        progress.record(&file, place(4295.0));
        assert_eq!(resume_at(&progress, &file, 4300.0), 0.0);
    }
}

#[cfg(test)]
mod picked_tests {
    use super::{Next, Picked};
    use serde_json::json;

    #[test]
    fn a_movie_opens_without_an_episode_and_an_episode_with_its_place_and_what_follows() {
        let movie = Picked::Movie {
            key: "dune-2021".to_string(),
        };
        assert_eq!(
            serde_json::to_value(&movie).unwrap(),
            json!({ "kind": "movie", "key": "dune-2021" })
        );
        assert_eq!(
            (movie.art(), movie.episode()),
            ("/api/movies/dune-2021".to_string(), None)
        );
        let episode = Picked::Show {
            key: "dark".to_string(),
            season: 1,
            episode: 10,
            next: Some(Next {
                season: 2,
                episode: 1,
                label: "S2 E1".to_string(),
                title: None,
                still: "/still".to_string(),
            }),
        };
        let json = serde_json::to_value(&episode).unwrap();
        assert_eq!(
            (&json["kind"], &json["season"], &json["episode"]),
            (&json!("show"), &json!(1), &json!(10))
        );
        assert_eq!(json["next"]["season"], 2);
        assert_eq!(
            (episode.art(), episode.episode()),
            ("/api/shows/dark".to_string(), Some((1, 10)))
        );
    }
}

async fn open(State(streams): State<Streams>, Who(profile): Who, Json(body): Json<OpenBody>) -> Response {
    let progress = streams.core.progress(profile).await;
    let chosen = match choose(&streams, &body, &progress) {
        Ok(chosen) => chosen,
        Err(response) => return response,
    };
    let refused = |status: StatusCode, reason: String| {
        eprintln!(
            "stream: {} can't open {}: {reason}",
            device_name(body.device.as_ref()),
            file_name(&chosen.file)
        );
        problem(status, reason)
    };
    let media = match examine(&streams, &chosen.file).await {
        Ok(media) => media,
        Err(error) => return refused(StatusCode::UNPROCESSABLE_ENTITY, error),
    };
    let Some(duration) = media.duration.filter(|duration| *duration > 0.0) else {
        return refused(
            StatusCode::UNPROCESSABLE_ENTITY,
            "This file has no length to stream".to_string(),
        );
    };
    let audio_index = body
        .audio
        .filter(|index| *index < media.audio.len())
        .or_else(|| media.audio.iter().position(|audio| audio.default))
        .or((!media.audio.is_empty()).then_some(0));
    let audio = audio_index.and_then(|index| media.audio.get(index));
    let dialogue = streams.core.dialogue(profile).await;
    let dialogue_on = dialogue.load(Ordering::SeqCst);
    let (hevc, copy_audio) = match probe::verdict(&media, audio, body.client) {
        Verdict::Plays { hevc, copy_audio } => (hevc, copy_audio && !dialogue_on),
        Verdict::Refused(reason) => return refused(StatusCode::UNPROCESSABLE_ENTITY, reason),
    };
    let handed_here = match (streams.core.screen().on, body.device.as_ref()) {
        (crate::screen::OnScreen::Moving { device: to, .. }, Some(asking)) => to.id == asking.id,
        _ => false,
    };
    let tv_playing = streams.core.now_playing().is_some() && !handed_here;
    let (ended, full) = {
        let mut live = streams.live.lock().unwrap();
        let mut ended = Vec::new();
        if let Some(replaced) = body.replaces.as_ref().and_then(|id| live.remove(id)) {
            eprintln!("stream {}: ended, replaced by a new one", short(&replaced.id));
            ended.push(replaced);
        }
        if let Some(device) = &body.device {
            let same: Vec<String> = live
                .iter()
                .filter(|(_, other)| other.device.as_ref().is_some_and(|other| other.id == device.id))
                .map(|(id, _)| id.clone())
                .collect();
            for id in same {
                if let Some(other) = live.remove(&id) {
                    eprintln!("stream {}: ended, {} opened another", short(&other.id), device.name);
                    ended.push(other);
                }
            }
        }
        let limit = if tv_playing { 1 } else { MOST_SESSIONS };
        (ended, live.len() >= limit)
    };
    for other in ended {
        other.remux.close();
    }
    if full {
        let reason = if tv_playing {
            "The TV is playing, so only one phone can watch at a time"
        } else {
            "Three phones are already watching"
        };
        return refused(StatusCode::TOO_MANY_REQUESTS, reason.to_string());
    }
    let start = body
        .start
        .filter(|at| at.is_finite() && *at >= 0.0 && *at < duration)
        .unwrap_or_else(|| resume_at(&progress, &chosen.file, duration));
    let id = new_id();
    let subtitles = subtitle_list(&streams, &chosen.file, &media, profile);
    let spec = Spec {
        source: chosen.file.clone(),
        audio: audio.map(|audio| audio.index),
        hevc,
        copy_audio: copy_audio || audio.is_none(),
        dialogue: dialogue_on,
        subtitles: subtitles.iter().filter_map(Subtitle::embedded).collect(),
    };
    if let Some(Source::Sidecar(path)) = subtitles
        .iter()
        .find(|subtitle| subtitle.default)
        .and_then(|subtitle| subtitle.source.clone())
    {
        let warming = streams.clone();
        tokio::spawn(async move {
            let _ = caption(&warming, path).await;
        });
    }
    let remux = match Remux::start(streams.root.join(&id), spec, start).await {
        Ok(remux) => remux,
        Err(error) => return refused(StatusCode::UNPROCESSABLE_ENTITY, error),
    };
    let delay = streams.core.subtitles().file_delay(&chosen.file);
    let video = media.video.as_ref();
    let bitrate = media.bitrate;
    let offset = remux.offset;
    let live = Arc::new(Live {
        id: id.clone(),
        remux,
        dialogue,
        profile,
        device: body.device.clone(),
        file: chosen.file.clone(),
        start,
        duration,
        bandwidth: bitrate.unwrap_or(8_000_000),
        resolution: video
            .map(|video| (video.width, video.height))
            .filter(|(width, _)| *width > 0),
        subtitles: subtitles.clone(),
        delay,
        title: chosen.tmdb.map(|tmdb| Title {
            tmdb,
            episode: chosen.picked.episode(),
        }),
        chapter_credits: media.credits,
        marks: OnceCell::new(),
    });
    let credits = live.credits();
    eprintln!(
        "stream {}: opened {} for {} at {start:.0} s (from {offset:.1} s), {}, audio {}",
        short(&id),
        file_name(&live.file),
        device_name(live.device.as_ref()),
        if hevc { "HEVC" } else { "H.264" },
        match (copy_audio, dialogue_on) {
            (true, _) => "copied",
            (false, true) => "converted, dialogue mode",
            (false, false) => "converted",
        }
    );
    streams.live.lock().unwrap().insert(id.clone(), live);
    let art = chosen.picked.art();
    Json(Opened {
        playlist: format!("/api/stream/{id}/main.m3u8"),
        id,
        picked: chosen.picked,
        title: chosen.title,
        caption: chosen.caption,
        poster: format!("{art}/poster"),
        backdrop: format!("{art}/backdrop"),
        start,
        offset,
        duration,
        audio: media.audio.clone(),
        audio_index,
        converted_audio: audio.is_some() && !copy_audio,
        subtitles,
        subtitle_delay: delay,
        credits,
        bitrate,
        heavy: bitrate.is_some_and(|rate| rate > CELLULAR_WARNING),
    })
    .into_response()
}

fn find(streams: &Streams, id: &str, asked: &str) -> Result<Arc<Live>, Response> {
    streams.live.lock().unwrap().get(id).cloned().ok_or_else(|| {
        eprintln!("stream {}: {asked} for a stream that has ended", short(id));
        problem(StatusCode::NOT_FOUND, "this video closed; open it again".to_string())
    })
}

fn body(kind: &'static str, cache: &'static str, bytes: impl Into<axum::body::Body>) -> Response {
    let mut response = bytes.into().into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(kind));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

async fn close(State(streams): State<Streams>, Path(id): Path<String>) -> Response {
    let closed = streams.live.lock().unwrap().remove(&id);
    if let Some(live) = closed {
        eprintln!("stream {}: closed by {}", short(&id), device_name(live.device.as_ref()));
        live.remux.close();
    }
    StatusCode::NO_CONTENT.into_response()
}

/// The master playlist: one variant, named with its codecs so a player knows
/// what it gets before it loads a segment.
fn master_playlist(bandwidth: u64, resolution: Option<(u32, u32)>, codecs: Option<&str>) -> String {
    let mut text = String::from("#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-INDEPENDENT-SEGMENTS\n");
    let _ = write!(text, "#EXT-X-STREAM-INF:BANDWIDTH={}", bandwidth * 6 / 5);
    if let Some((width, height)) = resolution {
        let _ = write!(text, ",RESOLUTION={width}x{height}");
    }
    if let Some(codecs) = codecs {
        let _ = write!(text, ",CODECS=\"{codecs}\"");
    }
    text.push_str("\nvideo.m3u8\n");
    text
}

async fn master(State(streams): State<Streams>, Path(id): Path<String>) -> Response {
    match find(&streams, &id, "the playlist") {
        Ok(live) => {
            live.remux.touch();
            body(
                PLAYLIST,
                "no-store",
                master_playlist(live.bandwidth, live.resolution, live.remux.codecs.as_deref()),
            )
        }
        Err(response) => response,
    }
}

async fn media_playlist(State(streams): State<Streams>, Path(id): Path<String>) -> Response {
    let live = match find(&streams, &id, "the video playlist") {
        Ok(live) => live,
        Err(response) => return response,
    };
    match live.remux.playlist(live.start).await {
        Some(text) => body(PLAYLIST, "no-store", text),
        None => problem(StatusCode::NOT_FOUND, "the video playlist is gone".to_string()),
    }
}

async fn file(State(streams): State<Streams>, Path((id, name)): Path<(String, String)>) -> Response {
    let live = match find(&streams, &id, &name) {
        Ok(live) => live,
        Err(response) => return response,
    };
    match live.remux.file(&name).await {
        Some(bytes) if name.ends_with(".mp4") => body("video/mp4", "private, max-age=3600", bytes),
        Some(bytes) => body("video/iso.segment", "private, max-age=3600", bytes),
        None => {
            eprintln!("stream {}: {name} isn't there", short(&id));
            problem(StatusCode::NOT_FOUND, format!("no {name}"))
        }
    }
}

#[derive(Serialize)]
struct Cues {
    cues: Vec<Cue>,
    /// Whether these are all the track's cues, or more follow as the remux
    /// reads on.
    complete: bool,
}

async fn subtitles(State(streams): State<Streams>, Path((id, number)): Path<(String, String)>) -> Response {
    let live = match find(&streams, &id, &format!("subtitles {number}")) {
        Ok(live) => live,
        Err(response) => return response,
    };
    let Some(source) = number
        .parse::<usize>()
        .ok()
        .and_then(|number| live.subtitles.get(number))
        .filter(|subtitle| subtitle.supported)
        .and_then(|subtitle| subtitle.source.clone())
    else {
        return problem(StatusCode::NOT_FOUND, format!("no subtitles {number}"));
    };
    let (cues, complete) = match source {
        Source::Embedded(track) => live.remux.cues(track).await,
        Source::Sidecar(path) => match caption(&streams, path).await {
            Ok(text) => (vtt::all_cues(&text), true),
            Err(error) => return problem(StatusCode::UNPROCESSABLE_ENTITY, error),
        },
    };
    Json(Cues {
        cues: vtt::shifted(cues, live.delay),
        complete,
    })
    .into_response()
}

/// A subtitle file beside the video as WebVTT, converted once however many
/// requests ask for it meanwhile.
async fn caption(streams: &Streams, path: PathBuf) -> Result<Arc<String>, String> {
    let mut converted = {
        let mut captions = streams.captions.lock().unwrap();
        match captions.get(&path) {
            Some(converted) => converted.clone(),
            None => {
                let (done, converted) = watch::channel(None);
                captions.insert(path.clone(), converted.clone());
                let captions = streams.captions.clone();
                tokio::spawn(async move {
                    let result = vtt::convert(&path).await.map(Arc::new);
                    if result.is_err() {
                        captions.lock().unwrap().remove(&path);
                    }
                    let _ = done.send(Some(result));
                });
                converted
            }
        }
    };
    let finished = converted
        .wait_for(Option::is_some)
        .await
        .map_err(|_| "the subtitles could not be converted".to_string())?;
    finished
        .clone()
        .unwrap_or_else(|| Err("the subtitles could not be converted".to_string()))
}

async fn marks(State(streams): State<Streams>, Path(id): Path<String>) -> Response {
    let live = match find(&streams, &id, "the marks") {
        Ok(live) => live,
        Err(response) => return response,
    };
    let found = live
        .marks
        .get_or_init(|| async {
            let Some(title) = live.title else {
                return Marks::default();
            };
            let duration = live.duration;
            let fetched = tokio::task::spawn_blocking(move || segments::fetch(title, duration)).await;
            let Ok(Ok(found)) = fetched else {
                return Marks::default();
            };
            Marks {
                segments: found
                    .segments
                    .iter()
                    .map(|segment| Mark {
                        kind: match segment.kind {
                            SegmentKind::Recap => "recap",
                            SegmentKind::Intro => "intro",
                            SegmentKind::Next => "next",
                        },
                        label: segment.kind.label(),
                        start: segment.start,
                        end: segment.end,
                    })
                    .collect(),
                credits: found.credits,
            }
        })
        .await;
    Json(Marks {
        segments: found.segments.clone(),
        credits: live.credits(),
    })
    .into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProgressBody {
    position: f64,
    played: f64,
    state: Moment,
}

async fn progress(
    State(streams): State<Streams>,
    Path(id): Path<String>,
    Json(report): Json<ProgressBody>,
) -> Response {
    let live = match find(&streams, &id, &format!("progress {:?}", report.state)) {
        Ok(live) => live,
        Err(response) => return response,
    };
    live.remux.touch();
    if !report.position.is_finite() || !report.played.is_finite() {
        return problem(
            StatusCode::BAD_REQUEST,
            "position and played must be numbers".to_string(),
        );
    }
    let ends = matches!(report.state, Moment::Stopped | Moment::Ended);
    let request = Request::Phone(Report {
        profile: live.profile,
        device: live.device.clone(),
        session: id.clone(),
        file: live.file.clone(),
        position: report.position.clamp(0.0, live.duration),
        duration: live.duration,
        credits: live.credits(),
        played: report.played.max(0.0),
        moment: report.state,
    });
    let answer = api::ask(&streams.core, request).await;
    let removed = if ends {
        streams.live.lock().unwrap().remove(&id)
    } else {
        None
    };
    if let Some(live) = removed {
        eprintln!(
            "stream {}: ended, {} reported {:?} at {:.0} s",
            short(&id),
            device_name(live.device.as_ref()),
            report.state,
            report.position
        );
        live.remux.close();
        return answer;
    }
    if answer.status().is_success() && live.sound_changed() {
        eprintln!(
            "stream {}: dialogue mode changed, asking {} to reopen",
            short(&id),
            device_name(live.device.as_ref())
        );
        return Json(serde_json::json!({ "ok": true, "reopen": "sound" })).into_response();
    }
    answer
}
