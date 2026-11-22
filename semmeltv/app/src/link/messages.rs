//! What the core and the tv say to each other, one JSON value per line.

use crate::bus::{Panel, Together};
use crate::command::Command;
use crate::library::Library;
use crate::player::{Finish, State};
use crate::profile::ProfileId;
use crate::progress::{Place, Progress, Tracks};
use crate::segments::Segment;
use crate::sidecar::Sidecar;
use crate::subtitles::{self, Remembered, Settings};
use crate::up_next::Card;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Raised whenever a message changes shape, so a core and a tv from
/// different builds refuse each other instead of misreading.
pub const PROTOCOL: u32 = 3;

/// The core's orders and news for the tv.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToTv {
    Welcome {
        protocol: u32,
        revision: String,
    },
    Library {
        revision: u64,
        library: Library,
    },
    Home(Home),
    Key(Command),
    Message(String),
    Panel(Panel),
    Together(Together),
    UpNext(Option<UpNextCard>),
    Play(Box<Play>),
    Command {
        play: String,
        command: Command,
    },
    Pause {
        play: String,
        paused: bool,
    },
    Mute {
        play: String,
        muted: bool,
    },
    Speed {
        play: String,
        speed: f64,
    },
    Seek {
        play: String,
        target: Target,
    },
    SkipSegment {
        play: String,
    },
    HideSkip {
        play: String,
    },
    AudioDevice {
        device: String,
        said: String,
    },
    Dialogue {
        on: bool,
        said: String,
    },
    Toast {
        heading: String,
        status: String,
        hold_ms: u64,
    },
    Segments {
        play: String,
        segments: Vec<Segment>,
        credits: Option<f64>,
    },
    Subtitles {
        id: u64,
        play: String,
        change: subtitles::Change,
    },
    Stop {
        play: String,
    },
}

impl ToTv {
    /// The order's kind, for logs.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Welcome { .. } => "welcome",
            Self::Library { .. } => "library",
            Self::Home(_) => "home",
            Self::Key(_) => "key",
            Self::Message(_) => "message",
            Self::Panel(_) => "panel",
            Self::Together(_) => "together",
            Self::UpNext(_) => "up next",
            Self::Play(_) => "play",
            Self::Command { .. } => "command",
            Self::Pause { .. } => "pause",
            Self::Mute { .. } => "mute",
            Self::Speed { .. } => "speed",
            Self::Seek { .. } => "seek",
            Self::SkipSegment { .. } => "skip segment",
            Self::HideSkip { .. } => "hide skip",
            Self::AudioDevice { .. } => "audio device",
            Self::Dialogue { .. } => "dialogue",
            Self::Toast { .. } => "toast",
            Self::Segments { .. } => "segments",
            Self::Subtitles { .. } => "subtitles",
            Self::Stop { .. } => "stop",
        }
    }
}

/// What the tv tells the core.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FromTv {
    Hello {
        protocol: u32,
        revision: String,
        playing: Option<Running>,
        ended: Option<Ended>,
    },
    Report(Box<Report>),
    Started {
        play: String,
    },
    Failed {
        play: String,
        error: String,
    },
    Finished(Ended),
    Key(Command),
    Ask(UiRequest),
    Answered {
        id: u64,
        message: String,
        ok: bool,
    },
    Remember {
        file: PathBuf,
        remembered: Remembered,
    },
}

/// The menu's home screen without the library, which travels on its own in
/// `ToTv::Library` and only when its revision changes.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Home {
    pub library_revision: u64,
    pub profile: ProfileId,
    pub people: Vec<(ProfileId, String)>,
    pub progress: Progress,
    pub offline: bool,
    pub downloaded: HashSet<PathBuf>,
    pub downloading: Option<PathBuf>,
    pub problem: Option<String>,
}

/// Everything the tv needs to start a video. `play` names this playback in
/// every later message about it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Play {
    pub play: String,
    pub owner: ProfileId,
    pub file: PathBuf,
    pub source: PathBuf,
    pub start: f64,
    pub paused: bool,
    pub tracks: Tracks,
    pub caption: String,
    pub subtitles: PlaySubtitles,
    pub audio_device: String,
    pub dialogue: bool,
    pub next: bool,
}

/// The owner's subtitle settings and what is remembered about this file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaySubtitles {
    pub settings: Settings,
    pub remembered: Remembered,
    pub sidecars: Vec<Sidecar>,
}

/// Where a seek goes: to a second of the video or by some seconds from here.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    To(f64),
    By(f64),
}

/// How a playback stands, sent every 500 ms while it runs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub play: Option<String>,
    pub state: State,
    pub place: Place,
    pub tracks: Tracks,
    pub played: f64,
    pub subtitles: Option<subtitles::Playing>,
}

