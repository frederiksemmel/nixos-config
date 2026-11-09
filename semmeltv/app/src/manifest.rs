use crate::tags::Tags;
use serde::Deserialize;
use serde::de::{self, Deserializer};
use std::fmt::{self, Display, Formatter, Write as _};
use std::fs;
use std::io;
use std::path::Path;

pub const FILE_NAME: &str = "library.toml";

const SECONDS_PER_DAY: u64 = 24 * 60 * 60;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    year: u16,
    month: u8,
    day: u8,
}

impl Date {
    pub fn from_unix(seconds: u64) -> Self {
        let shifted = (seconds / SECONDS_PER_DAY) as i64 + 719_468;
        let era = shifted.div_euclid(146_097);
        let day_of_era = shifted.rem_euclid(146_097);
        let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let shifted_month = (5 * day_of_year + 2) / 153;
        let month = if shifted_month < 10 {
            shifted_month + 3
        } else {
            shifted_month - 9
        };
        let year = year_of_era + era * 400 + i64::from(month <= 2);
        Self {
            year: year as u16,
            month: month as u8,
            day: (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u8,
        }
    }

    pub fn days(&self) -> i64 {
        let month = i64::from(self.month);
        let year = i64::from(self.year) - i64::from(month <= 2);
        let era = year.div_euclid(400);
        let year_of_era = year.rem_euclid(400);
        let shifted_month = if month > 2 { month - 3 } else { month + 9 };
        let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(self.day) - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }

    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('-');
        let year = parts.next()?.parse().ok()?;
        let month = parts.next()?.parse().ok()?;
        let day = parts.next()?.parse().ok()?;
        parts.next().is_none().then_some(Self { year, month, day })
    }
}

impl Display for Date {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

impl<'de> Deserialize<'de> for Date {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match toml::value::Datetime::deserialize(deserializer)? {
            toml::value::Datetime {
                date: Some(date),
                time: None,
                offset: None,
            } => Ok(Self {
                year: date.year,
                month: date.month,
                day: date.day,
            }),
            other => Err(de::Error::custom(format!("{other} is not a plain date"))),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tenths(pub u8);

impl Tenths {
    pub fn parse(text: &str) -> Option<Self> {
        Self::from_float(text.trim().parse().ok()?)
    }

    pub fn from_float(value: f64) -> Option<Self> {
        (0.0..=10.0)
            .contains(&value)
            .then(|| Self((value * 10.0).round() as u8))
    }
}

impl Display for Tenths {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.0 / 10, self.0 % 10)
    }
}

impl<'de> Deserialize<'de> for Tenths {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = match toml::Value::deserialize(deserializer)? {
            toml::Value::Float(value) => value,
            toml::Value::Integer(value) => value as f64,
            other => return Err(de::Error::custom(format!("{other} is not a rating out of 10"))),
        };
        Self::from_float(value).ok_or_else(|| de::Error::custom(format!("{value} is not between 0 and 10")))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scores {
    pub imdb: Option<Tenths>,
    pub rt: Option<u8>,
    pub popcorn: Option<u8>,
    pub metacritic: Option<u8>,
}

impl Scores {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn or(self, older: Self) -> Self {
        Self {
            imdb: self.imdb.or(older.imdb),
            rt: self.rt.or(older.rt),
            popcorn: self.popcorn.or(older.popcorn),
            metacritic: self.metacritic.or(older.metacritic),
        }
    }

