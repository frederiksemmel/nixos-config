use crate::artwork;
use crate::audio;
use crate::auth::gate::Who;
use crate::auth::{self, gate::Gate};
use crate::bus::{Bus, Event};
use crate::core::sound::Refusal;
use crate::core::{Core, Outcome, Request, ScreenCommand, ScreenKind, Seek};
use crate::details::{self, Details};
use crate::heatmap;
use crate::intent::{self, Control, Pick};
use crate::library::{Episode, Item, Library, Movie, Show};
use crate::manifest::{About, Date};
use crate::profile::ProfileId;
use crate::progress::Progress;
use crate::screen;
use crate::seek::Direction;
use crate::sidecar::{self, Sidecar};
use crate::storage;
use crate::stream;
use crate::subfetch::{self, Match, Wanted};
use crate::subtitles;
use crate::tags::Tags;
use crate::tmdb::Kind;
use crate::together::Spot;
use crate::tv;
use crate::watching::{self, Current};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Extension, Json, Router, middleware};
use serde::{Deserialize, Serialize};
use std::path::{Path as FsPath, PathBuf};
use std::time::UNIX_EPOCH;
use tokio::net::TcpListener;
use tokio::sync::broadcast;

const NEWEST: usize = 12;
const IMAGE_CACHE: &str = "private, max-age=86400";

pub struct Config {
    pub address: String,
    pub library: PathBuf,
    pub meta: PathBuf,
    pub stream: PathBuf,
    pub gate: Option<Gate>,
    pub feedback: PathBuf,
    pub grim: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "lowercase", deny_unknown_fields)]
enum ScreenRequest {
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
    Resume {
        device: screen::Device,
        movie: Option<String>,
        show: Option<String>,
        season: Option<u32>,
        episode: Option<u32>,
        position: f64,
    },
    Stop {
        device: screen::Device,
    },
    Failed {
        device: screen::Device,
        reason: String,
    },
}

fn pick_of(
    movie: Option<String>,
    show: Option<String>,
    season: Option<u32>,
    episode: Option<u32>,
) -> Result<Pick, String> {
    match (movie, show) {
        (Some(key), None) if season.is_none() && episode.is_none() => Ok(Pick::Movie(key)),
        (Some(_), None) => Err("a movie has no season or episode".to_string()),
        (None, Some(key)) => Ok(Pick::Show { key, season, episode }),
        _ => Err("give either movie or show".to_string()),
    }
}

impl TryFrom<ScreenRequest> for ScreenCommand {
    type Error = String;

    fn try_from(request: ScreenRequest) -> Result<Self, String> {
        Ok(match request {
            ScreenRequest::Flip { to, device, position } => Self::Flip { to, device, position },
            ScreenRequest::Play {
                device,
                movie,
                show,
                season,
                episode,
            } => Self::Play {
                device,
                pick: pick_of(movie, show, season, episode)?,
                takes_over: true,
            },
            ScreenRequest::Resume {
                device,
                movie,
                show,
                season,
                episode,
                position,
            } => Self::Resume {
                device,
                pick: pick_of(movie, show, season, episode)?,
                position,
            },
            ScreenRequest::Stop { device } => Self::Stop { device },
            ScreenRequest::Failed { device, reason } => Self::Failed { device, reason },
        })
    }
}

#[derive(Clone)]
struct Api {
    core: Core,
    root: PathBuf,
    library: PathBuf,
    stream: PathBuf,
    gate: Option<Gate>,
    feedback: crate::feedback::Feedback,
    deploy: crate::deploy::Deploy,
}

impl Api {
    fn art(&self) -> PathBuf {
        self.root.join(artwork::DIR)
    }
}

/// Binds the API's address, then serves it on a task of its own.
pub async fn spawn(config: Config, core: Core) -> std::io::Result<()> {
    let feedback = crate::feedback::Feedback::new(config.feedback, core.bus().clone(), config.grim);
    let deploy = crate::deploy::Deploy::new(feedback.clone(), PathBuf::from(crate::feedback::REVISION));
    let api = Api {
        core,
        root: config.meta,
        library: config.library,
        stream: config.stream,
        gate: config.gate,
        feedback,
        deploy,
    };
    let address = config.address;
    let listener = TcpListener::bind(&address)
        .await
        .map_err(|error| std::io::Error::new(error.kind(), format!("{address}: {error}")))?;
    eprintln!("api on http://{address}/api");
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router(api)).await {
            eprintln!("api: {error}");
        }
    });
    Ok(())
}

