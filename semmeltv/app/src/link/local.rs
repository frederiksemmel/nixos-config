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
    let (reporting, keys, finishing) = (tell.clone(), tell.clone(), tell.clone());
    Callbacks {
        report: Box::new(move |report| reporting(FromTv::Report(Box::new(report)))),
        key: Box::new(move |command| keys(FromTv::Key(command))),
        exit: Box::new(move |ended| finishing(FromTv::Finished(ended))),
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
            ToTv::Command { play, command } => {
                if let Some(player) = of(&play) {
                    player.command(command);
                }
            }
            ToTv::Pause { play, paused } => {
                if let Some(player) = of(&play) {
                    player.set_paused(paused);
                }
            }
            ToTv::Mute { play, muted } => {
                if let Some(player) = of(&play) {
                    player.set_muted(muted);
                }
            }
            ToTv::Speed { play, speed } => {
                if let Some(player) = of(&play) {
                    player.set_speed(speed);
                }
            }
            ToTv::Seek { play, target } => match (of(&play), target) {
                (Some(player), Target::To(seconds)) => player.seek_to(seconds),
                (Some(player), Target::By(seconds)) => player.seek_by(seconds),
                (None, _) => {}
            },
            ToTv::SkipSegment { play } => {
                if let Some(player) = of(&play) {
                    player.skip_segment();
                }
            }
            ToTv::HideSkip { play } => {
                if let Some(player) = of(&play) {
                    player.hide_skip();
                }
            }
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
                        (answer.message, answer.ok)
                    }
                    None => ("Nothing is playing".to_string(), false),
                };
                tell(FromTv::Answered { id, message, ok });
            }
            ToTv::Segments { .. }
            | ToTv::Welcome { .. }
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
    use crate::subtitles::Change;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::sync::mpsc::{self, Sender};

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
            title: None,
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
            "fine" => Ok(Player::without_mpv(&play.play, callbacks)),
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
        let (link, heard) = tv(|play, callbacks| Ok(Player::without_mpv(&play.play, callbacks)));
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
            Ok(Player::without_mpv(&play.play, quiet()))
        });
        link.send(ToTv::Play(play("a-play")));
        assert_eq!(next(&heard), FromTv::Key(crate::command::Command::Back));
    }

    fn quiet() -> Callbacks {
        Callbacks {
            report: Box::new(|_| {}),
            key: Box::new(|_| {}),
            exit: Box::new(|_| {}),
        }
    }
}
