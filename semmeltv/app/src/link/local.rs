//! The tv's end of a link inside the core's own process: a thread that owns
//! the player and carries out the core's orders on it, so the only thread
//! that ever waits on a player is this one.

use super::Link;
use super::messages::{FromTv, Play, Target, ToTv};
use crate::player::{Callbacks, Player};
use std::io;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::Duration;

type Tell = Arc<dyn Fn(FromTv) + Send + Sync>;

/// Starts the tv's end on a thread of its own. `tell` carries what it says
/// back to the core, and `start` opens a player for each `ToTv::Play`. The
/// thread ends once every clone of the returned link is dropped, and drops
/// its player with it.
pub fn spawn(
    tell: impl Fn(FromTv) + Send + Sync + 'static,
    start: impl FnMut(&Play, Callbacks) -> io::Result<Player> + Send + 'static,
) -> Link {
    let (link, orders) = Link::pair();
    let tell: Tell = Arc::new(tell);
    let spawned = thread::Builder::new()
        .name("tv".to_string())
        .spawn(move || drive(&orders, &tell, start));
    if let Err(error) = spawned {
        eprintln!("tv: {error}");
    }
    link
}

fn callbacks(tell: &Tell) -> Callbacks {
    let (reporting, keys, remembering, finishing) = (tell.clone(), tell.clone(), tell.clone(), tell.clone());
    Callbacks {
        report: Box::new(move |report| reporting(FromTv::Report(Box::new(report)))),
        key: Box::new(move |command| keys(FromTv::Key(command))),
        remember: Box::new(move |file, remembered| remembering(FromTv::Remember { file, remembered })),
        exit: Box::new(move |ended| finishing(FromTv::Finished(ended))),
    }
}

/// Carries out an order on the player it names, if that one still plays,
/// and has it report at once so the core sees the change.
fn control(player: Option<&Player>, act: impl FnOnce(&Player)) {
    if let Some(player) = player {
        act(player);
        player.report_now();
    }
}

