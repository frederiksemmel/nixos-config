mod fmp4;
mod index;
mod plan;
mod probe;
pub mod resume;
mod session;
mod vtt;

use crate::api::{self, Ask, Request, Shared, problem};
use crate::bus::Bus;
use crate::progress::Watched;
use crate::segments::{self, Kind as SegmentKind, Title};
use crate::sidecar;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use index::Index;
use plan::Plan;
use probe::{Client, Media, Verdict};
use serde::{Deserialize, Serialize};
use session::{Session, Spec};
use std::collections::HashMap;
use std::fmt::Write;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{OnceCell, watch};
use vtt::Source;

const REST: Duration = Duration::from_secs(180);
const FORGET: Duration = Duration::from_secs(12 * 3600);
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
    session: Arc<Session>,
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
}

type Converted = Option<Result<Arc<String>, String>>;
type Captions = HashMap<(Source, i64), watch::Receiver<Converted>>;

#[derive(Clone)]
struct Streams {
    shared: Arc<Shared>,
    bus: Bus,
    app: Ask,
    root: PathBuf,
    live: Arc<Mutex<HashMap<String, Arc<Live>>>>,
    probes: Arc<Mutex<HashMap<PathBuf, (Media, Index)>>>,
    captions: Arc<Mutex<Captions>>,
}

pub fn routes<S: Clone + Send + Sync + 'static>(shared: Arc<Shared>, bus: Bus, app: Ask, root: PathBuf) -> Router<S> {
    session::clear(&root);
    let streams = Streams {
        shared,
        bus,
        app,
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
        .route("/api/stream/{id}/init.mp4", get(init))
        .route("/api/stream/{id}/segment/{name}", get(segment))
        .route("/api/stream/{id}/subtitles/{name}", get(subtitles))
        .route("/api/stream/{id}/marks", get(marks))
        .route("/api/stream/{id}/progress", post(progress))
        .with_state(streams)
}

async fn reap(streams: Streams) {
    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;
        let (resting, gone): (Vec<Arc<Live>>, Vec<Arc<Live>>) = {
            let mut live = streams.live.lock().unwrap();
            let forgotten: Vec<String> = live
                .iter()
                .filter(|(_, live)| live.session.idle() > FORGET)
                .map(|(id, _)| id.clone())
                .collect();
            let gone = forgotten.iter().filter_map(|id| live.remove(id)).collect();
            let resting = live
                .values()
                .filter(|live| live.session.idle() > REST)
                .cloned()
                .collect();
            (resting, gone)
        };
        for live in resting {
            live.session.rest();
        }
        for live in gone {
            live.session.close();
        }
    }
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

#[derive(Serialize)]
struct Next {
    season: u32,
    episode: u32,
    label: String,
    title: Option<String>,
    still: String,
}

#[derive(Serialize)]
struct Opened {
    id: String,
    playlist: String,
    kind: &'static str,
    key: String,
    title: String,
    caption: String,
    season: Option<u32>,
    episode: Option<u32>,
    poster: String,
    backdrop: String,
    start: f64,
    duration: f64,
    audio: Vec<probe::Audio>,
    audio_index: Option<usize>,
    converted_audio: bool,
    subtitles: Vec<Subtitle>,
    subtitle_delay: f64,
    credits: Option<f64>,
    next: Option<Next>,
    bitrate: Option<u64>,
    heavy: bool,
}

struct Chosen {
    file: PathBuf,
    kind: &'static str,
    key: String,
    title: String,
    caption: String,
    season: Option<u32>,
    episode: Option<u32>,
    tmdb: Option<u64>,
    next: Option<Next>,
}

fn choose(streams: &Streams, body: &OpenBody) -> Result<Chosen, Response> {
    let library = streams.shared.library();
    let progress = streams.shared.progress();
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
                kind: "movie",
                key: movie.key.clone(),
                title: movie.title.clone(),
                caption: movie.year.map(|year| year.to_string()).unwrap_or_default(),
                season: None,
                episode: None,
                tmdb: movie.tmdb,
                next: None,
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
                kind: "show",
                key: show.key.clone(),
                title: show.title.clone(),
                caption: episode.label(),
                season: Some(episode.season),
                episode: Some(episode.number),
                tmdb: show.tmdb,
                next: show.episode_after(episode.season, episode.number).map(|next| Next {
                    season: next.season,
                    episode: next.number,
                    label: next.short_label(),
                    title: next.title.clone(),
                    still: still(next.season, next.number),
                }),
            })
        }
        _ => Err(problem(
            StatusCode::BAD_REQUEST,
            "give either movie or show".to_string(),
        )),
    }
}

