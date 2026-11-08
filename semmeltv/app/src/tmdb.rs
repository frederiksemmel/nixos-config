use crate::library::normalize;
use serde::Deserialize;
use std::io::Read;
use std::thread;
use std::time::Duration;

const API: &str = "https://api.themoviedb.org/3";
const IMAGES: &str = "https://image.tmdb.org/t/p";
const THROTTLE_RETRIES: usize = 3;
const THROTTLE_WAIT: Duration = Duration::from_secs(2);
const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;
const CANDIDATES: usize = 4;
const CLEAR_WINNER_MIN_VOTES: f64 = 500.0;
const CLEAR_WINNER_RATIO: f64 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Movie,
    Tv,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub id: u64,
    pub name: String,
    pub date: String,
    pub backdrop: Option<String>,
    pub runtime: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    Match(Found),
    Likely(Found),
    Unsure(Vec<Found>),
    Nothing,
}

#[derive(Debug, Default, Deserialize)]
struct Record {
    id: u64,
    #[serde(default, alias = "name")]
    title: Option<String>,
    #[serde(default, alias = "original_name")]
    original_title: Option<String>,
    #[serde(default, alias = "first_air_date")]
    release_date: Option<String>,
    backdrop_path: Option<String>,
    runtime: Option<u32>,
    #[serde(default)]
    vote_count: f64,
}

#[derive(Debug, Deserialize)]
struct Results {
    results: Vec<Record>,
}

#[derive(Debug, Deserialize)]
pub struct Season {
    episodes: Vec<SeasonEpisode>,
}

#[derive(Debug, Deserialize)]
pub struct SeasonEpisode {
    pub episode_number: u32,
    pub name: Option<String>,
    pub still_path: Option<String>,
    pub runtime: Option<u32>,
}

impl Season {
    pub fn episode(&self, number: u32) -> Option<&SeasonEpisode> {
        self.episodes.iter().find(|episode| episode.episode_number == number)
    }
}

impl From<&Record> for Found {
    fn from(record: &Record) -> Self {
        Self {
            id: record.id,
            name: record.title.clone().unwrap_or_default(),
            date: record.release_date.clone().unwrap_or_default(),
            backdrop: record.backdrop_path.clone(),
            runtime: record.runtime.filter(|minutes| *minutes > 0),
        }
    }
}

fn record_year(record: &Record) -> Option<u16> {
    record.release_date.as_deref()?.get(..4)?.parse().ok()
}

fn clear_winner<'a>(records: &[&'a Record]) -> Option<&'a Record> {
    let mut by_votes = records.to_vec();
    by_votes.sort_by(|a, b| b.vote_count.total_cmp(&a.vote_count));
    let [first, second, ..] = by_votes.as_slice() else {
        return None;
    };
    (first.vote_count >= CLEAR_WINNER_MIN_VOTES && first.vote_count >= CLEAR_WINNER_RATIO * second.vote_count)
        .then_some(*first)
}

fn classify(wanted: &str, year: Option<u16>, records: &[Record]) -> Lookup {
    if records.is_empty() {
        return Lookup::Nothing;
    }
    let wanted = normalize(wanted);
    let same_title: Vec<&Record> = records
        .iter()
        .filter(|record| {
            [&record.title, &record.original_title]
                .into_iter()
                .any(|name| name.as_deref().is_some_and(|name| normalize(name) == wanted))
        })
        .collect();
    let chosen: Vec<&Record> = match year {
        Some(year) => same_title
            .iter()
            .copied()
            .filter(|record| record_year(record).is_some_and(|found| found.abs_diff(year) <= 1))
            .collect(),
        None => same_title.clone(),
    };
    match chosen.as_slice() {
        [only] => Lookup::Match(Found::from(*only)),
        [first, ..] if year.is_some() => Lookup::Match(Found::from(*first)),
        many => {
            if let Some(winner) = clear_winner(many) {
                return Lookup::Likely(Found::from(winner));
            }
            let pool: Vec<&Record> = if chosen.is_empty() {
                records.iter().collect()
            } else {
                chosen
            };
            Lookup::Unsure(
                pool.iter()
                    .take(CANDIDATES)
                    .map(|record| Found::from(*record))
                    .collect(),
            )
        }
    }
}

