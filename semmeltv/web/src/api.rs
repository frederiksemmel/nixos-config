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
    #[serde(flatten)]
    pub watch: Watch,
}

#[derive(Debug, Deserialize)]
pub struct Season {
    pub number: u32,
    pub label: String,
    pub episodes: Vec<Episode>,
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

    pub async fn command(&self, text: &str) -> String {
        self.post("/api/command", json!({ "text": text })).await
    }
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
    fn search_text_is_escaped_for_the_query_string() {
        assert_eq!(encode("the bear & co"), "the%20bear%20%26%20co");
    }
}
