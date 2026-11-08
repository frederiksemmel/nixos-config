pub use crate::manifest::Scores;
use crate::manifest::{self, Date, EpisodeEntry, Manifest, MovieEntry, ReadError, ShowEntry};
use crate::tags::Tags;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Show(usize),
    Movie(usize),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Library {
    pub shows: Vec<Show>,
    pub movies: Vec<Movie>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Show {
    pub key: String,
    pub title: String,
    pub year: Option<u16>,
    pub tmdb: Option<u64>,
    pub imdb: Option<String>,
    pub ratings: Scores,
    pub seasons: Vec<Season>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Season {
    pub number: u32,
    pub episodes: Vec<Episode>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Episode {
    pub season: u32,
    pub number: u32,
    pub title: Option<String>,
    pub path: PathBuf,
    pub added: Date,
    pub runtime: Option<u32>,
    pub tags: Tags,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Movie {
    pub key: String,
    pub title: String,
    pub year: Option<u16>,
    pub tmdb: Option<u64>,
    pub imdb: Option<String>,
    pub ratings: Scores,
    pub path: PathBuf,
    pub added: Date,
    pub runtime: Option<u32>,
    pub tags: Tags,
}

impl Library {
    pub fn from_manifest(root: &Path, manifest: &Manifest, present: impl Fn(&Path) -> bool) -> Self {
        let mut library = Self {
            shows: manifest
                .show
                .iter()
                .filter_map(|entry| build_show(root, entry, &present))
                .collect(),
            movies: manifest
                .movie
                .iter()
                .filter(|entry| present(&root.join(&entry.path)))
                .map(|entry| build_movie(root, entry))
                .collect(),
        };
        library.sort();
        library
    }

    fn sort(&mut self) {
        self.shows
            .sort_by_key(|show| (Reverse(show.latest_addition()), sort_title(&show.title)));
        self.movies
            .sort_by_key(|movie| (Reverse(movie.added), sort_title(&movie.title), movie.year));
    }

    pub fn is_empty(&self) -> bool {
        self.shows.is_empty() && self.movies.is_empty()
    }

    pub fn uses_tmdb(&self) -> bool {
        let shows = self.shows.iter().map(|show| show.tmdb);
        shows
            .chain(self.movies.iter().map(|movie| movie.tmdb))
            .any(|id| id.is_some())
    }

    pub fn newest(&self, count: usize) -> Vec<Item> {
        let shows = self
            .shows
            .iter()
            .enumerate()
            .map(|(index, show)| (show.latest_addition(), sort_title(&show.title), Item::Show(index)));
        let movies = self
            .movies
            .iter()
            .enumerate()
            .map(|(index, movie)| (movie.added, sort_title(&movie.title), Item::Movie(index)));
        let mut items: Vec<_> = shows.chain(movies).collect();
        items.sort_by(|a, b| (Reverse(a.0), &a.1).cmp(&(Reverse(b.0), &b.1)));
        items.truncate(count);
        items.into_iter().map(|(_, _, item)| item).collect()
    }

    pub fn item_key(&self, item: Item) -> &str {
        match item {
            Item::Show(index) => &self.shows[index].key,
            Item::Movie(index) => &self.movies[index].key,
        }
    }

    pub fn find_item(&self, kind: Item, key: &str) -> Option<Item> {
        match kind {
            Item::Show(_) => self.shows.iter().position(|show| show.key == key).map(Item::Show),
            Item::Movie(_) => self.movies.iter().position(|movie| movie.key == key).map(Item::Movie),
        }
    }

    pub fn find_episode(&self, path: &Path) -> Option<(&Show, &Episode)> {
        self.shows.iter().find_map(|show| {
            show.seasons
                .iter()
                .flat_map(|season| &season.episodes)
                .find(|episode| episode.path == path)
                .map(|episode| (show, episode))
        })
    }

    pub fn paths(&self) -> impl Iterator<Item = &PathBuf> {
        let episodes = self.shows.iter().flat_map(Show::episodes).map(|episode| &episode.path);
        episodes.chain(self.movies.iter().map(|movie| &movie.path))
    }
}

impl Show {
    pub fn episodes(&self) -> impl Iterator<Item = &Episode> {
        self.seasons.iter().flat_map(|season| &season.episodes)
    }

    pub fn episode(&self, season: u32, number: u32) -> Option<&Episode> {
        self.seasons
            .iter()
            .find(|candidate| candidate.number == season)?
            .episodes
            .iter()
            .find(|episode| episode.number == number)
    }

    fn regular_episodes(&self) -> impl Iterator<Item = &Episode> {
        self.seasons
            .iter()
            .filter(|season| season.number > 0)
            .flat_map(|season| &season.episodes)
    }

    pub fn first_episode(&self) -> Option<&Episode> {
        self.regular_episodes()
            .next()
            .or_else(|| self.seasons.first()?.episodes.first())
    }

    pub fn episode_after(&self, season: u32, number: u32) -> Option<&Episode> {
        self.regular_episodes()
            .find(|episode| (episode.season, episode.number) > (season, number))
    }

    pub fn last_season(&self) -> u32 {
        self.seasons.iter().map(|season| season.number).max().unwrap_or(0)
    }

    pub fn episode_count(&self) -> usize {
        self.seasons.iter().map(|season| season.episodes.len()).sum()
    }

    fn latest_addition(&self) -> Date {
        self.episodes().map(|episode| episode.added).max().unwrap_or_default()
    }

    pub fn only_episode(&self) -> Option<&Episode> {
        match self.seasons.as_slice() {
            [Season { episodes, .. }] if episodes.len() == 1 => episodes.first(),
            _ => None,
        }
    }

    pub fn episode_key(&self, episode: &Episode) -> String {
        format!("{}-s{:02}e{:02}", self.key, episode.season, episode.number)
    }

    pub fn subtitle(&self) -> String {
        let seasons = self.seasons.iter().filter(|season| season.number > 0).count();
        match seasons {
            0 | 1 => plural(self.episode_count(), "episode"),
            _ => format!(
                "{} · {}",
                plural(seasons, "season"),
                plural(self.episode_count(), "episode")
            ),
        }
    }
}

impl Season {
    pub fn label(&self) -> String {
        match self.number {
            0 => "Specials".to_string(),
            number => format!("Season {number}"),
        }
    }
}

impl Episode {
    pub fn label(&self) -> String {
        match &self.title {
            Some(title) if self.season > 0 => format!("{}. {title}", self.number),
            Some(title) => title.clone(),
            None => format!("Episode {}", self.number),
        }
    }

    pub fn short_label(&self) -> String {
        match self.season {
            0 => format!("Special {}", self.number),
            season => format!("S{season} E{}", self.number),
        }
    }
}

fn plural(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

fn build_show(root: &Path, entry: &ShowEntry, present: &impl Fn(&Path) -> bool) -> Option<Show> {
    let mut seasons: BTreeMap<u32, Vec<Episode>> = BTreeMap::new();
    for episode in &entry.episodes {
        if present(&root.join(&episode.path)) {
            seasons
                .entry(episode.season)
                .or_default()
                .push(build_episode(root, episode));
        }
    }
    if seasons.is_empty() {
        return None;
    }
    let mut seasons: Vec<Season> = seasons
        .into_iter()
        .map(|(number, mut episodes)| {
            episodes.sort_by_key(|episode| episode.number);
            Season { number, episodes }
        })
        .collect();
    seasons.sort_by_key(|season| if season.number == 0 { u32::MAX } else { season.number });
    Some(Show {
        key: show_key(&entry.title),
        title: entry.title.clone(),
        year: entry.year,
        tmdb: entry.tmdb,
        imdb: entry.imdb.clone(),
        ratings: entry.ratings,
        seasons,
    })
}

fn build_episode(root: &Path, entry: &EpisodeEntry) -> Episode {
    Episode {
        season: entry.season,
        number: entry.episode,
        title: entry.title.clone(),
        path: root.join(&entry.path),
        added: entry.added,
        runtime: entry.runtime,
        tags: entry.tags(),
    }
}

fn build_movie(root: &Path, entry: &MovieEntry) -> Movie {
    Movie {
        key: movie_key(&entry.title, entry.year),
        title: entry.title.clone(),
        year: entry.year,
        tmdb: entry.tmdb,
        imdb: entry.imdb.clone(),
        ratings: entry.ratings,
        path: root.join(&entry.path),
        added: entry.added,
        runtime: entry.runtime,
        tags: entry.tags(),
    }
}

pub fn load(root: &Path) -> Result<Library, String> {
    let manifest = read_manifest(root)?;
    Ok(Library::from_manifest(root, &manifest, Path::is_file))
}

pub fn read_manifest(root: &Path) -> Result<Manifest, String> {
    let path = root.join(manifest::FILE_NAME);
    Manifest::read(&path).map_err(|error| match error {
        ReadError::Unreadable(error) if error.kind() == ErrorKind::NotFound => {
            format!("No {} in {} yet", manifest::FILE_NAME, root.display())
        }
        ReadError::Unreadable(error) => format!("{}: {error}", path.display()),
        ReadError::Invalid(error) => format!("{}: {error}", path.display()),
    })
}

pub fn load_saved(root: &Path, saved: &Path) -> Result<Library, String> {
    let manifest = Manifest::read(saved).map_err(|_| format!("No saved copy at {}", saved.display()))?;
    Ok(Library::from_manifest(root, &manifest, |_| true))
}

pub fn save_copy(root: &Path, saved: &Path) {
    let current = fs::read(root.join(manifest::FILE_NAME));
    if let Ok(bytes) = current
        && fs::read(saved).ok().as_ref() != Some(&bytes)
        && let Err(error) = manifest::write_atomically(saved, &bytes)
    {
        eprintln!("{}: {error}", saved.display());
    }
}

pub fn show_key(title: &str) -> String {
    slug(title)
}

pub fn movie_key(title: &str, year: Option<u16>) -> String {
    match year {
        Some(year) => format!("{}-{year}", slug(title)),
        None => slug(title),
    }
}

pub fn normalize(title: &str) -> String {
    let folded: String = title
        .to_lowercase()
        .chars()
        .filter(|c| *c != '\'')
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn slug(title: &str) -> String {
    normalize(title).replace(' ', "-")
}

pub fn sort_title(title: &str) -> String {
    let normalized = normalize(title);
    normalized
        .strip_prefix("the ")
        .map(str::to_string)
        .unwrap_or(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Tenths;

    fn library_with(text: &str, present: impl Fn(&Path) -> bool) -> Library {
        let manifest = Manifest::parse(text).unwrap();
        Library::from_manifest("/lib".as_ref(), &manifest, present)
    }

    fn library(text: &str) -> Library {
        library_with(text, |_| true)
    }

    fn movies(entries: &[(&str, &str)]) -> String {
        entries
            .iter()
            .map(|(title, added)| {
                format!("[[movie]]\ntitle = \"{title}\"\npath = \"Movies/{title}/f.mkv\"\nadded = {added}\n")
            })
            .collect()
    }

    fn show(title: &str, episodes: &[(u32, u32, &str)]) -> String {
        let lines: Vec<_> = episodes
            .iter()
            .map(|(season, episode, added)| {
                format!(
                    "{{ season = {season}, episode = {episode}, path = \"Series/{title}/{season}-{episode}.mkv\", added = {added} }},"
                )
            })
            .collect();
        format!("[[show]]\ntitle = \"{title}\"\nepisodes = [\n{}\n]\n", lines.join("\n"))
    }

    #[test]
    fn newest_films_come_first() {
        let library = library(&movies(&[
            ("Apple", "2024-01-01"),
            ("Zebra", "2026-01-01"),
            ("Mango", "2025-01-01"),
        ]));
        let order: Vec<_> = library.movies.iter().map(|movie| movie.title.as_str()).collect();
        assert_eq!(order, ["Zebra", "Mango", "Apple"]);
    }

    #[test]
    fn a_new_episode_brings_its_show_forward_but_episodes_stay_in_order() {
        let old = show(
            "Old Show",
            &[(2, 1, "2020-01-01"), (1, 2, "2026-01-01"), (1, 1, "2020-01-01")],
        );
        let library = library(&(old + &show("New Show", &[(1, 1, "2025-01-01")])));
        let order: Vec<_> = library.shows.iter().map(|show| show.title.as_str()).collect();
        assert_eq!(order, ["Old Show", "New Show"]);
        let show = &library.shows[0];
        let seasons: Vec<_> = show.seasons.iter().map(|season| season.number).collect();
        let episodes: Vec<_> = show.seasons[0].episodes.iter().map(|episode| episode.number).collect();
        assert_eq!((seasons, episodes), (vec![1, 2], vec![1, 2]));
    }

    #[test]
    fn titles_added_together_fall_back_to_the_alphabet_ignoring_the() {
        let library = library(&movies(&[
            ("The Zebra", "2024-01-01"),
            ("The Apple", "2024-01-01"),
            ("Mango", "2024-01-01"),
        ]));
        let order: Vec<_> = library.movies.iter().map(|movie| movie.title.as_str()).collect();
        assert_eq!(order, ["The Apple", "Mango", "The Zebra"]);
    }

    #[test]
    fn entries_whose_file_is_missing_are_skipped() {
        let text = show("Dark", &[(1, 1, "2024-01-01"), (1, 2, "2024-01-01")])
            + &show("Gone", &[(1, 1, "2024-01-01")])
            + &movies(&[("Up", "2024-01-01"), ("Lost", "2024-01-01")]);
        let library = library_with(&text, |path| {
            !path.to_string_lossy().contains("Gone")
                && !path.to_string_lossy().contains("Lost")
                && !path.ends_with("1-2.mkv")
        });
        assert_eq!(library.shows.len(), 1);
        assert_eq!(library.shows[0].episode_count(), 1);
        assert_eq!(library.movies.len(), 1);
    }

    #[test]
    fn specials_sort_last_and_labels_read_well() {
        let library = library(
            r#"[[show]]
title = "Dark"
episodes = [
  { season = 0, episode = 1, title = "The Making Of", path = "a.mkv", added = 2024-01-01 },
  { season = 1, episode = 2, title = "Lies", path = "b.mkv", added = 2024-01-01 },
  { season = 1, episode = 3, path = "c.mkv", added = 2024-01-01 },
]
"#,
        );
        let show = &library.shows[0];
        assert_eq!(show.seasons[1].label(), "Specials");
        let labels: Vec<_> = show.episodes().map(Episode::label).collect();
        assert_eq!(labels, ["2. Lies", "Episode 3", "The Making Of"]);
        let short: Vec<_> = show.episodes().map(Episode::short_label).collect();
        assert_eq!(short, ["S1 E2", "S1 E3", "Special 1"]);
    }

    #[test]
    fn newest_mixes_shows_and_films_with_one_card_per_title() {
        let text = show("Dark", &[(1, 1, "2026-03-01"), (1, 2, "2026-03-02")])
            + &movies(&[("Up", "2026-03-03"), ("Old", "2020-01-01"), ("Wind", "2026-03-02")]);
        let library = library(&text);
        let keys: Vec<_> = library
            .newest(3)
            .into_iter()
            .map(|item| library.item_key(item))
            .collect();
        assert_eq!(keys, ["up", "dark", "wind"]);
        assert_eq!(library.newest(99).len(), 4);
    }

    #[test]
    fn runtimes_and_tmdb_ids_come_from_the_manifest() {
        let with_ids = library(
            r#"[[show]]
title = "Dark"
episodes = [ { season = 1, episode = 1, path = "Series/Dark/e.mkv", added = 2024-01-01, runtime = 51 } ]

[[movie]]
title = "Up"
tmdb = 14160
path = "Movies/Up/Up.mkv"
added = 2024-01-01
runtime = 96
"#,
        );
        assert_eq!(with_ids.shows[0].episodes().next().unwrap().runtime, Some(51));
        assert_eq!(with_ids.movies[0].runtime, Some(96));
        assert!(with_ids.uses_tmdb());
        assert!(!library(&movies(&[("Wind", "2024-01-01")])).uses_tmdb());
    }

    #[test]
    fn ratings_come_from_the_manifest_and_may_be_partial() {
        let rated = library(
            r#"[[show]]
title = "Dark"
imdb = "tt5753856"
ratings = { imdb = 8.7 }
rated = 2026-11-01
episodes = [ { season = 1, episode = 1, path = "Series/Dark/e.mkv", added = 2024-01-01 } ]

[[movie]]
title = "Up"
imdb = "tt1049413"
ratings = { imdb = 8.3, rt = 98, metacritic = 88 }
rated = 2026-11-01
path = "Movies/Up/Up.mkv"
added = 2024-01-01
"#,
        );
        let dark = &rated.shows[0];
        assert_eq!(dark.imdb.as_deref(), Some("tt5753856"));
        assert_eq!((dark.ratings.imdb, dark.ratings.rt), (Some(Tenths(87)), None));
        let up = &rated.movies[0];
        assert_eq!(
            (up.ratings.imdb, up.ratings.rt, up.ratings.metacritic),
            (Some(Tenths(83)), Some(98), Some(88))
        );
        assert_eq!(up.ratings.imdb.unwrap().to_string(), "8.3");
        assert!(library(&movies(&[("Wind", "2024-01-01")])).movies[0].ratings.is_empty());
    }

    #[test]
    fn keys_ignore_punctuation_and_films_carry_their_year() {
        assert_eq!(show_key("Marvel's Daredevil"), "marvels-daredevil");
        assert_eq!(movie_key("Airplane!", Some(1980)), "airplane-1980");
        assert_eq!(movie_key("Wind", None), "wind");
    }

    #[test]
    fn a_saved_copy_keeps_titles_whose_files_are_out_of_reach() {
        let dir = std::env::temp_dir().join(format!("semmeltv-saved-{}", std::process::id()));
        let root = dir.join("mount");
        fs::create_dir_all(&root).unwrap();
        let saved = dir.join("saved.toml");
        fs::write(root.join(manifest::FILE_NAME), movies(&[("Up", "2024-01-01")])).unwrap();
        assert!(load(&root).unwrap().movies.is_empty());
        save_copy(&root, &saved);
        fs::remove_file(root.join(manifest::FILE_NAME)).unwrap();
        assert!(load(&root).is_err());
        assert_eq!(load_saved(&root, &saved).unwrap().movies[0].title, "Up");
        fs::remove_dir_all(dir).unwrap();
    }
}
