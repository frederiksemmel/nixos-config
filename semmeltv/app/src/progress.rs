use crate::library::{Episode, Show};
use crate::watching::Watching;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub season: u32,
    pub episode: u32,
    pub finished: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Watched {
    pub stopped_at: Option<f64>,
    pub duration: Option<f64>,
    pub last_played: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Place {
    pub position: f64,
    pub duration: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    pub shows: HashMap<String, Position>,
    pub files: HashMap<PathBuf, Watched>,
    #[serde(default)]
    pub watching: Watching,
}

impl Progress {
    pub fn record(&mut self, file: &Path, place: Place) {
        let watched = self.files.entry(file.to_path_buf()).or_default();
        if place.position > 0.0 {
            watched.stopped_at = Some(place.position);
        }
        if place.duration > 0.0 {
            watched.duration = Some(place.duration);
        }
    }

    pub fn played(&mut self, file: &Path, now: u64) {
        self.files.entry(file.to_path_buf()).or_default().last_played = Some(now);
    }

    pub fn finish(&mut self, file: &Path) {
        self.files.entry(file.to_path_buf()).or_default().stopped_at = None;
    }

    pub fn resume_episode<'a>(&self, show: &'a Show) -> Option<&'a Episode> {
        match self.shows.get(&show.key) {
            Some(position) if position.finished => show.episode_after(position.season, position.episode),
            Some(position) => show
                .episode(position.season, position.episode)
                .or_else(|| show.first_episode()),
            None => show.first_episode(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILM: Place = Place {
        position: 3840.0,
        duration: 7860.0,
    };

    #[test]
    fn a_stop_keeps_the_place_and_the_real_duration() {
        let mut progress = Progress::default();
        let file = Path::new("/lib/Up.mkv");
        progress.record(file, FILM);
        assert_eq!(
            progress.files[file],
            Watched {
                stopped_at: Some(3840.0),
                duration: Some(7860.0),
                last_played: None,
            }
        );
    }

    #[test]
    fn a_play_mpv_never_started_keeps_what_was_known() {
        let mut progress = Progress::default();
        let file = Path::new("/lib/Up.mkv");
        progress.record(file, FILM);
        progress.record(file, Place::default());
        assert_eq!(progress.files[file].stopped_at, Some(3840.0));
        assert_eq!(progress.files[file].duration, Some(7860.0));
    }

    #[test]
    fn a_finished_file_loses_its_place_but_keeps_its_duration() {
        let mut progress = Progress::default();
        let file = Path::new("/lib/Up.mkv");
        progress.record(file, FILM);
        progress.finish(file);
        assert_eq!(progress.files[file].stopped_at, None);
        assert_eq!(progress.files[file].duration, Some(7860.0));
    }

    #[test]
    fn playing_a_file_stamps_when_it_was_last_played() {
        let mut progress = Progress::default();
        let file = Path::new("/lib/Up.mkv");
        progress.record(file, FILM);
        progress.played(file, 1_800_000_000);
        progress.finish(file);
        assert_eq!(progress.files[file].last_played, Some(1_800_000_000));
        assert_eq!(progress.files[file].duration, Some(7860.0));
    }

    #[test]
    fn the_file_survives_a_save_and_a_load() {
        let mut progress = Progress::default();
        let position = Position {
            season: 1,
            episode: 2,
            finished: false,
        };
        progress.shows.insert("dark".to_string(), position);
        progress.record(Path::new("/lib/Dark.S01E02.mkv"), FILM);
        let text = serde_json::to_string(&progress).unwrap();
        assert_eq!(serde_json::from_str::<Progress>(&text).unwrap(), progress);
    }
}
