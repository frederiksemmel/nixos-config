use crate::manifest::{Scores, Tenths};
use crate::tmdb::Kind;
use serde::Deserialize;
use serde_json::Value;

const API: &str = "https://api.mdblist.com";
const STOPPING: [u16; 3] = [401, 403, 429];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Stop(String),
    Failed(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub imdb: Option<String>,
    pub scores: Scores,
}

#[derive(Debug, Deserialize)]
struct Answer {
    imdb_id: Option<String>,
    #[serde(default)]
    ratings: Vec<Rating>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Rating {
    source: String,
    #[serde(default)]
    value: Value,
    #[serde(default)]
    score: Value,
}

impl Rating {
    fn number(value: &Value) -> Option<f64> {
        match value {
            Value::Number(number) => number.as_f64(),
            Value::String(text) => text.trim().trim_end_matches('%').parse().ok(),
            _ => None,
        }
        .filter(|number| *number > 0.0)
    }

    fn value(&self) -> Option<f64> {
        Self::number(&self.value)
    }

    fn percent(&self) -> Option<u8> {
        self.value()
            .or_else(|| Self::number(&self.score))
            .filter(|percent| *percent <= 100.0)
            .map(|percent| percent.round() as u8)
    }

    fn stars(&self, most: f64) -> Option<Tenths> {
        self.value().filter(|value| *value <= most).and_then(Tenths::from_float)
    }

    fn tenths(&self) -> Option<Tenths> {
        match self.value() {
            Some(value) => Tenths::from_float(value),
            None => Tenths::from_float(Self::number(&self.score)? / 10.0),
        }
    }
}

fn reason(answer: Option<&Answer>, status: u16) -> String {
    answer
        .and_then(|answer| answer.error.clone())
        .filter(|error| !error.trim().is_empty())
        .unwrap_or_else(|| format!("HTTP {status}"))
}

pub fn parse(status: u16, body: &str) -> Result<Option<Found>, Error> {
    let answer = serde_json::from_str::<Answer>(body).ok();
    if status == 404 {
        return Ok(None);
    }
    if STOPPING.contains(&status) {
        return Err(Error::Stop(reason(answer.as_ref(), status)));
    }
    if status >= 400 {
        return Err(Error::Failed(reason(answer.as_ref(), status)));
    }
    let answer = answer.ok_or_else(|| Error::Failed("unreadable answer".to_string()))?;
    if let Some(error) = answer.error.as_deref() {
        let lower = error.to_lowercase();
        return if lower.contains("not found") {
            Ok(None)
        } else if lower.contains("limit") || lower.contains("key") {
            Err(Error::Stop(error.to_string()))
        } else {
            Err(Error::Failed(error.to_string()))
        };
    }
    let find = |source: &str| answer.ratings.iter().find(|rating| rating.source == source);
    let scores = Scores {
        imdb: find("imdb").and_then(Rating::tenths),
        rt: find("tomatoes").and_then(Rating::percent),
        popcorn: find("popcorn").and_then(Rating::percent),
        metacritic: find("metacritic").and_then(Rating::percent),
        letterboxd: find("letterboxd").and_then(|rating| rating.stars(5.0)),
        ebert: find("rogerebert").and_then(|rating| rating.stars(4.0)),
    };
    let imdb = answer
        .imdb_id
        .map(|id| id.trim().to_string())
        .filter(|id| id.starts_with("tt") && id.len() > 2);
    Ok(Some(Found { imdb, scores }))
}

pub fn path(kind: Kind, tmdb: u64) -> String {
    let kind = match kind {
        Kind::Movie => "movie",
        Kind::Tv => "show",
    };
    format!("{API}/tmdb/{kind}/{tmdb}")
}

pub struct Client {
    key: String,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(key: String) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .new_agent();
        Self { key, agent }
    }

    fn hidden(&self, text: String) -> String {
        text.replace(&self.key, "…")
    }

