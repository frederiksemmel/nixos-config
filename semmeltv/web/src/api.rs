use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone)]
pub struct Api {
    base: String,
    client: reqwest::Client,
    forwarded: Vec<(reqwest::header::HeaderName, reqwest::header::HeaderValue)>,
}

const FORWARDED: [reqwest::header::HeaderName; 4] = [
    reqwest::header::COOKIE,
    reqwest::header::ORIGIN,
    reqwest::header::USER_AGENT,
    reqwest::header::HOST,
];

#[derive(Clone, Debug, Deserialize)]
pub struct Ratings {
    pub imdb: Option<f64>,
    pub rotten_tomatoes: Option<u8>,
    pub fresh: Option<bool>,
    pub popcorn: Option<u8>,
    pub full: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Movie,
    Show,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Card {
    pub kind: Kind,
    pub key: String,
    pub title: String,
    pub years: Option<String>,
    pub subtitle: String,
    pub poster: Option<String>,
    pub backdrop: Option<String>,
    pub ratings: Ratings,
    #[serde(default)]
    pub finished: bool,
    #[serde(default)]
    pub because: Option<String>,
}

impl Card {
    pub fn href(&self) -> String {
        match self.kind {
            Kind::Movie => format!("/movies/{}", self.key),
            Kind::Show => format!("/shows/{}", self.key),
        }
    }

