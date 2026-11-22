//! The core's end of the link over a Unix socket. One tv at a time: a new
//! one replaces the one before. Nothing here waits on the tv for the core:
//! orders go through bounded queues, every write has 2 s, and a tv silent
//! for 15 s is dropped and killed, since a tv that froze whole cannot end
//! itself and systemd cannot see it inside cage's session.

use super::messages::{FromTv, PROTOCOL, ToTv};
use super::{LINE_LIMIT, Link, QUEUE, line_of};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender, error::TrySendError};
use tokio::task::JoinHandle;
use tokio::time::Instant;

const HELLO_TIME: Duration = Duration::from_secs(5);
const WRITE_TIME: Duration = Duration::from_secs(2);
const SILENT_TIME: Duration = Duration::from_secs(15);
const STEADY_EVERY: Duration = Duration::from_millis(500);
const STALL: Duration = Duration::from_secs(2);

/// What the socket tells the core: what a tv said, or that the tv it had
/// is gone.
#[derive(Debug, PartialEq)]
pub enum Heard {
    Said(Box<FromTv>),
    Gone,
}

/// A socket bound for the tv, not yet served.
pub struct Listening {
    listener: std::os::unix::net::UnixListener,
    revision: String,
}

/// Binds the tv's socket at `path`, readable and writable by this user
/// only. A socket left behind by an earlier core is replaced.
pub fn bind(path: &Path, revision: String) -> io::Result<Listening> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let listener = std::os::unix::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    Ok(Listening { listener, revision })
}

impl Listening {
    /// Serves tvs on the current Tokio runtime. `tell` hears what they say.
    pub fn serve(self, tell: impl Fn(Heard) + Send + Sync + 'static) -> io::Result<Link> {
        let listener = UnixListener::from_std(self.listener)?;
        let (link, orders) = Link::pair();
        let (forward, forwarded) = mpsc::unbounded_channel();
        thread::Builder::new().name("tv orders".to_string()).spawn(move || {
            for order in orders {
                if forward.send(order).is_err() {
                    return;
                }
            }
        })?;
        tokio::spawn(run(listener, forwarded, self.revision, Arc::new(tell)));
        Ok(link)
    }
}

type Tell = Arc<dyn Fn(Heard) + Send + Sync>;

struct Greeting {
    id: u64,
    pid: Option<i32>,
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    hello: FromTv,
}

enum Happened {
    Greeted(Box<Greeting>),
    Closed { id: u64, why: String, silent: bool },
}

/// The tv that is connected: its queue of orders and the tasks that read
/// from it and write to it, which stop when it is dropped.
struct Current {
    id: u64,
    pid: Option<i32>,
    orders: mpsc::Sender<ToTv>,
    tasks: [JoinHandle<()>; 2],
}

impl Drop for Current {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

async fn run(listener: UnixListener, mut orders: UnboundedReceiver<ToTv>, revision: String, tell: Tell) {
    let (events, mut happened) = mpsc::unbounded_channel();
    let mut current: Option<Current> = None;
    let mut next_id = 0;
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    next_id += 1;
                    tokio::spawn(greet(stream, next_id, revision.clone(), events.clone()));
                }
                Err(error) => {
                    eprintln!("tv: accepting: {error}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
            Some(event) = happened.recv() => match event {
                Happened::Greeted(greeting) => {
                    let Greeting { id, pid, reader, writer, hello } = *greeting;
                    if current.take().is_some() {
                        eprintln!("tv: a new tv took the place of the one connected");
                    }
                    let (queue, queued) = mpsc::channel(QUEUE);
                    let _ = queue.try_send(ToTv::Welcome { protocol: PROTOCOL, revision: revision.clone() });
                    let tasks = [
                        tokio::spawn(write_orders(writer, queued, id, events.clone())),
                        tokio::spawn(read_said(reader, id, tell.clone(), events.clone())),
                    ];
                    current = Some(Current { id, pid, orders: queue, tasks });
                    eprintln!("tv: connected");
                    tell(Heard::Said(Box::new(hello)));
                }
                Happened::Closed { id, why, silent } => {
                    if current.as_ref().is_some_and(|tv| tv.id == id) {
                        eprintln!("tv: {why}");
                        if let Some(pid) = current.as_ref().and_then(|tv| tv.pid).filter(|_| silent) {
                            kill(pid, &why);
                        }
                        current = None;
                        tell(Heard::Gone);
                    }
                }
            },
            Some(order) = orders.recv() => {
                let Some(tv) = &current else {
                    continue;
                };
                if let Err(TrySendError::Full(order)) = tv.orders.try_send(order) {
                    eprintln!("tv: not reading, dropped it at {}", order.name());
                    current = None;
                    tell(Heard::Gone);
                }
            }
        }
    }
}

