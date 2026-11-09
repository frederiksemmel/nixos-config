use crate::library::normalize;
use crate::manifest::{About, Certificate, Date, Member};
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
const CAST: usize = 10;
const CERTIFICATE_COUNTRIES: [&str; 3] = ["CH", "DE", "US"];
const MOVIE_EXTRAS: &str = "credits,release_dates,videos,external_ids";
const TV_EXTRAS: &str = "aggregate_credits,content_ratings,videos,external_ids";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Movie,
    Tv,
}

impl Kind {
    fn path(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Tv => "tv",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub id: u64,
    pub name: String,
    pub original: Option<String>,
    pub date: String,
    pub backdrop: Option<String>,
    pub poster: Option<String>,
    pub runtime: Option<u32>,
    pub last_aired: Option<u16>,
    pub running: bool,
    pub imdb: Option<String>,
    pub about: About,
    pub faces: Vec<Face>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Face {
    pub person: u64,
    pub path: String,
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
    poster_path: Option<String>,
    runtime: Option<u32>,
    last_air_date: Option<String>,
    status: Option<String>,
    #[serde(default)]
    vote_count: f64,
    tagline: Option<String>,
    overview: Option<String>,
    #[serde(default)]
    genres: Vec<Named>,
    original_language: Option<String>,
    number_of_seasons: Option<u32>,
    number_of_episodes: Option<u32>,
    #[serde(default)]
    networks: Vec<Named>,
    #[serde(default)]
    created_by: Vec<Named>,
    imdb_id: Option<String>,
    external_ids: Option<ExternalIds>,
    credits: Option<Credits>,
    aggregate_credits: Option<Credits>,
    release_dates: Option<Listing<Releases>>,
    content_ratings: Option<Listing<ContentRating>>,
    videos: Option<Listing<Video>>,
}

#[derive(Debug, Default, Deserialize)]
struct Named {
    name: String,
}

#[derive(Debug, Default, Deserialize)]
struct Credits {
    #[serde(default)]
    cast: Vec<CastRecord>,
    #[serde(default)]
    crew: Vec<CrewRecord>,
}

#[derive(Debug, Default, Deserialize)]
struct CastRecord {
    id: u64,
    name: String,
    character: Option<String>,
    profile_path: Option<String>,
    #[serde(default)]
    order: u32,
    #[serde(default)]
    total_episode_count: u32,
    #[serde(default)]
    roles: Vec<Role>,
}

#[derive(Debug, Default, Deserialize)]
struct Role {
    character: Option<String>,
    #[serde(default)]
    episode_count: u32,
}

#[derive(Debug, Default, Deserialize)]
struct CrewRecord {
    name: String,
    job: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Listing<T> {
    #[serde(default = "Vec::new")]
    results: Vec<T>,
}

#[derive(Debug, Deserialize)]
struct Releases {
    iso_3166_1: String,
    #[serde(default)]
    release_dates: Vec<Release>,
}

#[derive(Debug, Deserialize)]
struct Release {
    #[serde(default)]
    certification: String,
    #[serde(rename = "type", default)]
    kind: u8,
}

#[derive(Debug, Deserialize)]
struct ContentRating {
    iso_3166_1: String,
    #[serde(default)]
    rating: String,
}

#[derive(Debug, Deserialize)]
struct Video {
    site: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    key: Option<String>,
    #[serde(default)]
    official: bool,
    #[serde(default)]
    published_at: String,
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
    #[serde(default)]
    pub air_date: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ExternalIds {
    imdb_id: Option<String>,
}

impl Season {
    pub fn episodes(&self) -> &[SeasonEpisode] {
        &self.episodes
    }

    pub fn episode(&self, number: u32) -> Option<&SeasonEpisode> {
        self.episodes.iter().find(|episode| episode.episode_number == number)
    }
}

impl From<&Record> for Found {
    fn from(record: &Record) -> Self {
        Self {
            id: record.id,
            name: record.title.clone().unwrap_or_default(),
            original: record.original_title.clone(),
            date: record.release_date.clone().unwrap_or_default(),
            backdrop: record.backdrop_path.clone(),
            poster: record.poster_path.clone(),
            runtime: record.runtime.filter(|minutes| *minutes > 0),
            last_aired: record.last_air_date.as_deref().and_then(date_year),
            running: record.status.as_deref().is_some_and(still_running),
            imdb: record
                .imdb_id
                .clone()
                .or_else(|| record.external_ids.as_ref()?.imdb_id.clone())
                .filter(|imdb| imdb.starts_with("tt")),
            about: about(record),
            faces: cast(record)
                .iter()
                .filter_map(|member| {
                    Some(Face {
                        person: member.id,
                        path: member.profile_path.clone()?,
                    })
                })
                .collect(),
        }
    }
}

fn text(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn names(named: &[Named]) -> Vec<String> {
    named
        .iter()
        .map(|named| named.name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

fn cast(record: &Record) -> Vec<&CastRecord> {
    let credits = record.aggregate_credits.as_ref().or(record.credits.as_ref());
    let mut cast: Vec<&CastRecord> = credits.map(|credits| credits.cast.iter().collect()).unwrap_or_default();
    cast.sort_by_key(|member| (std::cmp::Reverse(member.total_episode_count), member.order));
    cast.truncate(CAST);
    cast
}

fn character(member: &CastRecord) -> Option<String> {
    let role = member
        .roles
        .iter()
        .filter(|role| text(&role.character).is_some())
        .max_by_key(|role| role.episode_count);
    text(role.map_or(&member.character, |role| &role.character))
}

fn certificate(record: &Record) -> Option<Certificate> {
    let found = |country: &str| -> Option<String> {
        if let Some(releases) = &record.release_dates {
            let releases = releases
                .results
                .iter()
                .find(|releases| releases.iso_3166_1 == country)?;
            let mut rated: Vec<&Release> = releases
                .release_dates
                .iter()
                .filter(|release| !release.certification.trim().is_empty())
                .collect();
            rated.sort_by_key(|release| release.kind != 3);
            return rated.first().map(|release| release.certification.trim().to_string());
        }
        let ratings = record.content_ratings.as_ref()?;
        let rating = ratings.results.iter().find(|rating| rating.iso_3166_1 == country)?;
        Some(rating.rating.trim().to_string()).filter(|rating| !rating.is_empty())
    };
    CERTIFICATE_COUNTRIES.iter().find_map(|country| {
        found(country).map(|rating| Certificate {
            country: country.to_string(),
            rating,
        })
    })
}

fn trailer(record: &Record) -> Option<String> {
    let videos = record.videos.as_ref()?;
    let mut found: Vec<&Video> = videos
        .results
        .iter()
        .filter(|video| video.site.as_deref() == Some("YouTube") && video.key.is_some())
        .filter(|video| matches!(video.kind.as_deref(), Some("Trailer" | "Teaser")))
        .collect();
    found.sort_by(|a, b| {
        let rank = |video: &Video| (video.kind.as_deref() != Some("Trailer"), !video.official);
        rank(a).cmp(&rank(b)).then_with(|| a.published_at.cmp(&b.published_at))
    });
    found.first().and_then(|video| video.key.clone())
}

fn date(value: &Option<String>) -> Option<Date> {
    Date::parse(value.as_deref()?)
}

fn about(record: &Record) -> About {
    let tv =
        record.number_of_seasons.is_some() || record.aggregate_credits.is_some() || record.content_ratings.is_some();
    let title = record.title.as_deref().map(normalize);
    let original_title = text(&record.original_title).filter(|original| Some(normalize(original)) != title);
    let directors = record
        .credits
        .iter()
        .flat_map(|credits| &credits.crew)
        .filter(|crew| crew.job.as_deref() == Some("Director"))
        .map(|crew| crew.name.clone())
        .fold(Vec::new(), |mut names: Vec<String>, name| {
            if !names.contains(&name) {
                names.push(name);
            }
            names
        });
    About {
        tagline: text(&record.tagline),
        plot: text(&record.overview),
        genres: names(&record.genres),
        original_title,
        language: text(&record.original_language),
        released: if tv { None } else { date(&record.release_date) },
        first_aired: if tv { date(&record.release_date) } else { None },
        last_aired: if tv { date(&record.last_air_date) } else { None },
        seasons: record.number_of_seasons.filter(|count| *count > 0),
        episodes: record.number_of_episodes.filter(|count| *count > 0),
        networks: names(&record.networks),
        directors,
        creators: names(&record.created_by),
        certificate: certificate(record),
        trailer: trailer(record),
        cast: cast(record)
            .into_iter()
            .map(|member| Member {
                name: member.name.trim().to_string(),
                character: character(member),
                person: Some(member.id),
            })
            .collect(),
        described: None,
    }
}

fn date_year(date: &str) -> Option<u16> {
    date.get(..4)?.parse().ok()
}

fn still_running(status: &str) -> bool {
    !matches!(status, "Ended" | "Canceled")
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
        let extras = match kind {
            Kind::Movie => MOVIE_EXTRAS,
            Kind::Tv => TV_EXTRAS,
        };
        let record: Record = self.get(
            &format!("{API}/{}/{id}", kind.path()),
            &[("append_to_response", extras)],
        )?;
        Ok(Found::from(&record))
    }

    pub fn season(&self, id: u64, season: u32) -> Result<Season, ureq::Error> {
        self.get(&format!("{API}/tv/{id}/season/{season}"), &[])
    }

    pub fn imdb(&self, kind: Kind, id: u64) -> Result<Option<String>, ureq::Error> {
        let ids: ExternalIds = self.get(&format!("{API}/{}/{id}/external_ids", kind.path()), &[])?;
        Ok(ids.imdb_id.filter(|imdb| !imdb.is_empty()))
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
            poster_path: None,
            runtime: None,
            last_air_date: None,
            status: None,
            vote_count: 0.0,
            ..Record::default()
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
    fn a_show_knows_when_it_last_aired_and_whether_it_still_runs() {
        let found = |json| Found::from(&serde_json::from_str::<Record>(json).unwrap());
        let ended = found(r#"{"id":2,"name":"Dark","last_air_date":"2020-06-27","status":"Ended"}"#);
        assert_eq!((ended.last_aired, ended.running), (Some(2020), false));
        let canceled = found(r#"{"id":3,"name":"Firefly","last_air_date":"2002-12-20","status":"Canceled"}"#);
        assert_eq!((canceled.last_aired, canceled.running), (Some(2002), false));
        let running = found(r#"{"id":4,"name":"Severance","last_air_date":"2025-03-21","status":"Returning Series"}"#);
        assert_eq!((running.last_aired, running.running), (Some(2025), true));
        let film = found(r#"{"id":1,"title":"Up","release_date":"2009-05-28"}"#);
        assert_eq!((film.last_aired, film.running), (None, false));
    }

    #[test]
    fn a_film_is_described_with_its_credits_certificate_and_trailer() {
        let json = r#"{"id":14160,"title":"Up","original_title":"Up","release_date":"2009-05-28","runtime":96,
            "tagline":"Change is in the air.","overview":"Carl, 78, ties balloons to his house.",
            "genres":[{"id":16,"name":"Animation"},{"id":35,"name":"Comedy"}],"original_language":"en",
            "imdb_id":"tt1049413",
            "credits":{"cast":[
                {"id":290,"name":"Christopher Plummer","character":"Charles Muntz (voice)","profile_path":"/cp.jpg","order":1},
                {"id":68812,"name":"Ed Asner","character":"Carl Fredricksen (voice)","profile_path":"/ea.jpg","order":0},
                {"id":3,"name":"Nobody","character":"","profile_path":null,"order":2}],
              "crew":[{"name":"Pete Docter","job":"Director"},{"name":"Pete Docter","job":"Director"},{"name":"Bob Peterson","job":"Writer"}]},
            "release_dates":{"results":[
                {"iso_3166_1":"US","release_dates":[{"certification":"PG","type":3}]},
                {"iso_3166_1":"CH","release_dates":[{"certification":"","type":3},{"certification":"0","type":4}]}]},
            "videos":{"results":[
                {"site":"YouTube","type":"Featurette","key":"f","official":true,"published_at":"2009-01-01"},
                {"site":"YouTube","type":"Teaser","key":"t","official":true,"published_at":"2008-01-01"},
                {"site":"YouTube","type":"Trailer","key":"second","official":true,"published_at":"2009-03-01"},
                {"site":"YouTube","type":"Trailer","key":"first","official":true,"published_at":"2009-02-01"}]}}"#;
        let found = Found::from(&serde_json::from_str::<Record>(json).unwrap());
        let about = &found.about;
        assert_eq!(found.imdb.as_deref(), Some("tt1049413"));
        assert_eq!(about.tagline.as_deref(), Some("Change is in the air."));
        assert_eq!(about.genres, ["Animation", "Comedy"]);
        assert_eq!(about.original_title, None);
        assert_eq!(about.released, Date::parse("2009-05-28"));
        assert_eq!((about.first_aired, about.seasons), (None, None));
        assert_eq!(about.directors, ["Pete Docter"]);
        assert_eq!(
            about.certificate,
            Some(Certificate {
                country: "CH".to_string(),
                rating: "0".to_string()
            })
        );
        assert_eq!(about.trailer.as_deref(), Some("first"));
        let cast: Vec<_> = about
            .cast
            .iter()
            .map(|member| (member.name.as_str(), member.character.as_deref()))
            .collect();
        assert_eq!(
            cast,
            [
                ("Ed Asner", Some("Carl Fredricksen (voice)")),
                ("Christopher Plummer", Some("Charles Muntz (voice)")),
                ("Nobody", None)
            ]
        );
        let faces: Vec<_> = found.faces.iter().map(|face| face.person).collect();
        assert_eq!(faces, [68812, 290]);
    }

    #[test]
    fn a_show_is_described_with_its_creators_networks_and_longest_roles() {
        let json = r#"{"id":48891,"name":"Brooklyn Nine-Nine","original_name":"Brooklyn Nine-Nine",
            "first_air_date":"2013-09-17","last_air_date":"2021-09-16","status":"Ended","original_language":"en",
            "number_of_seasons":8,"number_of_episodes":152,"networks":[{"name":"NBC"},{"name":"FOX"}],
            "created_by":[{"name":"Daniel J. Goor"},{"name":"Michael Schur"}],
            "external_ids":{"imdb_id":"tt2467372"},
            "aggregate_credits":{"cast":[
                {"id":19292,"name":"Adam Sandler","order":0,"total_episode_count":1,
                    "roles":[{"character":"Adam Sandler","episode_count":1}]},
                {"id":62861,"name":"Andy Samberg","profile_path":"/as.jpg","order":1,"total_episode_count":153,
                    "roles":[{"character":"Cameo","episode_count":1},{"character":"Jake Peralta","episode_count":152}]}]},
            "content_ratings":{"results":[{"iso_3166_1":"US","rating":"TV-14"},{"iso_3166_1":"DE","rating":"12"}]},
            "videos":{"results":[]}}"#;
        let found = Found::from(&serde_json::from_str::<Record>(json).unwrap());
        let about = &found.about;
        assert_eq!(found.imdb.as_deref(), Some("tt2467372"));
        assert_eq!(about.released, None);
        assert_eq!(
            (about.first_aired, about.last_aired),
            (Date::parse("2013-09-17"), Date::parse("2021-09-16"))
        );
        assert_eq!((about.seasons, about.episodes), (Some(8), Some(152)));
        assert_eq!(about.networks, ["NBC", "FOX"]);
        assert_eq!(about.creators, ["Daniel J. Goor", "Michael Schur"]);
        assert_eq!(
            about
                .certificate
                .as_ref()
                .map(|certificate| certificate.country.as_str()),
            Some("DE")
        );
        assert_eq!(about.cast[0].character.as_deref(), Some("Jake Peralta"));
        assert_eq!(about.cast[1].name, "Adam Sandler");
        assert_eq!(about.trailer, None);
    }

    #[test]
    fn an_original_title_is_kept_only_when_it_differs() {
        let json = r#"{"id":1,"title":"Amélie","original_title":"Le Fabuleux Destin d'Amélie Poulain","original_language":"fr"}"#;
        let about = Found::from(&serde_json::from_str::<Record>(json).unwrap()).about;
        assert_eq!(
            about.original_title.as_deref(),
            Some("Le Fabuleux Destin d'Amélie Poulain")
        );
        assert_eq!(about.language.as_deref(), Some("fr"));
    }

    #[test]
    fn no_results_is_nothing() {
        assert_eq!(classify("Anything", None, &[]), Lookup::Nothing);
    }
}
