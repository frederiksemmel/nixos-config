use crate::library::{Episode, Item, Library, Movie, Show};
use crate::progress::{Position, Progress, Watched};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SERIES: usize = 3;
pub const MOVIES: usize = 3;
pub const ENOUGH: f64 = 60.0;
const A_WHILE_DAYS: f64 = 7.0;
const UNKNOWN_AGE_DAYS: f64 = 90.0;
const SECONDS_PER_DAY: f64 = 86_400.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Series,
    Movie,
}

impl Kind {
    fn cap(self) -> usize {
        match self {
            Self::Series => SERIES,
            Self::Movie => MOVIES,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub kind: Kind,
    pub key: String,
    pub last_played: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dismissal {
    pub file: PathBuf,
    pub at: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Watching {
    entries: Vec<Entry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Current {
    pub item: Item,
    pub file: PathBuf,
    pub episode: Option<(u32, u32)>,
    pub watched: Watched,
    pub last_played: u64,
}

impl Current {
    pub fn fraction(&self) -> f32 {
        match (self.watched.stopped_at, self.watched.duration) {
            (Some(at), Some(duration)) if duration > 0.0 => (at / duration).clamp(0.0, 1.0) as f32,
            _ => 0.0,
        }
    }

    pub fn remaining(&self) -> Option<f64> {
        let (at, duration) = (self.watched.stopped_at?, self.watched.duration?);
        (duration > at).then_some(duration - at)
    }
}

impl Watching {
    pub fn from_entries(entries: Vec<Entry>) -> Self {
        Self { entries }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn played(&mut self, kind: Kind, key: &str, now: u64) -> bool {
        let before: Vec<_> = self.order();
        self.entries.retain(|entry| !(entry.kind == kind && entry.key == key));
        self.entries.insert(
            0,
            Entry {
                kind,
                key: key.to_string(),
                last_played: now,
            },
        );
        let mut kept = 0;
        self.entries.retain(|entry| {
            if entry.kind != kind {
                return true;
            }
            kept += 1;
            kept <= kind.cap()
        });
        self.order() != before
    }

    pub fn last_played(&self, kind: Kind, key: &str) -> Option<u64> {
        self.entries
            .iter()
            .find(|entry| entry.kind == kind && entry.key == key)
            .map(|entry| entry.last_played)
    }

    pub fn leave(&mut self, kind: Kind, key: &str) -> bool {
        let count = self.entries.len();
        self.entries.retain(|entry| !(entry.kind == kind && entry.key == key));
        self.entries.len() != count
    }

    fn order(&self) -> Vec<(Kind, String)> {
        self.entries
            .iter()
            .map(|entry| (entry.kind, entry.key.clone()))
            .collect()
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn identify(library: &Library, file: &Path) -> Option<(Kind, String)> {
    if let Some((show, _)) = library.find_episode(file) {
        return Some((Kind::Series, show.key.clone()));
    }
    let movie = library.movies.iter().find(|movie| movie.path == file)?;
    Some((Kind::Movie, movie.key.clone()))
}

pub fn record(progress: &mut Progress, library: &Library, file: &Path, played: f64, now: u64) -> bool {
    if played < ENOUGH {
        return false;
    }
    let Some((kind, key)) = identify(library, file) else {
        return false;
    };
    progress.watching.played(kind, &key, now)
}

pub fn finished(progress: &mut Progress, library: &Library, file: &Path) -> bool {
    if let Some((show, episode)) = library.find_episode(file) {
        return show.episode_after(episode.season, episode.number).is_none()
            && progress.watching.leave(Kind::Series, &show.key);
    }
    match identify(library, file) {
        Some((kind, key)) => progress.watching.leave(kind, &key),
        None => false,
    }
}

pub fn complete(progress: &mut Progress, library: &Library, file: &Path) {
    progress.finish(file);
    if let Some((show, episode)) = library.find_episode(file) {
        let this = (episode.season, episode.number);
        let ahead = progress
            .shows
            .get(&show.key)
            .is_some_and(|position| (position.season, position.episode) > this);
        if !ahead {
            let position = Position {
                season: episode.season,
                episode: episode.number,
                finished: true,
            };
            progress.shows.insert(show.key.clone(), position);
        }
    }
    finished(progress, library, file);
}

pub fn stopped(progress: &mut Progress, library: &Library, file: &Path) -> bool {
    let in_credits = progress.files.get(file).is_some_and(Watched::in_credits);
    if in_credits {
        complete(progress, library, file);
    }
    in_credits
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Watched,
    Unwatched,
}

fn last_episode(show: &Show) -> Option<&Episode> {
    show.episodes()
        .filter(|episode| episode.season > 0)
        .max_by_key(|episode| (episode.season, episode.number))
        .or_else(|| show.episodes().last())
}

pub fn mark(
    progress: &mut Progress,
    library: &Library,
    kind: Kind,
    key: &str,
    episode: Option<(u32, u32)>,
    mark: Mark,
) -> Option<Vec<PathBuf>> {
    let files: Vec<PathBuf> = match kind {
        Kind::Movie => vec![library.movies.iter().find(|movie| movie.key == key)?.path.clone()],
        Kind::Series => {
            let show = library.shows.iter().find(|show| show.key == key)?;
            match (episode, mark) {
                (Some((season, number)), _) => vec![show.episode(season, number)?.path.clone()],
                (None, Mark::Watched) => vec![last_episode(show)?.path.clone()],
                (None, Mark::Unwatched) => show.episodes().map(|episode| episode.path.clone()).collect(),
            }
        }
    };
    match mark {
        Mark::Watched => {
            for file in &files {
                complete(progress, library, file);
            }
        }
        Mark::Unwatched => {
            for file in &files {
                progress.unfinish(file);
            }
            match (kind, episode) {
                (Kind::Series, Some((season, number))) => {
                    let position = progress.shows.get_mut(key).filter(|position| {
                        (position.season, position.episode, position.finished) >= (season, number, false)
                    });
                    if let Some(position) = position {
                        *position = Position {
                            season,
                            episode: number,
                            finished: false,
                        };
                    }
                }
                (Kind::Series, None) => {
                    progress.shows.remove(key);
                    progress.watching.leave(kind, key);
                }
                (Kind::Movie, _) => {
                    progress.watching.leave(kind, key);
                }
            }
        }
    }
    Some(files)
}

pub fn movie_finished(progress: &Progress, movie: &Movie) -> bool {
    progress.files.get(&movie.path).is_some_and(|watched| watched.finished)
}

pub fn show_finished(progress: &Progress, show: &Show) -> bool {
    progress.shows.contains_key(&show.key) && progress.resume_episode(show).is_none()
}

pub fn current(library: &Library, progress: &Progress) -> Vec<Current> {
    let watched = |file: &Path| progress.files.get(file).copied().unwrap_or_default();
    progress
        .watching
        .entries
        .iter()
        .filter_map(|entry| match entry.kind {
            Kind::Series => {
                let index = library.shows.iter().position(|show| show.key == entry.key)?;
                let episode = progress.resume_episode(&library.shows[index])?;
                Some(Current {
                    item: Item::Show(index),
                    file: episode.path.clone(),
                    episode: Some((episode.season, episode.number)),
                    watched: watched(&episode.path),
                    last_played: entry.last_played,
                })
            }
            Kind::Movie => {
                let index = library.movies.iter().position(|movie| movie.key == entry.key)?;
                let movie = &library.movies[index];
                Some(Current {
                    item: Item::Movie(index),
                    file: movie.path.clone(),
                    episode: None,
                    watched: watched(&movie.path),
                    last_played: entry.last_played,
                })
            }
        })
        .collect()
}

struct Started {
    current: Current,
    share: f64,
    last_played: Option<u64>,
}

fn started_movie(progress: &Progress, index: usize, movie: &Movie) -> Option<Started> {
    let watched = *progress.files.get(&movie.path)?;
    let (at, duration) = (watched.stopped_at?, watched.duration?);
    if watched.finished || watched.in_credits() || duration <= 0.0 || at <= 0.0 {
        return None;
    }
    let entry = progress.watching.last_played(Kind::Movie, &movie.key);
    Some(Started {
        current: Current {
            item: Item::Movie(index),
            file: movie.path.clone(),
            episode: None,
            watched,
            last_played: watched.last_played.max(entry).unwrap_or(0),
        },
        share: at / duration,
        last_played: watched.last_played.max(entry),
    })
}

fn started_show(progress: &Progress, index: usize, show: &Show) -> Option<Started> {
    progress.shows.get(&show.key)?;
    let episode = progress.resume_episode(show)?;
    let regular: Vec<&Episode> = show.episodes().filter(|episode| episode.season > 0).collect();
    let all: Vec<&Episode> = if regular.is_empty() {
        show.episodes().collect()
    } else {
        regular
    };
    let seen = all.iter().filter(|candidate| progress.watched(show, candidate)).count();
    let watched = progress.files.get(&episode.path).copied().unwrap_or_default();
    let current = Current {
        item: Item::Show(index),
        file: episode.path.clone(),
        episode: Some((episode.season, episode.number)),
        watched,
        last_played: 0,
    };
    let share = (seen as f64 + f64::from(current.fraction())) / all.len().max(1) as f64;
    let played = progress.last_played(show.episodes().map(|episode| &episode.path));
    let last_played = played.max(progress.watching.last_played(Kind::Series, &show.key));
    Some(Started {
        current: Current {
            last_played: last_played.unwrap_or(0),
            ..current
        },
        share,
        last_played,
    })
}

fn dismissed(progress: &Progress, started: &Started) -> bool {
    progress
        .dismissed
        .iter()
        .any(|dismissal| dismissal.file == started.current.file && Some(dismissal.at) >= started.last_played)
}

pub fn relevance(share: f64, last_played: Option<u64>, now: u64) -> f64 {
    let days = last_played.map_or(UNKNOWN_AGE_DAYS, |then| {
        now.saturating_sub(then) as f64 / SECONDS_PER_DAY
    });
    share * (days + A_WHILE_DAYS) / (days + 2.0 * A_WHILE_DAYS)
}

pub fn forgotten(library: &Library, progress: &Progress, now: u64) -> Vec<Current> {
    let shows = library.shows.iter().enumerate().filter_map(|(index, show)| {
        let started = started_show(progress, index, show)?;
        Some((Kind::Series, show.key.as_str(), show.title.as_str(), started))
    });
    let movies = library.movies.iter().enumerate().filter_map(|(index, movie)| {
        let started = started_movie(progress, index, movie)?;
        Some((Kind::Movie, movie.key.as_str(), movie.title.as_str(), started))
    });
    let mut ranked: Vec<(f64, &str, Current)> = shows
        .chain(movies)
        .filter(|(kind, key, _, started)| {
            progress.watching.last_played(*kind, key).is_none() && !dismissed(progress, started)
        })
        .map(|(_, _, title, started)| {
            (
                relevance(started.share, started.last_played, now),
                title,
                started.current,
            )
        })
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    ranked.into_iter().map(|(_, _, current)| current).collect()
}

pub fn dismiss(progress: &mut Progress, library: &Library, kind: Kind, key: &str, now: u64) -> bool {
    let file = match kind {
        Kind::Series => library
            .shows
            .iter()
            .find(|show| show.key == key)
            .and_then(|show| progress.resume_episode(show))
            .map(|episode| episode.path.clone()),
        Kind::Movie => library
            .movies
            .iter()
            .find(|movie| movie.key == key)
            .map(|movie| movie.path.clone()),
    };
    let Some(file) = file else {
        return false;
    };
    progress.dismissed.retain(|dismissal| dismissal.file != file);
    progress.dismissed.push(Dismissal { file, at: now });
    true
}

pub fn prefetch(library: &Library, progress: &Progress) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for current in current(library, progress) {
        let Item::Show(index) = current.item else {
            continue;
        };
        paths.push(current.file.clone());
        let next = current
            .episode
            .and_then(|(season, number)| library.shows[index].episode_after(season, number));
        paths.extend(next.map(|episode| episode.path.clone()));
    }
    paths
}

pub fn downloads(library: &Library, progress: &Progress, playing: Option<&Path>) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    if let Some(file) = playing {
        paths.push(file.to_path_buf());
        if let Some((show, episode)) = library.find_episode(file)
            && let Some(next) = show.episode_after(episode.season, episode.number)
        {
            paths.push(next.path.clone());
        }
    }
    for path in prefetch(library, progress) {
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Season;
    use crate::progress::{Place, Position};

    fn episode(show: &str, season: u32, number: u32) -> Episode {
        Episode {
            season,
            number,
            path: PathBuf::from(format!("/lib/{show}/S{season:02}E{number:02}.mkv")),
            ..Episode::default()
        }
    }

    fn show(key: &str, seasons: &[(u32, u32)]) -> Show {
        Show {
            key: key.to_string(),
            title: key.to_string(),
            seasons: seasons
                .iter()
                .map(|&(number, count)| Season {
                    number,
                    episodes: (1..=count).map(|e| episode(key, number, e)).collect(),
                })
                .collect(),
            ..Show::default()
        }
    }

    fn movie(key: &str) -> Movie {
        Movie {
            key: key.to_string(),
            title: key.to_string(),
            path: PathBuf::from(format!("/lib/{key}.mkv")),
            ..Movie::default()
        }
    }

    fn library() -> Library {
        Library {
            shows: ["dark", "lost", "fargo", "severance"]
                .iter()
                .map(|key| show(key, &[(1, 3), (2, 2)]))
                .collect(),
            movies: ["up", "heat", "alien", "jaws"].iter().map(|key| movie(key)).collect(),
        }
    }

    fn file(show: &str, season: u32, number: u32) -> PathBuf {
        episode(show, season, number).path
    }

    fn keys(progress: &Progress) -> Vec<&str> {
        progress
            .watching
            .entries
            .iter()
            .map(|entry| entry.key.as_str())
            .collect()
    }

    #[test]
    fn a_minute_of_playback_is_needed_to_count() {
        let library = library();
        let mut progress = Progress::default();
        assert!(!record(&mut progress, &library, &file("dark", 1, 1), 59.0, 1));
        assert!(progress.watching.entries.is_empty());
        assert!(record(&mut progress, &library, &file("dark", 1, 1), 60.0, 2));
        assert_eq!(keys(&progress), ["dark"]);
    }

    #[test]
    fn the_latest_title_moves_to_the_front() {
        let library = library();
        let mut progress = Progress::default();
        record(&mut progress, &library, &file("dark", 1, 1), 90.0, 1);
        record(&mut progress, &library, Path::new("/lib/up.mkv"), 90.0, 2);
        record(&mut progress, &library, &file("dark", 1, 2), 90.0, 3);
        assert_eq!(keys(&progress), ["dark", "up"]);
        assert_eq!(progress.watching.entries[0].last_played, 3);
    }

    #[test]
    fn playing_on_keeps_the_order_but_reports_no_change() {
        let library = library();
        let mut progress = Progress::default();
        record(&mut progress, &library, &file("dark", 1, 1), 90.0, 1);
        assert!(!record(&mut progress, &library, &file("dark", 1, 1), 120.0, 2));
        assert_eq!(progress.watching.entries[0].last_played, 2);
    }

    #[test]
    fn three_series_and_three_movies_at_most() {
        let library = library();
        let mut progress = Progress::default();
        for (time, key) in ["dark", "lost", "fargo", "severance"].iter().enumerate() {
            record(&mut progress, &library, &file(key, 1, 1), 90.0, time as u64);
        }
        for (time, key) in ["up", "heat", "alien", "jaws"].iter().enumerate() {
            let path = PathBuf::from(format!("/lib/{key}.mkv"));
            record(&mut progress, &library, &path, 90.0, 10 + time as u64);
        }
        assert_eq!(keys(&progress), ["jaws", "alien", "heat", "severance", "fargo", "lost"]);
    }

    #[test]
    fn a_finished_movie_leaves() {
        let library = library();
        let mut progress = Progress::default();
        let up = Path::new("/lib/up.mkv");
        record(&mut progress, &library, up, 90.0, 1);
        assert!(finished(&mut progress, &library, up));
        assert!(progress.watching.entries.is_empty());
    }

    #[test]
    fn a_series_stays_between_episodes_and_leaves_after_the_last() {
        let library = library();
        let mut progress = Progress::default();
        record(&mut progress, &library, &file("dark", 1, 3), 90.0, 1);
        assert!(!finished(&mut progress, &library, &file("dark", 1, 3)));
        assert_eq!(keys(&progress), ["dark"]);
        assert!(finished(&mut progress, &library, &file("dark", 2, 2)));
        assert!(progress.watching.entries.is_empty());
    }

    fn at(progress: &mut Progress, show: &str, season: u32, episode: u32, finished: bool) {
        let position = Position {
            season,
            episode,
            finished,
        };
        progress.shows.insert(show.to_string(), position);
    }

    #[test]
    fn a_series_points_at_where_to_resume() {
        let library = library();
        let mut progress = Progress::default();
        record(&mut progress, &library, &file("dark", 1, 3), 90.0, 1);
        at(&mut progress, "dark", 1, 3, true);
        let current = current(&library, &progress);
        assert_eq!(current[0].item, Item::Show(0));
        assert_eq!(current[0].episode, Some((2, 1)));
        assert_eq!(current[0].file, file("dark", 2, 1));
    }

    #[test]
    fn a_started_episode_shows_how_far_it_got() {
        let library = library();
        let mut progress = Progress::default();
        record(&mut progress, &library, &file("dark", 1, 2), 90.0, 1);
        at(&mut progress, "dark", 1, 2, false);
        let place = Place {
            position: 900.0,
            duration: 3600.0,
            credits: None,
        };
        progress.record(&file("dark", 1, 2), place);
        let current = &current(&library, &progress)[0];
        assert_eq!(current.fraction(), 0.25);
        assert_eq!(current.remaining(), Some(2700.0));
    }

    #[test]
    fn titles_missing_from_the_library_are_skipped() {
        let library = library();
        let mut progress = Progress::default();
        progress.watching.played(Kind::Series, "gone", 1);
        progress.watching.played(Kind::Movie, "up", 2);
        let items: Vec<_> = current(&library, &progress).iter().map(|c| c.item).collect();
        assert_eq!(items, [Item::Movie(0)]);
    }

    #[test]
    fn watched_series_keep_the_resume_episode_and_the_next_one() {
        let library = library();
        let mut progress = Progress::default();
        record(&mut progress, &library, &file("dark", 1, 2), 90.0, 1);
        at(&mut progress, "dark", 1, 2, false);
        record(&mut progress, &library, Path::new("/lib/up.mkv"), 90.0, 2);
        record(&mut progress, &library, &file("lost", 1, 3), 90.0, 3);
        at(&mut progress, "lost", 1, 3, true);
        assert_eq!(
            prefetch(&library, &progress),
            [
                file("lost", 2, 1),
                file("lost", 2, 2),
                file("dark", 1, 2),
                file("dark", 1, 3)
            ]
        );
    }

    #[test]
    fn the_playing_title_comes_first_without_duplicates() {
        let library = library();
        let mut progress = Progress::default();
        record(&mut progress, &library, &file("dark", 1, 2), 90.0, 1);
        at(&mut progress, "dark", 1, 2, false);
        let up = Path::new("/lib/up.mkv");
        assert_eq!(
            downloads(&library, &progress, Some(up)),
            [up.to_path_buf(), file("dark", 1, 2), file("dark", 1, 3)]
        );
        assert_eq!(
            downloads(&library, &progress, Some(&file("dark", 1, 2))),
            [file("dark", 1, 2), file("dark", 1, 3)]
        );
    }

    const DAY: u64 = 86_400;
    const NOW: u64 = 1_000 * DAY;

    fn stop(progress: &mut Progress, path: &Path, position: f64, duration: f64, when: Option<u64>) {
        progress.record(
            path,
            Place {
                position,
                duration,
                credits: None,
            },
        );
        if let Some(when) = when {
            progress.played(path, when);
        }
    }

    fn forgotten_keys(library: &Library, progress: &Progress) -> Vec<String> {
        forgotten(library, progress, NOW)
            .iter()
            .map(|current| match current.item {
                Item::Show(index) => library.shows[index].key.clone(),
                Item::Movie(index) => library.movies[index].key.clone(),
            })
            .collect()
    }

    #[test]
    fn a_long_watch_left_long_ago_ranks_above_a_film_abandoned_after_minutes() {
        let library = library();
        let mut progress = Progress::default();
        stop(
            &mut progress,
            Path::new("/lib/up.mkv"),
            180.0,
            6000.0,
            Some(NOW - 60 * DAY),
        );
        at(&mut progress, "lost", 2, 1, false);
        stop(&mut progress, &file("lost", 2, 1), 1500.0, 3000.0, Some(NOW - 90 * DAY));
        stop(
            &mut progress,
            Path::new("/lib/heat.mkv"),
            3000.0,
            6000.0,
            Some(NOW - DAY),
        );
        assert_eq!(forgotten_keys(&library, &progress), ["lost", "heat", "up"]);
    }

    #[test]
    fn the_score_grows_with_the_share_watched_and_the_time_since() {
        assert_eq!(relevance(0.5, Some(NOW), NOW), 0.25);
        assert_eq!(relevance(0.6, Some(NOW - 7 * DAY), NOW), 0.4);
        assert!(relevance(0.5, None, NOW) > relevance(0.5, Some(NOW - 30 * DAY), NOW));
        assert!(relevance(0.05, Some(NOW - 365 * DAY), NOW) < relevance(0.5, Some(NOW - 14 * DAY), NOW));
    }

    #[test]
    fn finished_and_currently_watched_titles_are_not_forgotten() {
        let library = library();
        let mut progress = Progress::default();
        let up = Path::new("/lib/up.mkv");
        stop(&mut progress, up, 3000.0, 6000.0, Some(NOW - DAY));
        progress.finish(up);
        stop(
            &mut progress,
            Path::new("/lib/heat.mkv"),
            5800.0,
            6000.0,
            Some(NOW - DAY),
        );
        at(&mut progress, "dark", 2, 2, true);
        record(&mut progress, &library, Path::new("/lib/jaws.mkv"), 90.0, NOW);
        stop(&mut progress, Path::new("/lib/jaws.mkv"), 900.0, 6000.0, Some(NOW));
        at(&mut progress, "lost", 1, 1, false);
        assert_eq!(forgotten_keys(&library, &progress), ["lost"]);
    }

    #[test]
    fn a_dismissed_title_stays_away_until_it_plays_again() {
        let library = library();
        let mut progress = Progress::default();
        let up = Path::new("/lib/up.mkv");
        stop(&mut progress, up, 3000.0, 6000.0, Some(NOW - 30 * DAY));
        assert!(dismiss(&mut progress, &library, Kind::Movie, "up", NOW - DAY));
        assert!(forgotten_keys(&library, &progress).is_empty());
        assert_eq!(progress.files[up].stopped_at, Some(3000.0));
        progress.played(up, NOW);
        assert_eq!(forgotten_keys(&library, &progress), ["up"]);
        assert!(!dismiss(&mut progress, &library, Kind::Movie, "gone", NOW));
    }

    #[test]
    fn a_dismissal_outlives_a_new_key_for_the_title() {
        let mut library = library();
        let mut progress = Progress::default();
        let up = Path::new("/lib/up.mkv");
        stop(&mut progress, up, 3000.0, 6000.0, Some(NOW - 30 * DAY));
        at(&mut progress, "lost", 1, 2, false);
        assert!(dismiss(&mut progress, &library, Kind::Movie, "up", NOW));
        assert!(dismiss(&mut progress, &library, Kind::Series, "lost", NOW));
        library.movies[0].key = "up-2009".to_string();
        assert!(forgotten_keys(&library, &progress).is_empty());
    }

    const AIRPLANE: f64 = 5261.6;

    #[test]
    fn a_stop_in_the_credits_finishes_the_film() {
        let library = library();
        let mut progress = Progress::default();
        let up = Path::new("/lib/up.mkv");
        record(&mut progress, &library, up, 90.0, NOW);
        stop(&mut progress, up, 5009.9, AIRPLANE, Some(NOW));
        assert!(stopped(&mut progress, &library, up));
        assert!(progress.watching.entries.is_empty());
        assert!(progress.files[up].finished);
        assert_eq!(progress.files[up].stopped_at, None);
        assert!(forgotten_keys(&library, &progress).is_empty());
    }

    #[test]
    fn a_stop_before_the_credits_resumes_there() {
        let library = library();
        let mut progress = Progress::default();
        let up = Path::new("/lib/up.mkv");
        record(&mut progress, &library, up, 90.0, NOW);
        stop(&mut progress, up, 4900.0, AIRPLANE, Some(NOW));
        assert!(!stopped(&mut progress, &library, up));
        assert_eq!(keys(&progress), ["up"]);
        assert_eq!(progress.files[up].stopped_at, Some(4900.0));
        assert!(!progress.files[up].finished);
    }

    #[test]
    fn a_stop_in_the_credits_of_an_episode_moves_the_series_on() {
        let library = library();
        let mut progress = Progress::default();
        let episode = file("dark", 1, 2);
        record(&mut progress, &library, &episode, 90.0, NOW);
        at(&mut progress, "dark", 1, 2, false);
        progress.record(
            &episode,
            Place {
                position: 3420.0,
                duration: 3486.0,
                credits: Some(3409.7),
            },
        );
        assert!(stopped(&mut progress, &library, &episode));
        assert_eq!(keys(&progress), ["dark"]);
        let current = &current(&library, &progress)[0];
        assert_eq!(current.episode, Some((1, 3)));
        assert_eq!(current.fraction(), 0.0);
    }

    #[test]
    fn a_stop_in_the_credits_of_the_last_episode_ends_the_series() {
        let library = library();
        let mut progress = Progress::default();
        let last = file("dark", 2, 2);
        record(&mut progress, &library, &last, 90.0, NOW);
        at(&mut progress, "dark", 2, 2, false);
        stop(&mut progress, &last, 2990.0, 3000.0, Some(NOW));
        assert!(stopped(&mut progress, &library, &last));
        assert!(progress.watching.entries.is_empty());
        assert!(forgotten_keys(&library, &progress).is_empty());
    }

    #[test]
    fn marking_a_film_watched_and_unwatched() {
        let library = library();
        let mut progress = Progress::default();
        let up = Path::new("/lib/up.mkv");
        record(&mut progress, &library, up, 90.0, NOW);
        stop(&mut progress, up, 3000.0, 6000.0, Some(NOW));
        let marked = mark(&mut progress, &library, Kind::Movie, "up", None, Mark::Watched);
        assert_eq!(marked, Some(vec![up.to_path_buf()]));
        assert!(movie_finished(&progress, &library.movies[0]));
        assert!(progress.watching.entries.is_empty());
        assert_eq!(progress.files[up].stopped_at, None);
        mark(&mut progress, &library, Kind::Movie, "up", None, Mark::Unwatched);
        assert!(!movie_finished(&progress, &library.movies[0]));
        assert!(forgotten_keys(&library, &progress).is_empty());
        assert_eq!(
            mark(&mut progress, &library, Kind::Movie, "gone", None, Mark::Watched),
            None
        );
    }

    #[test]
    fn marking_an_unwatched_film_takes_it_off_currently_watching() {
        let library = library();
        let mut progress = Progress::default();
        let up = Path::new("/lib/up.mkv");
        record(&mut progress, &library, up, 90.0, NOW);
        stop(&mut progress, up, 3000.0, 6000.0, Some(NOW));
        mark(&mut progress, &library, Kind::Movie, "up", None, Mark::Unwatched);
        assert!(progress.watching.entries.is_empty());
        assert_eq!(progress.files[up].stopped_at, None);
    }

    #[test]
    fn marking_an_episode_watched_moves_the_series_past_it_but_never_back() {
        let library = library();
        let mut progress = Progress::default();
        let dark = &library.shows[0];
        let episode = Some((1, 2));
        mark(&mut progress, &library, Kind::Series, "dark", episode, Mark::Watched);
        assert_eq!(
            progress.resume_episode(dark).map(|e| (e.season, e.number)),
            Some((1, 3))
        );
        at(&mut progress, "dark", 2, 1, false);
        mark(
            &mut progress,
            &library,
            Kind::Series,
            "dark",
            Some((1, 1)),
            Mark::Watched,
        );
        assert_eq!(
            progress.resume_episode(dark).map(|e| (e.season, e.number)),
            Some((2, 1))
        );
    }

    #[test]
    fn marking_an_episode_unwatched_resumes_the_series_there() {
        let library = library();
        let mut progress = Progress::default();
        let dark = &library.shows[0];
        at(&mut progress, "dark", 2, 1, true);
        mark(
            &mut progress,
            &library,
            Kind::Series,
            "dark",
            Some((1, 2)),
            Mark::Unwatched,
        );
        assert_eq!(
            progress.resume_episode(dark).map(|e| (e.season, e.number)),
            Some((1, 2))
        );
        assert!(progress.watched(dark, &dark.seasons[0].episodes[0]));
        assert!(!progress.watched(dark, &dark.seasons[0].episodes[1]));
        mark(
            &mut progress,
            &library,
            Kind::Series,
            "dark",
            Some((2, 2)),
            Mark::Unwatched,
        );
        assert_eq!(
            progress.resume_episode(dark).map(|e| (e.season, e.number)),
            Some((1, 2))
        );
    }

    #[test]
    fn marking_a_whole_series_watched_and_unwatched() {
        let library = library();
        let mut progress = Progress::default();
        let dark = &library.shows[0];
        record(&mut progress, &library, &file("dark", 1, 2), 90.0, NOW);
        at(&mut progress, "dark", 1, 2, false);
        mark(&mut progress, &library, Kind::Series, "dark", None, Mark::Watched);
        assert!(show_finished(&progress, dark));
        assert!(progress.watching.entries.is_empty());
        assert!(dark.episodes().all(|episode| progress.watched(dark, episode)));
        let marked = mark(&mut progress, &library, Kind::Series, "dark", None, Mark::Unwatched).unwrap();
        assert_eq!(marked.len(), 5);
        assert!(!show_finished(&progress, dark));
        assert!(dark.episodes().all(|episode| !progress.watched(dark, episode)));
        assert!(forgotten_keys(&library, &progress).is_empty());
    }

    #[test]
    fn the_list_survives_a_save_and_a_load() {
        let library = library();
        let mut progress = Progress::default();
        record(&mut progress, &library, &file("dark", 1, 2), 90.0, 7);
        dismiss(&mut progress, &library, Kind::Series, "lost", 8);
        let text = serde_json::to_string(&progress).unwrap();
        assert_eq!(serde_json::from_str::<Progress>(&text).unwrap(), progress);
    }
}