impl Report {
    /// What the core assumes about `play` until its first report arrives.
    pub fn before(play: &Play) -> Self {
        Self {
            play: Some(play.play.clone()),
            state: State {
                position: play.start,
                paused: play.paused,
                speed: 1.0,
                ..State::default()
            },
            place: Place {
                position: play.start,
                ..Place::default()
            },
            tracks: play.tracks,
            ..Self::default()
        }
    }
}

/// A playback the tv kept running while the core was away.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Running {
    pub play: String,
    pub owner: ProfileId,
    pub file: PathBuf,
    pub position: f64,
    pub paused: bool,
}

/// How a playback ended, with its last place, so the core needs nothing more
/// from the player to record it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ended {
    pub play: String,
    pub finish: Finish,
    pub place: Place,
    pub tracks: Tracks,
    pub played: f64,
}

/// What the menu asks the core to do.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiRequest {
    PlayFile(PathBuf),
    WatchWithOnTv { show: String, with: ProfileId },
    TvProfile(ProfileId),
    TogetherOnTv(String),
}

/// The up-next card with its countdown as milliseconds, since an `Instant`
/// means nothing in another process.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UpNextCard {
    pub show: String,
    pub episode: String,
    pub tmdb: Option<u64>,
    pub season: u32,
    pub number: u32,
    pub remaining_ms: u64,
    pub total_ms: u64,
}

impl UpNextCard {
    pub fn of(card: &Card, now: Instant) -> Self {
        Self {
            show: card.show.clone(),
            episode: card.episode.clone(),
            tmdb: card.tmdb,
            season: card.season,
            number: card.number,
            remaining_ms: millis(card.left(now)),
            total_ms: millis(card.ends.saturating_duration_since(card.starts)),
        }
    }