fn drive(orders: &Receiver<ToTv>, tell: &Tell, mut start: impl FnMut(&Play, Callbacks) -> io::Result<Player>) {
    let mut playing: Option<(String, Player)> = None;
    for order in orders {
        let of = |play: &str| {
            playing
                .as_ref()
                .filter(|(current, _)| current == play)
                .map(|(_, player)| player)
        };
        let player = playing.as_ref().map(|(_, player)| player);
        match order {
            ToTv::Play(play) => match start(&play, callbacks(tell)) {
                Ok(player) => {
                    playing = Some((play.play.clone(), player));
                    tell(FromTv::Started { play: play.play });
                }
                Err(error) => tell(FromTv::Failed {
                    play: play.play,
                    error: error.to_string(),
                }),
            },
            ToTv::Command { play, command } => control(of(&play), |player| player.command(command)),
            ToTv::Pause { play, paused } => control(of(&play), |player| player.set_paused(paused)),
            ToTv::Mute { play, muted } => control(of(&play), |player| player.set_muted(muted)),
            ToTv::Speed { play, speed } => control(of(&play), |player| {
                player.set_speed(speed);
            }),
            ToTv::Seek { play, target } => control(of(&play), |player| match target {
                Target::To(seconds) => player.seek_to(seconds),
                Target::By(seconds) => player.seek_by(seconds),
            }),
            ToTv::SkipSegment { play } => control(of(&play), |player| {
                player.skip_segment();
            }),
            ToTv::HideSkip { play } => control(of(&play), |player| {
                player.hide_skip();
            }),
            ToTv::Stop { play } => {
                if let Some(player) = of(&play) {
                    player.stop();
                }
            }
            ToTv::AudioDevice { device, said } => {
                if let Some(player) = player {
                    player.set_audio_device(&device);
                    player.say(&said);
                }
            }
            ToTv::Dialogue { on, said } => {
                if let Some(player) = player {
                    player.set_dialogue(on);
                    player.say(&said);
                }
            }
            ToTv::Toast {
                heading,
                status,
                hold_ms,
            } => {
                if let Some(player) = player {
                    player.toast(&heading, &status, Duration::from_millis(hold_ms));
                }
            }
            ToTv::HideToast => {
                if let Some(player) = player {
                    player.hide_toast();
                }
            }
            ToTv::Subtitles { id, play, change } => {
                let (message, ok) = match of(&play) {
                    Some(player) => {
                        let answer = player.subtitles(change);
                        player.report_now();
                        (answer.message, answer.ok)
                    }
                    None => ("Nothing is playing".to_string(), false),
                };
                tell(FromTv::Answered { id, message, ok });
            }
            ToTv::Segments {
                play,
                segments,
                credits,
            } => control(of(&play), |player| player.set_segments(segments, credits)),
            ToTv::Welcome { .. }
            | ToTv::Library { .. }
            | ToTv::Home(_)
            | ToTv::Key(_)
            | ToTv::Message(_)
            | ToTv::Panel(_)
            | ToTv::Together(_)
            | ToTv::UpNext(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::messages::PlaySubtitles;
    use crate::profile::ProfileId;
    use crate::progress::Tracks;
    use crate::segments::{Kind, Segment};
    use crate::subtitles::Change;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::sync::mpsc::{self, Sender};
    use std::time::Instant;

    fn play(token: &str) -> Box<Play> {
        Box::new(Play {
            play: token.to_string(),
            owner: ProfileId::FIRST,
            file: PathBuf::from("/lib/Up.mkv"),
            source: PathBuf::from("/lib/Up.mkv"),
            start: 0.0,
            paused: false,
            tracks: Tracks::default(),
            caption: "Up".to_string(),
            subtitles: PlaySubtitles::default(),
            audio_device: String::new(),
            dialogue: false,
            next: false,
        })
    }

    fn tv(start: impl FnMut(&Play, Callbacks) -> io::Result<Player> + Send + 'static) -> (Link, Receiver<FromTv>) {
        let (said, heard): (Sender<FromTv>, _) = mpsc::channel();
        let said = Mutex::new(said);
        let link = spawn(
            move |message| {
                if !matches!(message, FromTv::Report(_)) {
                    let _ = said.lock().unwrap().send(message);
                }
            },
            start,
        );
        (link, heard)
    }

    fn next(heard: &Receiver<FromTv>) -> FromTv {
        heard.recv_timeout(Duration::from_secs(2)).unwrap()
    }

    #[test]
    fn a_play_that_starts_says_so_and_one_that_cannot_says_why() {
        let (link, heard) = tv(|play, callbacks| match play.play.as_str() {
            "fine" => Ok(Player::without_mpv(play, callbacks)),
            _ => Err(io::Error::new(io::ErrorKind::NotFound, "no mpv")),
        });
        link.send(ToTv::Play(play("fine")));
        assert_eq!(
            next(&heard),
            FromTv::Started {
                play: "fine".to_string()
            }
        );
        link.send(ToTv::Play(play("broken")));
        assert_eq!(
            next(&heard),
            FromTv::Failed {
                play: "broken".to_string(),
                error: "no mpv".to_string()
            }
        );
    }

    #[test]
    fn subtitles_are_answered_under_the_id_they_were_asked_with() {
        let (link, heard) = tv(|play, callbacks| Ok(Player::without_mpv(play, callbacks)));
        link.send(ToTv::Play(play("a-play")));
        next(&heard);
        link.send(ToTv::Subtitles {
            id: 7,
            play: "an-old-play".to_string(),
            change: Change::Toggle,
        });
        assert_eq!(
            next(&heard),
            FromTv::Answered {
                id: 7,
                message: "Nothing is playing".to_string(),
                ok: false
            }
        );
        link.send(ToTv::Subtitles {
            id: 8,
            play: "a-play".to_string(),
            change: Change::Toggle,
        });
        assert!(matches!(next(&heard), FromTv::Answered { id: 8, .. }));
    }

    #[test]
    fn keys_pressed_in_the_players_window_go_to_the_core() {
        let (link, heard) = tv(|play, callbacks| {
            (callbacks.key)(crate::command::Command::Back);
            Ok(Player::without_mpv(play, quiet()))
        });
        link.send(ToTv::Play(play("a-play")));
        assert_eq!(next(&heard), FromTv::Key(crate::command::Command::Back));
    }

    #[test]
    fn what_an_order_changed_is_reported_at_once_not_at_the_next_tick() {
        let (said, heard) = mpsc::channel();
        let said = Mutex::new(said);
        let link = spawn(
            move |message| {
                let _ = said.lock().unwrap().send(message);
            },
            |play, callbacks| Ok(Player::without_mpv(play, callbacks)),
        );
        link.send(ToTv::Play(play("a-play")));
        while !matches!(next(&heard), FromTv::Started { .. }) {}
        let report = |heard: &Receiver<FromTv>| loop {
            if let FromTv::Report(report) = next(heard) {
                return report;
            }
        };
        report(&heard);
        let ordered = std::time::Instant::now();
        link.send(ToTv::Speed {
            play: "a-play".to_string(),
            speed: 1.5,
        });
        let sped = report(&heard);
        assert!(
            ordered.elapsed() < Duration::from_millis(100),
            "{:?}",
            ordered.elapsed()
        );
        assert_eq!(sped.state.speed, 1.5);
        let ordered = std::time::Instant::now();
        link.send(ToTv::Mute {
            play: "a-play".to_string(),
            muted: true,
        });
        let muted = report(&heard);
        assert!(
            ordered.elapsed() < Duration::from_millis(100),
            "{:?}",
            ordered.elapsed()
        );
        assert!(muted.state.muted);
    }

    #[test]
    fn segments_reach_only_the_play_they_name() {
        let (said, heard) = mpsc::channel();
        let said = Mutex::new(said);
        let link = spawn(
            move |message| {
                let _ = said.lock().unwrap().send(message);
            },
            |play, callbacks| Ok(Player::without_mpv(play, callbacks)),
        );
        let intro = |play: &str| ToTv::Segments {
            play: play.to_string(),
            segments: vec![Segment {
                kind: Kind::Intro,
                start: 0.0,
                end: 40.0,
            }],
            credits: None,
        };
        let skip = || loop {
            if let FromTv::Report(report) = next(&heard) {
                break report.state.skip;
            }
        };
        link.send(ToTv::Play(play("a-play")));
        link.send(intro("an-old-play"));
        link.send(ToTv::Subtitles {
            id: 1,
            play: "a-play".to_string(),
            change: Change::Toggle,
        });
        while !matches!(next(&heard), FromTv::Answered { id: 1, .. }) {}
        skip();
        assert_eq!(skip(), None);
        link.send(intro("a-play"));
        let shown = Instant::now();
        while skip() != Some(Kind::Intro) {
            assert!(shown.elapsed() < Duration::from_secs(3), "no Skip intro");
        }
    }

    fn quiet() -> Callbacks {
        Callbacks {
            report: Box::new(|_| {}),
            key: Box::new(|_| {}),
            remember: Box::new(|_, _| {}),
            exit: Box::new(|_| {}),
        }
    }
}