async fn examine(streams: &Streams, file: &PathBuf) -> Result<(Media, Index), String> {
    if let Some(found) = streams.probes.lock().unwrap().get(file) {
        return Ok(found.clone());
    }
    let reading = file.clone();
    let index = tokio::task::spawn_blocking(move || index::read(&reading));
    let media = probe::probe(file).await?;
    let index = index
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| format!("This file can't be streamed: {error}"))?;
    streams
        .probes
        .lock()
        .unwrap()
        .insert(file.clone(), (media.clone(), index.clone()));
    Ok((media, index))
}

fn subtitle_list(streams: &Streams, file: &std::path::Path, media: &Media) -> Vec<Subtitle> {
    let settings = streams.shared.subtitles().settings();
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
            source: embedded
                .is_text()
                .then(|| Source::Embedded(file.to_path_buf(), embedded.index)),
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
    let Some(watched) = progress.files.get(file) else {
        return 0.0;
    };
    if watched.finished || watched.in_credits() {
        return 0.0;
    }
    watched.stopped_at.filter(|at| *at < duration - 10.0).unwrap_or(0.0)
}

async fn open(State(streams): State<Streams>, Json(body): Json<OpenBody>) -> Response {
    let chosen = match choose(&streams, &body) {
        Ok(chosen) => chosen,
        Err(response) => return response,
    };
    let (media, index) = match examine(&streams, &chosen.file).await {
        Ok(found) => found,
        Err(error) => return problem(StatusCode::UNPROCESSABLE_ENTITY, error),
    };
    let audio_index = body
        .audio
        .filter(|index| *index < media.audio.len())
        .or_else(|| media.audio.iter().position(|audio| audio.default))
        .or((!media.audio.is_empty()).then_some(0));
    let audio = audio_index.and_then(|index| media.audio.get(index));
    let (hevc, copy_audio) = match probe::verdict(&media, audio, body.client) {
        Verdict::Plays { hevc, copy_audio } => (hevc, copy_audio),
        Verdict::Refused(reason) => return problem(StatusCode::UNPROCESSABLE_ENTITY, reason),
    };
    let tv_playing = streams.bus.now_playing().is_some();
    {
        let mut live = streams.live.lock().unwrap();
        if let Some(replaced) = body.replaces.as_ref().and_then(|id| live.remove(id)) {
            replaced.session.close();
        }
        let limit = if tv_playing { 1 } else { MOST_SESSIONS };
        let watching = live.values().filter(|live| live.session.idle() < REST).count();
        if watching >= limit {
            let reason = if tv_playing {
                "The TV is playing, so only one phone can watch at a time"
            } else {
                "Three phones are already watching"
            };
            return problem(StatusCode::TOO_MANY_REQUESTS, reason.to_string());
        }
    }
    let plan = Plan::new(&index);
    let duration = plan.duration;
    let start = body
        .start
        .filter(|at| at.is_finite() && *at >= 0.0 && *at < duration)
        .unwrap_or_else(|| resume_at(&streams.shared.progress(), &chosen.file, duration));
    let id = new_id();
    let session = Session::new(
        plan,
        streams.root.join(&id),
        Spec {
            source: chosen.file.clone(),
            audio: audio.map(|audio| audio.index),
            hevc,
            copy_audio: copy_audio || audio.is_none(),
        },
    );
    session.start(start);
    let subtitles = subtitle_list(&streams, &chosen.file, &media);
    let delay = streams.shared.subtitles().file_delay(&chosen.file);
    if let Some(source) = subtitles
        .iter()
        .find(|subtitle| subtitle.default)
        .and_then(|subtitle| subtitle.source.clone())
    {
        let warming = streams.clone();
        tokio::spawn(async move {
            let _ = caption(&warming, source, delay).await;
        });
    }
    let video = media.video.as_ref();
    let bitrate = media.bitrate;
    let live = Arc::new(Live {
        session,
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
            episode: chosen.season.zip(chosen.episode),
        }),
        chapter_credits: media.credits,
        marks: OnceCell::new(),
    });
    let credits = live.credits();
    streams.live.lock().unwrap().insert(id.clone(), live);
    let art = format!("/api/{}s/{}", chosen.kind, chosen.key);
    Json(Opened {
        playlist: format!("/api/stream/{id}/main.m3u8"),
        id,
        kind: chosen.kind,
        key: chosen.key,
        title: chosen.title,
        caption: chosen.caption,
        season: chosen.season,
        episode: chosen.episode,
        poster: format!("{art}/poster"),
        backdrop: format!("{art}/backdrop"),
        start,
        duration,
        audio: media.audio.clone(),
        audio_index,
        converted_audio: audio.is_some() && !copy_audio,
        subtitles,
        subtitle_delay: delay,
        credits,
        next: chosen.next,
        bitrate,
        heavy: bitrate.is_some_and(|rate| rate > CELLULAR_WARNING),
    })
    .into_response()
}

