use crate::artwork;
use crate::bus::{Bus, Event};
use crate::details::{self, Details};
use crate::intent::{self, Control, Pick};
use crate::library::{Episode, Item, Library, Movie, Show};
use crate::progress::Progress;
use crate::tags::Tags;
use crate::tmdb::Kind;
use crate::watching::{self, Current};
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::path::{Path as FsPath, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::{broadcast, oneshot};

const ANSWER_TIME: Duration = Duration::from_secs(5);
const NEWEST: usize = 12;
const IMAGE_CACHE: &str = "public, max-age=86400";
const TRY_AGAIN_LATER: u16 = 1013;

pub struct Config {
    pub address: String,
    pub phone_microphone: Option<String>,
}

#[derive(Debug)]
pub enum Request {
    Understand(String),
    Play(Pick),
    Control(Control),
    PlayPause,
    Seek(Seek),
    Listen,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seek {
    To(f64),
    By(f64),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Outcome {
    pub message: String,
    pub ok: bool,
}

pub struct Pending {
    pub request: Request,
    pub reply: oneshot::Sender<Outcome>,
}

pub type Tv = Arc<dyn Fn(Pending) + Send + Sync>;

#[derive(Default)]
pub struct Shared {
    library: RwLock<Arc<Library>>,
    progress: RwLock<Arc<Progress>>,
}

impl Shared {
    pub fn set_library(&self, library: &Library) {
        *self.library.write().unwrap() = Arc::new(library.clone());
    }

    pub fn set_progress(&self, progress: &Progress) {
        *self.progress.write().unwrap() = Arc::new(progress.clone());
    }

    fn library(&self) -> Arc<Library> {
        self.library.read().unwrap().clone()
    }

    fn progress(&self) -> Arc<Progress> {
        self.progress.read().unwrap().clone()
    }
}

#[derive(Clone)]
struct Api {
    shared: Arc<Shared>,
    root: PathBuf,
    bus: Bus,
    tv: Tv,
    microphone: Option<Arc<Microphone>>,
}

impl Api {
    fn art(&self) -> PathBuf {
        self.root.join(artwork::DIR)
    }
}

struct Microphone {
    device: String,
    busy: tokio::sync::Mutex<()>,
}

pub fn spawn(config: Config, shared: Arc<Shared>, root: PathBuf, bus: Bus, tv: Tv) {
    let api = Api {
        shared,
        root,
        bus,
        tv,
        microphone: config.phone_microphone.map(|device| {
            Arc::new(Microphone {
                device,
                busy: tokio::sync::Mutex::new(()),
            })
        }),
    };
    let address = config.address;
    let spawned = thread::Builder::new().name("api".to_string()).spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build();
        let runtime = match runtime {
            Ok(runtime) => runtime,
            Err(error) => return eprintln!("api: {error}"),
        };
        runtime.block_on(async move {
            let listener = match TcpListener::bind(&address).await {
                Ok(listener) => listener,
                Err(error) => return eprintln!("api {address}: {error}"),
            };
            eprintln!("api on http://{address}/api");
            if let Err(error) = axum::serve(listener, router(api)).await {
                eprintln!("api: {error}");
            }
        });
    });
    if let Err(error) = spawned {
        eprintln!("api: {error}");
    }
}

fn router(api: Api) -> Router {
    Router::new()
        .route("/api/library", get(library))
        .route("/api/search", get(search))
        .route("/api/movies/{key}", get(movie))
        .route("/api/movies/{key}/backdrop", get(movie_backdrop))
        .route("/api/movies/{key}/poster", get(movie_poster))
        .route("/api/shows/{key}", get(show))
        .route("/api/shows/{key}/backdrop", get(show_backdrop))
        .route("/api/shows/{key}/poster", get(show_poster))
        .route("/api/shows/{key}/episodes/{season}/{episode}/still", get(still))
        .route("/api/play", post(play))
        .route("/api/command", post(command))
        .route("/api/listen", post(listen))
        .route("/api/playback", get(playback))
        .route("/api/playback/seek", post(seek))
        .route("/api/playback/{control}", post(control))
        .route("/api/events", get(events))
        .route("/api/microphone", get(microphone))
        .with_state(api)
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct Ratings {
    imdb: Option<f64>,
    rotten_tomatoes: Option<u8>,
    fresh: Option<bool>,
    popcorn: Option<u8>,
    full: Option<bool>,
}

impl From<&crate::library::Scores> for Ratings {
    fn from(scores: &crate::library::Scores) -> Self {
        Self {
            imdb: scores.imdb.map(|tenths| f64::from(tenths.0) / 10.0),
            rotten_tomatoes: scores.rt,
            fresh: scores.rt.map(|_| crate::details::Ratings::from(scores).fresh),
            popcorn: scores.popcorn,
            full: scores.popcorn.map(|_| crate::details::Ratings::from(scores).full),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum CardKind {
    Movie,
    Show,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct Card {
    kind: CardKind,
    key: String,
    title: String,
    year: Option<u16>,
    subtitle: String,
    poster: Option<String>,
    backdrop: Option<String>,
    ratings: Ratings,
}

fn movie_card(root: &FsPath, movie: &Movie) -> Card {
    Card {
        kind: CardKind::Movie,
        key: movie.key.clone(),
        title: movie.title.clone(),
        year: movie.year,
        subtitle: movie.year.map(|year| year.to_string()).unwrap_or_default(),
        poster: movie.poster(root).map(|_| format!("/api/movies/{}/poster", movie.key)),
        backdrop: movie.tmdb.map(|_| format!("/api/movies/{}/backdrop", movie.key)),
        ratings: Ratings::from(&movie.ratings),
    }
}

fn show_card(root: &FsPath, show: &Show) -> Card {
    Card {
        kind: CardKind::Show,
        key: show.key.clone(),
        title: show.title.clone(),
        year: show.year,
        subtitle: show.subtitle(),
        poster: show.poster(root).map(|_| format!("/api/shows/{}/poster", show.key)),
        backdrop: show.tmdb.map(|_| format!("/api/shows/{}/backdrop", show.key)),
        ratings: Ratings::from(&show.ratings),
    }
}

fn card(root: &FsPath, library: &Library, item: Item) -> Card {
    match item {
        Item::Movie(index) => movie_card(root, &library.movies[index]),
        Item::Show(index) => show_card(root, &library.shows[index]),
    }
}

#[derive(Serialize)]
struct WatchingCard {
    #[serde(flatten)]
    card: Card,
    season: Option<u32>,
    episode: Option<u32>,
    detail: String,
    progress: f32,
}

fn watching_card(root: &FsPath, library: &Library, current: &Current) -> WatchingCard {
    let episode = match (current.item, current.episode) {
        (Item::Show(index), Some((season, number))) => library.shows[index].episode(season, number),
        _ => None,
    };
    let parts = [
        episode.map(Episode::short_label),
        current.remaining().map(details::time_left),
    ];
    let detail: Vec<_> = parts.into_iter().flatten().collect();
    WatchingCard {
        card: card(root, library, current.item),
        season: current.episode.map(|(season, _)| season),
        episode: current.episode.map(|(_, number)| number),
        detail: detail.join(" · "),
        progress: current.fraction(),
    }
}

#[derive(Serialize)]
struct LibraryView {
    watching: Vec<WatchingCard>,
    newest: Vec<Card>,
    shows: Vec<Card>,
    movies: Vec<Card>,
}

async fn library(State(api): State<Api>) -> Json<LibraryView> {
    let library = api.shared.library();
    let progress = api.shared.progress();
    Json(LibraryView {
        watching: watching::current(&library, &progress)
            .iter()
            .map(|current| watching_card(&api.root, &library, current))
            .collect(),
        newest: library
            .newest(NEWEST)
            .into_iter()
            .map(|item| card(&api.root, &library, item))
            .collect(),
        shows: library.shows.iter().map(|show| show_card(&api.root, show)).collect(),
        movies: library
            .movies
            .iter()
            .map(|movie| movie_card(&api.root, movie))
            .collect(),
    })
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
}

#[derive(Serialize)]
struct Results {
    results: Vec<Card>,
}

async fn search(State(api): State<Api>, Query(query): Query<SearchQuery>) -> Json<Results> {
    let library = api.shared.library();
    let results = intent::search(&library, &query.q)
        .into_iter()
        .map(|item| card(&api.root, &library, item))
        .collect();
    Json(Results { results })
}

#[derive(Serialize)]
struct TagsView {
    video: Option<String>,
    resolution: Option<String>,
    audio: Option<String>,
}

impl From<&Tags> for TagsView {
    fn from(tags: &Tags) -> Self {
        Self {
            video: tags.video.clone(),
            resolution: tags.resolution.clone(),
            audio: tags.audio.clone(),
        }
    }
}

#[derive(Serialize)]
struct Watch {
    details: String,
    runtime_minutes: Option<u32>,
    stopped_at: Option<f64>,
    duration: Option<f64>,
    tags: TagsView,
}

fn watch(progress: &Progress, file: &std::path::Path, tags: &Tags, runtime: Option<u32>) -> Watch {
    let watched = progress.files.get(file).copied().unwrap_or_default();
    let duration = watched.duration.or(runtime.map(|minutes| f64::from(minutes) * 60.0));
    Watch {
        details: Details {
            episode: None,
            tags: Some(tags),
            duration,
            stopped_at: watched.stopped_at,
            download: "",
        }
        .line(),
        runtime_minutes: runtime,
        stopped_at: watched.stopped_at,
        duration,
        tags: TagsView::from(tags),
    }
}

#[derive(Serialize)]
struct MovieView {
    #[serde(flatten)]
    card: Card,
    #[serde(flatten)]
    watch: Watch,
}

async fn movie(State(api): State<Api>, Path(key): Path<String>) -> Response {
    let library = api.shared.library();
    let Some(movie) = library.movies.iter().find(|movie| movie.key == key) else {
        return not_found(format!("no movie {key}"));
    };
    let progress = api.shared.progress();
    Json(MovieView {
        card: movie_card(&api.root, movie),
        watch: watch(&progress, &movie.path, &movie.tags, movie.runtime),
    })
    .into_response()
}

#[derive(Serialize)]
struct EpisodeView {
    season: u32,
    episode: u32,
    title: Option<String>,
    label: String,
    short_label: String,
    still: Option<String>,
    #[serde(flatten)]
    watch: Watch,
}

#[derive(Serialize)]
struct SeasonView {
    number: u32,
    label: String,
    episodes: Vec<EpisodeView>,
}

#[derive(Serialize)]
struct Next {
    season: u32,
    episode: u32,
    label: String,
}

#[derive(Serialize)]
struct ShowView {
    #[serde(flatten)]
    card: Card,
    next: Option<Next>,
    seasons: Vec<SeasonView>,
}

fn episode_view(show: &Show, episode: &Episode, progress: &Progress) -> EpisodeView {
    EpisodeView {
        season: episode.season,
        episode: episode.number,
        title: episode.title.clone(),
        label: episode.label(),
        short_label: episode.short_label(),
        still: show.tmdb.map(|_| {
            format!(
                "/api/shows/{}/episodes/{}/{}/still",
                show.key, episode.season, episode.number
            )
        }),
        watch: watch(progress, &episode.path, &episode.tags, episode.runtime),
    }
}

async fn show(State(api): State<Api>, Path(key): Path<String>) -> Response {
    let library = api.shared.library();
    let Some(show) = library.shows.iter().find(|show| show.key == key) else {
        return not_found(format!("no show {key}"));
    };
    let progress = api.shared.progress();
    let next = progress.resume_episode(show).map(|episode| Next {
        season: episode.season,
        episode: episode.number,
        label: episode.short_label(),
    });
    let seasons = show
        .seasons
        .iter()
        .map(|season| SeasonView {
            number: season.number,
            label: season.label(),
            episodes: season
                .episodes
                .iter()
                .map(|episode| episode_view(show, episode, &progress))
                .collect(),
        })
        .collect();
    Json(ShowView {
        card: show_card(&api.root, show),
        next,
        seasons,
    })
    .into_response()
}

async fn movie_backdrop(State(api): State<Api>, Path(key): Path<String>, headers: HeaderMap) -> Response {
    let library = api.shared.library();
    let id = library
        .movies
        .iter()
        .find(|movie| movie.key == key)
        .and_then(|movie| movie.tmdb);
    match id {
        Some(id) => image(artwork::backdrop(&api.art(), Kind::Movie, id), &headers).await,
        None => not_found(format!("no backdrop for movie {key}")),
    }
}

async fn show_backdrop(State(api): State<Api>, Path(key): Path<String>, headers: HeaderMap) -> Response {
    let library = api.shared.library();
    let id = library
        .shows
        .iter()
        .find(|show| show.key == key)
        .and_then(|show| show.tmdb);
    match id {
        Some(id) => image(artwork::backdrop(&api.art(), Kind::Tv, id), &headers).await,
        None => not_found(format!("no backdrop for show {key}")),
    }
}

async fn movie_poster(State(api): State<Api>, Path(key): Path<String>, headers: HeaderMap) -> Response {
    let library = api.shared.library();
    let poster = library
        .movies
        .iter()
        .find(|movie| movie.key == key)
        .and_then(|movie| movie.poster(&api.root));
    match poster {
        Some(poster) => image(api.root.join(poster), &headers).await,
        None => not_found(format!("no poster for movie {key}")),
    }
}

async fn show_poster(State(api): State<Api>, Path(key): Path<String>, headers: HeaderMap) -> Response {
    let library = api.shared.library();
    let poster = library
        .shows
        .iter()
        .find(|show| show.key == key)
        .and_then(|show| show.poster(&api.root));
    match poster {
        Some(poster) => image(api.root.join(poster), &headers).await,
        None => not_found(format!("no poster for show {key}")),
    }
}

async fn still(
    State(api): State<Api>,
    Path((key, season, episode)): Path<(String, u32, u32)>,
    headers: HeaderMap,
) -> Response {
    let library = api.shared.library();
    let show = library.shows.iter().find(|show| show.key == key);
    let id = show
        .filter(|show| show.episode(season, episode).is_some())
        .and_then(|show| show.tmdb);
    match id {
        Some(id) => image(artwork::still(&api.art(), id, season, episode), &headers).await,
        None => not_found(format!("no still for {key} season {season} episode {episode}")),
    }
}

async fn image(path: PathBuf, headers: &HeaderMap) -> Response {
    let Ok(metadata) = tokio::fs::metadata(&path).await else {
        return not_found("no such image".to_string());
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_secs());
    let tag = format!("\"{:x}-{:x}\"", metadata.len(), modified);
    let cached = headers
        .get(header::IF_NONE_MATCH)
        .is_some_and(|value| value.as_bytes() == tag.as_bytes());
    let cache_headers = [(header::CACHE_CONTROL, IMAGE_CACHE.to_string()), (header::ETAG, tag)];
    if cached {
        return (StatusCode::NOT_MODIFIED, cache_headers).into_response();
    }
    match tokio::fs::read(&path).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "image/jpeg".to_string())], cache_headers, bytes).into_response(),
        Err(error) => problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{}: {error}", path.display()),
        ),
    }
}

#[derive(Deserialize)]
struct PlayBody {
    movie: Option<String>,
    show: Option<String>,
    season: Option<u32>,
    episode: Option<u32>,
}

async fn play(State(api): State<Api>, Json(body): Json<PlayBody>) -> Response {
    let library = api.shared.library();
    let pick = match (body.movie, body.show) {
        (Some(key), None) if library.movies.iter().any(|movie| movie.key == key) => Pick::Movie(key),
        (None, Some(key)) if library.shows.iter().any(|show| show.key == key) => Pick::Show {
            key,
            season: body.season,
            episode: body.episode,
        },
        (Some(key), None) | (None, Some(key)) => return not_found(format!("{key} is not in the library")),
        _ => return problem(StatusCode::BAD_REQUEST, "give either movie or show".to_string()),
    };
    ask(&api, Request::Play(pick)).await
}

#[derive(Deserialize)]
struct CommandBody {
    text: String,
}

async fn command(State(api): State<Api>, Json(body): Json<CommandBody>) -> Response {
    ask(&api, Request::Understand(body.text)).await
}

async fn listen(State(api): State<Api>) -> Response {
    ask(&api, Request::Listen).await
}

#[derive(Serialize)]
struct PlaybackView {
    now_playing: Option<crate::bus::NowPlaying>,
}

async fn playback(State(api): State<Api>) -> Json<PlaybackView> {
    Json(PlaybackView {
        now_playing: api.bus.now_playing(),
    })
}

async fn control(State(api): State<Api>, Path(name): Path<String>) -> Response {
    let request = match name.as_str() {
        "pause" => Request::Control(Control::Pause),
        "resume" => Request::Control(Control::Resume),
        "toggle" => Request::PlayPause,
        "stop" => Request::Control(Control::Stop),
        "mute" => Request::Control(Control::Mute),
        "unmute" => Request::Control(Control::Unmute),
        _ => return not_found(format!("no playback control {name}")),
    };
    ask(&api, request).await
}

#[derive(Deserialize)]
struct SeekBody {
    to: Option<f64>,
    by: Option<f64>,
}

async fn seek(State(api): State<Api>, Json(body): Json<SeekBody>) -> Response {
    let seek = match (body.to, body.by) {
        (Some(to), None) if to.is_finite() => Seek::To(to),
        (None, Some(by)) if by.is_finite() => Seek::By(by),
        _ => return problem(StatusCode::BAD_REQUEST, "give either to or by, in seconds".to_string()),
    };
    ask(&api, Request::Seek(seek)).await
}

async fn ask(api: &Api, request: Request) -> Response {
    let (reply, answer) = oneshot::channel();
    (api.tv)(Pending { request, reply });
    match tokio::time::timeout(ANSWER_TIME, answer).await {
        Ok(Ok(outcome)) => {
            let status = if outcome.ok {
                StatusCode::OK
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            };
            (status, Json(outcome)).into_response()
        }
        _ => problem(StatusCode::SERVICE_UNAVAILABLE, "the TV did not answer".to_string()),
    }
}

#[derive(Serialize)]
struct Problem {
    error: String,
}

fn problem(status: StatusCode, error: String) -> Response {
    (status, Json(Problem { error })).into_response()
}

fn not_found(error: String) -> Response {
    problem(StatusCode::NOT_FOUND, error)
}

async fn events(State(api): State<Api>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| stream_events(socket, api.bus))
}

