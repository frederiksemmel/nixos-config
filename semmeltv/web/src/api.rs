use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone)]
pub struct Api {
    base: String,
    client: reqwest::Client,
}

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
    pub subtitle: String,
    pub poster: Option<String>,
    pub backdrop: Option<String>,
    pub ratings: Ratings,
}

impl Card {
    pub fn href(&self) -> String {
        match self.kind {
            Kind::Movie => format!("/movies/{}", self.key),
            Kind::Show => format!("/shows/{}", self.key),
        }
    }
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
    pub newest: Vec<Card>,
    pub shows: Vec<Card>,
    pub movies: Vec<Card>,
}

#[derive(Debug, Deserialize)]
struct Results {
    results: Vec<Card>,
}

#[derive(Debug, Deserialize)]
pub struct Watch {
    pub details: String,
    pub stopped_at: Option<f64>,
    pub duration: Option<f64>,
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
    pub year: Option<u16>,
    #[serde(flatten)]
    pub watch: Watch,
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
    #[serde(flatten)]
    pub watch: Watch,
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
    pub label: String,
}

#[derive(Debug, Deserialize)]
pub struct Show {
    #[serde(flatten)]
    pub card: Card,
    pub next: Option<Next>,
    pub seasons: Vec<Season>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct NowPlaying {
    pub title: String,
    pub caption: String,
    pub movie: Option<String>,
    pub show: Option<String>,
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    pub loading: bool,
    pub jump: Option<i32>,
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

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct TitleKey {
    pub kind: String,
    pub key: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Download {
    #[serde(flatten)]
    pub title: TitleKey,
    pub name: String,
    pub bytes: u64,
    pub complete: usize,
    pub total: usize,
    pub last_watched: Option<u64>,
    pub cached_at: u64,
    pub keep: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Storage {
    pub disk: Disk,
    pub used: u64,
    pub marks: Option<Marks>,
    pub titles: Vec<Download>,
    pub next: Vec<TitleKey>,
}

#[derive(Debug, Deserialize)]
struct Answer {
    message: String,
    ok: bool,
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
        }
    }

    pub fn events_url(&self) -> String {
        format!("{}/api/events", self.base.replacen("http", "ws", 1))
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<Found<T>, String> {
        let response = self
            .client
            .get(format!("{}{path}", self.base))
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
        let sent = self
            .client
            .post(format!("{}{path}", self.base))
            .json(&body)
            .send()
            .await;
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

    pub async fn seek_by(&self, seconds: f64) -> String {
        self.post("/api/playback/seek", json!({ "by": seconds })).await
    }

    pub async fn skip(&self, direction: &str) -> String {
        self.post("/api/playback/skip", json!({ "direction": direction })).await
    }

    pub async fn command(&self, text: &str) -> String {
        self.post("/api/command", json!({ "text": text })).await
    }

    pub async fn subtitles(&self) -> Result<Subtitles, String> {
        match self.get("/api/subtitles").await? {
            Found::Here(subtitles) => Ok(subtitles),
            Found::Missing => Err("the TV has no subtitle settings".to_string()),
        }
    }

    pub async fn change_subtitles(&self, changes: Value) -> Result<Subtitles, String> {
        let response = self
            .client
            .put(format!("{}/api/subtitles", self.base))
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

    pub async fn tv(&self, control: &str) -> String {
        self.post(&format!("/api/tv/{}", encode(control)), json!({})).await
    }

    pub async fn tv_abilities(&self) -> Result<TvAbilities, String> {
        match self.get("/api/tv").await? {
            Found::Here(abilities) => Ok(abilities),
            Found::Missing => Err("the TV has no TV controls".to_string()),
        }
    }

    pub async fn choose_subtitles(&self, track: Option<i64>) -> String {
        self.post("/api/subtitles/track", json!({ "track": track })).await
    }

    pub async fn storage(&self) -> Result<Storage, String> {
        match self.get("/api/storage").await? {
            Found::Here(storage) => Ok(storage),
            Found::Missing => Err("This TV keeps no downloads".to_string()),
        }
    }

    pub async fn remove_download(&self, title: &TitleKey) -> (bool, String) {
        let sent = self
            .client
            .post(format!("{}/api/storage/remove", self.base))
            .json(&json!({ "kind": title.kind, "key": title.key }))
            .send()
            .await;
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
    pub volume: bool,
    pub pairing: bool,
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
        let playing = r#"{"type":"playback","now_playing":{"title":"Dune","caption":"2021","movie":"dune-2021",
            "show":null,"season":null,"episode":null,"position":12.5,"duration":9000.0,"paused":false,"muted":false,"loading":false}}"#;
        let Event::Playback { now_playing: Some(now) } = serde_json::from_str(playing).unwrap() else {
            panic!("a playback event");
        };
        assert_eq!(now.title, "Dune");
        assert_eq!(now.position, 12.5);
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