async fn greet(stream: tokio::net::UnixStream, id: u64, revision: String, events: UnboundedSender<Happened>) {
    let ours = unsafe { libc::getuid() };
    let pid = match stream.peer_cred() {
        Ok(peer) if peer.uid() == ours => peer.pid(),
        Ok(peer) => {
            eprintln!("tv: refused a connection from uid {}", peer.uid());
            return;
        }
        Err(error) => {
            eprintln!("tv: whose connection: {error}");
            return;
        }
    };
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = Vec::new();
    let hello = match tokio::time::timeout(HELLO_TIME, read_line(&mut reader, &mut line)).await {
        Ok(Ok(true)) => serde_json::from_slice::<FromTv>(&line).ok(),
        _ => None,
    };
    let Some(FromTv::Hello {
        protocol,
        revision: theirs,
        ..
    }) = &hello
    else {
        eprintln!("tv: a connection that did not say hello");
        return;
    };
    if *protocol != PROTOCOL || *theirs != revision {
        eprintln!("tv: a tv of another build (protocol {protocol}, {theirs}), told to go");
        let welcome = ToTv::Welcome {
            protocol: PROTOCOL,
            revision,
        };
        let _ = tokio::time::timeout(WRITE_TIME, writer.write_all(&line_of(&welcome))).await;
        return;
    }
    let hello = hello.expect("a hello");
    let _ = events.send(Happened::Greeted(Box::new(Greeting {
        id,
        pid,
        reader,
        writer,
        hello,
    })));
}

async fn write_orders(
    mut writer: OwnedWriteHalf,
    mut orders: mpsc::Receiver<ToTv>,
    id: u64,
    events: UnboundedSender<Happened>,
) {
    while let Some(order) = orders.recv().await {
        let written = tokio::time::timeout(WRITE_TIME, writer.write_all(&line_of(&order))).await;
        let why = match written {
            Ok(Ok(())) => continue,
            Ok(Err(error)) => format!("gone while writing: {error}"),
            Err(_) => format!("not reading, dropped it after {WRITE_TIME:?} on {}", order.name()),
        };
        let _ = events.send(Happened::Closed { id, why, silent: false });
        return;
    }
}

/// Reads what the tv says until it goes, breaks the protocol, or says
/// nothing for `SILENT_TIME` of time the core itself ran steadily: a check
/// that comes over `STALL` late means the core was stopped, starved or
/// suspended, and the silence counts again from then, so the core never
/// takes its own pause for the tv's.
async fn read_said(mut reader: BufReader<OwnedReadHalf>, id: u64, tell: Tell, events: UnboundedSender<Happened>) {
    let mut line = Vec::new();
    let (why, silent) = loop {
        let read = {
            let mut since = Instant::now();
            let mut checked = since;
            let read = read_line(&mut reader, &mut line);
            tokio::pin!(read);
            let mut checks = tokio::time::interval(STEADY_EVERY);
            loop {
                tokio::select! {
                    biased;
                    read = &mut read => break Some(read),
                    _ = checks.tick() => {
                        let now = Instant::now();
                        if now - checked > STALL {
                            eprintln!("tv: the core stalled for {:?}; the tv's silence counts from now", now - checked);
                            since = now;
                        }
                        checked = now;
                        if now - since >= SILENT_TIME {
                            break None;
                        }
                    }
                }
            }
        };
        match read {
            Some(Ok(true)) => match serde_json::from_slice::<FromTv>(&line) {
                Ok(said) => tell(Heard::Said(Box::new(said))),
                Err(error) => eprintln!("tv: a line the core does not understand: {error}"),
            },
            Some(Ok(false)) => break ("gone".to_string(), false),
            Some(Err(error)) => break (format!("dropped: {error}"), false),
            None => {
                break (
                    format!("silent for {SILENT_TIME:?} while the core ran steadily, dropped"),
                    true,
                );
            }
        }
    };
    let _ = events.send(Happened::Closed { id, why, silent });
}