    pub fn year(&self) -> Option<u16> {
        self.years.as_deref()?.get(..4)?.parse().ok()
    }
}

#[derive(Debug, Deserialize)]
pub struct Reporter {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct FeedbackSummary {
    pub id: String,
    pub at: u64,
    pub when: String,
    pub host: String,
    pub person: Option<Reporter>,
    pub device: Option<String>,
    pub page: Option<String>,
    pub message: String,
    pub screenshot: Option<String>,
    pub tv_screenshot: Option<String>,
    pub status: FeedbackStatus,
    pub marked_at: Option<u64>,
    pub marked_by: Option<String>,
    pub comments: Vec<FeedbackComment>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct FeedbackComment {
    pub at: u64,
    pub author: CommentAuthor,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommentAuthor {
    Person { name: String },
    Claude,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum FeedbackStatus {
    New,
    Reopened {
        was: Box<FeedbackStatus>,
    },
    NeedsReview {
        why: String,
        decide: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        questions: Vec<Question>,
    },
    Done {
        resolution: Resolution,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Question {
    pub ask: String,
    pub options: Vec<String>,
    pub recommended: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Resolution {
    Fixed { commit: String, what: String },
    NoChange { why: String },
}

#[derive(Debug, Deserialize)]
struct Remarked {
    status: FeedbackStatus,
}

#[derive(Debug, Deserialize)]
pub struct FeedbackList {
    pub dir: String,
    pub reports: Vec<FeedbackSummary>,
}

#[derive(Debug, Deserialize)]
pub struct WatchingCard {
    #[serde(flatten)]
    pub card: Card,
    pub detail: String,
    pub progress: f64,
}

#[derive(Debug, Deserialize)]
pub struct Library {
    pub watching: Vec<WatchingCard>,
    pub forgotten: Vec<WatchingCard>,
    pub newest: Vec<Card>,
    pub shows: Vec<Card>,
    pub movies: Vec<Card>,
}

#[derive(Debug, Deserialize)]
struct Results {
    results: Vec<Card>,
}

#[derive(Debug, Deserialize)]
pub struct Certificate {
    pub country: String,
    pub rating: String,
}

impl Certificate {
    pub fn label(&self) -> String {
        let age = !self.rating.is_empty() && self.rating.chars().all(|character| character.is_ascii_digit());
        if age {
            format!("{} {}+", self.country, self.rating)
        } else {
            format!("{} {}", self.country, self.rating)
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CastMember {
    pub name: String,
    pub character: Option<String>,
    pub photo: Option<String>,
    #[serde(default)]
    pub imdb: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct MoreScores {
    pub metacritic: Option<u8>,
    pub letterboxd: Option<f64>,
    pub ebert: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct About {
    pub tagline: Option<String>,
    pub plot: Option<String>,
    pub genres: Vec<String>,
    pub original_title: Option<String>,
    pub language: Option<String>,
    pub released: Option<String>,
    pub first_aired: Option<String>,
    pub last_aired: Option<String>,
    pub seasons: Option<u32>,
    pub episodes: Option<u32>,
    pub networks: Vec<String>,
    pub directors: Vec<String>,
    pub creators: Vec<String>,
    pub certificate: Option<Certificate>,
    pub trailer: Option<String>,
    pub imdb: Option<String>,
    pub tmdb: Option<String>,
    pub tmdb_id: Option<u64>,
    pub cast: Vec<CastMember>,
    pub scores: MoreScores,
}

#[derive(Debug, Deserialize)]
pub struct Played {
    pub last_played: Option<u64>,
    pub last_played_label: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Watch {
    pub details: String,
    pub stopped_at: Option<f64>,
    pub duration: Option<f64>,
    pub runtime_minutes: Option<u32>,
    #[serde(default)]
    pub tags: Tags,
}

#[derive(Debug, Default, Deserialize)]
pub struct Tags {
    pub video: Option<String>,
    pub resolution: Option<String>,
    pub audio: Option<String>,
}

impl Tags {
    pub fn line(&self) -> String {
        [&self.video, &self.resolution, &self.audio]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

impl Watch {
    pub fn fraction(&self) -> Option<f64> {
        let (at, of) = (self.stopped_at?, self.duration?);
        (of > 0.0 && at > 0.0).then(|| (at / of).clamp(0.0, 1.0))
    }
}

#[derive(Debug, Deserialize)]
pub struct Movie {
    #[serde(flatten)]
    pub card: Card,
    #[serde(flatten)]
    pub watch: Watch,
    #[serde(default)]
    pub about: About,
    #[serde(flatten)]
    pub played: Played,
    #[serde(default)]
    pub subtitles: Option<u32>,
    #[serde(default)]
    pub downloaded_by: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Episode {
    pub season: u32,
    pub episode: u32,
    pub label: String,
    pub short_label: String,
    pub still: Option<String>,
    pub ratings: Ratings,
    pub band: Option<String>,
    #[serde(default)]
    pub watched: bool,
    #[serde(flatten)]
    pub watch: Watch,
    #[serde(default)]
    pub subtitles: Option<u32>,
}

#[derive(Debug, PartialEq)]
pub struct HeatCell {
    pub episode: u32,
    pub rating: String,
    pub band: String,
}

#[derive(Debug, PartialEq)]
pub struct HeatRow {
    pub season: u32,
    pub cells: Vec<Option<HeatCell>>,
}

#[derive(Debug, Deserialize)]
pub struct Season {
    pub number: u32,
    pub label: String,
    pub episodes: Vec<Episode>,
}

impl Show {
    pub fn heat(&self) -> Vec<HeatRow> {
        let regular = || self.seasons.iter().filter(|season| season.number > 0);
        let rated = |episode: &&Episode| episode.ratings.imdb.is_some() && episode.band.is_some();
        let episodes = || regular().flat_map(|season| &season.episodes);
        if !episodes().any(|episode| rated(&episode)) {
            return Vec::new();
        }
        let columns = episodes().map(|episode| episode.episode).max().unwrap_or(0);
        regular()
            .map(|season| HeatRow {
                season: season.number,
                cells: (1..=columns)
                    .map(|number| {
                        let episode = season.episodes.iter().find(|episode| episode.episode == number);
                        episode.filter(rated).map(|episode| HeatCell {
                            episode: number,
                            rating: format!("{:.1}", episode.ratings.imdb.unwrap_or_default()),
                            band: episode.band.clone().unwrap_or_default(),
                        })
                    })
                    .collect(),
            })
            .collect()
    }
}

#[derive(Debug, Deserialize)]
pub struct Next {
    pub season: u32,
    #[serde(default)]
    pub episode: Option<u32>,
    pub label: String,
    pub title: Option<String>,
    pub stopped_at: Option<f64>,
    pub duration: Option<f64>,
}

impl Next {
    pub fn fraction(&self) -> Option<f64> {
        let (at, of) = (self.stopped_at?, self.duration?);
        (of > 0.0 && at > 0.0).then(|| (at / of).clamp(0.0, 1.0))
    }
}

#[derive(Debug, Deserialize)]
pub struct Show {
    #[serde(flatten)]
    pub card: Card,
    pub next: Option<Next>,
    pub seasons: Vec<Season>,
    #[serde(default)]
    pub about: About,
    #[serde(flatten)]
    pub played: Played,
    #[serde(default)]
    pub downloaded_by: Vec<String>,
    #[serde(default)]
    pub together: Together,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Together {
    #[serde(default)]
    pub partners: Vec<Partner>,
    #[serde(default)]
    pub others: Vec<Companion>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Companion {
    pub id: i64,
    pub name: String,
    pub color: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Partner {
    #[serde(flatten)]
    pub person: Companion,
    pub spot: Spot,
    #[serde(default)]
    pub with_me: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Spot {
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

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct NowPlaying {
    pub title: String,
    pub caption: String,
    #[serde(flatten)]
    pub playing: Playing,
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    #[serde(default = "normal_speed")]
    pub speed: f64,
    pub subtitle: Option<i64>,
    pub audio: Option<i64>,
    pub loading: bool,
    pub jump: Option<i32>,
    #[serde(default)]
    pub subtitle_delay: f64,
    #[serde(default)]
    pub skip: Option<Skip>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Skip {
    pub label: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Playing {
    Movie {
        movie: String,
    },
    Episode {
        show: String,
        season: u32,
        episode: u32,
    },
    #[default]
    File,
}

fn normal_speed() -> f64 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Voice {
    Listening,
    Transcribing,
    Heard {
        text: String,
        action: String,
        understood: bool,
    },
    Nothing,
    Cancelled,
    NoMicrophone,
    Failed {
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Event {
    Voice(Voice),
    Playback { now_playing: Option<NowPlaying> },
    Audio(Audio),
    Screen { screen: Screen },
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum PhonePlay {
    Starting { name: String },
    Playing { name: String, paused: bool },
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Screen {
    Tv,
    Moving { device: Device },
    Phone { device: Device, play: Option<PhonePlay> },
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Output {
    Tv,
    Headphones,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Link {
    Disconnected,
    Connecting,
    Connected,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Headphones {
    pub name: String,
    pub state: Link,
    #[serde(default)]
    pub volume: Option<Volume>,
    #[serde(default)]
    pub audio_delay: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct Volume {
    pub level: u8,
    pub muted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PairState {
    Scanning,
    Pairing,
    Paired,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Pairing {
    pub state: PairState,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Audio {
    pub output: Output,
    pub headphones: Option<Headphones>,
    pub pairing: Option<Pairing>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Outcome {
    message: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct Disk {
    pub total: u64,
    pub free: u64,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Marks {
    pub high: u64,
    pub low: u64,
    pub floor: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "key", rename_all = "lowercase")]
pub enum Stored {
    Movie(String),
    Series(String),
    Other(String),
}

impl Stored {
    pub fn parse(kind: &str, key: String) -> Option<Self> {
        match kind {
            "movie" => Some(Self::Movie(key)),
            "series" => Some(Self::Series(key)),
            "other" => Some(Self::Other(key)),
            _ => None,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Movie(_) => "movie",
            Self::Series(_) => "series",
            Self::Other(_) => "other",
        }
    }

    pub fn key(&self) -> &str {
        match self {
            Self::Movie(key) | Self::Series(key) | Self::Other(key) => key,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Keep {
    Playing,
    Watching,
    UpNext,
    Started,
}

impl Keep {
    pub fn label(self) -> &'static str {
        match self {
            Self::Playing => "Playing",
            Self::Watching => "Currently watching",
            Self::UpNext => "Up next",
            Self::Started => "Started",
        }
    }

    pub fn removable(self) -> bool {
        match self {
            Self::Playing | Self::UpNext => false,
            Self::Watching | Self::Started => true,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Download {
    #[serde(flatten)]
    pub title: Stored,
    pub name: String,
    pub bytes: u64,
    pub complete: usize,
    pub total: usize,
    pub last_watched: Option<u64>,
    pub cached_at: u64,
    pub keep: Option<Keep>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Storage {
    pub disk: Disk,
    pub used: u64,
    pub marks: Option<Marks>,
    pub titles: Vec<Download>,
    pub next: Vec<Stored>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Span {
    #[default]
    Day,
    Week,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct UsageTotals {
    pub tv: u64,
    pub phone: u64,
    pub plays: usize,
    pub people: usize,
    pub titles: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct UsageBucket {
    pub label: String,
    pub mark: String,
    pub tv: u64,
    pub phone: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct UsagePerson {
    pub name: String,
    pub tv: u64,
    pub phone: u64,
    pub plays: usize,
    pub days: usize,
    pub sign_ins: usize,
    pub favourite: Option<String>,
    pub last_seen: Option<String>,
    pub devices: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct UsageShare {
    pub name: String,
    pub seconds: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct UsageTitle {
    pub title: String,
    pub seconds: u64,
    pub plays: usize,
    pub people: Vec<UsageShare>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct UsagePlay {
    pub at: u64,
    pub when: String,
    pub name: String,
    pub device: String,
    pub tv: bool,
    pub title: String,
    pub seconds: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Usage {
    pub days: u64,
    pub totals: UsageTotals,
    pub span: Span,
    pub buckets: Vec<UsageBucket>,
    pub hours: [[u64; 24]; 7],
    pub people: Vec<UsagePerson>,
    pub titles: Vec<UsageTitle>,
    pub plays: Vec<UsagePlay>,
}

#[derive(Debug, Deserialize)]
struct Answer {
    message: String,
    ok: bool,
}

#[derive(Debug, Deserialize)]
struct Marked {
    message: String,
    ok: bool,
    watched: bool,
}

#[derive(Debug, Deserialize)]
struct Problem {
    error: String,
}

pub enum Found<T> {
    Here(T),
    Missing,
}

impl Api {
    pub fn new(base: String) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
            forwarded: Vec::new(),
        }
    }

    pub fn of(cx: &topcoat::context::Cx) -> Self {
        let mut api = topcoat::context::app_context::<Api>(cx).clone();
        let headers = topcoat::router::request::headers(cx);
        api.forwarded = FORWARDED
            .iter()
            .filter_map(|name| {
                let value = headers.get(name)?;
                let name = if name == reqwest::header::HOST {
                    reqwest::header::HeaderName::from_static("x-forwarded-host")
                } else {
                    name.clone()
                };
                Some((name, value.clone()))
            })
            .collect();
        api
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut request = self.client.request(method, format!("{}{path}", self.base));
        for (name, value) in &self.forwarded {
            request = request.header(name, value);
        }
        request
    }

    pub fn events_url(&self) -> String {
        format!("{}/api/events", self.base.replacen("http", "ws", 1))
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<Found<T>, String> {
        let response = self
            .request(reqwest::Method::GET, path)
            .send()
            .await
            .map_err(|error| format!("the TV is not answering: {error}"))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(Found::Missing);
        }
        let response = response.error_for_status().map_err(|error| error.to_string())?;
        response
            .json()
            .await
            .map(Found::Here)
            .map_err(|error| error.to_string())
    }

    async fn post(&self, path: &str, body: Value) -> String {
        let sent = self.request(reqwest::Method::POST, path).json(&body).send().await;
        let response = match sent {
            Ok(response) => response,
            Err(error) => return format!("The TV is not answering: {error}"),
        };
        let text = response.text().await.unwrap_or_default();
        serde_json::from_str::<Outcome>(&text)
            .map(|outcome| outcome.message)
            .or_else(|_| serde_json::from_str::<Problem>(&text).map(|problem| problem.error))
            .unwrap_or(text)
    }

    pub async fn me(&self) -> Result<Option<Me>, String> {
        let response = self
            .request(reqwest::Method::GET, "/api/auth/me")
            .send()
            .await
            .map_err(|error| format!("the TV is not answering: {error}"))?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Ok(None);
        }
        let response = response.error_for_status().map_err(|error| error.to_string())?;
        response.json().await.map(Some).map_err(|error| error.to_string())
    }

    pub async fn choices(&self) -> Result<Vec<Choice>, String> {
        match self.get::<Choices>("/api/auth/choices").await? {
            Found::Here(choices) => Ok(choices.profiles),
            Found::Missing => Err("the TV has no sign-in".to_string()),
        }
    }

    pub async fn invitee(&self, token: &str) -> Result<Result<Person, String>, String> {
        let response = self
            .request(reqwest::Method::GET, &format!("/api/auth/invite/{}", encode(token)))
            .send()
            .await
            .map_err(|error| format!("the TV is not answering: {error}"))?;
        let ok = response.status().is_success();
        let text = response.text().await.map_err(|error| error.to_string())?;
        if ok {
            #[derive(Deserialize)]
            struct Invitee {
                person: Person,
            }
            serde_json::from_str::<Invitee>(&text)
                .map(|found| Ok(found.person))
                .map_err(|error| error.to_string())
        } else {
            Ok(Err(
                serde_json::from_str::<Problem>(&text).map_or(text, |problem| problem.error)
            ))
        }
    }

    pub async fn tv_profile(&self) -> Result<TvProfile, String> {
        match self.get::<TvProfile>("/api/tv-profile").await? {
            Found::Here(found) => Ok(found),
            Found::Missing => Err("the TV has no profiles".to_string()),
        }
    }

    pub async fn members(&self) -> Result<Vec<Member>, String> {
        #[derive(Deserialize)]
        struct Members {
            members: Vec<Member>,
        }
        match self.get::<Members>("/api/auth/members").await? {
            Found::Here(found) => Ok(found.members),
            Found::Missing => Err("the TV has no sign-in".to_string()),
        }
    }

    pub async fn library(&self) -> Result<Library, String> {
        match self.get("/api/library").await? {
            Found::Here(library) => Ok(library),
            Found::Missing => Err("the TV has no library API".to_string()),
        }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Card>, String> {
        let path = format!("/api/search?q={}", encode(query));
        match self.get::<Results>(&path).await? {
            Found::Here(found) => Ok(found.results),
            Found::Missing => Ok(Vec::new()),
        }
    }

    pub async fn movie(&self, key: &str) -> Result<Found<Movie>, String> {
        self.get(&format!("/api/movies/{}", encode(key))).await
    }

    pub async fn show(&self, key: &str) -> Result<Found<Show>, String> {
        self.get(&format!("/api/shows/{}", encode(key))).await
    }

    pub async fn play(&self, pick: &Pick) -> String {
        self.post("/api/play", serde_json::to_value(pick).unwrap_or_default())
            .await
    }

    pub async fn control(&self, name: &str) -> String {
        self.post(&format!("/api/playback/{}", encode(name)), json!({})).await
    }

    pub async fn skip(&self, direction: &str) -> String {
        self.post("/api/playback/skip", json!({ "direction": direction })).await
    }

    pub async fn command(&self, text: &str) -> String {
        self.post("/api/command", json!({ "text": text })).await
    }

    pub async fn dialogue(&self) -> Result<bool, String> {
        #[derive(Deserialize)]
        struct Dialogue {
            on: bool,
        }
        match self.get::<Dialogue>("/api/audio/dialogue").await? {
            Found::Here(dialogue) => Ok(dialogue.on),
            Found::Missing => Err("This TV has no dialogue mode".to_string()),
        }
    }

    pub async fn set_dialogue(&self, on: bool) -> String {
        self.post("/api/audio/dialogue", json!({ "on": on })).await
    }

    pub async fn subtitles(&self) -> Result<Subtitles, String> {
        match self.get("/api/subtitles").await? {
            Found::Here(subtitles) => Ok(subtitles),
            Found::Missing => Err("the TV has no subtitle settings".to_string()),
        }
    }

    pub async fn change_subtitles(&self, changes: Value) -> Result<Subtitles, String> {
        let response = self
            .request(reqwest::Method::PUT, "/api/subtitles")
            .json(&changes)
            .send()
            .await
            .map_err(|error| format!("The TV is not answering: {error}"))?;
        let success = response.status().is_success();
        let text = response.text().await.map_err(|error| error.to_string())?;
        if success {
            serde_json::from_str(&text).map_err(|error| error.to_string())
        } else {
            Err(serde_json::from_str::<Problem>(&text).map_or(text, |problem| problem.error))
        }
    }

    pub async fn tv_abilities(&self) -> Result<TvAbilities, String> {
        match self.get("/api/tv").await? {
            Found::Here(abilities) => Ok(abilities),
            Found::Missing => Err("the TV has no TV controls".to_string()),
        }
    }

    pub async fn storage(&self) -> Result<Storage, String> {
        match self.get("/api/storage").await? {
            Found::Here(storage) => Ok(storage),
            Found::Missing => Err("This TV keeps no downloads".to_string()),
        }
    }

    pub async fn usage(&self, days: u64) -> Result<Usage, String> {
        match self.get(&format!("/api/usage?days={days}")).await? {
            Found::Here(usage) => Ok(usage),
            Found::Missing => Err("This TV keeps no usage yet".to_string()),
        }
    }

    pub async fn feedback(&self) -> Result<FeedbackList, String> {
        match self.get("/api/feedback").await? {
            Found::Here(list) => Ok(list),
            Found::Missing => Err("This TV keeps no feedback".to_string()),
        }
    }

    pub async fn mark_feedback(&self, id: &str, status: &FeedbackStatus) -> Result<FeedbackStatus, String> {
        let sent = self
            .request(reqwest::Method::POST, &format!("/api/feedback/{id}/status"))
            .json(status)
            .send()
            .await;
        let response = sent.map_err(|error| format!("The TV is not answering: {error}"))?;
        let text = response.text().await.unwrap_or_default();
        match serde_json::from_str::<Remarked>(&text) {
            Ok(remarked) => Ok(remarked.status),
            Err(_) => Err(serde_json::from_str::<Problem>(&text).map_or(text, |problem| problem.error)),
        }
    }

    pub async fn comment_feedback(&self, id: &str, text: &str) -> Result<FeedbackStatus, String> {
        let sent = self
            .request(reqwest::Method::POST, &format!("/api/feedback/{id}/comments"))
            .json(&json!({ "text": text }))
            .send()
            .await;
        let response = sent.map_err(|error| format!("The TV is not answering: {error}"))?;
        let text = response.text().await.unwrap_or_default();
        match serde_json::from_str::<Remarked>(&text) {
            Ok(remarked) => Ok(remarked.status),
            Err(_) => Err(serde_json::from_str::<Problem>(&text).map_or(text, |problem| problem.error)),
        }
    }

    pub async fn remove_download(&self, title: &Stored) -> (bool, String) {
        self.answer("/api/storage/remove", json!(title)).await
    }

    pub async fn dismiss_forgotten(&self, kind: Kind, key: &str) -> (bool, String) {
        let kind = match kind {
            Kind::Movie => "movie",
            Kind::Show => "show",
        };
        self.answer("/api/forgotten/dismiss", json!({ "kind": kind, "key": key }))
            .await
    }

    pub async fn mark_watched(&self, body: Value) -> Result<bool, String> {
        let sent = self
            .request(reqwest::Method::POST, "/api/watched")
            .json(&body)
            .send()
            .await;
        let response = sent.map_err(|error| format!("The TV is not answering: {error}"))?;
        let text = response.text().await.unwrap_or_default();
        match serde_json::from_str::<Marked>(&text) {
            Ok(marked) if marked.ok => Ok(marked.watched),
            Ok(marked) => Err(marked.message),
            Err(_) => Err(serde_json::from_str::<Problem>(&text).map_or(text, |problem| problem.error)),
        }
    }

    async fn answer(&self, path: &str, body: Value) -> (bool, String) {
        let sent = self.request(reqwest::Method::POST, path).json(&body).send().await;
        let response = match sent {
            Ok(response) => response,
            Err(error) => return (false, format!("The TV is not answering: {error}")),
        };
        let text = response.text().await.unwrap_or_default();
        match serde_json::from_str::<Answer>(&text) {
            Ok(answer) => (answer.ok, answer.message),
            Err(_) => (
                false,
                serde_json::from_str::<Problem>(&text).map_or(text, |problem| problem.error),
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Viewer,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Person {
    pub id: i64,
    pub name: String,
    pub role: Role,
    pub color: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

pub const CONTROL_TV: &str = "control-tv";
pub const HEADPHONES: &str = "headphones";
pub const DOWNLOAD: &str = "download";

impl Person {
    pub fn admin(&self) -> bool {
        self.role == Role::Admin
    }

    pub fn can(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|granted| granted == capability)
    }

    pub fn controls_tv(&self) -> bool {
        self.can(CONTROL_TV)
    }

    pub fn uses_headphones(&self) -> bool {
        self.can(HEADPHONES)
    }

    pub fn downloads(&self) -> bool {
        self.can(DOWNLOAD)
    }

    pub fn initial(&self) -> String {
        self.name
            .chars()
            .next()
            .map(|first| first.to_uppercase().collect())
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct PasskeyView {
    pub id: String,
    pub name: String,
    pub here: bool,
    pub created_at: u64,
    pub last_used_at: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Me {
    pub person: Person,
    pub others: Vec<Person>,
    pub passkeys: Vec<PasskeyView>,
    pub password: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Choice {
    #[serde(flatten)]
    pub person: Person,
    pub password: bool,
    pub passkey: bool,
    pub active: bool,
    pub signed_in: bool,
}

#[derive(Deserialize)]
struct Choices {
    profiles: Vec<Choice>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct TvProfile {
    pub name: Option<String>,
    pub mine: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Member {
    #[serde(flatten)]
    pub person: Person,
    pub password: bool,
    pub passkeys: usize,
    pub sessions: usize,
    pub last_seen_at: Option<u64>,
    pub invite_expires_at: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct SubtitleSettings {
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

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct SubtitleTrack {
    pub id: i64,
    pub title: Option<String>,
    pub label: String,
    pub forced: bool,
    pub hearing_impaired: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct SubtitlesPlaying {
    pub visible: bool,
    pub track: Option<i64>,
    pub tracks: Vec<SubtitleTrack>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct TvAbilities {
    pub kind: Option<String>,
    pub volume: bool,
    pub pairing: bool,
    #[serde(default)]
    pub paired: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Subtitles {
    pub settings: SubtitleSettings,
    pub playing: Option<SubtitlesPlaying>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Pick {
    Movie {
        movie: String,
    },
    Show {
        show: String,
        season: Option<u32>,
        episode: Option<u32>,
    },
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (byte as char).to_string(),
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_from_the_tv_are_understood() {
        let heard = r#"{"type":"voice","state":"heard","text":"play dune","action":"Playing Dune","understood":true}"#;
        assert_eq!(
            serde_json::from_str::<Event>(heard).unwrap(),
            Event::Voice(Voice::Heard {
                text: "play dune".to_string(),
                action: "Playing Dune".to_string(),
                understood: true
            })
        );
        let playing = r#"{"type":"playback","now_playing":{"title":"Dune","caption":"2021","kind":"movie","movie":"dune-2021",
            "position":12.5,"duration":9000.0,"paused":false,"muted":false,"loading":false}}"#;
        let Event::Playback { now_playing: Some(now) } = serde_json::from_str(playing).unwrap() else {
            panic!("a playback event");
        };
        assert_eq!(now.title, "Dune");
        assert_eq!(now.position, 12.5);
        assert_eq!(
            now.playing,
            Playing::Movie {
                movie: "dune-2021".to_string()
            }
        );
    }

    #[test]
    fn a_person_controls_the_tv_and_uses_headphones_only_with_those_capabilities() {
        let read = |capabilities: Value| {
            serde_json::from_value::<Person>(json!({
                "id": 2, "name": "Mami", "role": "viewer", "color": "#3E63DD", "capabilities": capabilities
            }))
            .unwrap()
        };
        assert!(read(json!(["control-tv"])).controls_tv());
        assert!(!read(json!(["control-tv"])).uses_headphones());
        assert!(read(json!(["headphones"])).uses_headphones());
        assert!(!read(json!(["headphones"])).controls_tv());
        assert!(!read(json!([])).controls_tv());
        assert!(read(json!(["download"])).downloads());
        assert!(!read(json!(["control-tv", "headphones"])).downloads());
    }

    #[test]
    fn a_title_names_who_downloaded_it_and_older_tvs_name_nobody() {
        let none = json!({ "imdb": null, "rotten_tomatoes": null, "fresh": null, "popcorn": null, "full": null });
        let movie = json!({ "kind": "movie", "key": "up-2009", "title": "Up", "subtitle": "", "poster": null,
            "backdrop": null, "ratings": none, "details": "", "stopped_at": null, "duration": null,
            "runtime_minutes": null, "last_played": null, "last_played_label": null });
        let older: Movie = serde_json::from_value(movie.clone()).unwrap();
        assert!(older.downloaded_by.is_empty());
        let mut newer = movie;
        newer["downloaded_by"] = json!(["Frederik"]);
        let newer: Movie = serde_json::from_value(newer).unwrap();
        assert_eq!(newer.downloaded_by, ["Frederik"]);
        let show = json!({ "kind": "show", "key": "dark", "title": "Dark", "subtitle": "", "poster": null,
            "backdrop": null, "ratings": none, "next": null, "seasons": [], "downloaded_by": ["Frederik", "Mami"] });
        let show: Show = serde_json::from_value(show).unwrap();
        assert_eq!(show.downloaded_by, ["Frederik", "Mami"]);
    }

    #[test]
    fn picks_are_the_json_the_tv_expects() {
        let movie = Pick::Movie {
            movie: "dune-2021".to_string(),
        };
        assert_eq!(serde_json::to_value(movie).unwrap(), json!({ "movie": "dune-2021" }));
        let episode = Pick::Show {
            show: "outlander".to_string(),
            season: Some(2),
            episode: Some(3),
        };
        assert_eq!(
            serde_json::to_value(episode).unwrap(),
            json!({ "show": "outlander", "season": 2, "episode": 3 })
        );
    }

    #[test]
    fn subtitle_settings_and_tracks_are_understood() {
        let settings = json!({ "shown": false, "languages": ["en", "de"], "size": 1.0, "position": 100.0,
            "color": "#FFFFFF", "outline": 1.65, "outline_color": "#000000", "shadow": 0.0, "background": 0.0,
            "restyle": false });
        let track = json!({ "id": 2, "language": "en", "title": "English SRT-SDH", "label": "English",
            "forced": false, "hearing_impaired": true, "external": false });
        let body = json!({ "settings": settings, "playing": { "visible": true, "track": 2, "tracks": [track] } });
        let subtitles: Subtitles = serde_json::from_value(body).unwrap();
        assert_eq!(subtitles.settings.languages, ["en", "de"]);
        let playing = subtitles.playing.unwrap();
        assert_eq!(playing.track, Some(2));
        assert!(playing.tracks[0].hearing_impaired);
        let idle: Subtitles = serde_json::from_value(json!({ "settings": settings, "playing": null })).unwrap();
        assert_eq!(idle.playing, None);
    }

    #[test]
    fn the_heat_grid_has_a_row_per_season_and_leaves_gaps_empty() {
        let none = json!({ "imdb": null, "rotten_tomatoes": null, "fresh": null, "popcorn": null, "full": null });
        let episode = |season: u32, number: u32, imdb: Option<f64>, band: Option<&str>| {
            json!({ "season": season, "episode": number, "label": "", "short_label": "", "still": null,
                "ratings": { "imdb": imdb, "rotten_tomatoes": null, "fresh": null, "popcorn": null, "full": null },
                "band": band, "details": "", "stopped_at": null, "duration": null })
        };
        let show = json!({ "kind": "show", "key": "dark", "title": "Dark", "subtitle": "", "poster": null,
            "backdrop": null, "ratings": none, "next": null, "seasons": [
                { "number": 1, "label": "Season 1", "episodes": [episode(1, 1, Some(8.1), Some("great")), episode(1, 3, None, None)] },
                { "number": 2, "label": "Season 2", "episodes": [episode(2, 2, Some(9.0), Some("awesome"))] },
                { "number": 0, "label": "Specials", "episodes": [episode(0, 1, Some(7.0), Some("good"))] },
            ] });
        let show: Show = serde_json::from_value(show).unwrap();
        let heat = show.heat();
        assert_eq!(heat.len(), 2);
        assert_eq!(heat[0].cells.len(), 3);
        assert_eq!(
            heat[0].cells[0],
            Some(HeatCell {
                episode: 1,
                rating: "8.1".to_string(),
                band: "great".to_string()
            })
        );
        assert_eq!((&heat[0].cells[1], &heat[0].cells[2]), (&None, &None));
        assert_eq!(heat[1].cells[1].as_ref().unwrap().rating, "9.0");
    }

    #[test]
    fn search_text_is_escaped_for_the_query_string() {
        assert_eq!(encode("the bear & co"), "the%20bear%20%26%20co");
    }
}
