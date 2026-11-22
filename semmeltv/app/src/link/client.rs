//! The tv's end of the link over the core's Unix socket. It connects and
//! reconnects, says `Hello` with what it plays, carries the core's orders to
//! the tv end that owns the player and the menu, and sends back what they
//! say. While the core is away, keys work on what the tv has and the last
//! ending waits for the next `Hello`.

use super::messages::{Ended, FromTv, PROTOCOL, Running, ToTv};
use super::{LINE_LIMIT, Link, line_of};
use crate::bus::Panel;
use crate::command::Command;
use crate::profile::ProfileId;
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const FIRST_RETRY: Duration = Duration::from_millis(500);
const LAST_RETRY: Duration = Duration::from_secs(5);
const WELCOME_TIME: Duration = Duration::from_secs(5);
const WRITE_TIME: Duration = Duration::from_secs(2);
const HEARTBEAT: Duration = Duration::from_secs(1);
const AWAY_FOR: Duration = Duration::from_secs(4);
/// The exit code of a tv told it is of another build than the core, so
/// systemd starts the one installed now.
pub const OTHER_BUILD: i32 = 3;

/// What the tv knows about its playback, for the next `Hello`.
#[derive(Default)]
struct Kept {
    plays: HashMap<String, (ProfileId, PathBuf)>,
    playing: Option<Running>,
    ended: Option<Ended>,
}

impl Kept {
    fn ordered(&mut self, order: &ToTv) {
        if let ToTv::Play(play) = order {
            self.plays.insert(play.play.clone(), (play.owner, play.file.clone()));
        }
    }

    fn said(&mut self, said: &FromTv) {
        match said {
            FromTv::Started { play } => {
                self.playing = self.plays.get(play).map(|(owner, file)| Running {
                    play: play.clone(),
                    owner: *owner,
                    file: file.clone(),
                    position: 0.0,
                    paused: false,
                });
            }
            FromTv::Report(report) => {
                if let Some(running) = self
                    .playing
                    .as_mut()
                    .filter(|running| report.play.as_deref() == Some(running.play.as_str()))
                {
                    running.position = report.place.position;
                    running.paused = report.state.paused;
                }
            }
            FromTv::Finished(ended) => {
                if self.playing.as_ref().is_some_and(|running| running.play == ended.play) {
                    self.playing = None;
                }
                self.plays.remove(&ended.play);
            }
            FromTv::Failed { play, .. } => {
                self.plays.remove(play);
            }
            _ => {}
        }
    }
}

struct Shared {
    revision: String,
    kept: Mutex<Kept>,
    writer: Mutex<Option<UnixStream>>,
    link: Link,
}

impl Shared {
    fn hello(&self) -> FromTv {
        let kept = self.kept.lock().unwrap();
        FromTv::Hello {
            protocol: PROTOCOL,
            revision: self.revision.clone(),
            playing: kept.playing.clone(),
            ended: kept.ended.clone(),
        }
    }

    fn connected(&self) -> bool {
        self.writer.lock().unwrap().is_some()
    }

    /// Writes to the core; on failure ends the connection, so the reader
    /// sees it too and reconnects.
    fn write(&self, said: &FromTv) -> bool {
        let mut writer = self.writer.lock().unwrap();
        let Some(stream) = writer.as_mut() else {
            return false;
        };
        if let Err(error) = stream.write_all(&line_of(said)) {
            eprintln!("tv: the core stopped reading: {error}");
            let _ = stream.shutdown(std::net::Shutdown::Both);
            *writer = None;
            return false;
        }
        true
    }

    /// What the tv does itself with what it would tell an absent core.
    fn alone(&self, said: FromTv) {
        match said {
            FromTv::Key(command) => self.key_alone(command),
            FromTv::Ask(_) => self.link.send(ToTv::Panel(Panel {
                heading: String::new(),
                status: "SemmelTV is restarting…".to_string(),
                hold: AWAY_FOR,
            })),
            FromTv::Finished(ended) => self.kept.lock().unwrap().ended = Some(ended),
            _ => {}
        }
    }

    fn key_alone(&self, command: Command) {
        let playing = self
            .kept
            .lock()
            .unwrap()
            .playing
            .as_ref()
            .map(|running| running.play.clone());
        match playing {
            Some(play) => self.link.send(ToTv::Command { play, command }),
            None => self.link.send(ToTv::Key(command)),
        }
    }
}

