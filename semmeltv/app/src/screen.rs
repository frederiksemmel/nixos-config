use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

pub const MOVING_FOR: Duration = Duration::from_secs(20);
pub const STARTING_FOR: Duration = Duration::from_secs(30);
pub static LOST_AFTER: LazyLock<Duration> =
    LazyLock::new(|| crate::timing::seconds("SEMMELTV_QUIET_SECONDS", Duration::from_secs(45)));

/// Which run of the app this is: a phone that sees it change knows the app
/// restarted, rather than someone moving or stopping what it played.
static BOOT: LazyLock<u64> = LazyLock::new(|| {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
});

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Title {
    Movie { movie: String },
    Episode { show: String, season: u32, episode: u32 },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Place {
    #[serde(flatten)]
    pub title: Title,
    pub name: String,
    pub caption: String,
    #[serde(skip)]
    pub file: PathBuf,
    pub position: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PhonePlay {
    Starting {
        place: Place,
        since: Instant,
    },
    Playing {
        session: String,
        place: Place,
        paused: bool,
        seen: Instant,
    },
}

impl PhonePlay {
    pub fn place(&self) -> &Place {
        match self {
            Self::Starting { place, .. } | Self::Playing { place, .. } => place,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Screen {
    #[default]
    Tv,
    Moving {
        to: Device,
        place: Place,
        since: Instant,
    },
    Phone {
        device: Device,
        play: Option<PhonePlay>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Tv,
    Phone(Device),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Flip {
        to: Target,
        position: Option<f64>,
    },
    Play {
        device: Device,
        place: Place,
    },
    Resume {
        device: Device,
        place: Place,
    },
    Started {
        device: Device,
        session: String,
        position: f64,
    },
    Report {
        device: Device,
        session: String,
        place: Place,
        paused: bool,
    },
    Stop {
        device: Device,
    },
    Stopped {
        device: Device,
        session: String,
    },
    Failed {
        device: Device,
        reason: String,
    },
    TvPlays,
    Tick,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    PauseTv,
    ResumeTv,
    StopTv,
    PlayTv(Place),
    Tell(Notice),
    ShowOnTv(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Quiet,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Notice {
    pub text: String,
    pub tone: Tone,
}

fn notice(text: String, tone: Tone) -> Effect {
    Effect::Tell(Notice { text, tone })
}

fn short(session: &str) -> &str {
    session.get(..8).unwrap_or(session)
}

impl Event {
    pub fn describe(&self) -> String {
        match self {
            Self::Flip {
                to: Target::Tv,
                position,
            } => match position {
                Some(position) => format!("flip to the TV at {position:.0} s"),
                None => "flip to the TV".to_string(),
            },
            Self::Flip {
                to: Target::Phone(device),
                ..
            } => format!("flip to {}", device.name),
            Self::Play { device, place } => {
                format!("{} plays {} from {:.0} s", device.name, place.name, place.position)
            }
            Self::Resume { device, place } => {
                format!(
                    "{} resumes {} at {:.0} s after the app restarted",
                    device.name, place.name, place.position
                )
            }
            Self::Started {
                device,
                session,
                position,
            } => {
                format!("{} started stream {} at {position:.0} s", device.name, short(session))
            }
            Self::Report {
                device,
                session,
                place,
                paused,
            } => format!(
                "{} reported stream {} {} at {:.0} s",
                device.name,
                short(session),
                if *paused { "paused" } else { "playing" },
                place.position
            ),
            Self::Stop { device } => format!("{} closed its player", device.name),
            Self::Stopped { device, session } => format!("{} stopped stream {}", device.name, short(session)),
            Self::Failed { device, reason } => format!("{} failed: {reason}", device.name),
            Self::TvPlays => "the TV plays something itself".to_string(),
            Self::Tick => "time passed".to_string(),
        }
    }

    pub fn is_tick(&self) -> bool {
        matches!(self, Self::Tick)
    }
}

impl Screen {
    pub fn summary(&self) -> String {
        match self {
            Self::Tv => "tv".to_string(),
            Self::Moving { to, place, .. } => format!("moving {} to {}", place.name, to.name),
            Self::Phone { device, play: None } => format!("{}, nothing playing", device.name),
            Self::Phone {
                device,
                play: Some(PhonePlay::Starting { place, .. }),
            } => {
                format!("{}, starting {}", device.name, place.name)
            }
            Self::Phone {
                device,
                play: Some(PhonePlay::Playing {
                    session, place, paused, ..
                }),
            } => format!(
                "{}, {} {} on stream {}",
                device.name,
                if *paused { "paused" } else { "playing" },
                place.name,
                short(session)
            ),
        }
    }

    pub fn allows_tv_playing(&self) -> bool {
        !matches!(self, Self::Phone { .. })
    }

    /// Whether `device` can play here without stopping anything: the TV
    /// plays nothing and no other phone plays or is being handed to.
    pub fn free_for(&self, device: &Device, tv: Option<&Place>) -> bool {
        match self {
            Self::Tv => tv.is_none(),
            Self::Moving { .. } => false,
            Self::Phone { device: holder, play } => holder == device || play.is_none(),
        }
    }

    pub fn step(&mut self, event: Event, tv: Option<&Place>, now: Instant) -> Vec<Effect> {
        let (next, mut effects) = transition(std::mem::take(self), event, tv, now);
        *self = next;
        if !self.allows_tv_playing() && tv.is_some() && !effects.contains(&Effect::StopTv) {
            effects.push(Effect::StopTv);
        }
        effects
    }
}

fn transition(screen: Screen, event: Event, tv: Option<&Place>, now: Instant) -> (Screen, Vec<Effect>) {
    use Screen::{Moving, Phone, Tv};
    match (screen, event) {
        (
            Tv,
            Event::Flip {
                to: Target::Phone(device),
                ..
            },
        ) => match tv {
            Some(place) => (
                Moving {
                    place: place.clone(),
                    since: now,
                    to: device.clone(),
                },
                vec![Effect::PauseTv, Effect::ShowOnTv(format!("Moving to {}…", device.name))],
            ),
            None => (Phone { device, play: None }, vec![]),
        },
        (
            Moving { to, place, since },
            Event::Flip {
                to: Target::Phone(device),
                ..
            },
        ) => (
            Moving {
                since: if to == device { since } else { now },
                to: device,
                place,
            },
            vec![],
        ),
        (
            Phone { device: current, play },
            Event::Flip {
                to: Target::Phone(device),
                ..
            },
        ) => {
            if current == device {
                return (Phone { device: current, play }, vec![]);
            }
            let play = play.map(|play| PhonePlay::Starting {
                place: play.place().clone(),
                since: now,
            });
            (Phone { device, play }, vec![])
        }
        (Tv, Event::Flip { to: Target::Tv, .. }) => (Tv, vec![]),
        (Moving { .. }, Event::Flip { to: Target::Tv, .. }) => (Tv, vec![Effect::ResumeTv]),
        (
            Phone { play: Some(play), .. },
            Event::Flip {
                to: Target::Tv,
                position,
            },
        ) => {
            let mut place = play.place().clone();
            if let Some(position) = position.filter(|position| position.is_finite() && *position >= 0.0) {
                place.position = position;
            }
            (Tv, vec![Effect::PlayTv(place)])
        }
        (Phone { play: None, .. }, Event::Flip { to: Target::Tv, .. }) => (Tv, vec![]),

        (Tv, Event::Resume { device, place }) if tv.is_none() => (
            Phone {
                device,
                play: Some(PhonePlay::Starting { place, since: now }),
            },
            vec![],
        ),
        (
            Phone {
                device: current,
                play: None,
            },
            Event::Resume { device, place },
        ) if current == device => (
            Phone {
                device,
                play: Some(PhonePlay::Starting { place, since: now }),
            },
            vec![],
        ),

        (_, Event::Play { device, place }) => {
            let effects = if tv.is_some() { vec![Effect::StopTv] } else { vec![] };
            (
                Phone {
                    device,
                    play: Some(PhonePlay::Starting { place, since: now }),
                },
                effects,
            )
        }

        (
            Moving { to, mut place, .. },
            Event::Started {
                device,
                session,
                position,
            },
        ) if to == device => {
            place.position = position;
            let effects = vec![
                Effect::StopTv,
                Effect::ShowOnTv(format!("Playing on {}", device.name)),
                notice(format!("Playing on {}", device.name), Tone::Quiet),
            ];
            (
                Phone {
                    device,
                    play: Some(PhonePlay::Playing {
                        session,
                        place,
                        paused: false,
                        seen: now,
                    }),
                },
                effects,
            )
        }
        (
            Phone {
                device: current,
                play: Some(play),
            },
            Event::Started {
                device,
                session,
                position,
            },
        ) if current == device => {
            let mut place = play.place().clone();
            place.position = position;
            (
                Phone {
                    device,
                    play: Some(PhonePlay::Playing {
                        session,
                        place,
                        paused: false,
                        seen: now,
                    }),
                },
                vec![],
            )
        }
        (
            Phone {
                device: current,
                play: Some(play),
            },
            Event::Report {
                device,
                session,
                place: reported,
                paused,
            },
        ) if current == device && play.place().file == reported.file => {
            let mut place = play.place().clone();
            place.position = reported.position;
            (
                Phone {
                    device,
                    play: Some(PhonePlay::Playing {
                        session,
                        place,
                        paused,
                        seen: now,
                    }),
                },
                vec![],
            )
        }
        (
            Phone {
                device: current,
                play: None,
            },
            Event::Report {
                device,
                session,
                place,
                paused,
            },
        ) if current == device => (
            Phone {
                device,
                play: Some(PhonePlay::Playing {
                    session,
                    place,
                    paused,
                    seen: now,
                }),
            },
            vec![],
        ),
        (
            Phone {
                device: current,
                play: Some(PhonePlay::Playing { session: playing, .. }),
            },
            Event::Stopped { device, session },
        ) if current == device && playing == session => (Phone { device, play: None }, vec![]),

        (Moving { to, .. }, Event::Stop { device }) if to == device => (Tv, vec![Effect::ResumeTv]),
        (
            Phone {
                device: current,
                play: Some(_),
            },
            Event::Stop { device },
        ) if current == device => (Phone { device, play: None }, vec![]),

        (Moving { to, .. }, Event::Failed { device, reason }) if to == device => (
            Tv,
            vec![
                Effect::ResumeTv,
                notice(
                    format!(
                        "It didn't start on {}: {}. Still on the TV",
                        device.name,
                        reason.trim_end_matches('.')
                    ),
                    Tone::Error,
                ),
            ],
        ),
        (
            Phone {
                device: current,
                play: Some(_),
            },
            Event::Failed { device, reason },
        ) if current == device => (
            Phone { device, play: None },
            vec![notice(format!("It stopped on {}: {reason}", current.name), Tone::Error)],
        ),

        (_, Event::TvPlays) => (Tv, vec![]),

        (Moving { to, place, since }, Event::Tick) => {
            if now.duration_since(since) >= MOVING_FOR {
                (
                    Tv,
                    vec![
                        Effect::ResumeTv,
                        notice(
                            format!("{} didn't start the video. Still on the TV", to.name),
                            Tone::Error,
                        ),
                    ],
                )
            } else {
                (Moving { to, place, since }, vec![])
            }
        }
        (
            Phone {
                device,
                play: Some(PhonePlay::Starting { place, since }),
            },
            Event::Tick,
        ) => {
            if now.duration_since(since) >= STARTING_FOR {
                let text = format!("{} didn't start the video", device.name);
                (Phone { device, play: None }, vec![notice(text, Tone::Error)])
            } else {
                (
                    Phone {
                        device,
                        play: Some(PhonePlay::Starting { place, since }),
                    },
                    vec![],
                )
            }
        }
        (
            Phone {
                device,
                play:
                    Some(PhonePlay::Playing {
                        session,
                        place,
                        paused,
                        seen,
                    }),
            },
            Event::Tick,
        ) => {
            if now.duration_since(seen) >= *LOST_AFTER {
                let text = format!("{} went quiet, so its place is kept", device.name);
                (Phone { device, play: None }, vec![notice(text, Tone::Quiet)])
            } else {
                (
                    Phone {
                        device,
                        play: Some(PhonePlay::Playing {
                            session,
                            place,
                            paused,
                            seen,
                        }),
                    },
                    vec![],
                )
            }
        }

        (screen, _) => (screen, vec![]),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct View {
    #[serde(flatten)]
    pub on: OnScreen,
    pub boot: u64,
    pub seq: u64,
    pub notice: Option<Said>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum OnScreen {
    Tv,
    Moving { device: Device, play: MovingView },
    Phone { device: Device, play: Option<PlayView> },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename = "moving")]
pub struct MovingView {
    #[serde(flatten)]
    pub place: Place,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum PlayView {
    Starting {
        #[serde(flatten)]
        place: Place,
    },
    Playing {
        #[serde(flatten)]
        place: Place,
        session: String,
        paused: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Said {
    pub id: u64,
    pub text: String,
    pub tone: Tone,
}

impl View {
    pub fn of(screen: &Screen, notice: Option<Said>) -> Self {
        let on = match screen {
            Screen::Tv => OnScreen::Tv,
            Screen::Moving { to, place, .. } => OnScreen::Moving {
                device: to.clone(),
                play: MovingView { place: place.clone() },
            },
            Screen::Phone { device, play } => OnScreen::Phone {
                device: device.clone(),
                play: play.as_ref().map(|play| match play {
                    PhonePlay::Starting { place, .. } => PlayView::Starting { place: place.clone() },
                    PhonePlay::Playing {
                        session, place, paused, ..
                    } => PlayView::Playing {
                        place: place.clone(),
                        session: session.clone(),
                        paused: *paused,
                    },
                }),
            },
        };
        Self {
            on,
            boot: *BOOT,
            seq: 0,
            notice,
        }
    }
}

impl Default for View {
    fn default() -> Self {
        Self::of(&Screen::Tv, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn phone(id: &str) -> Device {
        Device {
            id: id.to_string(),
            name: format!("{id} phone"),
        }
    }

    fn place(position: f64) -> Place {
        Place {
            title: Title::Episode {
                show: "outlander".to_string(),
                season: 2,
                episode: 3,
            },
            name: "Outlander".to_string(),
            caption: "S2 E3".to_string(),
            file: PathBuf::from("/lib/Outlander/S02E03.mkv"),
            position,
        }
    }

    fn film(position: f64) -> Place {
        Place {
            title: Title::Movie {
                movie: "dune-2021".to_string(),
            },
            name: "Dune".to_string(),
            caption: "2021".to_string(),
            file: PathBuf::from("/lib/Dune.mkv"),
            position,
        }
    }

    fn flip_to(device: &Device) -> Event {
        Event::Flip {
            to: Target::Phone(device.clone()),
            position: None,
        }
    }

    fn flip_tv(position: Option<f64>) -> Event {
        Event::Flip {
            to: Target::Tv,
            position,
        }
    }

    fn started(device: &Device, session: &str, position: f64) -> Event {
        Event::Started {
            device: device.clone(),
            session: session.to_string(),
            position,
        }
    }

    fn playing(device: &Device, session: &str, position: f64, now: Instant) -> Screen {
        Screen::Phone {
            device: device.clone(),
            play: Some(PhonePlay::Playing {
                session: session.to_string(),
                place: place(position),
                paused: false,
                seen: now,
            }),
        }
    }

    fn is_told(effects: &[Effect], tone: Tone) -> bool {
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Tell(Notice { tone: told, .. }) if *told == tone))
    }

    #[test]
    fn a_phone_finds_the_screen_free_only_where_nothing_else_plays() {
        let now = Instant::now();
        let (mine, other) = (phone("a"), phone("b"));
        assert!(Screen::Tv.free_for(&mine, None));
        assert!(!Screen::Tv.free_for(&mine, Some(&place(10.0))));
        let moving = Screen::Moving {
            to: other.clone(),
            place: place(10.0),
            since: now,
        };
        assert!(!moving.free_for(&mine, None));
        let idle = Screen::Phone {
            device: other.clone(),
            play: None,
        };
        assert!(idle.free_for(&mine, None));
        assert!(!playing(&other, "s1", 5.0, now).free_for(&mine, None));
        assert!(playing(&mine, "s1", 5.0, now).free_for(&mine, None));
    }

    #[test]
    fn flipping_to_a_phone_while_the_tv_is_idle_only_moves_where_play_goes() {
        let now = Instant::now();
        let iphone = phone("a");
        let mut screen = Screen::Tv;
        assert_eq!(screen.step(flip_to(&iphone), None, now), vec![]);
        assert_eq!(
            screen,
            Screen::Phone {
                device: iphone,
                play: None
            }
        );
    }

    #[test]
    fn flipping_to_a_phone_while_the_tv_plays_pauses_it_and_stops_it_once_the_phone_plays() {
        let now = Instant::now();
        let iphone = phone("a");
        let mut screen = Screen::Tv;
        assert_eq!(
            screen.step(flip_to(&iphone), Some(&place(600.0)), now),
            vec![Effect::PauseTv, Effect::ShowOnTv("Moving to a phone…".to_string())]
        );
        assert!(matches!(&screen, Screen::Moving { to, place, .. } if *to == iphone && place.position == 600.0));
        assert_eq!(screen.step(flip_to(&iphone), Some(&place(600.0)), now), vec![]);
        let effects = screen.step(started(&iphone, "s1", 600.0), Some(&place(600.0)), now);
        assert_eq!(effects[0], Effect::StopTv);
        assert!(effects.contains(&Effect::ShowOnTv("Playing on a phone".to_string())));
        assert!(is_told(&effects, Tone::Quiet));
        assert_eq!(screen, playing(&iphone, "s1", 600.0, now));
        assert!(!screen.allows_tv_playing());
    }

    #[test]
    fn a_handoff_that_fails_or_is_cancelled_or_times_out_goes_back_to_the_tv() {
        let now = Instant::now();
        let iphone = phone("a");
        let tv = place(600.0);

        let mut screen = Screen::Tv;
        screen.step(flip_to(&iphone), Some(&tv), now);
        let effects = screen.step(
            Event::Failed {
                device: iphone.clone(),
                reason: "AV1 video".to_string(),
            },
            Some(&tv),
            now,
        );
        assert_eq!(effects[0], Effect::ResumeTv);
        assert!(is_told(&effects, Tone::Error));
        assert_eq!(screen, Screen::Tv);

        screen.step(flip_to(&iphone), Some(&tv), now);
        assert_eq!(screen.step(flip_tv(None), Some(&tv), now), vec![Effect::ResumeTv]);
        assert_eq!(screen, Screen::Tv);

        screen.step(flip_to(&iphone), Some(&tv), now);
        assert_eq!(screen.step(Event::Tick, Some(&tv), now + MOVING_FOR / 2), vec![]);
        let effects = screen.step(Event::Tick, Some(&tv), now + MOVING_FOR);
        assert_eq!(effects[0], Effect::ResumeTv);
        assert_eq!(screen, Screen::Tv);
    }

    #[test]
    fn flipping_back_to_the_tv_plays_there_from_the_phones_position() {
        let now = Instant::now();
        let iphone = phone("a");
        let mut screen = playing(&iphone, "s1", 600.0, now);
        let effects = screen.step(flip_tv(Some(642.5)), None, now);
        assert_eq!(effects, vec![Effect::PlayTv(place(642.5))]);
        assert_eq!(screen, Screen::Tv);

        let mut screen = playing(&iphone, "s1", 600.0, now);
        assert_eq!(
            screen.step(flip_tv(None), None, now),
            vec![Effect::PlayTv(place(600.0))]
        );

        let mut screen = Screen::Phone {
            device: iphone,
            play: None,
        };
        assert_eq!(screen.step(flip_tv(None), None, now), vec![]);
        assert_eq!(screen, Screen::Tv);
    }

    #[test]
    fn the_newest_phone_takes_over_where_the_other_one_was() {
        let now = Instant::now();
        let (first, second) = (phone("a"), phone("b"));
        let mut screen = playing(&first, "s1", 900.0, now);
        assert_eq!(screen.step(flip_to(&second), None, now), vec![]);
        assert!(matches!(
            &screen,
            Screen::Phone { device, play: Some(PhonePlay::Starting { place, .. }) } if *device == second && place.position == 900.0
        ));
        assert_eq!(
            screen.step(
                Event::Report {
                    device: first.clone(),
                    session: "s1".to_string(),
                    place: place(950.0),
                    paused: false
                },
                None,
                now
            ),
            vec![]
        );
        assert_eq!(screen.step(started(&second, "s2", 900.0), None, now), vec![]);
        assert!(
            matches!(&screen, Screen::Phone { device, play: Some(PhonePlay::Playing { session, .. }) } if *device == second && session == "s2")
        );

        let mut screen = Screen::Moving {
            to: first.clone(),
            place: place(10.0),
            since: now,
        };
        screen.step(flip_to(&second), Some(&place(10.0)), now);
        assert!(matches!(&screen, Screen::Moving { to, .. } if *to == second));
        assert_eq!(
            screen.step(started(&first, "s3", 10.0), Some(&place(10.0)), now),
            vec![]
        );
    }

    #[test]
    fn play_from_a_phone_plays_there_and_stops_the_tv() {
        let now = Instant::now();
        let iphone = phone("a");
        let mut screen = Screen::Tv;
        let effects = screen.step(
            Event::Play {
                device: iphone.clone(),
                place: film(0.0),
            },
            Some(&place(100.0)),
            now,
        );
        assert_eq!(effects, vec![Effect::StopTv]);
        assert!(
            matches!(&screen, Screen::Phone { play: Some(PhonePlay::Starting { place, .. }), .. } if place.title == film(0.0).title)
        );
        assert_eq!(
            screen.step(
                Event::Play {
                    device: iphone.clone(),
                    place: place(5.0)
                },
                None,
                now
            ),
            vec![]
        );
        screen.step(started(&iphone, "s9", 5.0), None, now);
        assert_eq!(screen, playing(&iphone, "s9", 5.0, now));
    }

    #[test]
    fn a_phone_that_does_not_start_or_goes_quiet_keeps_its_place_and_stops() {
        let now = Instant::now();
        let iphone = phone("a");
        let mut screen = Screen::Phone {
            device: iphone.clone(),
            play: Some(PhonePlay::Starting {
                place: place(0.0),
                since: now,
            }),
        };
        let effects = screen.step(Event::Tick, None, now + STARTING_FOR);
        assert!(is_told(&effects, Tone::Error));
        assert_eq!(
            screen,
            Screen::Phone {
                device: iphone.clone(),
                play: None
            }
        );

        let mut screen = playing(&iphone, "s1", 300.0, now);
        screen.step(
            Event::Report {
                device: iphone.clone(),
                session: "s1".to_string(),
                place: place(330.0),
                paused: true,
            },
            None,
            now + Duration::from_secs(30),
        );
        assert_eq!(screen.step(Event::Tick, None, now + Duration::from_secs(70)), vec![]);
        let effects = screen.step(Event::Tick, None, now + Duration::from_secs(30) + *LOST_AFTER);
        assert!(is_told(&effects, Tone::Quiet));
        assert_eq!(
            screen,
            Screen::Phone {
                device: iphone.clone(),
                play: None
            }
        );

        let mut screen = playing(&iphone, "s1", 300.0, now);
        let effects = screen.step(
            Event::Failed {
                device: iphone.clone(),
                reason: "the network went away".to_string(),
            },
            None,
            now,
        );
        assert!(is_told(&effects, Tone::Error));
        assert_eq!(
            screen,
            Screen::Phone {
                device: iphone,
                play: None
            }
        );
    }

    #[test]
    fn a_report_from_the_episode_before_does_not_start_the_next_one() {
        let now = Instant::now();
        let iphone = phone("a");
        let mut screen = playing(&iphone, "s1", 1280.0, now);
        screen.step(
            Event::Play {
                device: iphone.clone(),
                place: film(0.0),
            },
            None,
            now,
        );
        screen.step(
            Event::Report {
                device: iphone.clone(),
                session: "s1".to_string(),
                place: place(1285.0),
                paused: false,
            },
            None,
            now,
        );
        assert!(matches!(
            &screen,
            Screen::Phone { play: Some(PhonePlay::Starting { place, .. }), .. } if *place == film(0.0)
        ));
        screen.step(started(&iphone, "s2", 0.0), None, now);
        screen.step(
            Event::Report {
                device: iphone.clone(),
                session: "s2".to_string(),
                place: film(12.0),
                paused: true,
            },
            None,
            now,
        );
        assert!(matches!(
            &screen,
            Screen::Phone { play: Some(PhonePlay::Playing { place, paused: true, .. }), .. } if *place == film(12.0)
        ));
    }

    #[test]
    fn a_phone_the_tv_gave_up_on_takes_its_play_back_with_its_next_report() {
        let now = Instant::now();
        let iphone = phone("a");
        let report = |device: &Device, position: f64| Event::Report {
            device: device.clone(),
            session: "s1".to_string(),
            place: place(position),
            paused: false,
        };
        let mut screen = playing(&iphone, "s1", 300.0, now);
        screen.step(Event::Tick, None, now + *LOST_AFTER);
        assert_eq!(
            screen,
            Screen::Phone {
                device: iphone.clone(),
                play: None
            }
        );
        let later = now + *LOST_AFTER + Duration::from_secs(5);
        assert_eq!(screen.step(report(&iphone, 352.0), None, later), vec![]);
        assert_eq!(screen, playing(&iphone, "s1", 352.0, later));

        let mut screen = Screen::Phone {
            device: iphone.clone(),
            play: None,
        };
        screen.step(report(&phone("b"), 352.0), None, later);
        assert!(matches!(screen, Screen::Phone { play: None, .. }));

        let mut screen = Screen::Tv;
        screen.step(report(&iphone, 352.0), None, later);
        assert_eq!(screen, Screen::Tv);
    }

    #[test]
    fn a_phone_resumes_after_a_restart_only_where_nothing_else_plays() {
        let now = Instant::now();
        let iphone = phone("a");
        let resume = |device: &Device| Event::Resume {
            device: device.clone(),
            place: place(1234.5),
        };
        let mut screen = Screen::Tv;
        assert_eq!(screen.step(resume(&iphone), None, now), vec![]);
        assert!(matches!(
            &screen,
            Screen::Phone { device, play: Some(PhonePlay::Starting { place, .. }) } if *device == iphone && place.position == 1234.5
        ));

        let mut screen = Screen::Phone {
            device: iphone.clone(),
            play: None,
        };
        screen.step(resume(&iphone), None, now);
        assert!(matches!(&screen, Screen::Phone { play: Some(_), .. }));

        let mut screen = Screen::Tv;
        assert_eq!(screen.step(resume(&iphone), Some(&place(10.0)), now), vec![]);
        assert_eq!(screen, Screen::Tv);

        let mut screen = playing(&phone("b"), "s2", 40.0, now);
        screen.step(resume(&iphone), None, now);
        assert_eq!(screen, playing(&phone("b"), "s2", 40.0, now));

        let mut screen = Screen::Phone {
            device: phone("b"),
            play: None,
        };
        screen.step(resume(&iphone), None, now);
        assert!(matches!(&screen, Screen::Phone { device, play: None } if *device == phone("b")));
    }

    #[test]
    fn closing_the_player_stops_what_it_was_starting() {
        let now = Instant::now();
        let iphone = phone("a");
        let mut screen = Screen::Phone {
            device: iphone.clone(),
            play: Some(PhonePlay::Starting {
                place: place(0.0),
                since: now,
            }),
        };
        assert_eq!(screen.step(Event::Stop { device: phone("b") }, None, now), vec![]);
        assert!(matches!(&screen, Screen::Phone { play: Some(_), .. }));
        assert_eq!(screen.step(Event::Stop { device: iphone.clone() }, None, now), vec![]);
        assert_eq!(
            screen,
            Screen::Phone {
                device: iphone.clone(),
                play: None
            }
        );

        let tv = place(600.0);
        let mut screen = Screen::Tv;
        screen.step(flip_to(&iphone), Some(&tv), now);
        assert_eq!(
            screen.step(Event::Stop { device: iphone }, Some(&tv), now),
            vec![Effect::ResumeTv]
        );
        assert_eq!(screen, Screen::Tv);
    }

    #[test]
    fn a_phone_that_stops_by_itself_leaves_the_screen_on_the_phone() {
        let now = Instant::now();
        let iphone = phone("a");
        let mut screen = playing(&iphone, "s1", 300.0, now);
        screen.step(
            Event::Stopped {
                device: iphone.clone(),
                session: "old".to_string(),
            },
            None,
            now,
        );
        assert_eq!(screen, playing(&iphone, "s1", 300.0, now));
        screen.step(
            Event::Stopped {
                device: iphone.clone(),
                session: "s1".to_string(),
            },
            None,
            now,
        );
        assert_eq!(
            screen,
            Screen::Phone {
                device: iphone,
                play: None
            }
        );
    }

    #[test]
    fn anything_played_on_the_tv_takes_the_screen_back() {
        let now = Instant::now();
        let iphone = phone("a");
        for mut screen in [
            playing(&iphone, "s1", 300.0, now),
            Screen::Moving {
                to: iphone.clone(),
                place: place(1.0),
                since: now,
            },
            Screen::Phone {
                device: iphone.clone(),
                play: None,
            },
            Screen::Tv,
        ] {
            assert_eq!(screen.step(Event::TvPlays, None, now), vec![]);
            assert_eq!(screen, Screen::Tv);
            assert!(screen.allows_tv_playing());
        }
    }

    #[test]
    fn the_phone_screen_never_lets_the_tv_play() {
        let now = Instant::now();
        let iphone = phone("a");
        let events = [
            flip_to(&iphone),
            started(&iphone, "s1", 5.0),
            Event::Tick,
            Event::Report {
                device: iphone.clone(),
                session: "s1".to_string(),
                place: place(9.0),
                paused: false,
            },
            Event::Play {
                device: iphone.clone(),
                place: film(0.0),
            },
            flip_to(&phone("b")),
            flip_tv(Some(3.0)),
            flip_to(&iphone),
        ];
        let mut tv: Option<Place> = Some(place(1.0));
        let mut screen = Screen::Tv;
        for event in events {
            for effect in screen.step(event, tv.as_ref(), now) {
                match effect {
                    Effect::StopTv => tv = None,
                    Effect::PlayTv(place) => tv = Some(place),
                    _ => {}
                }
            }
            assert!(
                tv.is_none() || screen.allows_tv_playing(),
                "the TV plays while {screen:?}"
            );
        }
    }

    #[test]
    fn the_tv_shows_only_the_handoff_itself() {
        let now = Instant::now();
        let iphone = phone("a");
        let tv = place(600.0);
        let shown = |effects: &[Effect]| {
            effects
                .iter()
                .filter(|effect| matches!(effect, Effect::ShowOnTv(_)))
                .count()
        };
        let mut screen = Screen::Tv;
        screen.step(flip_to(&iphone), Some(&tv), now);
        let failed = screen.step(
            Event::Failed {
                device: iphone.clone(),
                reason: "AV1 video".to_string(),
            },
            Some(&tv),
            now,
        );
        assert_eq!(shown(&failed), 0);
        let mut screen = playing(&iphone, "s1", 1.0, now);
        assert_eq!(shown(&screen.step(Event::Tick, None, now + *LOST_AFTER)), 0);
    }

    #[test]
    fn whatever_happens_on_a_phone_the_tv_is_told_to_stop() {
        let now = Instant::now();
        let iphone = phone("a");
        let tv = place(50.0);
        let mut screen = Screen::Phone {
            device: iphone.clone(),
            play: None,
        };
        assert_eq!(screen.step(Event::Tick, Some(&tv), now), vec![Effect::StopTv]);
        let mut screen = playing(&iphone, "s1", 1.0, now);
        let effects = screen.step(flip_to(&phone("b")), Some(&tv), now);
        assert_eq!(effects, vec![Effect::StopTv]);
    }

    #[test]
    fn the_screen_json_is_what_phones_read() {
        let now = Instant::now();
        let iphone = phone("a");
        let mut moving = Screen::Tv;
        moving.step(flip_to(&iphone), Some(&place(1228.0)), now);
        let device = json!({ "id": "a", "name": "a phone" });
        let rows = [
            ("the TV", Screen::Tv, json!({ "kind": "tv", "notice": null })),
            (
                "moving to a phone",
                moving,
                json!({
                    "kind": "moving",
                    "device": device,
                    "play": {
                        "state": "moving",
                        "kind": "episode",
                        "show": "outlander",
                        "season": 2,
                        "episode": 3,
                        "name": "Outlander",
                        "caption": "S2 E3",
                        "position": 1228.0,
                    },
                    "notice": null,
                }),
            ),
            (
                "an idle phone",
                Screen::Phone {
                    device: iphone.clone(),
                    play: None,
                },
                json!({ "kind": "phone", "device": device, "play": null, "notice": null }),
            ),
            (
                "a playing phone",
                playing(&iphone, "s1", 12.0, now),
                json!({
                    "kind": "phone",
                    "device": device,
                    "play": {
                        "state": "playing",
                        "kind": "episode",
                        "show": "outlander",
                        "season": 2,
                        "episode": 3,
                        "name": "Outlander",
                        "caption": "S2 E3",
                        "position": 12.0,
                        "session": "s1",
                        "paused": false,
                    },
                    "notice": null,
                }),
            ),
        ];
        let boot = View::of(&Screen::Tv, None).boot;
        assert!(boot > 0);
        for (name, screen, expected) in rows {
            let mut json = serde_json::to_value(View::of(&screen, None)).unwrap();
            assert!(json["boot"].is_u64(), "{name}");
            assert_eq!(json["boot"], boot, "{name}");
            assert_eq!(json["seq"], 0, "{name}");
            json.as_object_mut().unwrap().remove("boot");
            json.as_object_mut().unwrap().remove("seq");
            assert_eq!(json, expected, "{name}");
        }
    }
}