    pub fn ratings(&self, kind: Kind, tmdb: u64) -> Result<Option<Found>, Error> {
        let mut response = self
            .agent
            .get(path(kind, tmdb))
            .query("apikey", &self.key)
            .call()
            .map_err(|error| Error::Failed(self.hidden(error.to_string())))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|error| Error::Failed(self.hidden(error.to_string())))?;
        parse(status, &body).map_err(|error| match error {
            Error::Stop(reason) => Error::Stop(self.hidden(reason)),
            Error::Failed(reason) => Error::Failed(self.hidden(reason)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILM: &str = r#"{"id":14160,"imdb_id":"tt1049413","title":"Up","year":2009,"type":"movie","score":86,
        "ratings":[
            {"source":"imdb","value":8.3,"score":83,"votes":1200000},
            {"source":"metacritic","value":88,"score":88,"votes":37},
            {"source":"tomatoes","value":98,"score":98,"votes":296},
            {"source":"popcorn","value":90,"score":90,"votes":250000},
            {"source":"trakt","value":85,"score":85,"votes":50000},
            {"source":"letterboxd","value":4.1,"score":82,"votes":900000}
        ]}"#;

    const SHOW: &str = r#"{"id":70523,"imdb_id":"tt5753856","title":"Dark","type":"show",
        "ratings":[
            {"source":"imdb","value":8.7,"score":87,"votes":500000},
            {"source":"metacritic","value":null,"score":null,"votes":null},
            {"source":"tomatoes","value":95,"score":95,"votes":40},
            {"source":"popcorn","value":93,"score":93,"votes":2500}
        ]}"#;

    #[test]
    fn a_film_has_imdb_both_tomato_scores_and_metacritic() {
        let found = parse(200, FILM).unwrap().unwrap();
        assert_eq!(found.imdb.as_deref(), Some("tt1049413"));
        assert_eq!(
            found.scores,
            Scores {
                imdb: Some(Tenths(83)),
                rt: Some(98),
                popcorn: Some(90),
                metacritic: Some(88),
                letterboxd: Some(Tenths(41)),
                ebert: None,
            }
        );
    }

    #[test]
    fn a_show_has_rotten_tomatoes_too_and_empty_sources_are_absent() {
        let scores = parse(200, SHOW).unwrap().unwrap().scores;
        assert_eq!(
            (scores.imdb, scores.rt, scores.popcorn, scores.metacritic),
            (Some(Tenths(87)), Some(95), Some(93), None)
        );
    }

    #[test]
    fn a_zero_counts_as_no_score() {
        let json = r#"{"ratings":[{"source":"tomatoes","value":0,"score":0},{"source":"imdb","value":0}]}"#;
        assert!(parse(200, json).unwrap().unwrap().scores.is_empty());
    }

    #[test]
    fn the_normalised_score_stands_in_for_a_missing_value() {
        let json = r#"{"ratings":[{"source":"imdb","score":81},{"source":"popcorn","score":"77"}]}"#;
        let scores = parse(200, json).unwrap().unwrap().scores;
        assert_eq!((scores.imdb, scores.popcorn), (Some(Tenths(81)), Some(77)));
    }

    #[test]
    fn only_a_real_imdb_id_is_kept() {
        let found = parse(200, r#"{"imdb_id":"","ratings":[]}"#).unwrap().unwrap();
        assert_eq!(found, Found::default());
    }

    #[test]
    fn an_unknown_title_is_not_an_error() {
        assert_eq!(parse(404, r#"{"error":"Not found."}"#), Ok(None));
        assert_eq!(parse(200, r#"{"response":false,"error":"Movie not found"}"#), Ok(None));
    }

    #[test]
    fn limits_and_a_bad_key_stop_the_run() {
        assert_eq!(
            parse(429, r#"{"error":"Daily API limit exceeded!"}"#),
            Err(Error::Stop("Daily API limit exceeded!".to_string()))
        );
        assert_eq!(parse(401, "<html>"), Err(Error::Stop("HTTP 401".to_string())));
        assert!(matches!(parse(403, "{}"), Err(Error::Stop(_))));
        assert!(matches!(
            parse(200, r#"{"response":false,"error":"Invalid API key!"}"#),
            Err(Error::Stop(_))
        ));
    }

    #[test]
    fn other_failures_skip_only_this_title() {
        assert_eq!(parse(500, "oops"), Err(Error::Failed("HTTP 500".to_string())));
        assert!(matches!(parse(200, "<html>"), Err(Error::Failed(_))));
    }

    #[test]
    fn titles_are_looked_up_by_their_tmdb_id() {
        assert_eq!(path(Kind::Movie, 14160), "https://api.mdblist.com/tmdb/movie/14160");
        assert_eq!(path(Kind::Tv, 70523), "https://api.mdblist.com/tmdb/show/70523");
    }

    #[test]
    fn the_key_never_shows_in_a_message() {
        let client = Client::new("s3cr3tk3y".to_string());
        assert_eq!(
            client.hidden("https://api.mdblist.com/tmdb/movie/1?apikey=s3cr3tk3y".to_string()),
            "https://api.mdblist.com/tmdb/movie/1?apikey=…"
        );
    }
}