pub struct Client {
    token: String,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(token: String) -> Self {
        Self {
            token,
            agent: ureq::Agent::new_with_defaults(),
        }
    }

    fn get<T: serde::de::DeserializeOwned>(&self, url: &str, query: &[(&str, &str)]) -> Result<T, ureq::Error> {
        for _ in 0..THROTTLE_RETRIES {
            let mut request = self
                .agent
                .get(url)
                .header("Authorization", &format!("Bearer {}", self.token));
            for (name, value) in query {
                request = request.query(*name, *value);
            }
            match request.call() {
                Err(ureq::Error::StatusCode(429)) => thread::sleep(THROTTLE_WAIT),
                Err(error) => return Err(error),
                Ok(mut response) => return response.body_mut().read_json(),
            }
        }
        Err(ureq::Error::StatusCode(429))
    }

    fn search(&self, path: &str, title: &str, year: Option<(&str, u16)>) -> Result<Vec<Record>, ureq::Error> {
        let url = format!("{API}/search/{path}");
        let year_text = year.map(|(_, value)| value.to_string());
        let mut query = vec![("query", title), ("include_adult", "false")];
        if let (Some((name, _)), Some(value)) = (year, year_text.as_deref()) {
            query.push((name, value));
        }
        Ok(self.get::<Results>(&url, &query)?.results)
    }

    pub fn search_movie(&self, title: &str, year: Option<u16>) -> Result<Lookup, ureq::Error> {
        let records = self.search("movie", title, year.map(|year| ("primary_release_year", year)))?;
        self.settle("movie", title, year, records)
    }

    pub fn search_tv(&self, title: &str, year: Option<u16>) -> Result<Lookup, ureq::Error> {
        let records = self.search("tv", title, year.map(|year| ("first_air_date_year", year)))?;
        self.settle("tv", title, year, records)
    }

    fn settle(&self, path: &str, title: &str, year: Option<u16>, records: Vec<Record>) -> Result<Lookup, ureq::Error> {
        match classify(title, year, &records) {
            Lookup::Match(found) => Ok(Lookup::Match(found)),
            first if year.is_some() => {
                let wider = self.search(path, title, None)?;
                Ok(match classify(title, year, &wider) {
                    Lookup::Nothing => first,
                    second => second,
                })
            }
            first => Ok(first),
        }
    }

    pub fn details(&self, kind: Kind, id: u64) -> Result<Found, ureq::Error> {
        let path = match kind {
            Kind::Movie => "movie",
            Kind::Tv => "tv",
        };
        let record: Record = self.get(&format!("{API}/{path}/{id}"), &[])?;
        Ok(Found::from(&record))
    }

    pub fn season(&self, id: u64, season: u32) -> Result<Season, ureq::Error> {
        self.get(&format!("{API}/tv/{id}/season/{season}"), &[])
    }

    pub fn image(&self, size: &str, path: &str) -> Result<Vec<u8>, ureq::Error> {
        let url = format!("{IMAGES}/{size}/{}", path.trim_start_matches('/'));
        let response = self.agent.get(&url).call()?;
        let mut bytes = Vec::new();
        response
            .into_body()
            .into_reader()
            .take(MAX_IMAGE_BYTES)
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: u64, title: &str, date: &str) -> Record {
        Record {
            id,
            title: Some(title.to_string()),
            original_title: Some(title.to_string()),
            release_date: Some(date.to_string()),
            backdrop_path: None,
            runtime: None,
            vote_count: 0.0,
        }
    }

    fn voted(id: u64, title: &str, date: &str, vote_count: f64) -> Record {
        Record {
            vote_count,
            ..record(id, title, date)
        }
    }

    fn id(lookup: Lookup) -> Option<u64> {
        match lookup {
            Lookup::Match(found) => Some(found.id),
            _ => None,
        }
    }

    #[test]
    fn a_title_and_year_match_is_accepted() {
        let records = [record(1, "Dune", "1984-12-14"), record(2, "Dune", "2021-09-15")];
        assert_eq!(id(classify("Dune", Some(2021), &records)), Some(2));
        assert_eq!(id(classify("Dune", Some(2022), &records)), Some(2));
    }

    #[test]
    fn punctuation_does_not_block_a_match() {
        let records = [record(7, "Marvel's Daredevil", "2015-04-10")];
        assert_eq!(id(classify("Marvels Daredevil", None, &records)), Some(7));
    }

