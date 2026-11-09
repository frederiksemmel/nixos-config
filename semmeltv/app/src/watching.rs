use crate::library::{Item, Library};
use crate::progress::{Progress, Watched};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SERIES: usize = 3;
pub const MOVIES: usize = 3;
pub const ENOUGH: f64 = 60.0;

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
    use crate::library::{Episode, Movie, Season, Show};
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

    #[test]
    fn the_list_survives_a_save_and_a_load() {
        let library = library();
        let mut progress = Progress::default();
        record(&mut progress, &library, &file("dark", 1, 2), 90.0, 7);
        let text = serde_json::to_string(&progress).unwrap();
        assert_eq!(serde_json::from_str::<Progress>(&text).unwrap(), progress);
    }
}
