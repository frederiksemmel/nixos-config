use crate::manifest::{Scores, Tenths};
use serde::Deserialize;

const API: &str = "https://www.omdbapi.com/";
const MISSING: [&str; 2] = ["Incorrect IMDb ID.", "Error getting data."];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Stop(String),
    Failed(String),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Answer {
    response: String,
    error: Option<String>,
    #[serde(default)]
    ratings: Vec<Rating>,
    #[serde(rename = "imdbRating")]
    imdb_rating: Option<String>,
    metascore: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Rating {
    source: String,
    value: String,
}

fn percent(text: &str) -> Option<u8> {
    text.trim()
        .strip_suffix('%')?
        .trim()
        .parse()
        .ok()
        .filter(|value| *value <= 100)
}

fn out_of_hundred(text: &str) -> Option<u8> {
    let score = text.trim();
    let score = score.strip_suffix("/100").unwrap_or(score);
    score.trim().parse().ok().filter(|value| *value <= 100)
}

fn out_of_ten(text: &str) -> Option<Tenths> {
    let score = text.trim();
    Tenths::parse(score.strip_suffix("/10").unwrap_or(score))
}

pub fn parse(json: &str) -> Result<Option<Scores>, Error> {
    let answer: Answer =
        serde_json::from_str(json).map_err(|error| Error::Failed(format!("unreadable answer: {error}")))?;
    if answer.response != "True" {
        let error = answer.error.unwrap_or_else(|| "no reason given".to_string());
        return if MISSING.contains(&error.as_str()) || error.ends_with("not found!") {
            Ok(None)
        } else {
            Err(Error::Stop(error))
        };
    }
    let rating = |source: &str| {
        answer
            .ratings
            .iter()
            .find(|rating| rating.source == source)
            .map(|rating| rating.value.as_str())
    };
    let scores = Scores {
        imdb: rating("Internet Movie Database")
            .and_then(out_of_ten)
            .or_else(|| answer.imdb_rating.as_deref().and_then(out_of_ten)),
        rt: rating("Rotten Tomatoes").and_then(percent),
        metacritic: rating("Metacritic")
            .and_then(out_of_hundred)
            .or_else(|| answer.metascore.as_deref().and_then(out_of_hundred)),
    };
    Ok(Some(scores))
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

    pub fn ratings(&self, imdb: &str) -> Result<Option<Scores>, Error> {
        let mut response = self
            .agent
            .get(API)
            .query("i", imdb)
            .query("apikey", &self.key)
            .call()
            .map_err(|error| Error::Failed(self.hidden(error.to_string())))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|error| Error::Failed(self.hidden(error.to_string())))?;
        match parse(&body) {
            Err(Error::Failed(_)) if status >= 400 => Err(Error::Failed(format!("HTTP {status}"))),
            Err(Error::Stop(reason)) => Err(Error::Stop(self.hidden(reason))),
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILM: &str = r#"{"Title":"Up","imdbRating":"8.3","Metascore":"88",
        "Ratings":[{"Source":"Internet Movie Database","Value":"8.3/10"},
                   {"Source":"Rotten Tomatoes","Value":"98%"},
                   {"Source":"Metacritic","Value":"88/100"}],
        "Response":"True"}"#;

    #[test]
    fn a_film_has_all_three_scores() {
        let scores = parse(FILM).unwrap().unwrap();
        assert_eq!(scores.imdb, Some(Tenths(83)));
        assert_eq!(scores.rt, Some(98));
        assert_eq!(scores.metacritic, Some(88));
    }

    #[test]
    fn a_show_often_lacks_the_critics() {
        let json = r#"{"Title":"Dark","imdbRating":"8.7","Metascore":"N/A",
            "Ratings":[{"Source":"Internet Movie Database","Value":"8.7/10"}],"Response":"True"}"#;
        let scores = parse(json).unwrap().unwrap();
        assert_eq!(
            (scores.imdb, scores.rt, scores.metacritic),
            (Some(Tenths(87)), None, None)
        );
    }

    #[test]
    fn the_headline_fields_stand_in_for_a_missing_list() {
        let json = r#"{"imdbRating":"6.4","Metascore":"51","Response":"True"}"#;
        let scores = parse(json).unwrap().unwrap();
        assert_eq!((scores.imdb, scores.metacritic), (Some(Tenths(64)), Some(51)));
    }

    #[test]
    fn nothing_rated_is_an_empty_set_of_scores() {
        let json = r#"{"imdbRating":"N/A","Metascore":"N/A","Ratings":[],"Response":"True"}"#;
        assert!(parse(json).unwrap().unwrap().is_empty());
    }

    #[test]
    fn an_unknown_title_is_not_an_error() {
        assert_eq!(parse(r#"{"Response":"False","Error":"Incorrect IMDb ID."}"#), Ok(None));
        assert_eq!(parse(r#"{"Response":"False","Error":"Movie not found!"}"#), Ok(None));
    }

    #[test]
    fn the_daily_limit_and_a_bad_key_stop_the_run() {
        let limit = parse(r#"{"Response":"False","Error":"Request limit reached!"}"#);
        assert_eq!(limit, Err(Error::Stop("Request limit reached!".to_string())));
        assert!(matches!(
            parse(r#"{"Response":"False","Error":"Invalid API key!"}"#),
            Err(Error::Stop(_))
        ));
        assert!(matches!(parse("<html>"), Err(Error::Failed(_))));
    }

    #[test]
    fn the_key_never_shows_in_a_message() {
        let client = Client::new("s3cr3tk3y".to_string());
        assert_eq!(
            client.hidden("https://www.omdbapi.com/?i=tt1&apikey=s3cr3tk3y".to_string()),
            "https://www.omdbapi.com/?i=tt1&apikey=…"
        );
    }
}
