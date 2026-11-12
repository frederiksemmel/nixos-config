use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub const MOVING_FOR: Duration = Duration::from_secs(20);
pub const STARTING_FOR: Duration = Duration::from_secs(30);
pub const LOST_AFTER: Duration = Duration::from_secs(45);

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
    Started {
        device: Device,
        session: String,
        position: f64,
    },
    Report {
        device: Device,
        session: String,
        position: f64,
        paused: bool,
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

impl Screen {
    pub fn allows_tv_playing(&self) -> bool {
        !matches!(self, Self::Phone { .. })
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
                position,
                paused,
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
                play: Some(PhonePlay::Playing { session: playing, .. }),
            },
            Event::Stopped { device, session },
        ) if current == device && playing == session => (Phone { device, play: None }, vec![]),

        (Moving { to, .. }, Event::Failed { device, reason }) if to == device => (
            Tv,
            vec![
                Effect::ResumeTv,
                notice(
                    format!("It didn't start on {}: {reason}. Still on the TV", device.name),
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
            if now.duration_since(seen) >= LOST_AFTER {
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
    pub kind: &'static str,
    pub device: Option<Device>,
    pub play: Option<PlayView>,
    pub notice: Option<Said>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlayView {
    pub state: &'static str,
    #[serde(flatten)]
    pub place: Place,
    pub session: Option<String>,
    pub paused: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Said {
    pub id: u64,
    pub text: String,
    pub tone: Tone,
}

impl View {
    pub fn of(screen: &Screen, notice: Option<Said>) -> Self {
        let (kind, device, play) = match screen {
            Screen::Tv => ("tv", None, None),
            Screen::Moving { to, place, .. } => (
                "moving",
                Some(to.clone()),
                Some(PlayView {
                    state: "moving",
                    place: place.clone(),
                    session: None,
                    paused: true,
                }),
            ),
            Screen::Phone { device, play } => (
                "phone",
                Some(device.clone()),
                play.as_ref().map(|play| match play {
                    PhonePlay::Starting { place, .. } => PlayView {
                        state: "starting",
                        place: place.clone(),
                        session: None,
                        paused: false,
                    },
                    PhonePlay::Playing {
                        session, place, paused, ..
                    } => PlayView {
                        state: "playing",
                        place: place.clone(),
                        session: Some(session.clone()),
                        paused: *paused,
                    },
                }),
            ),
        };
        Self {
            kind,
            device,
            play,
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
                    position: 950.0,
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
                position: 330.0,
                paused: true,
            },
            None,
            now + Duration::from_secs(30),
        );
        assert_eq!(screen.step(Event::Tick, None, now + Duration::from_secs(70)), vec![]);
        let effects = screen.step(Event::Tick, None, now + Duration::from_secs(30) + LOST_AFTER);
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
    fn keeping_it_on_the_tv_never_leaves_the_switch_on_the_phone() {
        let now = Instant::now();
        let iphone = phone("a");
        let tv = place(1228.0);
        let mut screen = Screen::Tv;
        screen.step(flip_to(&iphone), Some(&tv), now);
        assert_eq!(View::of(&screen, None).kind, "moving");
        screen.step(flip_tv(None), Some(&tv), now);
        assert_eq!(View::of(&screen, None).kind, "tv");
        screen.step(flip_to(&iphone), Some(&tv), now);
        screen.step(Event::TvPlays, Some(&tv), now);
        assert_eq!(View::of(&screen, None).kind, "tv");
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
                position: 9.0,
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
        assert_eq!(shown(&screen.step(Event::Tick, None, now + LOST_AFTER)), 0);
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
    fn the_view_names_the_phone_and_what_it_plays() {
        let now = Instant::now();
        let iphone = phone("a");
        let view = View::of(&playing(&iphone, "s1", 12.0, now), None);
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["kind"], "phone");
        assert_eq!(json["device"]["name"], "a phone");
        assert_eq!(json["play"]["state"], "playing");
        assert_eq!(json["play"]["show"], "outlander");
        assert_eq!(json["play"]["season"], 2);
        assert_eq!(json["play"]["position"], 12.0);
        assert_eq!(json["play"]["session"], "s1");
        assert!(json["play"].get("file").is_none());
    }
}
