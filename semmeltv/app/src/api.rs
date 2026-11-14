use crate::artwork;
use crate::audio::{self, Audio};
use crate::bus::{Bus, Event};
use crate::details::{self, Details};
use crate::downloads::remote::Downloads;
use crate::heatmap;
use crate::intent::{self, Control, Pick};
use crate::library::{Episode, Item, Library, Movie, Show};
use crate::manifest::{About, Date};
use crate::progress::Progress;
use crate::seek::Direction;
use crate::sidecar::{self, Sidecar};
use crate::screen;
use crate::storage;
use crate::stream;
use crate::subfetch::{self, Match, Wanted};
use crate::subtitles::{self, Subtitles};
use crate::tags::Tags;
use crate::tmdb::Kind;
use crate::tv::{self, Television};
use crate::watching::{self, Current};
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
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
    pub library: PathBuf,
    pub downloads: Option<Downloads>,
    pub stream: PathBuf,
}

#[derive(Debug)]
pub enum Request {
    Understand(String),
    Play(Pick),
    Control(Control),
    PlayPause,
    Seek(Seek),
    Speed(f64),
    Listen,
    Subtitles(subtitles::Change),
    Storage(storage::Change),
    Dismiss(watching::Kind, String),
    Mark(watching::Kind, String, Option<(u32, u32)>, watching::Mark),
    Phone(stream::Report),
    Screen(ScreenCommand),
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "action", rename_all = "lowercase", deny_unknown_fields)]
pub enum ScreenCommand {
    Flip {
        to: ScreenKind,
        device: screen::Device,
        position: Option<f64>,
    },
    Play {
        device: screen::Device,
        movie: Option<String>,
        show: Option<String>,
        season: Option<u32>,
        episode: Option<u32>,
    },
    Failed {
        device: screen::Device,
        reason: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenKind {
    Tv,
    Phone,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seek {
    To(f64),
    By(f64),
    Step(Direction),
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

pub type Ask = Arc<dyn Fn(Pending) + Send + Sync>;

#[derive(Default)]
pub struct Shared {
    subtitles: Subtitles,
    pub storage: storage::Board,
    library: RwLock<Arc<Library>>,
    progress: RwLock<Arc<Progress>>,
    sidecars: RwLock<HashMap<PathBuf, usize>>,
    scans: AtomicU64,
}

impl Shared {
    pub fn new(subtitles: Subtitles) -> Self {
        Self {
            subtitles,
            ..Self::default()
        }
    }

    pub fn set_library(self: &Arc<Self>, library: &Library) {
        let library = Arc::new(library.clone());
        *self.library.write().unwrap() = library.clone();
        let scan = self.scans.fetch_add(1, Ordering::SeqCst) + 1;
        let shared = self.clone();
        let spawned = thread::Builder::new().name("sidecars".to_string()).spawn(move || {
            let mut counts = HashMap::new();
            for path in library.paths() {
                if shared.scans.load(Ordering::SeqCst) != scan {
                    return;
                }
                counts.insert(path.clone(), sidecar::find(path).len());
            }
            *shared.sidecars.write().unwrap() = counts;
        });
        if let Err(error) = spawned {
            eprintln!("sidecars: {error}");
        }
    }

    fn recount(&self, video: &FsPath) {
        let count = sidecar::find(video).len();
        self.sidecars.write().unwrap().insert(video.to_path_buf(), count);
    }

    fn subtitle_count(&self, video: &FsPath) -> usize {
        let sidecars = self.sidecars.read().unwrap().get(video).copied().unwrap_or(0);
        sidecars + self.subtitles.embedded(video).unwrap_or(0)
    }

    pub fn set_progress(&self, progress: &Progress) {
        *self.progress.write().unwrap() = Arc::new(progress.clone());
    }

    pub(crate) fn library(&self) -> Arc<Library> {
        self.library.read().unwrap().clone()
    }

    pub(crate) fn progress(&self) -> Arc<Progress> {
        self.progress.read().unwrap().clone()
    }

    pub(crate) fn subtitles(&self) -> &Subtitles {
        &self.subtitles
    }
}

#[derive(Clone)]
struct Api {
    shared: Arc<Shared>,
    root: PathBuf,
    bus: Bus,
    app: Ask,
    microphone: Option<Arc<Microphone>>,
    television: Television,
    audio: Option<Audio>,
    library: PathBuf,
    downloads: Option<Downloads>,
    stream: PathBuf,
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

pub fn spawn(
    config: Config,
    shared: Arc<Shared>,
    root: PathBuf,
    bus: Bus,
    app: Ask,
    television: Television,
    audio: Option<Audio>,
) {
    let api = Api {
        shared,
        root,
        bus,
        app,
        microphone: config.phone_microphone.map(|device| {
            Arc::new(Microphone {
                device,
                busy: tokio::sync::Mutex::new(()),
            })
        }),
        television,
        audio,
        library: config.library,
        downloads: config.downloads,
        stream: config.stream,
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
        .route("/api/forgotten/dismiss", post(dismiss))
        .route("/api/watched", post(mark_watched))
        .route("/api/search", get(search))
        .route("/api/movies/{key}", get(movie))
        .route("/api/movies/{key}/backdrop", get(movie_backdrop))
        .route("/api/movies/{key}/poster", get(movie_poster))
        .route("/api/shows/{key}", get(show))
        .route("/api/shows/{key}/backdrop", get(show_backdrop))
        .route("/api/shows/{key}/poster", get(show_poster))
        .route("/api/shows/{key}/episodes/{season}/{episode}/still", get(still))
        .route("/api/people/{person}/photo", get(photo))
        .route("/api/play", post(play))
        .route("/api/command", post(command))
        .route("/api/listen", post(listen))
        .route("/api/playback", get(playback))
        .route("/api/playback/seek", post(seek))
        .route("/api/playback/skip", post(skip))
        .route("/api/playback/speed", post(speed))
        .route("/api/playback/{control}", post(control))
        .route("/api/tv/{control}", post(tv))
        .route("/api/events", get(events))
        .route("/api/screen", get(screen_view).post(screen_command))
        .route("/api/microphone", get(microphone))
        .route("/api/audio", get(audio_view).post(choose_audio))
        .route("/api/audio/pair", post(pair_headphones).delete(cancel_pairing))
        .route("/api/audio/headphones", delete(forget_headphones))
        .route("/api/audio/delay", post(delay_sound))
        .route("/api/volume/{change}", post(volume))
        .route("/api/subtitles/fetch", post(fetch_subtitles))
        .merge(subtitles::routes(api.shared.subtitles.clone(), api.app.clone()))
        .merge(tv::routes(api.television.clone()))
        .merge(crate::client_log::routes())
        .merge(storage::routes(api.shared.storage.clone(), api.app.clone()))
        .merge(stream::routes(
            api.shared.clone(),
            api.bus.clone(),
            api.app.clone(),
            api.stream.clone(),
        ))
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

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
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
    years: Option<String>,
    subtitle: String,
    poster: Option<String>,
    backdrop: Option<String>,
    ratings: Ratings,
    finished: bool,
}

fn movie_card(root: &FsPath, progress: &Progress, movie: &Movie) -> Card {
    Card {
        kind: CardKind::Movie,
        key: movie.key.clone(),
        title: movie.title.clone(),
        years: movie.year.map(|year| year.to_string()),
        subtitle: movie.year.map(|year| year.to_string()).unwrap_or_default(),
        poster: movie.poster(root).map(|_| format!("/api/movies/{}/poster", movie.key)),
        backdrop: movie.tmdb.map(|_| format!("/api/movies/{}/backdrop", movie.key)),
        ratings: Ratings::from(&movie.ratings),
        finished: watching::movie_finished(progress, movie),
    }
}

fn show_card(root: &FsPath, progress: &Progress, show: &Show) -> Card {
    Card {
        kind: CardKind::Show,
        key: show.key.clone(),
        title: show.title.clone(),
        years: show.years(),
        subtitle: show.subtitle(),
        poster: show.poster(root).map(|_| format!("/api/shows/{}/poster", show.key)),
        backdrop: show.tmdb.map(|_| format!("/api/shows/{}/backdrop", show.key)),
        ratings: Ratings::from(&show.ratings),
        finished: watching::show_finished(progress, show),
    }
}

fn card(root: &FsPath, progress: &Progress, library: &Library, item: Item) -> Card {
    match item {
        Item::Movie(index) => movie_card(root, progress, &library.movies[index]),
        Item::Show(index) => show_card(root, progress, &library.shows[index]),
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

fn watching_card(root: &FsPath, progress: &Progress, library: &Library, current: &Current) -> WatchingCard {
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
        card: card(root, progress, library, current.item),
        season: current.episode.map(|(season, _)| season),
        episode: current.episode.map(|(_, number)| number),
        detail: detail.join(" · "),
        progress: current.fraction(),
    }
}

#[derive(Serialize)]
struct LibraryView {
    watching: Vec<WatchingCard>,
    forgotten: Vec<WatchingCard>,
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
            .map(|current| watching_card(&api.root, &progress, &library, current))
            .collect(),
        forgotten: watching::forgotten(&library, &progress, watching::now())
            .iter()
            .map(|current| watching_card(&api.root, &progress, &library, current))
            .collect(),
        newest: library
            .newest(NEWEST)
            .into_iter()
            .map(|item| card(&api.root, &progress, &library, item))
            .collect(),
        shows: library
            .shows
            .iter()
            .map(|show| show_card(&api.root, &progress, show))
            .collect(),
        movies: library
            .movies
            .iter()
            .map(|movie| movie_card(&api.root, &progress, movie))
            .collect(),
    })
}

#[derive(Deserialize)]
struct Dismiss {
    kind: CardKind,
    key: String,
}

async fn dismiss(State(api): State<Api>, Json(body): Json<Dismiss>) -> Response {
    let kind = match body.kind {
        CardKind::Movie => watching::Kind::Movie,
        CardKind::Show => watching::Kind::Series,
    };
    ask_app(&api.app, Request::Dismiss(kind, body.key)).await
}

#[derive(Deserialize)]
struct MarkWatched {
    kind: CardKind,
    key: String,
    watched: bool,
    season: Option<u32>,
    episode: Option<u32>,
}

#[derive(Serialize)]
struct Marked {
    message: String,
    ok: bool,
    watched: bool,
}

fn is_watched(library: &Library, progress: &Progress, body: &MarkWatched) -> Option<bool> {
    match body.kind {
        CardKind::Movie => {
            let movie = library.movies.iter().find(|movie| movie.key == body.key)?;
            Some(watching::movie_finished(progress, movie))
        }
        CardKind::Show => {
            let show = library.shows.iter().find(|show| show.key == body.key)?;
            match (body.season, body.episode) {
                (Some(season), Some(number)) => Some(progress.watched(show, show.episode(season, number)?)),
                _ => Some(watching::show_finished(progress, show)),
            }
        }
    }
}

async fn mark_watched(State(api): State<Api>, Json(body): Json<MarkWatched>) -> Response {
    let kind = match body.kind {
        CardKind::Movie => watching::Kind::Movie,
        CardKind::Show => watching::Kind::Series,
    };
    let episode = match (body.kind, body.season, body.episode) {
        (CardKind::Show, Some(season), Some(number)) => Some((season, number)),
        (_, None, None) => None,
        _ => {
            return problem(
                StatusCode::BAD_REQUEST,
                "season and episode go together, and only with a show".to_string(),
            );
        }
    };
    let mark = if body.watched {
        watching::Mark::Watched
    } else {
        watching::Mark::Unwatched
    };
    let request = Request::Mark(kind, body.key.clone(), episode, mark);
    let Some(outcome) = answer(&api.app, request).await else {
        return unanswered();
    };
    let watched = is_watched(&api.shared.library(), &api.shared.progress(), &body);
    let marked = Marked {
        message: outcome.message,
        ok: outcome.ok && watched.is_some(),
        watched: watched.unwrap_or(false),
    };
    (status(marked.ok), Json(marked)).into_response()
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
    let progress = api.shared.progress();
    let results = intent::search(&library, &query.q)
        .into_iter()
        .map(|item| card(&api.root, &progress, &library, item))
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
    subtitles: usize,
}

fn watch(progress: &Progress, file: &std::path::Path, tags: &Tags, runtime: Option<u32>, subtitles: usize) -> Watch {
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
        subtitles,
    }
}

#[derive(Serialize)]
struct CertificateView {
    country: String,
    rating: String,
}

#[derive(Serialize)]
struct CastView {
    name: String,
    character: Option<String>,
    photo: Option<String>,
}

#[derive(Serialize)]
struct MoreScores {
    metacritic: Option<u8>,
    letterboxd: Option<f64>,
    ebert: Option<f64>,
}

#[derive(Serialize)]
struct AboutView {
    tagline: Option<String>,
    plot: Option<String>,
    genres: Vec<String>,
    original_title: Option<String>,
    language: Option<String>,
    released: Option<String>,
    first_aired: Option<String>,
    last_aired: Option<String>,
    seasons: Option<u32>,
    episodes: Option<u32>,
    networks: Vec<String>,
    directors: Vec<String>,
    creators: Vec<String>,
    certificate: Option<CertificateView>,
    trailer: Option<String>,
    imdb: Option<String>,
    tmdb: Option<String>,
    cast: Vec<CastView>,
    scores: MoreScores,
}

fn tenths(score: Option<crate::manifest::Tenths>) -> Option<f64> {
    score.map(|tenths| f64::from(tenths.0) / 10.0)
}

fn about_view(
    art: &FsPath,
    about: &About,
    kind: Kind,
    tmdb: Option<u64>,
    imdb: Option<&str>,
    scores: &crate::library::Scores,
) -> AboutView {
    let date = |date: Option<Date>| date.map(|date| date.to_string());
    let kind_path = match kind {
        Kind::Movie => "movie",
        Kind::Tv => "tv",
    };
    AboutView {
        tagline: about.tagline.clone(),
        plot: about.plot.clone(),
        genres: about.genres.clone(),
        original_title: about.original_title.clone(),
        language: about
            .language
            .as_deref()
            .map(|code| subtitles::language_name(code).map_or_else(|| code.to_string(), str::to_string)),
        released: date(about.released),
        first_aired: date(about.first_aired),
        last_aired: date(about.last_aired),
        seasons: about.seasons,
        episodes: about.episodes,
        networks: about.networks.clone(),
        directors: about.directors.clone(),
        creators: about.creators.clone(),
        certificate: about.certificate.as_ref().map(|certificate| CertificateView {
            country: certificate.country.clone(),
            rating: certificate.rating.clone(),
        }),
        trailer: about
            .trailer
            .as_ref()
            .map(|key| format!("https://www.youtube.com/watch?v={key}")),
        imdb: imdb.map(|id| format!("https://www.imdb.com/title/{id}/")),
        tmdb: tmdb.map(|id| format!("https://www.themoviedb.org/{kind_path}/{id}")),
        cast: about
            .cast
            .iter()
            .map(|member| CastView {
                name: member.name.clone(),
                character: member.character.clone(),
                photo: member
                    .person
                    .filter(|person| artwork::face(art, *person).is_file())
                    .map(|person| format!("/api/people/{person}/photo")),
            })
            .collect(),
        scores: MoreScores {
            metacritic: scores.metacritic,
            letterboxd: tenths(scores.letterboxd),
            ebert: tenths(scores.ebert),
        },
    }
}

#[derive(Serialize)]
struct Played {
    last_played: Option<u64>,
    last_played_label: Option<String>,
}

fn played(last_played: Option<u64>) -> Played {
    Played {
        last_played,
        last_played_label: last_played.map(|then| details::when(watching::now(), then)),
    }
}

#[derive(Serialize)]
struct MovieView {
    #[serde(flatten)]
    card: Card,
    #[serde(flatten)]
    watch: Watch,
    about: AboutView,
    #[serde(flatten)]
    played: Played,
}

async fn movie(State(api): State<Api>, Path(key): Path<String>) -> Response {
    let library = api.shared.library();
    let Some(movie) = library.movies.iter().find(|movie| movie.key == key) else {
        return not_found(format!("no movie {key}"));
    };
    let progress = api.shared.progress();
    let last_played = progress
        .last_played([&movie.path])
        .max(progress.watching.last_played(watching::Kind::Movie, &movie.key));
    Json(MovieView {
        card: movie_card(&api.root, &progress, movie),
        watch: watch(
            &progress,
            &movie.path,
            &movie.tags,
            movie.runtime,
            api.shared.subtitle_count(&movie.path),
        ),
        about: about_view(
            &api.art(),
            &movie.about,
            Kind::Movie,
            movie.tmdb,
            movie.imdb.as_deref(),
            &movie.ratings,
        ),
        played: played(last_played),
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
    ratings: Ratings,
    band: Option<String>,
    watched: bool,
    last_played: Option<u64>,
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
    title: Option<String>,
    stopped_at: Option<f64>,
    duration: Option<f64>,
}

#[derive(Serialize)]
struct ShowView {
    #[serde(flatten)]
    card: Card,
    next: Option<Next>,
    seasons: Vec<SeasonView>,
    about: AboutView,
    #[serde(flatten)]
    played: Played,
}

fn episode_view(shared: &Shared, show: &Show, episode: &Episode, progress: &Progress) -> EpisodeView {
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
        ratings: Ratings::from(&episode.ratings),
        band: episode
            .ratings
            .imdb
            .map(|rating| heatmap::band(rating).label.to_lowercase()),
        watched: progress.watched(show, episode),
        last_played: progress.last_played([&episode.path]),
        watch: watch(
            progress,
            &episode.path,
            &episode.tags,
            episode.runtime,
            shared.subtitle_count(&episode.path),
        ),
    }
}

async fn show(State(api): State<Api>, Path(key): Path<String>) -> Response {
    let library = api.shared.library();
    let Some(show) = library.shows.iter().find(|show| show.key == key) else {
        return not_found(format!("no show {key}"));
    };
    let progress = api.shared.progress();
    let next = progress.resume_episode(show).map(|episode| {
        let place = watch(&progress, &episode.path, &episode.tags, episode.runtime, 0);
        Next {
            season: episode.season,
            episode: episode.number,
            label: episode.short_label(),
            title: episode.title.clone(),
            stopped_at: place.stopped_at,
            duration: place.duration,
        }
    });
    let last_played = progress
        .last_played(show.episodes().map(|episode| &episode.path))
        .max(progress.watching.last_played(watching::Kind::Series, &show.key));
    let seasons = show
        .seasons
        .iter()
        .map(|season| SeasonView {
            number: season.number,
            label: season.label(),
            episodes: season
                .episodes
                .iter()
                .map(|episode| episode_view(&api.shared, show, episode, &progress))
                .collect(),
        })
        .collect();
    Json(ShowView {
        card: show_card(&api.root, &progress, show),
        next,
        seasons,
        about: about_view(
            &api.art(),
            &show.about,
            Kind::Tv,
            show.tmdb,
            show.imdb.as_deref(),
            &show.ratings,
        ),
        played: played(last_played),
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

async fn photo(State(api): State<Api>, Path(person): Path<u64>, headers: HeaderMap) -> Response {
    image(artwork::face(&api.art(), person), &headers).await
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

async fn tv(State(api): State<Api>, Path(name): Path<String>) -> Response {
    let control = match name.as_str() {
        "on" => Control::TvOn,
        "off" => Control::TvOff,
        "semmeltv" => Control::ShowSemmelTv,
        _ => return not_found(format!("no TV control {name}")),
    };
    ask(&api, Request::Control(control)).await
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

#[derive(Deserialize)]
struct SkipBody {
    direction: Direction,
}

async fn skip(State(api): State<Api>, Json(body): Json<SkipBody>) -> Response {
    ask(&api, Request::Seek(Seek::Step(body.direction))).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpeedBody {
    speed: f64,
}

async fn speed(State(api): State<Api>, Json(body): Json<SpeedBody>) -> Response {
    ask(&api, Request::Speed(body.speed)).await
}

async fn ask(api: &Api, request: Request) -> Response {
    ask_app(&api.app, request).await
}

async fn answer(app: &Ask, request: Request) -> Option<Outcome> {
    let (reply, answer) = oneshot::channel();
    app(Pending { request, reply });
    tokio::time::timeout(ANSWER_TIME, answer).await.ok()?.ok()
}

fn status(ok: bool) -> StatusCode {
    if ok {
        StatusCode::OK
    } else {
        StatusCode::UNPROCESSABLE_ENTITY
    }
}

fn unanswered() -> Response {
    problem(StatusCode::SERVICE_UNAVAILABLE, "the TV did not answer".to_string())
}

pub async fn ask_app(app: &Ask, request: Request) -> Response {
    match answer(app, request).await {
        Some(outcome) => (status(outcome.ok), Json(outcome)).into_response(),
        None => unanswered(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FetchBody {
    movie: Option<String>,
    show: Option<String>,
    season: Option<u32>,
    episode: Option<u32>,
    language: Option<String>,
}

#[derive(Serialize)]
struct FetchedView {
    message: String,
    file: String,
    language: String,
    label: String,
    release: String,
    matched: Match,
    shown: bool,
}

struct Chosen {
    video: PathBuf,
    imdb: Option<String>,
    season: Option<u32>,
    episode: Option<u32>,
    name: String,
}

fn chosen(library: &Library, body: &FetchBody) -> Result<Chosen, Response> {
    match (&body.movie, &body.show, body.season, body.episode) {
        (Some(key), None, None, None) => {
            let movie = library
                .movies
                .iter()
                .find(|movie| &movie.key == key)
                .ok_or_else(|| not_found(format!("no movie {key}")))?;
            Ok(Chosen {
                video: movie.path.clone(),
                imdb: movie.imdb.clone(),
                season: None,
                episode: None,
                name: movie.title.clone(),
            })
        }
        (None, Some(key), Some(season), Some(number)) => {
            let show = library
                .shows
                .iter()
                .find(|show| &show.key == key)
                .ok_or_else(|| not_found(format!("no show {key}")))?;
            let episode = show
                .episode(season, number)
                .ok_or_else(|| not_found(format!("{} has no season {season} episode {number}", show.title)))?;
            Ok(Chosen {
                video: episode.path.clone(),
                imdb: show.imdb.clone(),
                season: Some(season),
                episode: Some(number),
                name: format!("{} {}", show.title, episode.short_label()),
            })
        }
        _ => Err(problem(
            StatusCode::BAD_REQUEST,
            "Send {\"movie\": key} or {\"show\": key, \"season\": n, \"episode\": n}".to_string(),
        )),
    }
}

async fn fetch_subtitles(State(api): State<Api>, Json(body): Json<FetchBody>) -> Response {
    let Some(downloads) = api.downloads.clone() else {
        return problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "This TV can't fetch subtitles".to_string(),
        );
    };
    let library = api.shared.library();
    let chosen = match chosen(&library, &body) {
        Ok(chosen) => chosen,
        Err(response) => return response,
    };
    let Some(imdb) = chosen.imdb else {
        return problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("{} has no IMDb id, so I can't look up its subtitles", chosen.name),
        );
    };
    let asked = body
        .language
        .or_else(|| api.shared.subtitles.settings().languages.first().cloned())
        .unwrap_or_else(|| "en".to_string());
    let Some(language) = subtitles::language_code(&asked) else {
        return problem(StatusCode::BAD_REQUEST, format!("{asked} is not a language I know"));
    };
    let label = subtitles::language_name(language).unwrap_or(language).to_string();
    let Ok(relative) = chosen.video.strip_prefix(&api.library) else {
        return problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("{} is outside the library", chosen.video.display()),
        );
    };
    let video = chosen.video.clone();
    let hashed = tokio::task::spawn_blocking(move || subfetch::hash(&video).ok())
        .await
        .ok()
        .flatten();
    let wanted = Wanted {
        video: relative.to_string_lossy().into_owned(),
        imdb,
        season: chosen.season,
        episode: chosen.episode,
        language: language.to_string(),
        hash: hashed.as_ref().map(|(hash, _)| hash.clone()),
        size: hashed.map(|(_, size)| size),
    };
    let fetched = match tokio::task::spawn_blocking(move || downloads.subtitles(&wanted)).await {
        Ok(Ok(fetched)) => fetched,
        Ok(Err(error)) => return problem(StatusCode::UNPROCESSABLE_ENTITY, error),
        Err(error) => return problem(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    let path = api.library.join(&fetched.file);
    let (video, shared) = (chosen.video.clone(), api.shared.clone());
    let found = tokio::task::spawn_blocking(move || {
        shared.recount(&video);
        sidecar::find(&video)
    })
    .await
    .unwrap_or_default();
    let sidecar = found
        .into_iter()
        .find(|sidecar| sidecar.path == path)
        .unwrap_or_else(|| Sidecar {
            path,
            language: Some(language.to_string()),
            title: label.clone(),
            forced: false,
            hearing_impaired: false,
        });
    let mut shown = false;
    if api.shared.subtitles.plays(&chosen.video) {
        let (reply, answer) = oneshot::channel();
        (api.app)(Pending {
            request: Request::Subtitles(subtitles::Change::Add {
                video: chosen.video.clone(),
                sidecar,
            }),
            reply,
        });
        shown = matches!(tokio::time::timeout(ANSWER_TIME, answer).await, Ok(Ok(outcome)) if outcome.ok);
    }
    let message = if shown {
        format!("{label} subtitles added")
    } else {
        format!("{label} subtitles saved for {}", chosen.name)
    };
    Json(FetchedView {
        message,
        file: fetched.file,
        language: fetched.language,
        label,
        release: fetched.release,
        matched: fetched.matched,
        shown,
    })
    .into_response()
}

#[derive(Serialize)]
struct Problem {
    error: String,
}

pub fn problem(status: StatusCode, error: String) -> Response {
    (status, Json(Problem { error })).into_response()
}

fn not_found(error: String) -> Response {
    problem(StatusCode::NOT_FOUND, error)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioBody {
    output: audio::Output,
}

fn no_headphones() -> Response {
    not_found("this TV has no Bluetooth headphones".to_string())
}

async fn audio_view(State(api): State<Api>) -> Response {
    match api.audio {
        Some(audio) => Json(audio.view()).into_response(),
        None => no_headphones(),
    }
}

async fn choose_audio(State(api): State<Api>, Json(body): Json<AudioBody>) -> Response {
    match api.audio {
        Some(audio) => Json(audio.choose(body.output)).into_response(),
        None => no_headphones(),
    }
}

async fn pair_headphones(State(api): State<Api>) -> Response {
    match api.audio.map(|audio| audio.pair()) {
        Some(Ok(view)) => Json(view).into_response(),
        Some(Err(audio::Busy)) => problem(StatusCode::CONFLICT, "headphones are already being paired".to_string()),
        None => no_headphones(),
    }
}

async fn cancel_pairing(State(api): State<Api>) -> Response {
    match api.audio {
        Some(audio) => Json(audio.cancel_pairing()).into_response(),
        None => no_headphones(),
    }
}

async fn forget_headphones(State(api): State<Api>) -> Response {
    let Some(audio) = api.audio else {
        return no_headphones();
    };
    match tokio::task::spawn_blocking(move || audio.forget()).await {
        Ok(view) => Json(view).into_response(),
        Err(error) => problem(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DelayBody {
    by: i32,
}

async fn delay_sound(State(api): State<Api>, Json(body): Json<DelayBody>) -> Response {
    let Some(audio) = api.audio else {
        return no_headphones();
    };
    match tokio::task::spawn_blocking(move || audio.delay_sound(body.by)).await {
        Ok(Ok(view)) => Json(view).into_response(),
        Ok(Err(error)) => problem(StatusCode::CONFLICT, error),
        Err(error) => problem(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn volume(State(api): State<Api>, Path(change): Path<String>) -> Response {
    let change = match change.as_str() {
        "up" => audio::VolumeChange::Up,
        "down" => audio::VolumeChange::Down,
        "mute" => audio::VolumeChange::Mute,
        _ => return not_found(format!("no volume change {change}")),
    };
    if let Some(audio) = api.audio.clone() {
        let changed = tokio::task::spawn_blocking(move || audio.change_volume(change)).await;
        match changed {
            Ok(Some(Ok(message))) => return (StatusCode::OK, Json(Outcome { message, ok: true })).into_response(),
            Ok(Some(Err(message))) => {
                return (StatusCode::UNPROCESSABLE_ENTITY, Json(Outcome { message, ok: false })).into_response();
            }
            Ok(None) => {}
            Err(error) => return problem(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
        }
    }
    let television = State(api.television);
    match change {
        audio::VolumeChange::Up => tv::volume_up(television).await,
        audio::VolumeChange::Down => tv::volume_down(television).await,
        audio::VolumeChange::Mute => tv::mute(television).await,
    }
}

async fn screen_view(State(api): State<Api>) -> Json<screen::View> {
    Json(api.bus.screen_state())
}

async fn screen_command(State(api): State<Api>, Json(command): Json<ScreenCommand>) -> Response {
    ask(&api, Request::Screen(command)).await
}

async fn events(State(api): State<Api>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| stream_events(socket, api.bus))
}

async fn stream_events(mut socket: WebSocket, bus: Bus) {
    let (now_playing, mut events) = bus.subscribe();
    if send_event(&mut socket, &Event::Playback { now_playing }).await.is_err() {
        return;
    }
    if let Some(audio) = bus.audio_state()
        && send_event(&mut socket, &Event::Audio(audio)).await.is_err()
    {
        return;
    }
    let screen = bus.screen_state();
    if send_event(&mut socket, &Event::Screen { screen }).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(event) => if send_event(&mut socket, &event).await.is_err() { return },
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let now_playing = bus.now_playing();
                    if send_event(&mut socket, &Event::Playback { now_playing }).await.is_err() { return }
                    if let Some(audio) = bus.audio_state()
                        && send_event(&mut socket, &Event::Audio(audio)).await.is_err() { return }
                    let screen = bus.screen_state();
                    if send_event(&mut socket, &Event::Screen { screen }).await.is_err() { return }
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
        library.movies[0].imdb = Some("tt1160419".to_string());
        library.movies[0].ratings.letterboxd = Some(crate::manifest::Tenths(41));
        library.movies[0].about = About {
            genres: vec!["Science Fiction".to_string()],
            language: Some("en".to_string()),
            released: Date::parse("2021-09-15"),
            directors: vec!["Denis Villeneuve".to_string()],
            trailer: Some("n9xhJrPXop4".to_string()),
            cast: vec![crate::manifest::Member {
                name: "Timothée Chalamet".to_string(),
                character: Some("Paul Atreides".to_string()),
                person: Some(1_190_668),
            }],
            ..About::default()
        };
        let outlander = library.shows.iter_mut().find(|show| show.key == "outlander").unwrap();
        outlander.seasons[0].episodes[0].ratings.imdb = Some(crate::manifest::Tenths(84));
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
            credits: None,
        };
        progress.record(&dune, place);
        let up = crate::progress::Place {
            position: 600.0,
            duration: 5400.0,
            credits: None,
        };
        progress.record(&library().movies[1].path, up);
        shared.set_progress(&progress);
        let bus = Bus::default();
        let (asked, received) = mpsc::channel();
        let app: Ask = Arc::new(move |pending: Pending| {
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
            app,
            microphone: microphone.map(|device| {
                Arc::new(Microphone {
                    device,
                    busy: tokio::sync::Mutex::new(()),
                })
            }),
            television: Television::new(tv::Settings {
                kind: None,
                renderer: None,
                token: std::env::temp_dir().join("semmeltv-api-tv-token"),
            }),
            audio: None,
            library: PathBuf::from("/lib"),
            downloads: None,
            stream: std::env::temp_dir().join(format!("semmeltv-api-stream-{}", std::process::id())),
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
        assert_eq!(body["forgotten"][0]["key"], "up-2009");
        assert_eq!(body["forgotten"][0]["detail"], "1h 20m left");
        assert_eq!(body["forgotten"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn dismissing_a_forgotten_title_asks_the_tv() {
        let server = serve(no_root(), None);
        let (status, body) = post(
            &server,
            "/api/forgotten/dismiss",
            json!({ "kind": "show", "key": "the-bear" }),
        );
        assert_eq!(status, 200);
        assert_eq!(body["ok"], true);
        assert_eq!(server.asked.recv().unwrap(), r#"Dismiss(Series, "the-bear")"#);
    }

    #[test]
    fn marking_a_title_asks_the_tv_and_answers_with_its_state() {
        let server = serve(no_root(), None);
        let (status, body) = post(
            &server,
            "/api/watched",
            json!({ "kind": "show", "key": "outlander", "season": 1, "episode": 1, "watched": true }),
        );
        assert_eq!(status, 200);
        assert_eq!((&body["ok"], &body["watched"]), (&json!(true), &json!(true)));
        assert_eq!(
            server.asked.recv().unwrap(),
            r#"Mark(Series, "outlander", Some((1, 1)), Watched)"#
        );
        let (_, body) = post(
            &server,
            "/api/watched",
            json!({ "kind": "movie", "key": "dune-2021", "watched": false }),
        );
        assert_eq!(body["watched"], false);
        assert_eq!(
            server.asked.recv().unwrap(),
            r#"Mark(Movie, "dune-2021", None, Unwatched)"#
        );
        let (status, _) = post(
            &server,
            "/api/watched",
            json!({ "kind": "movie", "key": "dune-2021", "season": 1, "episode": 1, "watched": true }),
        );
        assert_eq!(status, 400);
        let (status, body) = post(
            &server,
            "/api/watched",
            json!({ "kind": "movie", "key": "gone", "watched": true }),
        );
        assert_eq!((status, &body["ok"]), (422, &json!(false)));
    }

    #[test]
    fn cards_say_whether_a_title_is_finished() {
        let server = serve(no_root(), None);
        let (_, body) = get(&server, "/api/library");
        assert_eq!(body["movies"][0]["finished"], false);
        assert_eq!(body["shows"][0]["finished"], false);
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
        assert_eq!(
            body["next"],
            json!({ "season": 2, "episode": 1, "label": "S2 E1", "title": null, "stopped_at": null, "duration": null })
        );
        let watched: Vec<bool> = body["seasons"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|season| season["episodes"].as_array().unwrap())
            .map(|episode| episode["watched"].as_bool().unwrap())
            .collect();
        assert_eq!(watched, [true, true, false]);
        assert_eq!(body["last_played"], 1);
        assert_eq!(body["seasons"].as_array().unwrap().len(), 2);
        assert_eq!(body["seasons"][0]["episodes"][1]["label"], "Episode 2");
        let first = &body["seasons"][0]["episodes"][0];
        assert_eq!(
            (&first["ratings"]["imdb"], &first["band"]),
            (&json!(8.4), &json!("great"))
        );
        let second = &body["seasons"][0]["episodes"][1];
        assert_eq!(
            (&second["ratings"]["imdb"], &second["band"]),
            (&Value::Null, &Value::Null)
        );
        assert_eq!(get(&server, "/api/shows/nothing").0, 404);
        let dune = get(&server, "/api/movies/dune-2021").1;
        assert_eq!(dune["title"], "Dune");
        assert_eq!(dune["last_played"], 2);
        assert!(dune["last_played_label"].as_str().unwrap().ends_with("1970"));
        let about = &dune["about"];
        assert_eq!(about["language"], "English");
        assert_eq!(about["released"], "2021-09-15");
        assert_eq!(about["directors"], json!(["Denis Villeneuve"]));
        assert_eq!(about["trailer"], "https://www.youtube.com/watch?v=n9xhJrPXop4");
        assert_eq!(about["imdb"], "https://www.imdb.com/title/tt1160419/");
        assert_eq!(about["tmdb"], "https://www.themoviedb.org/movie/438631");
        assert_eq!(about["scores"]["letterboxd"], 4.1);
        assert_eq!(
            about["cast"],
            json!([{ "name": "Timothée Chalamet", "character": "Paul Atreides", "photo": null }])
        );
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
        assert_eq!(
            post(&server, "/api/playback/skip", json!({ "direction": "back" })).0,
            200
        );
        assert_eq!(server.asked.recv().unwrap(), "Seek(Step(Back))");
        assert_eq!(post(&server, "/api/playback/skip", json!({ "direction": "up" })).0, 422);
        assert_eq!(post(&server, "/api/playback/dance", json!({})).0, 404);
        for (name, control) in [("on", "TvOn"), ("off", "TvOff"), ("semmeltv", "ShowSemmelTv")] {
            assert_eq!(post(&server, &format!("/api/tv/{name}"), json!({})).0, 200, "{name}");
            assert_eq!(server.asked.recv().unwrap(), format!("Control({control})"));
        }
        assert_eq!(post(&server, "/api/tv/dance", json!({})).0, 404);
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
        assert_eq!(next()["screen"]["kind"], "tv");
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

    #[test]
    fn a_tv_without_headphones_offers_no_audio_choice() {
        let server = serve(no_root(), None);
        assert_eq!(get(&server, "/api/audio").0, 404);
        assert_eq!(post(&server, "/api/audio", json!({ "output": "headphones" })).0, 404);
        assert_eq!(post(&server, "/api/audio/pair", json!({})).0, 404);
    }

    #[test]
    fn volume_keys_without_headphones_go_to_the_tv() {
        let server = serve(no_root(), None);
        let (status, body) = post(&server, "/api/volume/up", json!({}));
        assert_eq!(status, 422);
        assert_eq!(body["message"], "Volume control is not supported by this TV");
        assert_eq!(post(&server, "/api/volume/sideways", json!({})).0, 404);
    }
}