fn router(api: Api) -> Router {
    let gate = api.gate.clone();
    let routes = Router::new()
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
        .route("/api/shows/{key}/together/{partner}", put(join).delete(leave))
        .route("/api/shows/{key}/together/{partner}/catch-up", post(catch_up))
        .route("/api/push", get(push_key))
        .route("/api/push/subscriptions", post(subscribe).delete(unsubscribe))
        .route("/api/people/{person}/photo", get(photo))
        .route("/api/play", post(play))
        .route("/api/command", post(command))
        .route("/api/playback", get(playback))
        .route("/api/playback/seek", post(seek))
        .route("/api/playback/skip", post(skip))
        .route("/api/playback/speed", post(speed))
        .route("/api/playback/{control}", post(control))
        .route("/api/restarting", post(restarting))
        .route("/api/tv/{control}", post(tv))
        .route("/api/tv-profile", get(tv_profile).post(show_my_profile))
        .route("/api/events", get(events))
        .route("/api/screen", get(screen_view).post(screen_command))
        .route("/api/audio", get(audio_view).post(choose_audio))
        .route("/api/audio/pair", post(pair_headphones).delete(cancel_pairing))
        .route("/api/audio/headphones", delete(forget_headphones))
        .route("/api/audio/delay", post(delay_sound))
        .route("/api/audio/dialogue", get(dialogue).post(set_dialogue))
        .route("/api/volume/{change}", post(volume))
        .route("/api/subtitles/fetch", post(fetch_subtitles))
        .route("/api/usage", get(usage))
        .merge(subtitles::routes(api.core.clone()))
        .merge(tv::routes(api.core.television().clone()))
        .merge(crate::client_log::routes())
        .merge(crate::feedback::routes(api.feedback.clone()))
        .merge(crate::deploy::routes(api.deploy.clone()))
        .merge(storage::routes(api.core.clone()))
        .merge(stream::routes(api.core.clone(), api.stream.clone()));
    match gate {
        Some(gate) => routes
            .merge(auth::http::routes(gate.clone()))
            .with_state(api)
            .layer(middleware::from_fn_with_state(gate, auth::gate::guard)),
        None => routes.with_state(api),
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    because: Option<String>,
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
        because: None,
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
        because: None,
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

async fn library(State(api): State<Api>, Who(profile): Who) -> Json<LibraryView> {
    let library = api.core.library();
    let progress = api.core.progress(profile).await;
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

async fn dismiss(State(api): State<Api>, Who(profile): Who, Json(body): Json<Dismiss>) -> Response {
    let kind = match body.kind {
        CardKind::Movie => watching::Kind::Movie,
        CardKind::Show => watching::Kind::Series,
    };
    ask(&api.core, Request::Dismiss(kind, body.key, profile)).await
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

async fn mark_watched(State(api): State<Api>, Who(profile): Who, Json(body): Json<MarkWatched>) -> Response {
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
    let request = Request::Mark(kind, body.key.clone(), episode, mark, profile);
    let Some(outcome) = api.core.ask(request).await else {
        return unanswered();
    };
    let watched = is_watched(&api.core.library(), &*api.core.progress(profile).await, &body);
    let marked = Marked {
        message: outcome.message,
        ok: outcome.ok && watched.is_some(),
        watched: watched.unwrap_or(false),
    };
    (status(marked.ok), Json(marked)).into_response()
}

#[derive(Deserialize)]
struct UsageQuery {
    days: Option<u64>,
}

async fn usage(State(api): State<Api>, Query(query): Query<UsageQuery>) -> Response {
    let days = query.days.unwrap_or(30).clamp(1, 3_660);
    let now = watching::now();
    let db = api.core.db();
    let events = db
        .usage_since(now.saturating_sub(days * 86_400))
        .await
        .map_err(|error| error.to_string());
    let profiles = db.profiles().await.map_err(|error| error.to_string());
    match events.and_then(|events| Ok((events, profiles?))) {
        Ok((events, profiles)) => {
            let names = profiles.into_iter().map(|profile| (profile.id, profile.name)).collect();
            Json(crate::usage::summary(&events, &names, now, days, details::local_offset)).into_response()
        }
        Err(error) => problem(StatusCode::INTERNAL_SERVER_ERROR, error),
    }
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
}

#[derive(Serialize)]
struct Results {
    results: Vec<Card>,
}

async fn search(State(api): State<Api>, Who(profile): Who, Query(query): Query<SearchQuery>) -> Json<Results> {
    let library = api.core.library();
    let progress = api.core.progress(profile).await;
    let results = intent::search(&library, &query.q)
        .into_iter()
        .map(|hit| Card {
            because: hit.because,
            ..card(&api.root, &progress, &library, hit.item)
        })
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
            stopped_at: watched.stopped_at(),
            download: "",
        }
        .line(),
        runtime_minutes: runtime,
        stopped_at: watched.stopped_at(),
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
    imdb: String,
}

fn person_link(member: &crate::manifest::Member) -> String {
    match &member.imdb {
        Some(id) => format!("https://www.imdb.com/name/{id}/"),
        None => {
            let query: String = member
                .name
                .bytes()
                .map(|byte| match byte {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (byte as char).to_string(),
                    b' ' => "+".to_string(),
                    _ => format!("%{byte:02X}"),
                })
                .collect();
            format!("https://www.imdb.com/find/?q={query}&s=nm")
        }
    }
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
    tmdb_id: Option<u64>,
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
        tmdb_id: tmdb,
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
                imdb: person_link(member),
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
    downloaded_by: Vec<String>,
}

async fn movie(State(api): State<Api>, Who(profile): Who, Path(key): Path<String>) -> Response {
    let library = api.core.library();
    let Some(movie) = library.movies.iter().find(|movie| movie.key == key) else {
        return not_found(format!("no movie {key}"));
    };
    let progress = api.core.progress(profile).await;
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
            api.core.subtitle_count(&movie.path),
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
        downloaded_by: movie.downloaded_by.iter().cloned().collect(),
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
    downloaded_by: Vec<String>,
    together: TogetherView,
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum SpotView {
    NotStarted,
    At {
        season: u32,
        episode: u32,
        label: String,
        title: Option<String>,
        stopped_at: Option<f64>,
        duration: Option<f64>,
    },
    Finished,
}

#[derive(Serialize)]
struct PersonView {
    id: i64,
    name: String,
    color: String,
}

#[derive(Serialize)]
struct PartnerView {
    #[serde(flatten)]
    person: PersonView,
    spot: SpotView,
    with_me: bool,
}

#[derive(Serialize)]
struct TogetherView {
    me: i64,
    partners: Vec<PartnerView>,
    others: Vec<PersonView>,
}

fn spot_view(progress: &Progress, show: &Show, spot: Spot) -> SpotView {
    match spot {
        Spot::NotStarted => SpotView::NotStarted,
        Spot::Finished => SpotView::Finished,
        Spot::At {
            season,
            episode,
            stopped_at,
        } => {
            let found = show.episode(season, episode);
            let duration =
                found.and_then(|episode| watch(progress, &episode.path, &episode.tags, episode.runtime, 0).duration);
            SpotView::At {
                season,
                episode,
                label: found.map_or_else(|| format!("S{season} E{episode}"), Episode::short_label),
                title: found.and_then(|episode| episode.title.clone()),
                stopped_at,
                duration,
            }
        }
    }
}

async fn together_view(core: &Core, profile: ProfileId, show: &Show, mine: &Progress) -> TogetherView {
    let partners = core.db().partners(profile, &show.key).await.unwrap_or_else(|error| {
        eprintln!("together: partners of {} in {}: {error}", profile.0, show.key);
        Vec::new()
    });
    let people = core.db().profiles().await.unwrap_or_else(|error| {
        eprintln!("together: profiles: {error}");
        Vec::new()
    });
    let person = |found: &auth::store::Profile| {
        let person = auth::Person::from(found);
        PersonView {
            id: person.id.0,
            name: person.name,
            color: person.color,
        }
    };
    let here = Spot::of(mine, show);
    let mut views = Vec::new();
    for partner in &partners {
        let Some(found) = people.iter().find(|found| found.id == *partner) else {
            continue;
        };
        let theirs = core.progress(*partner).await;
        let spot = Spot::of(&theirs, show);
        views.push(PartnerView {
            person: person(found),
            spot: spot_view(&theirs, show, spot),
            with_me: spot.same_as(here),
        });
    }
    let others = people
        .iter()
        .filter(|found| found.id != profile && !partners.contains(&found.id))
        .map(person)
        .collect();
    TogetherView {
        me: profile.0,
        partners: views,
        others,
    }
}

async fn set_together(api: &Api, profile: ProfileId, key: &str, partner: i64, on: bool) -> Response {
    let library = api.core.library();
    let Some(show) = library.shows.iter().find(|show| show.key == key) else {
        return not_found(format!("no show {key}"));
    };
    let partner = ProfileId(partner);
    if on && partner == profile {
        return problem(
            StatusCode::BAD_REQUEST,
            "You can't watch together with yourself".to_string(),
        );
    }
    let Some(name) = api.core.profile_name(partner).await else {
        return not_found(format!("no profile {}", partner.0));
    };
    let others = match api.core.db().partners(partner, &show.key).await {
        Ok(others) => others.len(),
        Err(error) => return problem(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    match api.core.db().set_together(profile, partner, &show.key, on).await {
        Ok(true) if on && others > 0 => Json(Outcome::done(format!(
            "{name} and the {others} watching {} with them are now in your group",
            show.title
        )))
        .into_response(),
        Ok(true) if on => Json(Outcome::done(format!("You watch {} together with {name}", show.title))).into_response(),
        Ok(true) if partner == profile => Json(Outcome::done(format!(
            "You no longer watch {} together with the others",
            show.title
        )))
        .into_response(),
        Ok(true) => Json(Outcome::done(format!(
            "{name} no longer watches {} with your group",
            show.title
        )))
        .into_response(),
        Ok(false) => problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "The TV hasn't loaded its library yet".to_string(),
        ),
        Err(error) => problem(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn join(State(api): State<Api>, Who(profile): Who, Path((key, partner)): Path<(String, i64)>) -> Response {
    set_together(&api, profile, &key, partner, true).await
}

async fn leave(State(api): State<Api>, Who(profile): Who, Path((key, partner)): Path<(String, i64)>) -> Response {
    set_together(&api, profile, &key, partner, false).await
}

async fn catch_up(State(api): State<Api>, Who(profile): Who, Path((key, partner)): Path<(String, i64)>) -> Response {
    let library = api.core.library();
    let Some(show) = library.shows.iter().find(|show| show.key == key) else {
        return not_found(format!("no show {key}"));
    };
    let partner = ProfileId(partner);
    let partners = match api.core.db().partners(profile, &show.key).await {
        Ok(partners) => partners,
        Err(error) => return problem(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    if !partners.contains(&partner) {
        let name = api
            .core
            .profile_name(partner)
            .await
            .unwrap_or_else(|| "them".to_string());
        let refused = Outcome::refused(format!("You don't watch {} together with {name}", show.title));
        return (status(false), Json(refused)).into_response();
    }
    let request = Request::CatchUp {
        show: show.key.clone(),
        to: partner,
        by: profile,
    };
    ask(&api.core, request).await
}

#[derive(Deserialize)]
struct PushKeys {
    p256dh: String,
    auth: String,
}

#[derive(Deserialize)]
struct PushSubscribe {
    endpoint: String,
    keys: PushKeys,
}

#[derive(Deserialize)]
struct PushUnsubscribe {
    endpoint: String,
}

async fn push_key(State(api): State<Api>) -> Response {
    match api.core.push().key().await {
        Ok(key) => Json(serde_json::json!({ "key": key })).into_response(),
        Err(error) => problem(StatusCode::SERVICE_UNAVAILABLE, error),
    }
}

/// Apple's, Google's and Mozilla's push services, and none of the TV's
/// neighbours: the TV posts to whatever address a browser subscribed with.
fn push_service(endpoint: &str) -> bool {
    url::Url::parse(endpoint).is_ok_and(|url| {
        url.scheme() == "https"
            && url.port().is_none()
            && matches!(url.host(), Some(url::Host::Domain(host))
                if host.contains('.') && !host.ends_with(".local") && !host.ends_with(".ts.net") && !host.ends_with(".semmeltv.ch"))
    })
}

async fn subscribe(State(api): State<Api>, Who(profile): Who, Json(body): Json<PushSubscribe>) -> Response {
    if !push_service(&body.endpoint) {
        return problem(
            StatusCode::BAD_REQUEST,
            "That is not a push service's address".to_string(),
        );
    }
    let subscription = crate::db::Subscription {
        endpoint: body.endpoint,
        p256dh: body.keys.p256dh,
        auth: body.keys.auth,
    };
    let new = match api.core.db().subscribe(profile, &subscription, watching::now()).await {
        Ok(new) => new,
        Err(error) => return problem(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    };
    if !new {
        return Json(Outcome::done("Notifications are on for this device")).into_response();
    }
    let admin = matches!(api.core.db().profile(profile).await, Ok(Some(found)) if found.role == auth::Role::Admin);
    let body = if admin {
        "You'll hear here when the TV plays something for you, when someone you watch a series with carries on with it, and when someone adds to the library."
    } else {
        "You'll hear here when the TV plays something for you, and when someone you watch a series with carries on with it."
    };
    let notice = crate::push::Notice {
        title: "Notifications are on".to_string(),
        body: body.to_string(),
        url: "/settings".to_string(),
        tag: "notifications-on".to_string(),
    };
    let push = api.core.push().clone();
    tokio::spawn(async move { push.notify(profile, &notice).await });
    Json(Outcome::done("Notifications are on for this device")).into_response()
}

async fn unsubscribe(State(api): State<Api>, Json(body): Json<PushUnsubscribe>) -> Response {
    match api.core.db().unsubscribe(&body.endpoint).await {
        Ok(()) => Json(Outcome::done("Notifications are off for this device")).into_response(),
        Err(error) => problem(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

fn episode_view(core: &Core, show: &Show, episode: &Episode, progress: &Progress) -> EpisodeView {
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
            core.subtitle_count(&episode.path),
        ),
    }
}

async fn show(State(api): State<Api>, Who(profile): Who, Path(key): Path<String>) -> Response {
    let library = api.core.library();
    let Some(show) = library.shows.iter().find(|show| show.key == key) else {
        return not_found(format!("no show {key}"));
    };
    let progress = api.core.progress(profile).await;
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
                .map(|episode| episode_view(&api.core, show, episode, &progress))
                .collect(),
        })
        .collect();
    let together = together_view(&api.core, profile, show, &progress).await;
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
        downloaded_by: show.downloaded_by().into_iter().map(str::to_string).collect(),
        together,
    })
    .into_response()
}

async fn movie_backdrop(
    State(api): State<Api>,
    Path(key): Path<String>,
    Query(sized): Query<Sized>,
    headers: HeaderMap,
) -> Response {
    let library = api.core.library();
    let id = library
        .movies
        .iter()
        .find(|movie| movie.key == key)
        .and_then(|movie| movie.tmdb);
    match id {
        Some(id) => image(artwork::backdrop(&api.art(), Kind::Movie, id), sized, &headers).await,
        None => not_found(format!("no backdrop for movie {key}")),
    }
}

async fn show_backdrop(
    State(api): State<Api>,
    Path(key): Path<String>,
    Query(sized): Query<Sized>,
    headers: HeaderMap,
) -> Response {
    let library = api.core.library();
    let id = library
        .shows
        .iter()
        .find(|show| show.key == key)
        .and_then(|show| show.tmdb);
    match id {
        Some(id) => image(artwork::backdrop(&api.art(), Kind::Tv, id), sized, &headers).await,
        None => not_found(format!("no backdrop for show {key}")),
    }
}

async fn movie_poster(
    State(api): State<Api>,
    Path(key): Path<String>,
    Query(sized): Query<Sized>,
    headers: HeaderMap,
) -> Response {
    let library = api.core.library();
    let poster = library
        .movies
        .iter()
        .find(|movie| movie.key == key)
        .and_then(|movie| movie.poster(&api.root));
    match poster {
        Some(poster) => image(api.root.join(poster), sized, &headers).await,
        None => not_found(format!("no poster for movie {key}")),
    }
}

async fn show_poster(
    State(api): State<Api>,
    Path(key): Path<String>,
    Query(sized): Query<Sized>,
    headers: HeaderMap,
) -> Response {
    let library = api.core.library();
    let poster = library
        .shows
        .iter()
        .find(|show| show.key == key)
        .and_then(|show| show.poster(&api.root));
    match poster {
        Some(poster) => image(api.root.join(poster), sized, &headers).await,
        None => not_found(format!("no poster for show {key}")),
    }
}

async fn photo(
    State(api): State<Api>,
    Path(person): Path<u64>,
    Query(sized): Query<Sized>,
    headers: HeaderMap,
) -> Response {
    image(artwork::face(&api.art(), person), sized, &headers).await
}

async fn still(
    State(api): State<Api>,
    Path((key, season, episode)): Path<(String, u32, u32)>,
    Query(sized): Query<Sized>,
    headers: HeaderMap,
) -> Response {
    let library = api.core.library();
    let show = library.shows.iter().find(|show| show.key == key);
    let id = show
        .filter(|show| show.episode(season, episode).is_some())
        .and_then(|show| show.tmdb);
    match id {
        Some(id) => image(artwork::still(&api.art(), id, season, episode), sized, &headers).await,
        None => not_found(format!("no still for {key} season {season} episode {episode}")),
    }
}

#[derive(Deserialize)]
struct Sized {
    w: Option<u32>,
}

async fn image(path: PathBuf, sized: Sized, headers: &HeaderMap) -> Response {
    let Ok(metadata) = tokio::fs::metadata(&path).await else {
        return not_found("no such image".to_string());
    };
    let (path, metadata, kind) = match sized.w {
        Some(width) => {
            let original = path.clone();
            match tokio::task::spawn_blocking(move || artwork::sized(&original, width)).await {
                Ok(Ok(made)) => match tokio::fs::metadata(&made).await {
                    Ok(made_metadata) => (made, made_metadata, "image/webp"),
                    Err(_) => (path, metadata, "image/jpeg"),
                },
                Ok(Err(error)) => {
                    eprintln!("api: {}: no {width} px picture: {error}", path.display());
                    (path, metadata, "image/jpeg")
                }
                Err(_) => (path, metadata, "image/jpeg"),
            }
        }
        None => (path, metadata, "image/jpeg"),
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
        Ok(bytes) => ([(header::CONTENT_TYPE, kind.to_string())], cache_headers, bytes).into_response(),
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

async fn play(State(api): State<Api>, Who(profile): Who, Json(body): Json<PlayBody>) -> Response {
    let library = api.core.library();
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
    ask(&api.core, Request::Play(pick, profile)).await
}

#[derive(Deserialize)]
struct CommandBody {
    text: String,
}

async fn command(State(api): State<Api>, Who(profile): Who, Json(body): Json<CommandBody>) -> Response {
    ask(&api.core, Request::Understand(body.text, profile)).await
}

#[derive(Serialize)]
struct PlaybackView {
    now_playing: Option<crate::bus::NowPlaying>,
}

async fn playback(State(api): State<Api>) -> Json<PlaybackView> {
    Json(PlaybackView {
        now_playing: api.core.now_playing(),
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
        "skip-segment" => Request::Seek(Seek::Segment),
        "hide-skip" => Request::HideSkip,
        _ => return not_found(format!("no playback control {name}")),
    };
    ask(&api.core, request).await
}

/// SemmelTV's stop hook says it restarts for an update: the TV shows the
/// update card, and what plays is picked up after the restart.
async fn restarting(State(api): State<Api>) -> Response {
    ask(&api.core, Request::Restarting).await
}

async fn tv_profile(State(api): State<Api>, Who(profile): Who) -> Response {
    let shown = api.core.tv_profile();
    let name = api.core.profile_name(shown).await;
    Json(serde_json::json!({ "profile": shown, "name": name, "mine": shown == profile })).into_response()
}

async fn show_my_profile(State(api): State<Api>, Who(profile): Who) -> Response {
    ask(&api.core, Request::TvProfile(profile)).await
}

async fn tv(State(api): State<Api>, Path(name): Path<String>) -> Response {
    let control = match name.as_str() {
        "on" => Control::TvOn,
        "off" => Control::TvOff,
        "semmeltv" => Control::ShowSemmelTv,
        _ => return not_found(format!("no TV control {name}")),
    };
    ask(&api.core, Request::Control(control)).await
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
    ask(&api.core, Request::Seek(seek)).await
}

#[derive(Deserialize)]
struct SkipBody {
    direction: Direction,
}

async fn skip(State(api): State<Api>, Json(body): Json<SkipBody>) -> Response {
    ask(&api.core, Request::Seek(Seek::Step(body.direction))).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpeedBody {
    speed: f64,
}

async fn speed(State(api): State<Api>, Json(body): Json<SpeedBody>) -> Response {
    ask(&api.core, Request::Speed(body.speed)).await
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

pub async fn ask(core: &Core, request: Request) -> Response {
    match core.ask(request).await {
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

async fn fetch_subtitles(State(api): State<Api>, Who(profile): Who, Json(body): Json<FetchBody>) -> Response {
    let Some(downloads) = api.core.downloads().cloned() else {
        return problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "This TV can't fetch subtitles".to_string(),
        );
    };
    let library = api.core.library();
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
        .or_else(|| api.core.subtitles().settings_of(profile).languages.first().cloned())
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
    let (video, core) = (chosen.video.clone(), api.core.clone());
    let found = tokio::task::spawn_blocking(move || {
        core.recount(&video);
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
    let add = subtitles::Change::Add {
        video: chosen.video.clone(),
        sidecar,
    };
    let shown = api
        .core
        .ask(Request::Subtitles(add))
        .await
        .is_some_and(|outcome| outcome.ok);
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

fn unheard(refusal: Refusal) -> Response {
    match refusal {
        Refusal::NoHeadphones => not_found("this TV has no Bluetooth headphones".to_string()),
        Refusal::Busy => problem(StatusCode::CONFLICT, "headphones are already being paired".to_string()),
        Refusal::Failed(error) => problem(StatusCode::CONFLICT, error),
    }
}

fn heard(view: Result<audio::View, Refusal>) -> Response {
    match view {
        Ok(view) => Json(view).into_response(),
        Err(refusal) => unheard(refusal),
    }
}

async fn audio_view(State(api): State<Api>) -> Response {
    heard(api.core.sound())
}

async fn choose_audio(State(api): State<Api>, Who(profile): Who, Json(body): Json<AudioBody>) -> Response {
    heard(api.core.choose_output(profile, body.output).await)
}

async fn pair_headphones(State(api): State<Api>, Who(profile): Who) -> Response {
    heard(api.core.pair_headphones(profile).await)
}

async fn cancel_pairing(State(api): State<Api>) -> Response {
    heard(api.core.cancel_pairing().await)
}

async fn forget_headphones(State(api): State<Api>, Who(profile): Who) -> Response {
    heard(api.core.forget_headphones(profile).await)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DelayBody {
    by: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DialogueBody {
    on: bool,
}

async fn dialogue(State(api): State<Api>, Who(profile): Who) -> Response {
    let on = api
        .core
        .dialogue(profile)
        .await
        .load(std::sync::atomic::Ordering::SeqCst);
    Json(serde_json::json!({ "on": on })).into_response()
}

async fn set_dialogue(State(api): State<Api>, Who(profile): Who, Json(body): Json<DialogueBody>) -> Response {
    let outcome = api.core.set_dialogue(profile, body.on).await;
    (status(outcome.ok), Json(outcome)).into_response()
}

async fn delay_sound(State(api): State<Api>, Who(profile): Who, Json(body): Json<DelayBody>) -> Response {
    heard(api.core.delay_sound(profile, body.by).await)
}

async fn volume(State(api): State<Api>, Path(change): Path<String>) -> Response {
    let change = match change.as_str() {
        "up" => audio::VolumeChange::Up,
        "down" => audio::VolumeChange::Down,
        "mute" => audio::VolumeChange::Mute,
        _ => return not_found(format!("no volume change {change}")),
    };
    match api.core.change_volume(change).await {
        Ok(Some(Ok(message))) => return (StatusCode::OK, Json(Outcome { message, ok: true })).into_response(),
        Ok(Some(Err(message))) => {
            return (StatusCode::UNPROCESSABLE_ENTITY, Json(Outcome { message, ok: false })).into_response();
        }
        Ok(None) => {}
        Err(refusal) => return unheard(refusal),
    }
    let television = State(api.core.television().clone());
    match change {
        audio::VolumeChange::Up => tv::volume_up(television).await,
        audio::VolumeChange::Down => tv::volume_down(television).await,
        audio::VolumeChange::Mute => tv::mute(television).await,
    }
}

/// Which device asks, so the answer can say what that device does.
#[derive(Deserialize)]
struct DeviceQuery {
    device: Option<String>,
}

async fn screen_view(State(api): State<Api>, Query(query): Query<DeviceQuery>) -> Response {
    Json(api.core.screen().for_device(query.device.as_deref())).into_response()
}

fn without_control(command: ScreenCommand) -> Option<ScreenCommand> {
    match command {
        ScreenCommand::Flip { .. } => None,
        ScreenCommand::Play { device, pick, .. } => Some(ScreenCommand::Play {
            device,
            pick,
            takes_over: false,
        }),
        other => Some(other),
    }
}

async fn screen_command(
    State(api): State<Api>,
    Who(profile): Who,
    person: Option<Extension<auth::Person>>,
    Json(request): Json<ScreenRequest>,
) -> Response {
    let command = match ScreenCommand::try_from(request) {
        Ok(command) => command,
        Err(error) => return problem(StatusCode::UNPROCESSABLE_ENTITY, error),
    };
    let command = match person {
        Some(Extension(person)) if !person.can(auth::Capability::ControlTv) => without_control(command),
        _ => Some(command),
    };
    match command {
        Some(command) => ask(&api.core, Request::Screen(command, profile)).await,
        None => problem(StatusCode::FORBIDDEN, auth::Capability::ControlTv.refusal().to_string()),
    }
}

async fn events(State(api): State<Api>, Query(query): Query<DeviceQuery>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| stream_events(socket, api.core.bus().clone(), query.device))
}

async fn stream_events(socket: WebSocket, bus: Bus, device: Option<String>) {
    let mut socket = Events { socket, device };
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
            message = socket.socket.recv() => match message {
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return,
                Some(Ok(_)) => {}
            },
        }
    }
}

/// An events socket and the device it reaches, which reads each screen with
/// what it does.
struct Events {
    socket: WebSocket,
    device: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename = "screen")]
struct ScreenFor<'a> {
    screen: screen::ForDevice<'a>,
}

fn event_text(event: &Event, device: Option<&str>) -> String {
    match event {
        Event::Screen { screen } => serde_json::to_string(&ScreenFor {
            screen: screen.for_device(device),
        }),
        other => serde_json::to_string(other),
    }
    .unwrap_or_default()
}

async fn send_event(socket: &mut Events, event: &Event) -> Result<(), axum::Error> {
    let text = event_text(event, socket.device.as_deref());
    socket.socket.send(Message::Text(text.into())).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{Notice, NowPlaying};
    use crate::import::library_from_paths;
    use crate::profile::ProfileId;
    use crate::progress::Position;
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;

    #[test]
    fn each_events_socket_reads_the_screen_with_what_its_own_device_does() {
        let screen = screen::Screen::Phone {
            device: screen::Device {
                id: "a".to_string(),
                name: "iPhone".to_string(),
            },
            play: None,
        };
        let event = Event::Screen {
            screen: screen::View::of(&screen, None),
        };
        let read = |device| serde_json::from_str::<Value>(&event_text(&event, device)).unwrap();
        assert_eq!(read(Some("a"))["type"], "screen");
        assert_eq!(read(Some("a"))["screen"]["kind"], "phone");
        assert_eq!(read(Some("a"))["screen"]["here"], json!({ "do": "hold" }));
        assert_eq!(read(Some("b"))["screen"]["here"], json!({ "do": "close" }));
        let other = Event::Playback { now_playing: None };
        assert_eq!(event_text(&other, Some("a")), serde_json::to_string(&other).unwrap());
    }

    #[test]
    fn a_phone_asks_for_a_movie_or_a_show_and_never_both() {
        let read = |body: Value| {
            let request = serde_json::from_value::<ScreenRequest>(body).map_err(|error| error.to_string())?;
            ScreenCommand::try_from(request)
        };
        let device = json!({ "id": "p1", "name": "phone" });
        let play = read(json!({ "action": "play", "device": device, "show": "dark", "season": 1, "episode": 2 }));
        assert!(matches!(
            play.unwrap(),
            ScreenCommand::Play {
                pick: Pick::Show {
                    season: Some(1),
                    episode: Some(2),
                    ..
                },
                ..
            }
        ));
        let resume = read(json!({ "action": "resume", "device": device, "movie": "dune-2021", "position": 61.0 }));
        assert!(matches!(
            resume.unwrap(),
            ScreenCommand::Resume {
                pick: Pick::Movie(_),
                position: 61.0,
                ..
            }
        ));
        assert!(read(json!({ "action": "play", "device": device, "movie": "dune-2021", "show": "dark" })).is_err());
        assert!(read(json!({ "action": "play", "device": device })).is_err());
        assert!(read(json!({ "action": "play", "device": device, "movie": "dune-2021", "season": 1 })).is_err());
        assert!(read(json!({ "action": "stop", "device": device, "movie": "dune-2021" })).is_err());
    }

    #[test]
    fn without_tv_control_a_phone_plays_only_on_itself_and_never_moves_the_screen() {
        let read =
            |body: Value| ScreenCommand::try_from(serde_json::from_value::<ScreenRequest>(body).unwrap()).unwrap();
        let device = json!({ "id": "p1", "name": "phone" });
        let play = read(json!({ "action": "play", "device": device, "movie": "dune-2021" }));
        assert!(matches!(play, ScreenCommand::Play { takes_over: true, .. }));
        assert!(matches!(
            without_control(play),
            Some(ScreenCommand::Play { takes_over: false, .. })
        ));
        for to in ["phone", "tv"] {
            assert_eq!(
                without_control(read(json!({ "action": "flip", "device": device, "to": to }))),
                None
            );
        }
        let stop = read(json!({ "action": "stop", "device": device }));
        assert_eq!(without_control(stop.clone()), Some(stop));
    }

    struct Server {
        base: String,
        bus: Bus,
        core: Core,
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
                imdb: None,
            }],
            ..About::default()
        };
        for (id, show) in (100..).zip(library.shows.iter_mut()) {
            show.id = id;
        }
        let outlander = library.shows.iter_mut().find(|show| show.key == "outlander").unwrap();
        outlander.seasons[0].episodes[0].ratings.imdb = Some(crate::manifest::Tenths(84));
        library
    }

    fn serve(root: PathBuf) -> Server {
        let mut progress = Progress::default();
        let position = Position {
            season: 1,
            episode: 2,
            finished: true,
        };
        progress.shows.insert("outlander".to_string(), position);
        let the_bear = Position {
            season: 1,
            episode: 1,
            finished: true,
        };
        progress.shows.insert("the-bear".to_string(), the_bear);
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
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (core, asked) = runtime.block_on(async {
            let (core, inbox) = Core::detached(library()).await;
            let asked = Core::answering(inbox, |request| Outcome {
                message: "done".to_string(),
                ok: !matches!(request, Request::Understand(..)),
            });
            (core, asked)
        });
        core.publish_progress(ProfileId::FIRST, progress);
        let bus = core.bus().clone();
        let feedback_dir = std::env::temp_dir().join(format!(
            "semmeltv-api-feedback-{}-{}",
            std::process::id(),
            SERVED.fetch_add(1, Ordering::SeqCst)
        ));
        let revision = feedback_dir.join("revision");
        let feedback = crate::feedback::Feedback::new(feedback_dir, bus.clone(), None);
        let api = Api {
            core: core.clone(),
            root,
            library: PathBuf::from("/lib"),
            stream: std::env::temp_dir().join(format!("semmeltv-api-stream-{}", std::process::id())),
            gate: None,
            feedback: feedback.clone(),
            deploy: crate::deploy::Deploy::new(feedback, revision),
        };
        let listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        runtime.spawn(async move { axum::serve(listener, router(api)).await });
        Server {
            base,
            bus,
            core,
            asked,
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

    static SERVED: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn usage_answers_who_watched_what_in_the_window_asked_for() {
        let server = serve(no_root());
        let now = watching::now();
        let db = server.core.db();
        server
            ._runtime
            .block_on(db.note_play("tv-a", now - 3_600, ProfileId::FIRST, "TV", "Dune", 1_800.0))
            .unwrap();
        server
            ._runtime
            .block_on(db.note_play("phone-b", now - 20 * 86_400, ProfileId::FIRST, "iPhone", "Up", 600.0))
            .unwrap();
        let (status, week) = get(&server, "/api/usage?days=7");
        assert_eq!(status, 200, "{week}");
        assert_eq!(week["days"], 7);
        assert_eq!(week["span"], "day");
        assert_eq!(week["buckets"].as_array().unwrap().len(), 7);
        assert_eq!(week["totals"]["tv"], 1_800);
        assert_eq!(week["totals"]["phone"], 0);
        assert_eq!(week["plays"][0]["title"], "Dune");
        assert_eq!(week["plays"][0]["tv"], true);
        assert_eq!(week["hours"].as_array().unwrap().len(), 7);
        let (_, month) = get(&server, "/api/usage?days=30");
        assert_eq!(month["totals"]["plays"], 2);
        assert_eq!(month["people"][0]["phone"], 600);
        assert_eq!(get(&server, "/api/usage").1["days"], 30);
        assert_eq!(get(&server, "/api/usage?days=0").1["days"], 1);
    }

    #[test]
    fn a_deploy_is_asked_for_on_the_tv_and_beast_reports_how_it_went() {
        let server = serve(no_root());
        let (status, refused) = post(&server, "/api/deploy", json!({}));
        assert_eq!(status, 409, "{refused}");
        let (_, before) = get(&server, "/api/deploy");
        assert_eq!(before["checked_at"], Value::Null);
        assert_eq!(before["pending"], Value::Null);

        let (_, sent) = post(&server, "/api/feedback", json!({ "message": "A deploy button" }));
        let id = sent["id"].as_str().unwrap().to_string();
        let dir = std::path::PathBuf::from(get(&server, "/api/feedback").1["dir"].as_str().unwrap());
        std::fs::write(dir.join("revision"), "aaaaaaa1\n").unwrap();
        let main = json!({ "commits": [
            { "sha": "ccccccc3", "at": 30, "subject": "semmeltv: a deploy button", "reports": [id], "tv": true },
            { "sha": "bbbbbbb2", "at": 20, "subject": "fstux: a font", "tv": false },
            { "sha": "aaaaaaa1", "at": 10, "subject": "semmeltv: older", "tv": true },
        ] });
        let (status, told) = post(&server, "/api/deploy/main", main);
        assert_eq!(status, 200, "{told}");
        assert_eq!(told["revision"], "aaaaaaa1");
        assert_eq!(told["main"], "ccccccc3");
        assert_eq!(told["others"], 1);
        assert_eq!(told["pending"][0]["subject"], "semmeltv: a deploy button");
        assert_eq!(told["pending"][0]["reports"][0]["id"], id.as_str());
        assert_eq!(told["pending"][0]["reports"][0]["message"], "A deploy button");
        assert_eq!(told["asked"], Value::Null);

        let (status, asked) = post(&server, "/api/deploy", json!({}));
        assert_eq!(status, 200, "{asked}");
        assert_eq!(asked["asked"]["by"], "someone");
        let (_, running) = post(&server, "/api/deploy/progress", json!({ "state": "running" }));
        assert_eq!(running["asked"], Value::Null);
        assert_eq!(running["run"]["state"], "running");
        assert_eq!(running["run"]["by"], "someone");
        assert_eq!(post(&server, "/api/deploy", json!({})).0, 409);
        let (_, failed) = post(
            &server,
            "/api/deploy/progress",
            json!({ "state": "failed", "message": "the build failed" }),
        );
        assert_eq!(failed["run"]["state"], "failed");
        assert_eq!(failed["run"]["message"], "the build failed");
        assert_eq!(post(&server, "/api/deploy", json!({})).0, 200);

        std::fs::write(dir.join("revision"), "ccccccc3").unwrap();
        let (_, now) = get(&server, "/api/deploy");
        assert_eq!(now["pending"], json!([]));
        assert_eq!(now["others"], 0);
        std::fs::write(dir.join("revision"), "deadbeef").unwrap();
        assert_eq!(get(&server, "/api/deploy").1["pending"], Value::Null);
    }

    #[test]
    fn beast_holds_a_deploy_while_the_tv_plays_and_the_page_shows_it_waits() {
        let server = serve(no_root());
        assert_eq!(get(&server, "/api/deploy/watching").1, json!({ "watching": [] }));
        server.bus.playback(Some(NowPlaying {
            title: "Dune".to_string(),
            paused: true,
            ..NowPlaying::default()
        }));
        let (status, watching) = get(&server, "/api/deploy/watching");
        assert_eq!(status, 200, "{watching}");
        assert_eq!(watching["watching"][0]["device"], "the TV");
        assert_eq!(watching["watching"][0]["title"], "Dune");
        assert_eq!(
            watching["watching"][0]["paused"], true,
            "a pause still counts as watching"
        );

        let (status, held) = post(&server, "/api/deploy/wait", json!({}));
        assert_eq!(status, 200, "{held}");
        assert_eq!(held["waiting"]["by"], "someone");
        assert_eq!(get(&server, "/api/deploy").1["waiting"]["by"], "someone");
        let (_, running) = post(&server, "/api/deploy/progress", json!({ "state": "running" }));
        assert_eq!(running["waiting"], Value::Null);
        assert_eq!(running["run"]["by"], "someone");

        post(&server, "/api/deploy/wait", json!({}));
        let mut response = agent()
            .delete(format!("{}/api/deploy/wait", server.base))
            .call()
            .unwrap();
        let stopped: Value = response.body_mut().read_json().unwrap();
        assert_eq!(stopped["waiting"], Value::Null);

        server.bus.playback(None);
        assert_eq!(get(&server, "/api/deploy/watching").1, json!({ "watching": [] }));
    }

    #[test]
    fn feedback_is_kept_as_files_and_read_back_newest_first() {
        let server = serve(no_root());
        let trail = json!([{ "kind": "tap", "message": "button \"Play\"" }]);
        let sent = json!({ "message": "The list jumped", "page": "/shows/dark", "client": { "device": "iPhone 1a2b", "trail": trail } });
        let (status, first) = post(&server, "/api/feedback", sent.clone());
        assert_eq!(status, 201, "{first}");
        let (_, second) = post(&server, "/api/feedback", sent);
        let (status, list) = get(&server, "/api/feedback");
        assert_eq!(status, 200);
        assert_eq!(list["reports"][0]["id"], second["id"]);
        assert_eq!(list["reports"][1]["id"], first["id"]);
        assert_eq!(list["reports"][0]["device"], "iPhone 1a2b");
        assert_eq!(list["reports"][0]["page"], "/shows/dark");
        let (status, report) = get(&server, &format!("/api/feedback/{}", first["id"].as_str().unwrap()));
        assert_eq!(status, 200);
        assert_eq!(report["message"], "The list jumped");
        assert_eq!(report["client"]["trail"], trail);
        assert!(report["tv"]["screen"].is_object());
        assert_eq!(get(&server, "/api/feedback/..%2Fsemmeltv").0, 404);
        assert_eq!(post(&server, "/api/feedback", json!({ "message": "  " })).0, 422);
    }

    #[test]
    fn feedback_is_new_until_someone_marks_it_and_lists_by_status() {
        let server = serve(no_root());
        let sent = json!({ "message": "The sheet reopens" });
        let (_, first) = post(&server, "/api/feedback", sent.clone());
        let (_, second) = post(&server, "/api/feedback", sent);
        let (first, second) = (first["id"].as_str().unwrap(), second["id"].as_str().unwrap());
        let (_, list) = get(&server, "/api/feedback");
        assert_eq!(list["reports"][0]["status"], json!({ "state": "new" }));
        assert_eq!(list["reports"][0]["marked_at"], Value::Null);

        let review = json!({ "state": "needs_review", "why": "Two ways to fix it", "decide": "Which one" });
        let (status, marked) = post(&server, &format!("/api/feedback/{first}/status"), review.clone());
        assert_eq!(status, 200, "{marked}");
        assert_eq!(marked["status"], review);

        let (_, new) = get(&server, "/api/feedback?state=new");
        let ids: Vec<&str> = new["reports"]
            .as_array()
            .unwrap()
            .iter()
            .map(|report| report["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, [second]);
        let (_, waiting) = get(&server, "/api/feedback?state=needs_review");
        assert_eq!(waiting["reports"][0]["id"], first);
        assert_eq!(waiting["reports"][0]["status"], review);
        assert!(waiting["reports"][0]["marked_at"].as_u64().is_some());
        let (_, report) = get(&server, &format!("/api/feedback/{first}"));
        assert_eq!(report["status"], review);
        assert_eq!(report["message"], "The sheet reopens");

        let done = json!({ "state": "done", "resolution": { "kind": "no_change", "why": "Works as meant" } });
        assert_eq!(post(&server, &format!("/api/feedback/{first}/status"), done).0, 200);
        let (_, finished) = get(&server, "/api/feedback?state=done");
        assert_eq!(finished["reports"][0]["id"], first);
        assert_eq!(
            post(
                &server,
                &format!("/api/feedback/{first}/status"),
                json!({ "state": "new" })
            )
            .0,
            200
        );
        assert_eq!(
            get(&server, "/api/feedback?state=new").1["reports"]
                .as_array()
                .unwrap()
                .len(),
            2
        );

        assert_eq!(
            post(
                &server,
                "/api/feedback/20200101-000000-0000/status",
                json!({ "state": "new" })
            )
            .0,
            404
        );
        assert_eq!(
            post(
                &server,
                &format!("/api/feedback/{first}/status"),
                json!({ "state": "needs_review", "why": " ", "decide": "x" })
            )
            .0,
            422
        );
    }

    #[test]
    fn a_comment_joins_the_thread_and_reopens_a_settled_report() {
        let server = serve(no_root());
        let (_, sent) = post(&server, "/api/feedback", json!({ "message": "The sheet reopens" }));
        let id = sent["id"].as_str().unwrap();
        let review = json!({ "state": "needs_review", "why": "Two ways", "decide": "a) close it, b) keep it" });
        assert_eq!(
            post(&server, &format!("/api/feedback/{id}/status"), review.clone()).0,
            200
        );

        let (status, answer) = post(
            &server,
            &format!("/api/feedback/{id}/comments"),
            json!({ "text": " 1a " }),
        );
        assert_eq!(status, 201, "{answer}");
        assert_eq!(answer["comment"]["text"], "1a");
        assert_eq!(
            answer["comment"]["author"],
            json!({ "kind": "person", "name": "someone" })
        );
        assert_eq!(answer["status"], json!({ "state": "reopened", "was": review }));

        let (_, reopened) = get(&server, "/api/feedback?state=reopened");
        assert_eq!(reopened["reports"][0]["id"], id);
        assert_eq!(reopened["reports"][0]["comments"][0]["text"], "1a");
        assert_eq!(get(&server, "/api/feedback?state=needs_review").1["reports"], json!([]));
        let (_, report) = get(&server, &format!("/api/feedback/{id}"));
        assert_eq!(report["comments"].as_array().unwrap().len(), 1);

        let (_, again) = post(
            &server,
            &format!("/api/feedback/{id}/comments"),
            json!({ "text": "and hurry" }),
        );
        assert_eq!(again["status"]["state"], "reopened");
        assert_eq!(again["status"]["was"], review);
        assert_eq!(
            post(
                &server,
                &format!("/api/feedback/{id}/comments"),
                json!({ "text": "  " })
            )
            .0,
            422
        );
        assert_eq!(
            post(
                &server,
                "/api/feedback/20200101-000000-0000/comments",
                json!({ "text": "x" })
            )
            .0,
            404
        );
        assert_eq!(
            post(
                &server,
                &format!("/api/feedback/{id}/status"),
                json!({ "state": "reopened", "was": { "state": "new" } })
            )
            .0,
            422
        );
    }

    fn no_root() -> PathBuf {
        PathBuf::from("/nonexistent")
    }

    #[test]
    fn the_library_json_lists_cards_with_progress() {
        let server = serve(no_root());
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
        let finished: Vec<(&str, bool)> = ["shows", "movies"]
            .iter()
            .flat_map(|list| body[list].as_array().unwrap())
            .map(|card| (card["key"].as_str().unwrap(), card["finished"].as_bool().unwrap()))
            .collect();
        assert_eq!(
            finished,
            [
                ("the-bear", true),
                ("outlander", false),
                ("dune-2021", false),
                ("up-2009", false)
            ]
        );
    }

    #[test]
    fn a_show_lists_its_seasons_and_where_to_carry_on() {
        let server = serve(no_root());
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
        assert_eq!(about["tmdb_id"], 438631);
        assert_eq!(about["scores"]["letterboxd"], 4.1);
        assert_eq!(
            about["cast"],
            json!([{ "name": "Timothée Chalamet", "character": "Paul Atreides", "photo": null,
                     "imdb": "https://www.imdb.com/find/?q=Timoth%C3%A9e+Chalamet&s=nm" }])
        );
    }

    fn send(server: &Server, method: &str, path: &str) -> (u16, Value) {
        let url = format!("{}{path}", server.base);
        let mut response = match method {
            "PUT" => agent().put(url).send_empty(),
            "DELETE" => agent().delete(url).call(),
            _ => agent().post(url).send_empty(),
        }
        .unwrap();
        let status = response.status().as_u16();
        (status, response.body_mut().read_json().unwrap_or(Value::Null))
    }

    #[test]
    fn a_show_says_who_watches_it_together_with_me_and_where_they_are() {
        let server = serve(no_root());
        let together = |server: &Server| get(server, "/api/shows/outlander").1["together"].clone();
        let names = |people: &Value| -> Vec<String> {
            people
                .as_array()
                .unwrap()
                .iter()
                .map(|person| person["name"].as_str().unwrap().to_string())
                .collect()
        };
        let before = together(&server);
        assert_eq!(before["partners"], json!([]));
        assert_eq!(names(&before["others"]), ["Mami", "Miriam", "Ramu"]);
        assert_eq!(
            send(&server, "PUT", "/api/shows/outlander/together/2"),
            (
                200,
                json!({ "message": "You watch Outlander together with Mami", "ok": true })
            )
        );
        let after = together(&server);
        assert_eq!(names(&after["partners"]), ["Mami"]);
        assert_eq!(after["partners"][0]["spot"], json!({ "state": "not_started" }));
        assert_eq!(after["partners"][0]["with_me"], false);
        assert_eq!(names(&after["others"]), ["Miriam", "Ramu"]);
        assert_eq!(send(&server, "PUT", "/api/shows/outlander/together/1").0, 400);
        assert_eq!(send(&server, "PUT", "/api/shows/outlander/together/99").0, 404);
        assert_eq!(send(&server, "PUT", "/api/shows/nothing/together/2").0, 404);
        let stranger = send(&server, "POST", "/api/shows/outlander/together/3/catch-up");
        assert_eq!(
            stranger,
            (
                422,
                json!({ "message": "You don't watch Outlander together with Miriam", "ok": false })
            )
        );
        assert_eq!(server.asked.try_iter().count(), 0);
        assert_eq!(send(&server, "POST", "/api/shows/outlander/together/2/catch-up").0, 200);
        assert_eq!(
            server.asked.try_iter().collect::<Vec<_>>(),
            [r#"CatchUp { show: "outlander", to: ProfileId(2), by: ProfileId(1) }"#]
        );
        assert_eq!(send(&server, "DELETE", "/api/shows/outlander/together/2").0, 200);
        assert_eq!(together(&server)["partners"], json!([]));
    }

    #[test]
    fn a_phone_turns_notifications_on_and_off_and_hears_that_they_are_on() {
        let server = serve(no_root());
        let (status, key) = get(&server, "/api/push");
        assert_eq!(status, 200);
        assert_eq!(key["key"].as_str().unwrap().len(), 87);
        let phone = json!({
            "endpoint": "https://web.push.apple.com/QGz8d3",
            "keys": { "p256dh": "BKey", "auth": "secret" },
        });
        assert_eq!(
            post(&server, "/api/push/subscriptions", phone.clone()),
            (
                200,
                json!({ "message": "Notifications are on for this device", "ok": true })
            )
        );
        let push = server.core.push().clone();
        let mut sent = Vec::new();
        for _ in 0..100 {
            sent = push.sent();
            if !sent.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(sent[0].0, "https://web.push.apple.com/QGz8d3");
        assert_eq!(sent[0].1.title, "Notifications are on");
        assert!(sent[0].1.body.contains("adds to the library"));
        for _ in 0..3 {
            assert_eq!(post(&server, "/api/push/subscriptions", phone.clone()).0, 200);
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert_eq!(push.sent().len(), 1, "opening Settings again says nothing");
        for endpoint in [
            "http://web.push.apple.com/x",
            "https://127.0.0.1/x",
            "https://localhost:8090/api",
            "https://semmeltv5.semmeltv.ch/api/play",
        ] {
            let body = json!({ "endpoint": endpoint, "keys": { "p256dh": "B", "auth": "a" } });
            assert_eq!(post(&server, "/api/push/subscriptions", body).0, 400, "{endpoint}");
        }
        let mut response = agent()
            .delete(format!("{}/api/push/subscriptions", server.base))
            .force_send_body()
            .send_json(json!({ "endpoint": "https://web.push.apple.com/QGz8d3" }))
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        assert_eq!(response.body_mut().read_json::<Value>().unwrap()["ok"], true);
        let left = server
            ._runtime
            .block_on(server.core.db().subscriptions(ProfileId::FIRST))
            .unwrap();
        assert_eq!(left, []);
    }

    #[test]
    fn artwork_is_served_with_caching_headers() {
        let root = std::env::temp_dir().join(format!("semmeltv-api-art-{}", std::process::id()));
        let art = root.join(artwork::DIR);
        let file = artwork::backdrop(&art, Kind::Movie, 438_631);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"jpeg").unwrap();
        std::fs::write(artwork::poster(&art, Kind::Movie, 438_631), b"poster").unwrap();
        let server = serve(root.clone());
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

        let broken = agent().get(format!("{url}?w=320")).call().unwrap();
        assert_eq!(broken.headers()["content-type"], "image/jpeg");
        let face = artwork::face(&art, 7);
        std::fs::create_dir_all(face.parent().unwrap()).unwrap();
        image::RgbImage::from_pixel(artwork::FACE_WIDTH, artwork::FACE_HEIGHT, image::Rgb([90, 90, 90]))
            .save_with_format(&face, image::ImageFormat::Jpeg)
            .unwrap();
        let mut small = agent()
            .get(format!("{}/api/people/7/photo?w=100", server.base))
            .call()
            .unwrap();
        assert_eq!(small.status(), 200);
        assert_eq!(small.headers()["content-type"], "image/webp");
        assert_eq!(small.headers()["cache-control"], IMAGE_CACHE);
        assert!(small.body_mut().read_to_vec().unwrap().starts_with(b"RIFF"));
        assert!(face.with_file_name("7.w160.webp").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_event_stream_starts_with_playback_and_carries_notices() {
        let server = serve(no_root());
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
        server.bus.notice(Notice {
            heading: "Dune".to_string(),
            status: "Not found".to_string(),
            ok: false,
        });
        assert_eq!(
            next(),
            json!({ "type": "notice", "heading": "Dune", "status": "Not found", "ok": false })
        );
        server.bus.playback(None);
        assert_eq!(next(), json!({ "type": "playback", "now_playing": null }));
    }

    #[test]
    fn api_routes_forward_to_the_core() {
        let server = serve(no_root());
        let unsupported = "Volume control is not supported by this TV";
        let rows: Vec<(&str, &str, Value, u16, Vec<(&str, Value)>, Option<&str>)> = vec![
            (
                "POST",
                "/api/forgotten/dismiss",
                json!({ "kind": "show", "key": "the-bear" }),
                200,
                vec![("/ok", json!(true))],
                Some(r#"Dismiss(Series, "the-bear", ProfileId(1))"#),
            ),
            (
                "POST",
                "/api/watched",
                json!({ "kind": "show", "key": "outlander", "season": 1, "episode": 1, "watched": true }),
                200,
                vec![("/ok", json!(true)), ("/watched", json!(true))],
                Some(r#"Mark(Series, "outlander", Some((1, 1)), Watched, ProfileId(1))"#),
            ),
            (
                "POST",
                "/api/watched",
                json!({ "kind": "movie", "key": "dune-2021", "watched": false }),
                200,
                vec![("/watched", json!(false))],
                Some(r#"Mark(Movie, "dune-2021", None, Unwatched, ProfileId(1))"#),
            ),
            (
                "POST",
                "/api/watched",
                json!({ "kind": "movie", "key": "dune-2021", "season": 1, "episode": 1, "watched": true }),
                400,
                vec![],
                None,
            ),
            (
                "POST",
                "/api/watched",
                json!({ "kind": "movie", "key": "gone", "watched": true }),
                422,
                vec![("/ok", json!(false))],
                Some(r#"Mark(Movie, "gone", None, Watched, ProfileId(1))"#),
            ),
            (
                "GET",
                "/api/search?q=outl",
                Value::Null,
                200,
                vec![("/results/0/title", json!("Outlander")), ("/results/1", Value::Null)],
                None,
            ),
            (
                "GET",
                "/api/search?q=zzz",
                Value::Null,
                200,
                vec![("/results", json!([]))],
                None,
            ),
            (
                "POST",
                "/api/play",
                json!({ "show": "outlander", "season": 2, "episode": 1 }),
                200,
                vec![("", json!({ "message": "done", "ok": true }))],
                Some(r#"Play(Show { key: "outlander", season: Some(2), episode: Some(1) }, ProfileId(1))"#),
            ),
            ("POST", "/api/play", json!({ "movie": "jaws" }), 404, vec![], None),
            ("POST", "/api/play", json!({}), 400, vec![], None),
            (
                "POST",
                "/api/playback/pause",
                json!({}),
                200,
                vec![],
                Some("Control(Pause)"),
            ),
            (
                "POST",
                "/api/playback/seek",
                json!({ "by": -30.0 }),
                200,
                vec![],
                Some("Seek(By(-30.0))"),
            ),
            (
                "POST",
                "/api/playback/skip",
                json!({ "direction": "back" }),
                200,
                vec![],
                Some("Seek(Step(Back))"),
            ),
            (
                "POST",
                "/api/playback/skip-segment",
                json!({}),
                200,
                vec![],
                Some("Seek(Segment)"),
            ),
            (
                "POST",
                "/api/playback/hide-skip",
                json!({}),
                200,
                vec![],
                Some("HideSkip"),
            ),
            (
                "POST",
                "/api/playback/skip",
                json!({ "direction": "up" }),
                422,
                vec![],
                None,
            ),
            ("POST", "/api/playback/dance", json!({}), 404, vec![], None),
            ("POST", "/api/restarting", json!({}), 200, vec![], Some("Restarting")),
            ("POST", "/api/tv/on", json!({}), 200, vec![], Some("Control(TvOn)")),
            ("POST", "/api/tv/off", json!({}), 200, vec![], Some("Control(TvOff)")),
            (
                "POST",
                "/api/tv/semmeltv",
                json!({}),
                200,
                vec![],
                Some("Control(ShowSemmelTv)"),
            ),
            ("POST", "/api/tv/dance", json!({}), 404, vec![], None),
            ("GET", "/api/audio", Value::Null, 404, vec![], None),
            (
                "POST",
                "/api/audio",
                json!({ "output": "headphones" }),
                404,
                vec![],
                None,
            ),
            ("POST", "/api/audio/pair", json!({}), 404, vec![], None),
            (
                "POST",
                "/api/volume/up",
                json!({}),
                422,
                vec![("/message", json!(unsupported))],
                None,
            ),
            ("POST", "/api/volume/sideways", json!({}), 404, vec![], None),
            ("POST", "/api/command", json!({ "text": "play" }), 422, vec![], None),
        ];
        for (method, path, sent, status, answer, asked) in rows {
            let row = format!("{method} {path} {sent}");
            let (got, reply) = match method {
                "GET" => get(&server, path),
                _ => post(&server, path, sent),
            };
            assert_eq!(got, status, "{row}: {reply}");
            for (pointer, want) in answer {
                assert_eq!(reply.pointer(pointer).unwrap_or(&Value::Null), &want, "{row} {pointer}");
            }
            if let Some(asked) = asked {
                assert_eq!(server.asked.recv().unwrap(), asked, "{row}");
            }
        }
    }

    #[test]
    fn dialogue_mode_is_off_until_switched_on_and_stays_on() {
        let server = serve(no_root());
        assert_eq!(get(&server, "/api/audio/dialogue"), (200, json!({ "on": false })));
        let (status, _) = post(&server, "/api/audio/dialogue", json!({ "on": true }));
        assert_eq!(status, 200);
        assert_eq!(get(&server, "/api/audio/dialogue"), (200, json!({ "on": true })));
        assert_eq!(post(&server, "/api/audio/dialogue", json!({ "on": "yes" })).0, 422);
    }
}