    fn render(&self) -> String {
        let parts: Vec<String> = [
            self.imdb.map(|score| format!("imdb = {score}")),
            self.rt.map(|score| format!("rt = {score}")),
            self.popcorn.map(|score| format!("popcorn = {score}")),
            self.metacritic.map(|score| format!("metacritic = {score}")),
        ]
        .into_iter()
        .flatten()
        .collect();
        format!("{{ {} }}", parts.join(", "))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(default)]
    pub show: Vec<ShowEntry>,
    #[serde(default)]
    pub movie: Vec<MovieEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowEntry {
    pub title: String,
    pub year: Option<u16>,
    pub tmdb: Option<u64>,
    pub imdb: Option<String>,
    #[serde(default)]
    pub ratings: Scores,
    pub rated: Option<Date>,
    #[serde(default)]
    pub review: bool,
    #[serde(default)]
    pub episodes: Vec<EpisodeEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeEntry {
    pub season: u32,
    pub episode: u32,
    pub title: Option<String>,
    pub path: String,
    pub added: Date,
    pub runtime: Option<u32>,
    pub video: Option<String>,
    pub resolution: Option<String>,
    pub audio: Option<String>,
    #[serde(default)]
    pub review: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MovieEntry {
    pub title: String,
    pub year: Option<u16>,
    pub tmdb: Option<u64>,
    pub imdb: Option<String>,
    #[serde(default)]
    pub ratings: Scores,
    pub rated: Option<Date>,
    pub path: String,
    pub added: Date,
    pub runtime: Option<u32>,
    pub video: Option<String>,
    pub resolution: Option<String>,
    pub audio: Option<String>,
    #[serde(default)]
    pub review: bool,
}

impl EpisodeEntry {
    pub fn tags(&self) -> Tags {
        Tags {
            video: self.video.clone(),
            resolution: self.resolution.clone(),
            audio: self.audio.clone(),
        }
    }
}

impl MovieEntry {
    pub fn tags(&self) -> Tags {
        Tags {
            video: self.video.clone(),
            resolution: self.resolution.clone(),
            audio: self.audio.clone(),
        }
    }
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|error| error.to_string())
    }

    pub fn read(path: &Path) -> Result<Self, ReadError> {
        let text = fs::read_to_string(path).map_err(ReadError::Unreadable)?;
        Self::parse(&text).map_err(ReadError::Invalid)
    }

    pub fn write(&self, path: &Path) -> io::Result<()> {
        write_atomically(path, self.render().as_bytes())
    }

    pub fn render(&self) -> String {
        let mut text = String::new();
        for show in &self.show {
            let _ = writeln!(text, "[[show]]");
            render_title(&mut text, &show.title, show.year, show.tmdb);
            render_ratings(&mut text, show.imdb.as_deref(), &show.ratings, show.rated);
            render_review(&mut text, show.review);
            let _ = writeln!(text, "episodes = [");
            for episode in &show.episodes {
                let title = episode
                    .title
                    .as_ref()
                    .map_or(String::new(), |title| format!("title = {}, ", quoted(title)));
                let tags: String = tag_lines(&episode.tags())
                    .map(|(key, value)| format!(", {key} = {}", quoted(value)))
                    .collect();
                let runtime = episode
                    .runtime
                    .map_or(String::new(), |minutes| format!(", runtime = {minutes}"));
                let review = if episode.review { ", review = true" } else { "" };
                let _ = writeln!(
                    text,
                    "  {{ season = {}, episode = {}, {title}path = {}, added = {}{runtime}{tags}{review} }},",
                    episode.season,
                    episode.episode,
                    quoted(&episode.path),
                    episode.added,
                );
            }
            let _ = writeln!(text, "]\n");
        }
        for movie in &self.movie {
            let _ = writeln!(text, "[[movie]]");
            render_title(&mut text, &movie.title, movie.year, movie.tmdb);
            render_ratings(&mut text, movie.imdb.as_deref(), &movie.ratings, movie.rated);
            let _ = writeln!(text, "path = {}", quoted(&movie.path));
            let _ = writeln!(text, "added = {}", movie.added);
            if let Some(minutes) = movie.runtime {
                let _ = writeln!(text, "runtime = {minutes}");
            }
            for (key, value) in tag_lines(&movie.tags()) {
                let _ = writeln!(text, "{key} = {}", quoted(value));
            }
            render_review(&mut text, movie.review);
            let _ = writeln!(text);
        }
        text.pop();
        text
    }

    pub fn review_count(&self) -> usize {
        let shows = self.show.iter().filter(|show| show.review).count();
        let episodes = self.show.iter().flat_map(|show| &show.episodes);
        let movies = self.movie.iter().filter(|movie| movie.review).count();
        shows + episodes.filter(|episode| episode.review).count() + movies
    }
}

#[derive(Debug)]
pub enum ReadError {
    Unreadable(io::Error),
    Invalid(String),
}

pub fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("toml.tmp");
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)
}

fn quoted(text: &str) -> String {
    let mut quoted = String::from("\"");
    for character in text.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            control if control.is_control() => {
                let _ = write!(quoted, "\\u{:04X}", control as u32);
            }
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

fn tag_lines(tags: &Tags) -> impl Iterator<Item = (&'static str, &String)> {
    [
        ("video", &tags.video),
        ("resolution", &tags.resolution),
        ("audio", &tags.audio),
    ]
    .into_iter()
    .filter_map(|(key, value)| Some((key, value.as_ref()?)))
}

fn render_title(text: &mut String, title: &str, year: Option<u16>, tmdb: Option<u64>) {
    let _ = writeln!(text, "title = {}", quoted(title));
    if let Some(year) = year {
        let _ = writeln!(text, "year = {year}");
    }
    if let Some(tmdb) = tmdb {
        let _ = writeln!(text, "tmdb = {tmdb}");
    }
}

fn render_ratings(text: &mut String, imdb: Option<&str>, ratings: &Scores, rated: Option<Date>) {
    if let Some(imdb) = imdb {
        let _ = writeln!(text, "imdb = {}", quoted(imdb));
    }
    if !ratings.is_empty() {
        let _ = writeln!(text, "ratings = {}", ratings.render());
    }
    if let Some(rated) = rated {
        let _ = writeln!(text, "rated = {rated}");
    }
}

fn render_review(text: &mut String, review: bool) {
    if review {
        let _ = writeln!(text, "review = true");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[[show]]
title = "Marvel's \"Dark\""
year = 2017
tmdb = 70523
imdb = "tt5753856"
ratings = { imdb = 8.7, rt = 94, popcorn = 92, metacritic = 74 }
rated = 2026-11-01
review = true
episodes = [
  { season = 1, episode = 1, title = "Secrets", path = "Series/Dark/S01/Dark.S01E01.mkv", added = 2024-05-01, runtime = 51, video = "HEVC 10-bit", resolution = "1080p", audio = "5.1" },
  { season = 1, episode = 2, path = "Series/Dark/S01/Dark.S01E02.mkv", added = 2024-05-02, review = true },
]

[[movie]]
title = "Airplane!"
year = 1980
tmdb = 813
imdb = "tt0080339"
ratings = { imdb = 7.7, rt = 97, popcorn = 89, metacritic = 78 }
rated = 2026-11-01
path = "Movies/Airplane 1980/Airplane.1980.mkv"
added = 2023-01-09
runtime = 88
video = "H.264"
resolution = "720p"
audio = "AAC"

[[movie]]
title = "Wind"
path = "Movies/Wind/WIND.avi"
added = 2022-12-31
review = true
"#;

    #[test]
    fn the_file_survives_a_round_trip_character_for_character() {
        let manifest = Manifest::parse(SAMPLE).unwrap();
        assert_eq!(manifest.show[0].episodes[1].added, Date::parse("2024-05-02").unwrap());
        assert_eq!(manifest.show[0].episodes[0].runtime, Some(51));
        assert_eq!(manifest.movie[0].runtime, Some(88));
        assert_eq!(manifest.review_count(), 3);
        assert_eq!(manifest.movie[0].imdb.as_deref(), Some("tt0080339"));
        assert_eq!(manifest.movie[0].ratings.imdb, Some(Tenths(77)));
        assert_eq!(manifest.show[0].ratings.rt, Some(94));
        assert_eq!(manifest.movie[0].ratings.popcorn, Some(89));
        assert_eq!(manifest.movie[1].ratings, Scores::default());
        assert_eq!(manifest.render(), SAMPLE);
    }

    #[test]
    fn an_empty_file_is_an_empty_library() {
        assert_eq!(Manifest::parse("").unwrap(), Manifest::default());
    }

    #[test]
    fn typos_are_refused() {
        let text = "[[movie]]\ntitle = \"Up\"\npath = \"Movies/Up.mkv\"\nadded = 2024-01-01\nreveiw = true\n";
        assert!(Manifest::parse(text).unwrap_err().contains("reveiw"));
    }

    #[test]
    fn added_must_be_a_date() {
        let text = "[[movie]]\ntitle = \"Up\"\npath = \"Up.mkv\"\nadded = 2024-01-01T10:00:00\n";
        assert!(Manifest::parse(text).is_err());
    }

    #[test]
    fn ratings_are_out_of_ten_and_checked() {
        let movie = |ratings: &str| {
            Manifest::parse(&format!(
                "[[movie]]\ntitle = \"Up\"\npath = \"Up.mkv\"\nadded = 2024-01-01\nratings = {ratings}\n"
            ))
        };
        assert_eq!(movie("{ imdb = 8 }").unwrap().movie[0].ratings.imdb, Some(Tenths(80)));
        assert_eq!(Tenths(80).to_string(), "8.0");
        assert!(movie("{ imdb = 11.2 }").is_err());
        assert!(movie("{ tomatoes = 80 }").is_err());
        assert_eq!(Tenths::parse("8.06"), Some(Tenths(81)));
        assert_eq!(Tenths::parse("N/A"), None);
    }

    #[test]
    fn days_count_from_the_unix_epoch() {
        assert_eq!(Date::parse("1970-01-01").unwrap().days(), 0);
        assert_eq!(Date::parse("2000-03-01").unwrap().days(), 11_017);
        assert_eq!(
            Date::from_unix(1_794_009_600).days() * SECONDS_PER_DAY as i64,
            1_794_009_600
        );
    }

    #[test]
    fn dates_come_from_unix_time_and_sort() {
        assert_eq!(Date::from_unix(0).to_string(), "1970-01-01");
        assert_eq!(Date::from_unix(951_782_400).to_string(), "2000-02-29");
        assert_eq!(Date::from_unix(1_794_009_600).to_string(), "2026-11-07");
        assert!(Date::parse("2024-12-31") < Date::parse("2025-01-01"));
        assert_eq!(Date::parse("2024-12"), None);
    }
}
