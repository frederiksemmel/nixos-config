use crate::library::{Episode, Show};
use crate::watching::{Dismissal, Watching};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const CREDITS_FREE: f64 = 20.0 * 60.0;
const CREDITS_DIVISOR: f64 = 15.0;
const CREDITS_LONGEST: f64 = 10.0 * 60.0;

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
    #[serde(default)]
    pub finished: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits: Option<f64>,
}

impl Watched {
    pub fn credits_start(&self) -> Option<f64> {
        let duration = self.duration.filter(|duration| *duration > 0.0)?;
        let found = self.credits.filter(|start| (duration / 2.0..duration).contains(start));
        let estimated = ((duration - CREDITS_FREE) / CREDITS_DIVISOR).min(CREDITS_LONGEST);
        found.or((estimated > 0.0).then_some(duration - estimated))
    }

    pub fn in_credits(&self) -> bool {
        match (self.stopped_at, self.credits_start()) {
            (Some(at), Some(start)) => at >= start,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Place {
    pub position: f64,
    pub duration: f64,
    pub credits: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    pub shows: HashMap<String, Position>,
    pub files: HashMap<PathBuf, Watched>,
    #[serde(default)]
    pub watching: Watching,
    #[serde(default)]
    pub dismissed: Vec<Dismissal>,
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
        if place.credits.is_some() {
            watched.credits = place.credits;
        }
    }

    pub fn played(&mut self, file: &Path, now: u64) {
        self.files.entry(file.to_path_buf()).or_default().last_played = Some(now);
    }

    pub fn finish(&mut self, file: &Path) {
        let watched = self.files.entry(file.to_path_buf()).or_default();
        watched.stopped_at = None;
        watched.finished = true;
    }

    pub fn unfinish(&mut self, file: &Path) {
        if let Some(watched) = self.files.get_mut(file) {
            watched.stopped_at = None;
            watched.finished = false;
        }
    }

    pub fn watched(&self, show: &Show, episode: &Episode) -> bool {
        let finished = self.files.get(&episode.path).is_some_and(|watched| watched.finished);
        let passed = self.shows.get(&show.key).is_some_and(|position| {
            let place = (position.season, position.episode);
            let this = (episode.season, episode.number);
            episode.season > 0 && (this < place || (this == place && position.finished))
        });
        finished || passed
    }

    pub fn last_played(&self, files: impl IntoIterator<Item = impl AsRef<Path>>) -> Option<u64> {
        files
            .into_iter()
            .filter_map(|file| self.files.get(file.as_ref())?.last_played)
            .max()
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
        credits: None,
    };

    fn stopped(position: f64, duration: f64, credits: Option<f64>) -> Watched {
        Watched {
            stopped_at: Some(position),
            duration: Some(duration),
            credits,
            ..Watched::default()
        }
    }

    #[test]
    fn without_a_known_start_the_credits_grow_with_the_length_up_to_ten_minutes() {
        let left = |minutes: f64| {
            let duration = minutes * 60.0;
            let watched = stopped(0.0, duration, None);
            watched.credits_start().map(|start| duration - start)
        };
        assert_eq!(left(20.0), None);
        assert_eq!(left(35.0), Some(60.0));
        assert_eq!(left(95.0), Some(300.0));
        assert_eq!(left(170.0), Some(600.0));
        assert_eq!(left(240.0), Some(600.0));
    }

    #[test]
    fn a_stop_in_the_credits_of_airplane_counts_and_one_before_does_not() {
        assert!(stopped(5009.9, 5261.6, None).in_credits());
        assert!(stopped(5245.0, 5261.6, None).in_credits());
        assert!(!stopped(4900.0, 5261.6, None).in_credits());
        assert!(!Watched::default().in_credits());
    }

    #[test]
    fn a_known_credits_start_wins_over_the_estimate_unless_it_cannot_be_right() {
        assert!(stopped(3420.0, 3486.0, Some(3409.7)).in_credits());
        assert!(!stopped(3400.0, 3486.0, Some(3409.7)).in_credits());
        assert!(stopped(1315.0, 1320.0, Some(1330.0)).in_credits());
        assert!(!stopped(1200.0, 1320.0, Some(300.0)).in_credits());
    }

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
                finished: false,
                credits: None,
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
        assert!(progress.files[file].finished);
    }

    #[test]
    fn a_known_credits_start_is_kept_with_the_place() {
        let mut progress = Progress::default();
        let file = Path::new("/lib/Up.mkv");
        progress.record(
            file,
            Place {
                credits: Some(7500.0),
                ..FILM
            },
        );
        progress.record(file, FILM);
        assert_eq!(progress.files[file].credits, Some(7500.0));
    }

    #[test]
    fn an_unwatched_file_loses_its_place_and_its_finish() {
        let mut progress = Progress::default();
        let file = Path::new("/lib/Up.mkv");
        progress.record(file, FILM);
        progress.finish(file);
        progress.unfinish(file);
        assert_eq!(progress.files[file].stopped_at, None);
        assert!(!progress.files[file].finished);
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
    fn episodes_before_the_place_reached_or_finished_ones_count_as_watched() {
        use crate::library::Season;
        let episode = |season, number| Episode {
            season,
            number,
            path: PathBuf::from(format!("/lib/Dark.S{season:02}E{number:02}.mkv")),
            ..Episode::default()
        };
        let show = Show {
            key: "dark".to_string(),
            seasons: vec![
                Season {
                    number: 1,
                    episodes: vec![episode(1, 1), episode(1, 2), episode(1, 3)],
                },
                Season {
                    number: 2,
                    episodes: vec![episode(2, 1)],
                },
                Season {
                    number: 0,
                    episodes: vec![episode(0, 1)],
                },
            ],
            ..Show::default()
        };
        let watched = |progress: &Progress| -> Vec<bool> {
            show.episodes()
                .map(|episode| progress.watched(&show, episode))
                .collect()
        };
        let mut progress = Progress::default();
        assert_eq!(watched(&progress), [false; 5]);
        let at = |finished| Position {
            season: 1,
            episode: 2,
            finished,
        };
        progress.shows.insert("dark".to_string(), at(false));
        assert_eq!(watched(&progress), [true, false, false, false, false]);
        progress.shows.insert("dark".to_string(), at(true));
        assert_eq!(watched(&progress), [true, true, false, false, false]);
        progress.finish(Path::new("/lib/Dark.S02E01.mkv"));
        progress.finish(Path::new("/lib/Dark.S00E01.mkv"));
        assert_eq!(watched(&progress), [true, true, false, true, true]);
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