/// Starts the tv's end of the socket at `path`. `link` reaches the tv end
/// that owns the player and the menu; `said` is what that end and the menu
/// tell the core. `revision` names this build to the core.
pub fn start(path: PathBuf, link: Link, said: Receiver<FromTv>, revision: String) {
    let shared = Arc::new(Shared {
        revision,
        kept: Mutex::default(),
        writer: Mutex::default(),
        link,
    });
    let posting = shared.clone();
    thread::Builder::new()
        .name("core post".to_string())
        .spawn(move || post(&posting, &said))
        .expect("a thread for the core's post");
    thread::Builder::new()
        .name("core link".to_string())
        .spawn(move || connect(&path, &shared))
        .expect("a thread for the core's link");
}

/// Passes what the tv says to the core while it is there, keeps what the
/// next `Hello` needs, and beats once a second when there is nothing to say.
fn post(shared: &Shared, said: &Receiver<FromTv>) {
    let mut last = Instant::now();
    loop {
        match said.recv_timeout(HEARTBEAT) {
            Ok(message) => {
                shared.kept.lock().unwrap().said(&message);
                if shared.write(&message) {
                    last = Instant::now();
                } else {
                    shared.alone(message);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if last.elapsed() >= HEARTBEAT && shared.connected() && shared.write(&FromTv::Report(Box::default())) {
            last = Instant::now();
        }
    }
}

fn connect(path: &Path, shared: &Shared) {
    let mut wait = FIRST_RETRY;
    let mut told = false;
    loop {
        match UnixStream::connect(path) {
            Ok(stream) => {
                wait = FIRST_RETRY;
                told = false;
                if let Err(error) = session(stream, shared) {
                    eprintln!("tv: the core went away: {error}");
                }
                *shared.writer.lock().unwrap() = None;
            }
            Err(error) if !told => {
                eprintln!("tv: no core at {}: {error}", path.display());
                told = true;
            }
            Err(_) => {}
        }
        thread::sleep(wait);
        wait = (wait * 2).min(LAST_RETRY);
    }
}

fn session(stream: UnixStream, shared: &Shared) -> io::Result<()> {
    stream.set_write_timeout(Some(WRITE_TIME))?;
    let mut writer = stream.try_clone()?;
    writer.write_all(&line_of(&shared.hello()))?;
    let mut reader = BufReader::new(stream);
    reader.get_ref().set_read_timeout(Some(WELCOME_TIME))?;
    let mut line = Vec::new();
    if !read_line(&mut reader, &mut line)? {
        return Err(io::Error::new(ErrorKind::UnexpectedEof, "closed before its welcome"));
    }
    match serde_json::from_slice(&line) {
        Ok(ToTv::Welcome { protocol, revision }) if protocol == PROTOCOL && revision == shared.revision => {}
        Ok(ToTv::Welcome { protocol, revision }) => {
            eprintln!(
                "tv: the core is another build (protocol {protocol}, {revision}); exiting so the installed one starts"
            );
            std::process::exit(OTHER_BUILD);
        }
        _ => return Err(io::Error::new(ErrorKind::InvalidData, "no welcome")),
    }
    reader.get_ref().set_read_timeout(None)?;
    shared.kept.lock().unwrap().ended = None;
    *shared.writer.lock().unwrap() = Some(writer);
    eprintln!("tv: connected to the core");
    while read_line(&mut reader, &mut line)? {
        match serde_json::from_slice::<ToTv>(&line) {
            Ok(order) => {
                shared.kept.lock().unwrap().ordered(&order);
                shared.link.send(order);
            }
            Err(error) => eprintln!("tv: an order the tv does not understand: {error}"),
        }
    }
    Err(io::Error::new(ErrorKind::UnexpectedEof, "closed"))
}

/// Connects as a tv and prints every line the core sends. The core takes it
/// for its tv, so the real one loses the link until it reconnects.
pub fn dump(path: &Path) -> io::Result<()> {
    let stream = UnixStream::connect(path)?;
    let mut writer = stream.try_clone()?;
    let hello = FromTv::Hello {
        protocol: PROTOCOL,
        revision: super::revision(),
        playing: None,
        ended: None,
    };
    writer.write_all(&line_of(&hello))?;
    let mut reader = BufReader::new(stream);
    let mut line = Vec::new();
    let mut out = io::stdout().lock();
    while read_line(&mut reader, &mut line)? {
        out.write_all(&line)?;
        out.write_all(b"\n")?;
    }
    Ok(())
}

/// Reads one line into `line`, without its newline. False at the end of the
/// stream; an error for a line longer than `LINE_LIMIT`.
fn read_line(reader: &mut impl BufRead, line: &mut Vec<u8>) -> io::Result<bool> {
    line.clear();
    reader.by_ref().take(LINE_LIMIT as u64 + 1).read_until(b'\n', line)?;
    if line.last() == Some(&b'\n') {
        line.pop();
        return Ok(true);
    }
    if line.len() > LINE_LIMIT {
        return Err(io::Error::new(ErrorKind::InvalidData, "a line over 16 MB"));
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::messages::{Play, Report};
    use crate::player::{Finish, State};
    use crate::progress::{Place, Tracks};

    fn alone() -> (Shared, Receiver<ToTv>) {
        let (link, orders) = Link::pair();
        let shared = Shared {
            revision: "this-build".to_string(),
            kept: Mutex::default(),
            writer: Mutex::default(),
            link,
        };
        (shared, orders)
    }

    fn play() -> ToTv {
        ToTv::Play(Box::new(Play {
            play: "a-play".to_string(),
            owner: ProfileId(2),
            file: PathBuf::from("/lib/Up.mkv"),
            source: PathBuf::from("/cache/Up.mkv"),
            start: 0.0,
            paused: false,
            tracks: Tracks::default(),
            caption: "Up".to_string(),
            subtitles: Default::default(),
            audio_device: String::new(),
            dialogue: false,
            next: false,
        }))
    }

    fn an_ending() -> Ended {
        Ended {
            play: "a-play".to_string(),
            file: PathBuf::from("/lib/Up.mkv"),
            owner: ProfileId(2),
            finish: Finish::Stopped,
            place: Place {
                position: 600.0,
                duration: 5400.0,
                credits: None,
            },
            tracks: Tracks::default(),
            played: 590.0,
        }
    }

    #[test]
    fn the_next_hello_says_what_plays_where_and_how_it_ended_while_the_core_was_away() {
        let (shared, _orders) = alone();
        shared.kept.lock().unwrap().ordered(&play());
        let started = FromTv::Started {
            play: "a-play".to_string(),
        };
        shared.kept.lock().unwrap().said(&started);
        let report = FromTv::Report(Box::new(Report {
            play: Some("a-play".to_string()),
            state: State {
                paused: true,
                ..State::default()
            },
            place: Place {
                position: 512.0,
                ..Place::default()
            },
            ..Report::default()
        }));
        shared.kept.lock().unwrap().said(&report);
        let FromTv::Hello { playing, ended, .. } = shared.hello() else {
            unreachable!()
        };
        assert_eq!(
            playing,
            Some(Running {
                play: "a-play".to_string(),
                owner: ProfileId(2),
                file: PathBuf::from("/lib/Up.mkv"),
                position: 512.0,
                paused: true,
            })
        );
        assert_eq!(ended, None);
        shared.kept.lock().unwrap().said(&FromTv::Finished(an_ending()));
        shared.alone(FromTv::Finished(an_ending()));
        let FromTv::Hello {
            playing, ended: kept, ..
        } = shared.hello()
        else {
            unreachable!()
        };
        assert_eq!((playing, kept), (None, Some(an_ending())));
    }

    #[test]
    fn keys_work_on_what_the_tv_has_while_the_core_is_away() {
        let (shared, orders) = alone();
        shared.alone(FromTv::Key(Command::Down));
        assert_eq!(orders.try_recv(), Ok(ToTv::Key(Command::Down)));
        shared.kept.lock().unwrap().ordered(&play());
        shared.kept.lock().unwrap().said(&FromTv::Started {
            play: "a-play".to_string(),
        });
        shared.alone(FromTv::Key(Command::Left));
        assert_eq!(
            orders.try_recv(),
            Ok(ToTv::Command {
                play: "a-play".to_string(),
                command: Command::Left
            })
        );
        shared.alone(FromTv::Ask(crate::link::messages::UiRequest::TvProfile(ProfileId(3))));
        let Ok(ToTv::Panel(panel)) = orders.try_recv() else {
            panic!("a panel");
        };
        assert_eq!(panel.status, "SemmelTV is restarting…");
    }
}
