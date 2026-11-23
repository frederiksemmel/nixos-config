use crate::core::Core;
use crate::details;
use crate::library::Show;
use crate::profile::ProfileId;
use crate::progress::{Progress, Viewing, Watched};
use crate::watching::Kind;
use serde::Serialize;
use std::path::Path;

const SAME_PLACE: f64 = 60.0;
const STOP_AHEAD: f64 = 10.0;
const PLAYED_STEP: f64 = 3.0;

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

    pub fn line(self, with_me: bool) -> String {
        match self {
            Self::NotStarted => "Hasn't started yet".to_string(),
            Self::Finished => "Finished the series".to_string(),
            Self::At { .. } if with_me => "At your place".to_string(),
            Self::At {
                season,
                episode,
                stopped_at,
            } => {
                let label = match season {
                    0 => format!("Special {episode}"),
                    season => format!("S{season} E{episode}"),
                };
                match stopped_at {
                    Some(at) => format!("At {label} · {}", details::time_in(at)),
                    None => format!("Up next {label}"),
                }
            }
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

/// A place in the episode someone plays where others in their
/// watch-together group stopped, and what playback says when it pauses there.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Stop {
    pub at: f64,
    pub said: String,
}

/// Where `partners` (name and progress) stopped in `season` `episode` of
/// `show` further on than `from`, earliest first. Partners less than a
/// minute apart share one stop, at the earliest of them.
pub fn stops(show: &Show, season: u32, episode: u32, from: f64, partners: &[(String, &Progress)]) -> Vec<Stop> {
    let mut places: Vec<(f64, &str)> = partners
        .iter()
        .filter_map(|(name, progress)| match Spot::of(progress, show) {
            Spot::At {
                season: their_season,
                episode: their_episode,
                stopped_at: Some(at),
            } if (their_season, their_episode) == (season, episode) && at > from + STOP_AHEAD => {
                Some((at, name.as_str()))
            }
            _ => None,
        })
        .collect();
    places.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut stops: Vec<(f64, Vec<&str>)> = Vec::new();
    for (at, name) in places {
        match stops.last_mut() {
            Some((first, names)) if at - *first < SAME_PLACE => names.push(name),
            _ => stops.push((at, vec![name])),
        }
    }
    stops
        .into_iter()
        .map(|(at, names)| {
            let names = match names.split_last() {
                Some((last, [])) => (*last).to_string(),
                Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
                None => String::new(),
            };
            Stop {
                at,
                said: format!("Paused where {names} stopped"),
            }
        })
        .collect()
}

/// The stops ahead of `from` in `file` for `profile`, when it is an episode
/// of a show they watch together with others.
pub async fn stops_for(core: &Core, profile: ProfileId, file: &Path, from: f64) -> Vec<Stop> {
    let library = core.library();
    let Some((show, episode)) = library.find_episode(file) else {
        return Vec::new();
    };
    let partners = core.db().partners(profile, &show.key).await.unwrap_or_else(|error| {
        eprintln!("together: partners of {} in {}: {error}", profile.0, show.key);
        Vec::new()
    });
    if partners.is_empty() {
        return Vec::new();
    }
    let profiles = core.db().profiles().await.unwrap_or_else(|error| {
        eprintln!("together: profiles: {error}");
        Vec::new()
    });
    let mut theirs = Vec::new();
    for found in profiles.iter().filter(|found| partners.contains(&found.id)) {
        theirs.push((found.name.clone(), core.progress(found.id).await));
    }
    let theirs: Vec<(String, &Progress)> = theirs
        .iter()
        .map(|(name, progress)| (name.clone(), &**progress))
        .collect();
    stops(show, episode.season, episode.number, from, &theirs)
}

/// The stop that playing on from `from` to `to` passed, taken out of
/// `stops` so it pauses once. A step longer than a few seconds is a seek,
/// which passes none.
pub fn reached(stops: &mut Vec<Stop>, from: f64, to: f64) -> Option<Stop> {
    if !(to > from && to - from <= PLAYED_STEP) {
        return None;
    }
    let index = stops.iter().position(|stop| from < stop.at && stop.at <= to)?;
    Some(stops.remove(index))
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
    fn a_spot_reads_as_the_episode_and_how_far_in() {
        let at = |season, stopped_at| Spot::At {
            season,
            episode: 4,
            stopped_at,
        };
        assert_eq!(at(2, Some(750.0)).line(false), "At S2 E4 · 12m in");
        assert_eq!(at(0, Some(4000.0)).line(false), "At Special 4 · 1h 06m in");
        assert_eq!(at(2, None).line(false), "Up next S2 E4");
        assert_eq!(at(2, Some(750.0)).line(true), "At your place");
        assert_eq!(Spot::NotStarted.line(false), "Hasn't started yet");
        assert_eq!(Spot::Finished.line(false), "Finished the series");
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

    #[test]
    fn a_stop_is_where_a_partner_stopped_further_on_in_the_episode_i_play() {
        let show = show();
        let (mut mami, mut michelle, mut ramu, mut miriam) = (
            Progress::default(),
            Progress::default(),
            Progress::default(),
            Progress::default(),
        );
        stopped(&mut mami, &show, 1, 2, 1200.0);
        stopped(&mut michelle, &show, 1, 2, 1230.0);
        stopped(&mut ramu, &show, 1, 2, 2000.0);
        stopped(&mut miriam, &show, 2, 1, 600.0);
        let partners = [
            ("Ramu".to_string(), &ramu),
            ("Mami".to_string(), &mami),
            ("Michelle".to_string(), &michelle),
            ("Miriam".to_string(), &miriam),
        ];
        let ramus = Stop {
            at: 2000.0,
            said: "Paused where Ramu stopped".to_string(),
        };
        assert_eq!(
            stops(&show, 1, 2, 300.0, &partners),
            [
                Stop {
                    at: 1200.0,
                    said: "Paused where Mami and Michelle stopped".to_string()
                },
                ramus.clone(),
            ]
        );
        assert_eq!(stops(&show, 1, 2, 1225.0, &partners), [ramus]);
        assert_eq!(stops(&show, 1, 1, 0.0, &partners), []);
    }

    #[test]
    fn playing_on_past_a_stop_reaches_it_once_and_a_seek_past_it_does_not() {
        let stop = |at| Stop {
            at,
            said: format!("at {at}"),
        };
        let mut stops = vec![stop(600.0), stop(900.0)];
        assert_eq!(reached(&mut stops, 599.0, 599.5), None);
        assert_eq!(reached(&mut stops, 599.5, 600.1), Some(stop(600.0)));
        assert_eq!(reached(&mut stops, 599.5, 600.1), None);
        assert_eq!(reached(&mut stops, 700.0, 960.0), None);
        assert_eq!(reached(&mut stops, 960.0, 880.0), None);
        assert_eq!(reached(&mut stops, 899.6, 900.0), Some(stop(900.0)));
        assert!(stops.is_empty());
    }
}