    pub fn card(&self, now: Instant) -> Card {
        let ends = now + Duration::from_millis(self.remaining_ms);
        let total = Duration::from_millis(self.total_ms.max(self.remaining_ms));
        Card {
            show: self.show.clone(),
            episode: self.episode.clone(),
            tmdb: self.tmdb,
            season: self.season,
            number: self.number,
            starts: ends.checked_sub(total).unwrap_or(now),
            ends,
        }
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Mate;
    use crate::library::{Episode, Movie, Season, Show};
    use crate::manifest::{About, Date, Scores, Tenths};
    use crate::segments::Kind;
    use crate::tags::Tags;
    use std::fmt::Debug;

    fn crosses<T: Serialize + for<'de> Deserialize<'de> + PartialEq + Debug>(message: &T) {
        let line = serde_json::to_string(message).unwrap();
        assert!(!line.contains('\n'), "one line: {line}");
        let back: T = serde_json::from_str(&line).unwrap();
        assert_eq!(&back, message, "{line}");
    }

    fn play() -> Play {
        Play {
            play: "a-play".to_string(),
            owner: ProfileId(2),
            file: PathBuf::from("/lib/Dark/S01E02.mkv"),
            source: PathBuf::from("/cache/Dark/S01E02.mkv"),
            start: 612.5,
            paused: false,
            tracks: Tracks {
                audio: Some(2),
                subtitle: Some(3),
                subtitles_shown: Some(true),
            },
            caption: "Dark · S1 E2".to_string(),
            subtitles: PlaySubtitles {
                settings: Settings::default(),
                remembered: Remembered {
                    delay: -0.5,
                    embedded: Some(4),
                },
                sidecars: vec![Sidecar {
                    path: PathBuf::from("/lib/Dark/S01E02.de.srt"),
                    language: Some("de".to_string()),
                    title: "German".to_string(),
                    forced: false,
                    hearing_impaired: true,
                }],
            },
            audio_device: "alsa/plughw:CARD=vc4hdmi0".to_string(),
            dialogue: true,
            next: true,
        }
    }

    fn report() -> Report {
        Report {
            play: Some("a-play".to_string()),
            state: State {
                position: 640.25,
                duration: 2730.0,
                paused: false,
                muted: true,
                speed: 1.25,
                subtitle: Some(3),
                subtitle_delay: -0.5,
                audio: Some(2),
                started: true,
                jump: Some(30),
                skip: Some(Kind::Intro),
            },
            place: Place {
                position: 640.25,
                duration: 2730.0,
                credits: Some(2650.0),
            },
            tracks: Tracks::default(),
            played: 27.75,
            subtitles: Some(subtitles::Playing::default()),
        }
    }

    fn ended() -> Ended {
        Ended {
            play: "a-play".to_string(),
            finish: Finish::Next,
            place: report().place,
            tracks: report().tracks,
            played: 1200.0,
        }
    }

    #[test]
    fn every_order_for_the_tv_survives_the_trip_as_one_line() {
        let orders = [
            ToTv::Welcome {
                protocol: PROTOCOL,
                revision: "abc123".to_string(),
            },
            ToTv::Library {
                revision: 7,
                library: library(3, 4),
            },
            ToTv::Home(Home {
                library_revision: 7,
                profile: ProfileId(2),
                people: vec![(ProfileId(1), "Frederik".to_string())],
                progress: Progress::default(),
                offline: true,
                downloaded: HashSet::from([PathBuf::from("/lib/Up.mkv")]),
                downloading: Some(PathBuf::from("/lib/Dark/S01E03.mkv")),
                problem: Some("pCloud is unreachable".to_string()),
            }),
            ToTv::Key(Command::Down),
            ToTv::Message("Dialogue mode on".to_string()),
            ToTv::Panel(Panel {
                heading: "Dark season 2".to_string(),
                status: "Confirm on the phone's Downloads page.".to_string(),
                hold: Duration::from_secs(60),
            }),
            ToTv::Together(Together {
                show: "dark".to_string(),
                people: vec![Mate {
                    id: ProfileId(2),
                    name: "Mami".to_string(),
                    line: "Up next S1 E2".to_string(),
                    paired: true,
                }],
            }),
            ToTv::UpNext(Some(UpNextCard {
                show: "Dark".to_string(),
                episode: "S1 E3 · Past and Present".to_string(),
                tmdb: Some(70523),
                season: 1,
                number: 3,
                remaining_ms: 7250,
                total_ms: 10_000,
            })),
            ToTv::UpNext(None),
            ToTv::Play(Box::new(play())),
            ToTv::Command {
                play: "a-play".to_string(),
                command: Command::Left,
            },
            ToTv::Pause {
                play: "a-play".to_string(),
                paused: true,
            },
            ToTv::Mute {
                play: "a-play".to_string(),
                muted: true,
            },
            ToTv::Speed {
                play: "a-play".to_string(),
                speed: 1.25,
            },
            ToTv::Seek {
                play: "a-play".to_string(),
                target: Target::To(90.0),
            },
            ToTv::Seek {
                play: "a-play".to_string(),
                target: Target::By(-10.0),
            },
            ToTv::SkipSegment {
                play: "a-play".to_string(),
            },
            ToTv::HideSkip {
                play: "a-play".to_string(),
            },
            ToTv::AudioDevice {
                device: "bluealsa".to_string(),
                said: "Sound on the headphones".to_string(),
            },
            ToTv::Dialogue {
                on: true,
                said: "Dialogue mode on".to_string(),
            },
            ToTv::Toast {
                heading: String::new(),
                status: "Speed 1.25×".to_string(),
                hold_ms: 3000,
            },
            ToTv::Segments {
                play: "a-play".to_string(),
                segments: vec![Segment {
                    kind: Kind::Intro,
                    start: 30.0,
                    end: 95.5,
                }],
                credits: Some(2650.0),
            },
            ToTv::Subtitles {
                id: 9,
                play: "a-play".to_string(),
                change: subtitles::Change::Add {
                    video: PathBuf::from("/lib/Up.mkv"),
                    sidecar: play().subtitles.sidecars[0].clone(),
                },
            },
            ToTv::Subtitles {
                id: 10,
                play: "a-play".to_string(),
                change: subtitles::Change::Track(None),
            },
            ToTv::Stop {
                play: "a-play".to_string(),
            },
        ];
        for order in &orders {
            crosses(order);
        }
    }

    #[test]
    fn everything_the_tv_says_survives_the_trip_as_one_line() {
        let said = [
            FromTv::Hello {
                protocol: PROTOCOL,
                revision: "abc123".to_string(),
                playing: Some(Running {
                    play: "a-play".to_string(),
                    owner: ProfileId(2),
                    file: PathBuf::from("/lib/Up.mkv"),
                    position: 1234.5,
                    paused: true,
                }),
                ended: Some(ended()),
            },
            FromTv::Hello {
                protocol: PROTOCOL,
                revision: "abc123".to_string(),
                playing: None,
                ended: None,
            },
            FromTv::Report(Box::new(report())),
            FromTv::Report(Box::default()),
            FromTv::Started {
                play: "a-play".to_string(),
            },
            FromTv::Failed {
                play: "a-play".to_string(),
                error: "No such file or directory (os error 2)".to_string(),
            },
            FromTv::Finished(ended()),
            FromTv::Key(Command::Back),
            FromTv::Ask(UiRequest::PlayFile(PathBuf::from("/lib/Up.mkv"))),
            FromTv::Ask(UiRequest::WatchWithOnTv {
                show: "dark".to_string(),
                with: ProfileId(2),
            }),
            FromTv::Ask(UiRequest::TvProfile(ProfileId(3))),
            FromTv::Ask(UiRequest::TogetherOnTv("dark".to_string())),
            FromTv::Answered {
                id: 9,
                message: "Subtitles off".to_string(),
                ok: true,
            },
            FromTv::Remember {
                file: PathBuf::from("/lib/Up.mkv"),
                remembered: Remembered {
                    delay: 1.5,
                    embedded: None,
                },
            },
        ];
        for message in &said {
            crosses(message);
        }
    }

    #[test]
    fn the_up_next_countdown_crosses_as_time_left() {
        let now = Instant::now();
        let card = Card {
            show: "Dark".to_string(),
            episode: "S1 E3".to_string(),
            tmdb: None,
            season: 1,
            number: 3,
            starts: now - Duration::from_secs(4),
            ends: now + Duration::from_secs(6),
        };
        let sent = UpNextCard::of(&card, now);
        assert_eq!((sent.remaining_ms, sent.total_ms), (6000, 10_000));
        let later = now + Duration::from_secs(1);
        let drawn = sent.card(later);
        assert_eq!(drawn.seconds_left(later), 6);
        assert!((drawn.share_left(later) - 0.6).abs() < 1e-3);
        let over = UpNextCard::of(&card, now + Duration::from_secs(60));
        assert_eq!(over.remaining_ms, 0);
        assert_eq!(over.card(later).seconds_left(later), 0);
    }

    #[test]
    fn a_library_of_five_thousand_episodes_crosses_in_under_sixteen_megabytes() {
        let library = library(100, 50);
        let episodes: usize = library
            .shows
            .iter()
            .flat_map(|show| &show.seasons)
            .map(|season| season.episodes.len())
            .sum();
        assert_eq!(episodes, 5000);
        let order = ToTv::Library { revision: 1, library };
        let line = serde_json::to_string(&order).unwrap();
        assert!(line.len() < 16 << 20, "{} bytes", line.len());
        let back: ToTv = serde_json::from_str(&line).unwrap();
        assert_eq!(back, order);
    }

    fn library(shows: usize, episodes: usize) -> Library {
        let scores = Scores {
            imdb: Tenths::from_float(8.7),
            rt: Some(94),
            popcorn: Some(91),
            metacritic: Some(77),
            letterboxd: Tenths::from_float(4.1),
            ebert: None,
        };
        let about = About {
            tagline: Some("Everything is connected".to_string()),
            plot: Some(
                "A missing child sets four families on a frantic hunt for answers as they unearth a \
                 mind-bending mystery that spans three generations."
                    .to_string(),
            ),
            genres: vec!["Drama".to_string(), "Mystery".to_string()],
            released: Date::parse("2017-12-01"),
            ..About::default()
        };
        let tags = Tags {
            video: Some("HEVC".to_string()),
            resolution: Some("1080p".to_string()),
            audio: Some("DDP 5.1".to_string()),
        };
        Library {
            shows: (0..shows)
                .map(|show| Show {
                    id: show as i64,
                    key: format!("show-{show}"),
                    title: format!("Show number {show}"),
                    year: Some(2017),
                    last_aired: Some(2020),
                    running: false,
                    tmdb: Some(70_000 + show as u64),
                    imdb: Some(format!("tt{:07}", show)),
                    ratings: scores,
                    seasons: vec![Season {
                        number: 1,
                        episodes: (1..=episodes as u32)
                            .map(|number| Episode {
                                id: (show * 1000) as i64 + i64::from(number),
                                season: 1,
                                number,
                                title: Some(format!("The episode called {number}")),
                                path: PathBuf::from(format!(
                                    "/srv/semmeltv/library/Series/Show number {show} (2017)/Season 01/\
                                     Show.number.{show}.S01E{number:02}.1080p.WEB.h265-GROUP.mkv"
                                )),
                                added: Date::parse("2024-05-01").unwrap(),
                                runtime: Some(52),
                                ratings: scores,
                                tags: tags.clone(),
                                downloaded_by: Some("Frederik".to_string()),
                            })
                            .collect(),
                    }],
                    about: about.clone(),
                })
                .collect(),
            movies: vec![Movie {
                id: 1,
                key: "up-2009".to_string(),
                title: "Up".to_string(),
                year: Some(2009),
                path: PathBuf::from("/lib/Up.mkv"),
                ratings: scores,
                about,
                ..Movie::default()
            }],
        }
    }
}