fn find(streams: &Streams, id: &str) -> Result<Arc<Live>, Response> {
    streams
        .live
        .lock()
        .unwrap()
        .get(id)
        .cloned()
        .ok_or_else(|| problem(StatusCode::NOT_FOUND, "this video closed; open it again".to_string()))
}

fn body(kind: &'static str, cache: &'static str, bytes: impl Into<axum::body::Body>) -> Response {
    let mut response = bytes.into().into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(kind));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

async fn close(State(streams): State<Streams>, Path(id): Path<String>) -> Response {
    if let Some(live) = streams.live.lock().unwrap().remove(&id) {
        live.session.close();
    }
    StatusCode::NO_CONTENT.into_response()
}

fn quoted(text: &str) -> String {
    text.replace('"', "'")
}

async fn master(State(streams): State<Streams>, Path(id): Path<String>) -> Response {
    let live = match find(&streams, &id) {
        Ok(live) => live,
        Err(response) => return response,
    };
    let codecs = match live.session.init(live.start).await {
        Ok((_, codecs)) => codecs,
        Err(error) => return problem(StatusCode::UNPROCESSABLE_ENTITY, error),
    };
    let mut text = String::from("#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-INDEPENDENT-SEGMENTS\n");
    let supported: Vec<&Subtitle> = live.subtitles.iter().filter(|subtitle| subtitle.supported).collect();
    let mut names: Vec<String> = Vec::new();
    for subtitle in &supported {
        let mut name = quoted(&subtitle.name);
        let base = name.clone();
        let mut copy = 2;
        while names.contains(&name) {
            name = format!("{base} {copy}");
            copy += 1;
        }
        names.push(name.clone());
        let _ = write!(
            text,
            "#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs\",NAME=\"{name}\",{}DEFAULT={},AUTOSELECT={},FORCED={},URI=\"subtitles/{}.m3u8\"\n",
            subtitle
                .language
                .as_ref()
                .map(|language| format!("LANGUAGE=\"{}\",", quoted(language)))
                .unwrap_or_default(),
            if subtitle.default { "YES" } else { "NO" },
            if subtitle.default || subtitle.forced {
                "YES"
            } else {
                "NO"
            },
            if subtitle.forced { "YES" } else { "NO" },
            subtitle.id
        );
    }
    let _ = write!(text, "#EXT-X-STREAM-INF:BANDWIDTH={}", live.bandwidth * 6 / 5);
    if let Some((width, height)) = live.resolution {
        let _ = write!(text, ",RESOLUTION={width}x{height}");
    }
    if let Some(codecs) = codecs {
        let _ = write!(text, ",CODECS=\"{codecs}\"");
    }
    if !supported.is_empty() {
        text.push_str(",SUBTITLES=\"subs\"");
    }
    text.push_str("\nvideo.m3u8\n");
    body(PLAYLIST, "no-store", text)
}