async fn stream_events(mut socket: WebSocket, bus: Bus) {
    let (now_playing, mut events) = bus.subscribe();
    if send_event(&mut socket, &Event::Playback { now_playing }).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(event) => if send_event(&mut socket, &event).await.is_err() { return },
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let now_playing = bus.now_playing();
                    if send_event(&mut socket, &Event::Playback { now_playing }).await.is_err() { return }
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            message = socket.recv() => match message {
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return,
                Some(Ok(_)) => {}
            },
        }
    }
}

async fn send_event(socket: &mut WebSocket, event: &Event) -> Result<(), axum::Error> {
    let text = serde_json::to_string(event).unwrap_or_default();
    socket.send(Message::Text(text.into())).await
}

async fn microphone(State(api): State<Api>, upgrade: WebSocketUpgrade) -> Response {
    match api.microphone {
        Some(microphone) => upgrade.on_upgrade(move |socket| stream_microphone(socket, microphone)),
        None => not_found("this TV takes no phone microphone".to_string()),
    }
}

async fn stream_microphone(mut socket: WebSocket, microphone: Arc<Microphone>) {
    let Ok(_streaming) = microphone.busy.try_lock() else {
        let frame = CloseFrame {
            code: TRY_AGAIN_LATER,
            reason: "another phone is streaming".into(),
        };
        let _ = socket.send(Message::Close(Some(frame))).await;
        return;
    };
    let player = tokio::process::Command::new("aplay")
        .args(["-q", "-D", &microphone.device])
        .args(["-t", "raw", "-f", "S16_LE", "-c", "1", "-r", "16000", "-B", "200000"])
        .stdin(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let mut player = match player {
        Ok(player) => player,
        Err(error) => return eprintln!("phone microphone: aplay: {error}"),
    };
    eprintln!("phone microphone streaming");
    if let Some(mut input) = player.stdin.take() {
        while let Some(Ok(message)) = socket.recv().await {
            match message {
                Message::Binary(bytes) => {
                    if let Err(error) = input.write_all(&bytes).await {
                        eprintln!("phone microphone: {error}");
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    }
    let _ = player.wait().await;
    eprintln!("phone microphone stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{NowPlaying, Voice};
    use crate::import::library_from_paths;
    use crate::progress::Position;
    use serde_json::{Value, json};
    use std::sync::mpsc;

    struct Server {
        base: String,
        bus: Bus,
        asked: mpsc::Receiver<String>,
        _runtime: tokio::runtime::Runtime,
    }

    fn library() -> Library {
        let mut library = library_from_paths(&[
            "Series/Outlander/Outlander.S01E01.mkv",
            "Series/Outlander/Outlander.S01E02.mkv",
            "Series/Outlander/Outlander.S02E01.mkv",
            "Series/The Bear/The.Bear.S01E01.mkv",
            "Movies/Dune (2021)/Dune.2021.1080p.mkv",
            "Movies/Up (2009)/Up.2009.mkv",
        ]);
        library.movies[0].tmdb = Some(438_631);
        library
    }

    fn serve(root: PathBuf, microphone: Option<String>) -> Server {
        let shared = Arc::new(Shared::default());
        shared.set_library(&library());
        let mut progress = Progress::default();
        let position = Position {
            season: 1,
            episode: 2,
            finished: true,
        };
        progress.shows.insert("outlander".to_string(), position);
        progress.watching.played(watching::Kind::Series, "outlander", 1);
        progress.watching.played(watching::Kind::Movie, "dune-2021", 2);
        let dune = library().movies[0].path.clone();
        let place = crate::progress::Place {
            position: 3600.0,
            duration: 9000.0,
        };
        progress.record(&dune, place);
        shared.set_progress(&progress);
        let bus = Bus::default();
        let (asked, received) = mpsc::channel();
        let tv: Tv = Arc::new(move |pending: Pending| {
            let _ = asked.send(format!("{:?}", pending.request));
            let ok = !matches!(pending.request, Request::Understand(_));
            let _ = pending.reply.send(Outcome {
                message: "done".to_string(),
                ok,
            });
        });
        let api = Api {
            shared,
            root,
            bus: bus.clone(),
            tv,
            microphone: microphone.map(|device| {
                Arc::new(Microphone {
                    device,
                    busy: tokio::sync::Mutex::new(()),
                })
            }),
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        runtime.spawn(async move { axum::serve(listener, router(api)).await });
        Server {
            base,
            bus,
            asked: received,
            _runtime: runtime,
        }
    }

    fn agent() -> ureq::Agent {
        ureq::Agent::config_builder().http_status_as_error(false).build().into()
    }

    fn get(server: &Server, path: &str) -> (u16, Value) {
        let mut response = agent().get(format!("{}{path}", server.base)).call().unwrap();
        let status = response.status().as_u16();
        (status, response.body_mut().read_json().unwrap_or(Value::Null))
    }

    fn post(server: &Server, path: &str, body: Value) -> (u16, Value) {
        let mut response = agent().post(format!("{}{path}", server.base)).send_json(body).unwrap();
        let status = response.status().as_u16();
        (status, response.body_mut().read_json().unwrap_or(Value::Null))
    }

    fn no_root() -> PathBuf {
        PathBuf::from("/nonexistent")
    }

    #[test]
    fn the_library_lists_shows_and_movies_with_their_artwork() {
        let server = serve(no_root(), None);
        let (status, body) = get(&server, "/api/library");
        assert_eq!(status, 200);
        let titles = |list: &str| -> Vec<String> {
            body[list]
                .as_array()
                .unwrap()
                .iter()
                .map(|card| card["title"].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(titles("shows").len(), 2);
        assert!(titles("movies").contains(&"Dune".to_string()));
        assert_eq!(titles("newest").len(), 4);
        let dune = body["movies"]
            .as_array()
            .unwrap()
            .iter()
            .find(|card| card["title"] == "Dune")
            .unwrap();
        assert_eq!(dune["kind"], "movie");
        assert_eq!(dune["backdrop"], "/api/movies/dune-2021/backdrop");
        assert_eq!(dune["poster"], Value::Null);
        let watching: Vec<(&str, &str)> = body["watching"]
            .as_array()
            .unwrap()
            .iter()
            .map(|card| (card["key"].as_str().unwrap(), card["detail"].as_str().unwrap()))
            .collect();
        assert_eq!(watching, [("dune-2021", "1h 30m left"), ("outlander", "S2 E1")]);
        assert_eq!(body["watching"][0]["progress"], 0.4);
        assert_eq!(body["watching"][1]["season"], 2);
    }

    #[test]
    fn search_finds_titles_while_they_are_typed() {
        let server = serve(no_root(), None);
        let titles = |query: &str| -> Vec<String> {
            get(&server, &format!("/api/search?q={query}")).1["results"]
                .as_array()
                .unwrap()
                .iter()
                .map(|card| card["title"].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(titles("outl"), ["Outlander"]);
        assert_eq!(titles("bear"), ["The Bear"]);
        assert_eq!(titles("outlandr"), ["Outlander"]);
        assert!(titles("zzz").is_empty());
    }

    #[test]
    fn a_show_lists_its_seasons_and_where_to_carry_on() {
        let server = serve(no_root(), None);
        let (status, body) = get(&server, "/api/shows/outlander");
        assert_eq!(status, 200);
        assert_eq!(body["next"], json!({ "season": 2, "episode": 1, "label": "S2 E1" }));
        assert_eq!(body["seasons"].as_array().unwrap().len(), 2);
        assert_eq!(body["seasons"][0]["episodes"][1]["label"], "Episode 2");
        assert_eq!(get(&server, "/api/shows/nothing").0, 404);
        assert_eq!(get(&server, "/api/movies/dune-2021").1["title"], "Dune");
    }

    #[test]
    fn playing_goes_to_the_tv_and_reports_its_answer() {
        let server = serve(no_root(), None);
        let (status, body) = post(
            &server,
            "/api/play",
            json!({ "show": "outlander", "season": 2, "episode": 1 }),
        );
        assert_eq!((status, body), (200, json!({ "message": "done", "ok": true })));
        assert_eq!(
            server.asked.recv().unwrap(),
            format!(
                "{:?}",
                Request::Play(Pick::Show {
                    key: "outlander".to_string(),
                    season: Some(2),
                    episode: Some(1)
                })
            )
        );
        assert_eq!(post(&server, "/api/play", json!({ "movie": "jaws" })).0, 404);
        assert_eq!(post(&server, "/api/play", json!({})).0, 400);
        assert_eq!(post(&server, "/api/playback/pause", json!({})).0, 200);
        assert_eq!(server.asked.recv().unwrap(), "Control(Pause)");
        assert_eq!(post(&server, "/api/playback/seek", json!({ "by": -30.0 })).0, 200);
        assert_eq!(server.asked.recv().unwrap(), "Seek(By(-30.0))");
        assert_eq!(post(&server, "/api/playback/dance", json!({})).0, 404);
        assert_eq!(post(&server, "/api/command", json!({ "text": "play" })).0, 422);
    }

    #[test]
    fn artwork_is_served_with_caching_headers() {
        let root = std::env::temp_dir().join(format!("semmeltv-api-art-{}", std::process::id()));
        let art = root.join(artwork::DIR);
        let file = artwork::backdrop(&art, Kind::Movie, 438_631);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"jpeg").unwrap();
        std::fs::write(artwork::poster(&art, Kind::Movie, 438_631), b"poster").unwrap();
        let server = serve(root.clone(), None);
        let url = format!("{}/api/movies/dune-2021/backdrop", server.base);
        let mut response = agent().get(&url).call().unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-type"], "image/jpeg");
        assert_eq!(response.headers()["cache-control"], IMAGE_CACHE);
        let tag = response.headers()["etag"].to_str().unwrap().to_string();
        assert_eq!(response.body_mut().read_to_vec().unwrap(), b"jpeg");
        let again = agent().get(&url).header("If-None-Match", &tag).call().unwrap();
        assert_eq!(again.status(), 304);
        assert_eq!(get(&server, "/api/movies/up-2009/backdrop").0, 404);
        let (_, dune) = get(&server, "/api/movies/dune-2021");
        assert_eq!(dune["poster"], "/api/movies/dune-2021/poster");
        let mut poster = agent()
            .get(format!("{}/api/movies/dune-2021/poster", server.base))
            .call()
            .unwrap();
        assert_eq!(poster.body_mut().read_to_vec().unwrap(), b"poster");
        assert_eq!(get(&server, "/api/movies/up-2009/poster").0, 404);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_event_stream_starts_with_playback_and_carries_voice_events() {
        let server = serve(no_root(), None);
        server.bus.playback(Some(NowPlaying {
            title: "Dune".to_string(),
            ..NowPlaying::default()
        }));
        let url = format!("{}/api/events", server.base.replace("http", "ws"));
        let (mut socket, _) = tungstenite::connect(url).unwrap();
        let mut next = || -> Value { serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap() };
        let first = next();
        assert_eq!(first["type"], "playback");
        assert_eq!(first["now_playing"]["title"], "Dune");
        server.bus.voice(Voice::Listening);
        assert_eq!(next(), json!({ "type": "voice", "state": "listening" }));
        server.bus.playback(None);
        assert_eq!(next(), json!({ "type": "playback", "now_playing": null }));
    }

    #[test]
    fn without_a_phone_microphone_its_socket_is_missing() {
        let server = serve(no_root(), None);
        let url = format!("{}/api/microphone", server.base.replace("http", "ws"));
        match tungstenite::connect(url) {
            Err(tungstenite::Error::Http(response)) => assert_eq!(response.status(), 404),
            other => panic!("expected a 404, got {:?}", other.map(|_| ())),
        }
    }
}
