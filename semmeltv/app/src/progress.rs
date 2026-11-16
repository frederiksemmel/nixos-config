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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tracks {
    pub audio: Option<i64>,
    pub subtitle: Option<i64>,
    pub subtitles_shown: Option<bool>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Viewing {
    #[default]
    NotStarted,
    InProgress {
        at: f64,
        tracks: Tracks,
    },
    Finished,
}

/// Where the credits start in a file of `duration` seconds: at the `known`
/// start when it lies in the second half, or else estimated from the length.
pub fn credits_start(duration: f64, known: Option<f64>) -> Option<f64> {
    if duration <= 0.0 {
        return None;
    }
    let found = known.filter(|start| (duration / 2.0..duration).contains(start));
    let estimated = ((duration - CREDITS_FREE) / CREDITS_DIVISOR).min(CREDITS_LONGEST);
    found.or((estimated > 0.0).then_some(duration - estimated))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Watched {
    pub viewing: Viewing,
    pub duration: Option<f64>,
    pub credits: Option<f64>,
    pub last_played: Option<u64>,
}

impl Watched {
    pub fn stopped_at(&self) -> Option<f64> {
        match self.viewing {
            Viewing::InProgress { at, .. } => Some(at),
            _ => None,
        }
    }

    pub fn finished(&self) -> bool {
        self.viewing == Viewing::Finished
    }

    pub fn tracks(&self) -> Tracks {
        match self.viewing {
            Viewing::InProgress { tracks, .. } => tracks,
            _ => Tracks::default(),
        }
    }

    pub fn credits_start(&self) -> Option<f64> {
        credits_start(self.duration?, self.credits)
    }

    pub fn in_credits(&self) -> bool {
        match (self.stopped_at(), self.credits_start()) {
            (Some(at), Some(start)) => at >= start,
            _ => false,
        }
    }

    pub fn resume_at(&self) -> f64 {
        if self.in_credits() {
            return 0.0;
        }
        self.stopped_at().unwrap_or(0.0)
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
            watched.viewing = Viewing::InProgress {
                at: place.position,
                tracks: watched.tracks(),
            };
        }
        if place.duration > 0.0 {
            watched.duration = Some(place.duration);
        }
        if place.credits.is_some() {
            watched.credits = place.credits;
        }
    }

    pub fn keep_tracks(&mut self, file: &Path, tracks: Tracks) {
        if let Some(Watched {
            viewing: Viewing::InProgress { tracks: kept, .. },
            ..
        }) = self.files.get_mut(file)
        {
            *kept = tracks;
        }
    }

    pub fn played(&mut self, file: &Path, now: u64) {
        self.files.entry(file.to_path_buf()).or_default().last_played = Some(now);
    }

    pub fn finish(&mut self, file: &Path) {
        self.files.entry(file.to_path_buf()).or_default().viewing = Viewing::Finished;
    }

    pub fn unfinish(&mut self, file: &Path) {
        if let Some(watched) = self.files.get_mut(file) {
            watched.viewing = Viewing::NotStarted;
        }
    }

    pub fn watched(&self, show: &Show, episode: &Episode) -> bool {
        let finished = self.files.get(&episode.path).is_some_and(Watched::finished);
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
            viewing: Viewing::InProgress {
                at: position,
                tracks: Tracks::default(),
            },
            duration: Some(duration),
            credits,
            ..Watched::default()
        }
    }

    #[test]
    fn a_stop_is_in_the_credits_from_the_known_or_estimated_start() {
        let estimated = [
            (20.0, None),
            (35.0, Some(60.0)),
            (95.0, Some(300.0)),
            (170.0, Some(600.0)),
            (240.0, Some(600.0)),
        ];
        for (minutes, left) in estimated {
            let duration = minutes * 60.0;
            let watched = stopped(0.0, duration, None);
            assert_eq!(
                watched.credits_start().map(|start| duration - start),
                left,
                "{minutes} minutes"
            );
        }
        let stops = [
            (stopped(5009.9, 5261.6, None), true),
            (stopped(5245.0, 5261.6, None), true),
            (stopped(4900.0, 5261.6, None), false),
            (Watched::default(), false),
            (stopped(3420.0, 3486.0, Some(3409.7)), true),
            (stopped(3400.0, 3486.0, Some(3409.7)), false),
            (stopped(1315.0, 1320.0, Some(1330.0)), true),
            (stopped(1200.0, 1320.0, Some(300.0)), false),
        ];
        for (watched, in_credits) in stops {
            assert_eq!(watched.in_credits(), in_credits, "{watched:?}");
        }
    }

    #[test]
    fn a_record_only_overwrites_what_it_knows() {
        let credited = Place {
            credits: Some(7500.0),
            ..FILM
        };
        let cases = [
            ("a stop", vec![FILM], None),
            ("a play mpv never started", vec![FILM, Place::default()], None),
            ("a stop without the known credits", vec![credited, FILM], Some(7500.0)),
        ];
        for (name, places, credits) in cases {
            let mut progress = Progress::default();
            let file = Path::new("/lib/Up.mkv");
            for place in places {
                progress.record(file, place);
            }
            assert_eq!(progress.files[file].stopped_at(), Some(3840.0), "{name}");
            assert_eq!(progress.files[file].duration, Some(7860.0), "{name}");
            assert_eq!(progress.files[file].credits, credits, "{name}");
        }
    }

    #[test]
    fn a_finished_file_played_again_resumes_where_and_with_the_tracks_it_stopped_the_second_time() {
        let mut progress = Progress::default();
        let file = Path::new("/lib/The.White.Lotus.S02E07.mkv");
        let episode = Place {
            position: 3600.0,
            duration: 4300.0,
            credits: None,
        };
        let tracks = Tracks {
            audio: Some(2),
            subtitle: Some(3),
            subtitles_shown: Some(true),
        };
        progress.record(file, episode);
        progress.keep_tracks(file, tracks);
        progress.finish(file);
        assert_eq!(progress.files[file].resume_at(), 0.0);
        assert_eq!(progress.files[file].tracks(), Tracks::default());
        let again = Place {
            position: 3887.0,
            ..episode
        };
        progress.record(file, again);
        progress.keep_tracks(file, tracks);
        progress.record(file, again);
        assert!(!progress.files[file].finished());
        assert_eq!(progress.files[file].resume_at(), 3887.0);
        assert_eq!(progress.files[file].tracks(), tracks);
        progress.record(
            file,
            Place {
                position: 4200.0,
                ..episode
            },
        );
        assert_eq!(progress.files[file].resume_at(), 0.0);
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
}