/// Ends a tv that went silent, so that systemd starts it again. Never this
/// process itself, which a tv in the same process would be.
fn kill(pid: i32, why: &str) {
    if u32::try_from(pid).ok() == Some(std::process::id()) {
        eprintln!("tv: the silent tv is this process {pid} ({why}); not ending it");
        return;
    }
    eprintln!("tv: ending the silent tv, process {pid}, so that it starts again: {why}");
    if unsafe { libc::kill(pid, libc::SIGKILL) } != 0 {
        eprintln!("tv: could not end process {pid}: {}", io::Error::last_os_error());
    }
}

/// Reads one line into `line`, without its newline. False at the end of the
/// stream; an error for a line longer than `LINE_LIMIT`.
pub async fn read_line(reader: &mut (impl AsyncBufRead + Unpin), line: &mut Vec<u8>) -> io::Result<bool> {
    line.clear();
    (&mut *reader)
        .take(LINE_LIMIT as u64 + 1)
        .read_until(b'\n', line)
        .await?;
    if line.last() == Some(&b'\n') {
        line.pop();
        return Ok(true);
    }
    if line.len() > LINE_LIMIT {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "a line over 16 MB"));
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Command;
    use crate::link::client;
    use crate::link::messages::Play;
    use crate::profile::ProfileId;
    use std::io::{BufRead, BufReader as StdBufReader, Read, Write};
    use std::os::unix::net::UnixStream;
    use std::os::unix::process::CommandExt;
    use std::path::PathBuf;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Instant;

    fn socket(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("semmeltv-socket-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("tv.sock")
    }

    /// A core serving `path`; what it hears arrives on the receiver.
    fn core(path: &Path) -> (Link, Receiver<Heard>) {
        let (heard, hearing) = channel();
        let heard = std::sync::Mutex::new(heard);
        let link = bind(path, "this-build".to_string())
            .unwrap()
            .serve(move |said| {
                let _ = heard.lock().unwrap().send(said);
            })
            .unwrap();
        (link, hearing)
    }

    fn hello(revision: &str) -> FromTv {
        FromTv::Hello {
            protocol: PROTOCOL,
            revision: revision.to_string(),
            playing: None,
            ended: None,
        }
    }

    /// A tv that says hello: its stream, and a reader of what comes back.
    fn tv(path: &Path, revision: &str) -> (UnixStream, StdBufReader<UnixStream>) {
        let mut stream = UnixStream::connect(path).unwrap();
        stream.write_all(&line_of(&hello(revision))).unwrap();
        let reader = StdBufReader::new(stream.try_clone().unwrap());
        (stream, reader)
    }

    fn next_line(reader: &mut StdBufReader<UnixStream>) -> Option<ToTv> {
        reader.get_ref().set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => serde_json::from_str(&line).ok(),
        }
    }

    fn heard(hearing: &Receiver<Heard>) -> Heard {
        hearing
            .recv_timeout(Duration::from_secs(5))
            .expect("the core heard something")
    }

    fn welcome() -> Option<ToTv> {
        Some(ToTv::Welcome {
            protocol: PROTOCOL,
            revision: "this-build".to_string(),
        })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_tv_that_says_hello_is_welcomed_and_gets_its_orders_in_order() {
        let path = socket("hello");
        let (link, hearing) = core(&path);
        let (_stream, mut reader) = tv(&path, "this-build");
        assert_eq!(heard(&hearing), Heard::Said(Box::new(hello("this-build"))));
        link.send(ToTv::Message("one".to_string()));
        link.send(ToTv::Key(Command::Down));
        let read = tokio::task::spawn_blocking(move || {
            [next_line(&mut reader), next_line(&mut reader), next_line(&mut reader)]
        });
        assert_eq!(
            read.await.unwrap(),
            [
                welcome(),
                Some(ToTv::Message("one".to_string())),
                Some(ToTv::Key(Command::Down))
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_second_tv_takes_the_place_of_the_first() {
        let path = socket("second");
        let (_link, hearing) = core(&path);
        let (_first, mut first_reader) = tv(&path, "this-build");
        heard(&hearing);
        let (_second, _) = tv(&path, "this-build");
        assert_eq!(heard(&hearing), Heard::Said(Box::new(hello("this-build"))));
        let closed = tokio::task::spawn_blocking(move || [next_line(&mut first_reader), next_line(&mut first_reader)]);
        assert_eq!(
            closed.await.unwrap(),
            [welcome(), None],
            "the first tv's connection closes"
        );
        assert!(
            hearing.recv_timeout(Duration::from_millis(300)).is_err(),
            "a replaced tv is not gone"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_tv_of_another_build_is_told_the_cores_and_closed() {
        let path = socket("build");
        let (_link, hearing) = core(&path);
        let (_stream, mut reader) = tv(&path, "another-build");
        let read = tokio::task::spawn_blocking(move || [next_line(&mut reader), next_line(&mut reader)]);
        assert_eq!(read.await.unwrap(), [welcome(), None]);
        assert!(hearing.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_tv_that_stops_reading_is_dropped_and_sending_never_waits_for_it() {
        let path = socket("stops");
        let (link, hearing) = core(&path);
        let (_stream, _never_read) = tv(&path, "this-build");
        heard(&hearing);
        let big = "x".repeat(1 << 20);
        let started = Instant::now();
        let mut slowest = Duration::ZERO;
        let gone = loop {
            let sent = Instant::now();
            link.send(ToTv::Message(big.clone()));
            slowest = slowest.max(sent.elapsed());
            if let Ok(heard) = hearing.recv_timeout(Duration::from_millis(20)) {
                break heard;
            }
            assert!(started.elapsed() < Duration::from_secs(10), "still not dropped");
        };
        assert_eq!(gone, Heard::Gone);
        assert!(slowest < Duration::from_millis(50), "a send took {slowest:?}");
        assert!(
            started.elapsed() < WRITE_TIME + Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_line_over_sixteen_megabytes_ends_the_connection() {
        let path = socket("long");
        let (_link, hearing) = core(&path);
        let (mut stream, _) = tv(&path, "this-build");
        heard(&hearing);
        tokio::task::spawn_blocking(move || {
            let chunk = vec![b'x'; 1 << 20];
            for _ in 0..17 {
                if stream.write_all(&chunk).is_err() {
                    return;
                }
            }
            let _ = stream.write_all(b"\n");
        });
        assert_eq!(heard(&hearing), Heard::Gone);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_connection_from_another_user_is_refused() {
        if unsafe { libc::getuid() } != 0 {
            eprintln!("skipped: only root can connect as another user");
            return;
        }
        let path = socket("uid");
        let (_link, hearing) = core(&path);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        std::fs::set_permissions(path.parent().unwrap(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "link::socket::tests::say_hello_and_wait", "--ignored"])
            .env("SEMMELTV_TEST_SOCKET", &path)
            .uid(65534)
            .spawn()
            .unwrap();
        child.wait().unwrap();
        assert!(hearing.recv_timeout(Duration::from_millis(500)).is_err());
    }

    #[test]
    #[ignore = "a_connection_from_another_user_is_refused runs it as another user"]
    fn say_hello_and_wait() {
        let Some(path) = std::env::var_os("SEMMELTV_TEST_SOCKET") else {
            return;
        };
        let (_stream, mut reader) = tv(Path::new(&path), "this-build");
        let mut rest = Vec::new();
        let _ = reader.read_to_end(&mut rest);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_tv_end_of_the_socket_passes_orders_on_and_says_what_its_tv_says() {
        let path = socket("client");
        let (core_link, hearing) = core(&path);
        let (tv_link, tv_orders) = Link::pair();
        let (said, telling) = channel();
        client::start(path, tv_link, telling, "this-build".to_string(), None);
        let Heard::Said(hello) = heard(&hearing) else {
            panic!("a hello");
        };
        assert!(matches!(*hello, FromTv::Hello { playing: None, .. }));
        core_link.send(ToTv::Play(Box::new(Play {
            play: "a-play".to_string(),
            owner: ProfileId(2),
            file: PathBuf::from("/lib/Up.mkv"),
            source: PathBuf::from("/lib/Up.mkv"),
            start: 0.0,
            paused: false,
            tracks: crate::progress::Tracks::default(),
            caption: "Up".to_string(),
            subtitles: Default::default(),
            audio_device: String::new(),
            dialogue: false,
            next: false,
        })));
        let order = tokio::task::spawn_blocking(move || {
            loop {
                match tv_orders.recv_timeout(Duration::from_secs(5)).ok() {
                    Some(ToTv::Panel(_)) => {}
                    other => break other,
                }
            }
        });
        assert!(matches!(order.await.unwrap(), Some(ToTv::Play(_))));
        said.send(FromTv::Started {
            play: "a-play".to_string(),
        })
        .unwrap();
        let started = loop {
            match heard(&hearing) {
                Heard::Said(said) if matches!(*said, FromTv::Report(_)) => {}
                other => break other,
            }
        };
        assert_eq!(
            started,
            Heard::Said(Box::new(FromTv::Started {
                play: "a-play".to_string()
            }))
        );
    }

    /// Waits on Tokio's clock for the next thing the core hears, so the
    /// runtime keeps running and a paused clock moves on.
    async fn hear(hearing: &Receiver<Heard>, within: Duration) -> Option<Heard> {
        let deadline = tokio::time::Instant::now() + within;
        while tokio::time::Instant::now() < deadline {
            if let Ok(heard) = hearing.try_recv() {
                return Some(heard);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        None
    }

    #[tokio::test(start_paused = true)]
    async fn a_core_that_was_paused_does_not_take_its_pause_for_a_silent_tv() {
        let path = socket("paused");
        let (_link, hearing) = core(&path);
        let (mut stream, _reader) = tv(&path, "this-build");
        assert_eq!(
            hear(&hearing, Duration::from_secs(1)).await,
            Some(Heard::Said(Box::new(hello("this-build"))))
        );
        tokio::time::advance(Duration::from_secs(20)).await;
        stream.write_all(&line_of(&FromTv::Report(Box::default()))).unwrap();
        assert_eq!(
            hear(&hearing, Duration::from_secs(1)).await,
            Some(Heard::Said(Box::new(FromTv::Report(Box::default())))),
            "the tv is still connected, not dropped and killed"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_tv_silent_for_fifteen_seconds_of_a_steady_core_is_dropped() {
        let path = socket("silent");
        let (_link, hearing) = core(&path);
        let (_stream, _reader) = tv(&path, "this-build");
        hear(&hearing, Duration::from_secs(1)).await;
        let started = tokio::time::Instant::now();
        assert_eq!(hear(&hearing, Duration::from_secs(30)).await, Some(Heard::Gone));
        let after = started.elapsed();
        assert!(
            after >= SILENT_TIME && after < SILENT_TIME + Duration::from_secs(1),
            "{after:?}"
        );
    }
}