async fn media_playlist(State(streams): State<Streams>, Path(id): Path<String>) -> Response {
    match find(&streams, &id) {
        Ok(live) => body(PLAYLIST, "no-store", live.session.plan.media_playlist(live.start)),
        Err(response) => response,
    }
}

async fn init(State(streams): State<Streams>, Path(id): Path<String>) -> Response {
    let live = match find(&streams, &id) {
        Ok(live) => live,
        Err(response) => return response,
    };
    match live.session.init(live.start).await {
        Ok((bytes, _)) => body("video/mp4", "private, max-age=3600", bytes.as_ref().clone()),
        Err(error) => problem(StatusCode::UNPROCESSABLE_ENTITY, error),
    }
}

async fn segment(State(streams): State<Streams>, Path((id, name)): Path<(String, String)>) -> Response {
    let live = match find(&streams, &id) {
        Ok(live) => live,
        Err(response) => return response,
    };
    let Some(number) = name
        .strip_suffix(".m4s")
        .and_then(|number| number.parse::<usize>().ok())
    else {
        return problem(StatusCode::NOT_FOUND, format!("no segment {name}"));
    };
    match live.session.segment(number).await {
        Ok(bytes) => body("video/iso.segment", "private, max-age=3600", bytes),
        Err(error) => problem(StatusCode::SERVICE_UNAVAILABLE, error),
    }
}

async fn subtitles(State(streams): State<Streams>, Path((id, name)): Path<(String, String)>) -> Response {
    let live = match find(&streams, &id) {
        Ok(live) => live,
        Err(response) => return response,
    };
    let (number, playlist) = match (name.strip_suffix(".m3u8"), name.strip_suffix(".vtt")) {
        (Some(number), _) => (number, true),
        (_, Some(number)) => (number, false),
        _ => return problem(StatusCode::NOT_FOUND, format!("no subtitles {name}")),
    };
    let Some(subtitle) = number
        .parse::<usize>()
        .ok()
        .and_then(|number| live.subtitles.get(number))
        .filter(|subtitle| subtitle.supported)
    else {
        return problem(StatusCode::NOT_FOUND, format!("no subtitles {name}"));
    };
    if playlist {
        return body(
            PLAYLIST,
            "no-store",
            plan::subtitle_playlist(live.duration, &format!("{}.vtt", subtitle.id)),
        );
    }
    let Some(source) = subtitle.source.clone() else {
        return problem(StatusCode::NOT_FOUND, format!("no subtitles {name}"));
    };
    let text = match caption(&streams, source, live.delay).await {
        Ok(text) => text,
        Err(error) => return problem(StatusCode::UNPROCESSABLE_ENTITY, error),
    };
    body(
        "text/vtt; charset=utf-8",
        "private, max-age=3600",
        text.as_ref().clone(),
    )
}

async fn caption(streams: &Streams, source: Source, delay: f64) -> Result<Arc<String>, String> {
    let key = (source.clone(), (delay * 1000.0).round() as i64);
    let mut converted = {
        let mut captions = streams.captions.lock().unwrap();
        match captions.get(&key) {
            Some(converted) => converted.clone(),
            None => {
                let (done, converted) = watch::channel(None);
                captions.insert(key.clone(), converted.clone());
                let captions = streams.captions.clone();
                tokio::spawn(async move {
                    let result = vtt::convert(&source, delay).await.map(Arc::new);
                    if result.is_err() {
                        captions.lock().unwrap().remove(&key);
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
    let live = match find(&streams, &id) {
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
    let live = match find(&streams, &id) {
        Ok(live) => live,
        Err(response) => return response,
    };
    live.session.keep_alive();
    if !report.position.is_finite() || !report.played.is_finite() {
        return problem(
            StatusCode::BAD_REQUEST,
            "position and played must be numbers".to_string(),
        );
    }
    let request = Request::Phone(Report {
        device: live.device.clone(),
        session: id,
        file: live.file.clone(),
        position: report.position.clamp(0.0, live.duration),
        duration: live.duration,
        credits: live.credits(),
        played: report.played.max(0.0),
        moment: report.state,
    });
    api::ask_app(&streams.app, request).await
}