    #[test]
    fn several_same_titles_without_a_year_are_unsure() {
        let records = [record(1, "Dark", "2017-12-01"), record(2, "Dark", "2005-01-01")];
        assert!(matches!(classify("Dark", None, &records), Lookup::Unsure(found) if found.len() == 2));
    }

    #[test]
    fn a_far_better_known_title_wins_without_a_year() {
        let records = [
            voted(196108, "Chernobyl", "2022-03-30", 6.0),
            voted(87108, "Chernobyl", "2019-05-06", 8510.0),
        ];
        assert!(matches!(classify("Chernobyl", None, &records), Lookup::Likely(found) if found.id == 87108));
    }

    #[test]
    fn close_or_little_known_titles_stay_unsure() {
        let close = [
            voted(1082195, "The Order", "2024-12-05", 988.0),
            voted(9616, "The Order", "2003-09-05", 393.0),
        ];
        let obscure = [
            voted(1, "Wind", "1998-09-18", 100.0),
            voted(2, "Wind", "2003-01-01", 1.0),
        ];
        assert!(matches!(classify("The Order", None, &close), Lookup::Unsure(_)));
        assert!(matches!(classify("Wind", None, &obscure), Lookup::Unsure(_)));
    }

    #[test]
    fn a_different_title_is_unsure() {
        let records = [record(3, "Casino Royale", "2006-11-14")];
        assert!(matches!(
            classify("James Bond Casino Royale", Some(2006), &records),
            Lookup::Unsure(_)
        ));
    }

    #[test]
    fn a_wrong_year_is_unsure() {
        let records = [record(3, "Wind", "1992-09-11")];
        assert!(matches!(classify("Wind", Some(2005), &records), Lookup::Unsure(_)));
    }

    #[test]
    fn movie_and_tv_responses_share_a_record() {
        let movie = r#"{"results":[{"id":1,"title":"Up","original_title":"Up","release_date":"2009-05-28",
            "backdrop_path":"/up.jpg","poster_path":null,"popularity":1.5}]}"#;
        let tv = r#"{"results":[{"id":2,"name":"Dark","original_name":"Dark","first_air_date":null,
            "backdrop_path":null}]}"#;
        let movie: Results = serde_json::from_str(movie).unwrap();
        let tv: Results = serde_json::from_str(tv).unwrap();
        assert_eq!(id(classify("Up", Some(2009), &movie.results)), Some(1));
        assert_eq!(id(classify("Dark", None, &tv.results)), Some(2));
    }

    #[test]
    fn seasons_expose_names_and_stills() {
        let json = r#"{"episodes":[{"episode_number":1,"name":"Secrets","still_path":"/a.jpg"},
            {"episode_number":2,"name":null,"still_path":null}]}"#;
        let season: Season = serde_json::from_str(json).unwrap();
        assert_eq!(season.episode(1).unwrap().still_path.as_deref(), Some("/a.jpg"));
        assert!(season.episode(2).unwrap().name.is_none());
        assert!(season.episode(3).is_none());
    }

    #[test]
    fn details_and_seasons_carry_their_runtimes() {
        let found = |json| Found::from(&serde_json::from_str::<Record>(json).unwrap()).runtime;
        assert_eq!(
            found(r#"{"id":1,"title":"Up","backdrop_path":null,"runtime":96}"#),
            Some(96)
        );
        assert_eq!(
            found(r#"{"id":1,"title":"Soon","backdrop_path":null,"runtime":0}"#),
            None
        );
        assert_eq!(
            found(r#"{"id":1,"title":"Soon","backdrop_path":null,"runtime":null}"#),
            None
        );
        assert_eq!(
            found(r#"{"id":2,"name":"Dark","backdrop_path":null,"episode_run_time":[60]}"#),
            None
        );
        let json = r#"{"episodes":[{"episode_number":1,"name":null,"still_path":null,"runtime":52},
            {"episode_number":2,"name":null,"still_path":null,"runtime":null}]}"#;
        let season: Season = serde_json::from_str(json).unwrap();
        assert_eq!(season.episode(1).unwrap().runtime, Some(52));
        assert_eq!(season.episode(2).unwrap().runtime, None);
    }

    #[test]
    fn no_results_is_nothing() {
        assert_eq!(classify("Anything", None, &[]), Lookup::Nothing);
    }
}
