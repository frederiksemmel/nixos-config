use crate::library::Show;
use crate::progress::{Progress, Viewing, Watched};
use crate::watching::Kind;

const SAME_PLACE: f64 = 60.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Spot {
    NotStarted,
    At {
        season: u32,
        episode: u32,
        stopped_at: Option<f64>,
    },
    Finished,
}

impl Spot {
    pub fn of(progress: &Progress, show: &Show) -> Self {
        if !progress.shows.contains_key(&show.key) {
            return Self::NotStarted;
        }
        match progress.resume_episode(show) {
            Some(episode) => Self::At {
                season: episode.season,
                episode: episode.number,
                stopped_at: progress
                    .files
                    .get(&episode.path)
                    .map(Watched::resume_at)
                    .filter(|at| *at > 0.0),
            },
            None => Self::Finished,
        }
    }

    pub fn same_as(self, other: Self) -> bool {
        match (self, other) {
            (
                Self::At {
                    season,
                    episode,
                    stopped_at,
                },
                Self::At {
                    season: other_season,
                    episode: other_episode,
                    stopped_at: other_stopped_at,
                },
            ) => {
                (season, episode) == (other_season, other_episode)
                    && (stopped_at.unwrap_or(0.0) - other_stopped_at.unwrap_or(0.0)).abs() < SAME_PLACE
            }
            _ => self == other,
        }
    }
}

/// Moves `mine` to where `theirs` carries on in `show`: the same episode at
/// the same time, with my own audio and subtitle choice. False when they
/// haven't started it.
pub fn catch_up(mine: &mut Progress, theirs: &Progress, show: &Show, now: u64) -> bool {
    let Some(position) = theirs.shows.get(&show.key).copied() else {
        return false;
    };
    mine.shows.insert(show.key.clone(), position);
    if let Some(episode) = theirs.resume_episode(show) {
        let their_file = theirs.files.get(&episode.path).copied().unwrap_or_default();
        let my_file = mine.files.entry(episode.path.clone()).or_default();
        match their_file.viewing {
            Viewing::InProgress { at, .. } if their_file.resume_at() > 0.0 => {
                my_file.viewing = Viewing::InProgress {
                    at,
                    tracks: my_file.tracks(),
                };
            }
            _ if my_file.stopped_at().is_some() => my_file.viewing = Viewing::NotStarted,
            _ => {}
        }
        my_file.duration = my_file.duration.or(their_file.duration);
        my_file.credits = my_file.credits.or(their_file.credits);
    }
    mine.watching.played(Kind::Series, &show.key, now);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::{Place, Position, Tracks};
    use crate::watching;

    fn show() -> Show {
        crate::import::library_from_paths(&[
            "Series/Dark/Dark.S01E01.mkv",
            "Series/Dark/Dark.S01E02.mkv",
            "Series/Dark/Dark.S02E01.mkv",
        ])
        .shows[0]
            .clone()
    }

    fn at(season: u32, episode: u32, finished: bool) -> Position {
        Position {
            season,
            episode,
            finished,
        }
    }

    fn stopped(progress: &mut Progress, show: &Show, season: u32, episode: u32, seconds: f64) {
        let file = &show.episode(season, episode).unwrap().path;
        progress.shows.insert(show.key.clone(), at(season, episode, false));
        let place = Place {
            position: seconds,
            duration: 3000.0,
            credits: None,
        };
        progress.record(file, place);
    }

    #[test]
    fn a_spot_is_the_episode_someone_carries_on_with_and_how_far_in() {
        let show = show();
        let mut progress = Progress::default();
        assert_eq!(Spot::of(&progress, &show), Spot::NotStarted);
        let (season, episode) = (show.seasons[0].number, show.seasons[0].episodes[0].number);
        stopped(&mut progress, &show, season, episode, 600.0);
        assert_eq!(
            Spot::of(&progress, &show),
            Spot::At {
                season,
                episode,
                stopped_at: Some(600.0)
            }
        );
        let last = show.episodes().last().unwrap();
        let last = (last.season, last.number);
        progress.shows.insert(show.key.clone(), at(last.0, last.1, true));
        assert_eq!(Spot::of(&progress, &show), Spot::Finished);
    }

    #[test]
    fn two_spots_in_one_episode_less_than_a_minute_apart_are_the_same() {
        let here = |stopped_at| Spot::At {
            season: 1,
            episode: 2,
            stopped_at,
        };
        assert!(here(Some(600.0)).same_as(here(Some(640.0))));
        assert!(!here(Some(600.0)).same_as(here(Some(700.0))));
        assert!(here(None).same_as(here(Some(20.0))));
        assert!(!here(None).same_as(Spot::NotStarted));
        assert!(Spot::Finished.same_as(Spot::Finished));
    }

    #[test]
    fn catching_up_takes_their_episode_and_time_but_keeps_my_tracks_and_lands_in_my_started() {
        let show = show();
        let episodes: Vec<(u32, u32)> = show
            .episodes()
            .map(|episode| (episode.season, episode.number))
            .collect();
        let (first, second) = (episodes[0], episodes[1]);
        let mut theirs = Progress::default();
        stopped(&mut theirs, &show, second.0, second.1, 1200.0);
        let mut mine = Progress::default();
        stopped(&mut mine, &show, first.0, first.1, 300.0);
        let second_file = show.episode(second.0, second.1).unwrap().path.clone();
        mine.files.entry(second_file.clone()).or_default().viewing = Viewing::InProgress {
            at: 30.0,
            tracks: Tracks {
                audio: Some(2),
                subtitle: None,
                subtitles_shown: Some(false),
            },
        };
        assert!(catch_up(&mut mine, &theirs, &show, 77));
        assert_eq!(mine.shows[&show.key], at(second.0, second.1, false));
        assert_eq!(
            mine.files[&second_file].viewing,
            Viewing::InProgress {
                at: 1200.0,
                tracks: Tracks {
                    audio: Some(2),
                    subtitle: None,
                    subtitles_shown: Some(false),
                },
            }
        );
        assert_eq!(mine.files[&second_file].duration, Some(3000.0));
        assert!(Spot::of(&mine, &show).same_as(Spot::of(&theirs, &show)));
        assert_eq!(mine.watching.last_played(watching::Kind::Series, &show.key), Some(77));
        assert!(mine.watched(&show, show.episode(first.0, first.1).unwrap()));
    }

    #[test]
    fn catching_up_to_someone_between_episodes_starts_my_next_one_from_its_beginning() {
        let show = show();
        let episodes: Vec<(u32, u32)> = show
            .episodes()
            .map(|episode| (episode.season, episode.number))
            .collect();
        let (first, second) = (episodes[0], episodes[1]);
        let mut theirs = Progress::default();
        theirs.shows.insert(show.key.clone(), at(first.0, first.1, true));
        let mut mine = Progress::default();
        stopped(&mut mine, &show, second.0, second.1, 900.0);
        assert!(catch_up(&mut mine, &theirs, &show, 5));
        let second_file = &show.episode(second.0, second.1).unwrap().path;
        assert_eq!(mine.files[second_file].viewing, Viewing::NotStarted);
        assert_eq!(
            Spot::of(&mine, &show),
            Spot::At {
                season: second.0,
                episode: second.1,
                stopped_at: None
            }
        );
    }

    #[test]
    fn nobody_catches_up_to_someone_who_has_not_started() {
        let show = show();
        let mut mine = Progress::default();
        stopped(&mut mine, &show, 1, 2, 100.0);
        let before = mine.clone();
        assert!(!catch_up(&mut mine, &Progress::default(), &show, 5));
        assert_eq!(mine, before);
    }
}
