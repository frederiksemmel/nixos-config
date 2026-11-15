use crate::command::Command;
use crate::library::Library;
use crate::profile::ProfileId;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const COUNTDOWN: Duration = Duration::from_secs(10);

/// The episode that plays by itself once the countdown runs out, after the
/// one before it ended on the TV.
#[derive(Clone, Debug, PartialEq)]
pub struct UpNext {
    pub file: PathBuf,
    pub owner: ProfileId,
    pub card: Card,
}

/// What the TV draws while it counts down.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub show: String,
    pub episode: String,
    pub tmdb: Option<u64>,
    pub season: u32,
    pub number: u32,
    pub starts: Instant,
    pub ends: Instant,
}

impl Card {
    pub fn left(&self, now: Instant) -> Duration {
        self.ends.saturating_duration_since(now)
    }

    pub fn seconds_left(&self, now: Instant) -> u64 {
        let left = self.left(now);
        left.as_secs() + u64::from(left.subsec_nanos() > 0)
    }

    pub fn share_left(&self, now: Instant) -> f32 {
        let total = self.ends.saturating_duration_since(self.starts);
        if total.is_zero() {
            return 0.0;
        }
        self.left(now).as_secs_f32() / total.as_secs_f32()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    PlayNow,
    Cancel,
    Nothing,
}

impl UpNext {
    /// The next regular episode of the show whose episode `ended`, across
    /// seasons, or nothing after a special or the last episode in the library.
    pub fn after(library: &Library, ended: &Path, owner: ProfileId, now: Instant) -> Option<Self> {
        let (show, episode) = library.find_episode(ended)?;
        if episode.season == 0 {
            return None;
        }
        let next = show.episode_after(episode.season, episode.number)?;
        Some(Self {
            file: next.path.clone(),
            owner,
            card: Card {
                show: show.title.clone(),
                episode: format!("{} · {}", next.short_label(), next.label()),
                tmdb: show.tmdb,
                season: next.season,
                number: next.number,
                starts: now,
                ends: now + COUNTDOWN,
            },
        })
    }

    pub fn press(command: Command) -> Press {
        match command {
            Command::Select | Command::PlayPause => Press::PlayNow,
            Command::Back => Press::Cancel,
            _ => Press::Nothing,
        }
    }

    pub fn due(&self, now: Instant) -> bool {
        now >= self.card.ends
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{Episode, Season, Show};

    fn episode(season: u32, number: u32, title: &str) -> Episode {
        Episode {
            season,
            number,
            title: Some(title.to_string()),
            path: PathBuf::from(format!("/lib/Series/Dark/S{season:02}E{number:02}.mkv")),
            ..Episode::default()
        }
    }

    fn library() -> Library {
        Library {
            shows: vec![Show {
                key: "dark".to_string(),
                title: "Dark".to_string(),
                tmdb: Some(70523),
                seasons: vec![
                    Season {
                        number: 0,
                        episodes: vec![episode(0, 1, "Inside Dark")],
                    },
                    Season {
                        number: 1,
                        episodes: vec![episode(1, 1, "Secrets"), episode(1, 2, "Lies")],
                    },
                    Season {
                        number: 2,
                        episodes: vec![episode(2, 1, "Beginnings and Endings")],
                    },
                ],
                ..Show::default()
            }],
            ..Library::default()
        }
    }

    fn path(season: u32, number: u32) -> PathBuf {
        PathBuf::from(format!("/lib/Series/Dark/S{season:02}E{number:02}.mkv"))
    }

    #[test]
    fn the_next_episode_of_the_season_is_up_next_with_its_card() {
        let now = Instant::now();
        let up = UpNext::after(&library(), &path(1, 1), ProfileId(2), now).unwrap();
        assert_eq!((up.file, up.owner), (path(1, 2), ProfileId(2)));
        assert_eq!(up.card.show, "Dark");
        assert_eq!(up.card.episode, "S1 E2 · 2. Lies");
        assert_eq!((up.card.tmdb, up.card.season, up.card.number), (Some(70523), 1, 2));
        assert_eq!(up.card.ends, now + COUNTDOWN);
    }

    #[test]
    fn a_seasons_last_episode_goes_on_to_the_first_of_the_next_season() {
        let up = UpNext::after(&library(), &path(1, 2), ProfileId::FIRST, Instant::now()).unwrap();
        assert_eq!(up.file, path(2, 1));
        assert_eq!(up.card.episode, "S2 E1 · 1. Beginnings and Endings");
    }

    #[test]
    fn nothing_is_up_next_after_the_last_episode_a_special_or_a_film() {
        let library = library();
        let now = Instant::now();
        assert_eq!(UpNext::after(&library, &path(2, 1), ProfileId::FIRST, now), None);
        assert_eq!(UpNext::after(&library, &path(0, 1), ProfileId::FIRST, now), None);
        assert_eq!(
            UpNext::after(&library, Path::new("/lib/Movies/Up.mkv"), ProfileId::FIRST, now),
            None
        );
    }

    #[test]
    fn ok_plays_at_once_back_cancels_and_other_keys_do_nothing() {
        assert_eq!(UpNext::press(Command::Select), Press::PlayNow);
        assert_eq!(UpNext::press(Command::PlayPause), Press::PlayNow);
        assert_eq!(UpNext::press(Command::Back), Press::Cancel);
        for command in [Command::Up, Command::Down, Command::Left, Command::Right] {
            assert_eq!(UpNext::press(command), Press::Nothing);
        }
    }

    #[test]
    fn the_countdown_runs_out_after_ten_seconds_and_counts_whole_seconds_up() {
        let now = Instant::now();
        let up = UpNext::after(&library(), &path(1, 1), ProfileId::FIRST, now).unwrap();
        assert!(!up.due(now + COUNTDOWN - Duration::from_millis(1)));
        assert!(up.due(now + COUNTDOWN));
        assert_eq!(up.card.seconds_left(now), 10);
        assert_eq!(up.card.seconds_left(now + Duration::from_millis(1)), 10);
        assert_eq!(up.card.seconds_left(now + Duration::from_millis(9001)), 1);
        assert_eq!(up.card.seconds_left(now + COUNTDOWN + Duration::from_secs(1)), 0);
        assert!((up.card.share_left(now + COUNTDOWN / 2) - 0.5).abs() < 1e-6);
    }
}
