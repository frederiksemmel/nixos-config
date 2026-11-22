//! Playback on the TV set's own player. The TV decodes what the Pi can't: the
//! Pi serves it the file over HTTP and steers its UPnP AVTransport.

use super::{Finish, REPORT_EVERY, State};
use crate::command::Command;
use crate::link::QUEUE;
use crate::link::messages::{Ended, Report, Target, ToTv};
use crate::profile::ProfileId;
use crate::progress::{Place, Tracks};
use crate::seek::{Direction, Scrub};
use crate::stream::probe::Media;
use crate::tv::upnp::{Renderer, element};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::thread;
use std::time::{Duration, Instant};

const AUDIO: [&str; 5] = ["aac", "ac3", "eac3", "mp3", "mp2"];
const TICK: Duration = Duration::from_millis(100);
const POLL: Duration = Duration::from_secs(1);
const RETRY: Duration = Duration::from_secs(2);
const START_WAIT: Duration = Duration::from_secs(45);
const GONE_AFTER: Duration = Duration::from_secs(20);
const RELOAD_GRACE: Duration = Duration::from_secs(15);
const END_MARGIN: f64 = 30.0;
const LONG_JUMP: i32 = 60;
const CHUNK: usize = 64 * 1024;

/// Which titles the TV set plays itself, and the port the Pi serves them on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handoff {
    pub codecs: Vec<String>,
    pub port: u16,
}

impl Handoff {
    pub fn takes(&self, media: &Media) -> bool {
        media.video.as_ref().is_some_and(|video| {
            let beyond_the_pi = video.codec == "h264" && (video.ten_bit || video.width > 1920 || video.height > 1088);
            beyond_the_pi || self.codecs.iter().any(|codec| *codec == video.codec)
        })
    }
}

/// What the TV gets: the file as it is, or ffmpeg's MPEG-TS with one chosen
/// audio track, converted to AC-3 when the TV can't play it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feed {
    File,
    Remux { audio: usize, channels: u32, convert: bool },
}

pub fn feed(media: &Media, chosen: Option<i64>) -> Feed {
    let Some(first) = media.audio.iter().find(|audio| audio.default).or(media.audio.first()) else {
        return Feed::File;
    };
    let wanted = chosen
        .and_then(|track| usize::try_from(track - 1).ok())
        .and_then(|index| media.audio.get(index))
        .unwrap_or(first);
    let plays = AUDIO.contains(&wanted.codec.as_str());
    if wanted.index == first.index && plays {
        Feed::File
    } else {
        Feed::Remux {
            audio: wanted.index,
            channels: wanted.channels,
            convert: !plays,
        }
    }
}

fn mime(path: &Path, feed: Feed) -> &'static str {
    if feed != Feed::File {
        return "video/mp2t";
    }
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("mp4" | "m4v") => "video/mp4",
        Some("avi") => "video/x-msvideo",
        Some("ts" | "m2ts") => "video/mp2t",
        _ => "video/x-mkv",
    }
}

fn features(feed: Feed) -> &'static str {
    match feed {
        Feed::File => "DLNA.ORG_OP=01;DLNA.ORG_CI=0;DLNA.ORG_FLAGS=01700000000000000000000000000000",
        Feed::Remux { .. } => "DLNA.ORG_OP=00;DLNA.ORG_CI=1;DLNA.ORG_FLAGS=01300000000000000000000000000000",
    }
}

fn remux_args(path: &Path, from: f64, feed: Feed) -> Vec<String> {
    let Feed::Remux {
        audio,
        channels,
        convert,
    } = feed
    else {
        return Vec::new();
    };
    let mut args: Vec<String> = ["-nostdin", "-v", "error", "-ss"]
        .into_iter()
        .map(str::to_string)
        .collect();
    args.push(format!("{from:.3}"));
    args.push("-i".to_string());
    args.push(path.display().to_string());
    args.extend(["-map", "0:v:0", "-map"].map(str::to_string));
    args.push(format!("0:a:{audio}"));
    args.extend(["-c:v", "copy"].map(str::to_string));
    if convert {
        args.extend(["-c:a", "ac3", "-b:a", "640k"].map(str::to_string));
        if channels > 6 {
            args.extend(["-ac", "6"].map(str::to_string));
        }
    } else {
        args.extend(["-c:a", "copy"].map(str::to_string));
    }
    args.extend(["-f", "mpegts", "pipe:1"].map(str::to_string));
    args
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.0).round() as u64;
    format!("{}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
}

fn seconds(clock: &str) -> Option<f64> {
    let mut total = 0.0;
    let mut parts = 0;
    for part in clock.trim().split(':') {
        total = total * 60.0 + part.parse::<f64>().ok()?;
        parts += 1;
    }
    (parts == 3).then_some(total)
}

fn didl(title: &str, url: &str, mime: &str, feed: Feed, duration: Option<f64>) -> String {
    let duration = duration.map_or_else(String::new, |duration| format!(" duration=\"{}.000\"", clock(duration)));
    format!(
        "<DIDL-Lite xmlns=\"urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" \
         xmlns:upnp=\"urn:schemas-upnp-org:metadata-1-0/upnp/\"><item id=\"semmeltv\" parentID=\"-1\" restricted=\"1\">\
         <dc:title>{}</dc:title><upnp:class>object.item.videoItem</upnp:class>\
         <res protocolInfo=\"http-get:*:{mime}:{}\"{duration}>{}</res></item></DIDL-Lite>",
        escape(title),
        features(feed),
        escape(url)
    )
}

/// The address of this machine on the way to `host`, where the TV can reach it.
fn local_address(host: &str) -> io::Result<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect((host, 9))?;
    Ok(socket.local_addr()?.ip())
}

struct Served {
    path: PathBuf,
    token: String,
    feed: Feed,
    mime: &'static str,
}

struct Request {
    head: bool,
    target: String,
    range: Option<(u64, Option<u64>)>,
}

fn read_request(reader: &mut impl BufRead) -> io::Result<Request> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut words = line.split_whitespace();
    let method = words.next().unwrap_or_default().to_string();
    let target = words.next().unwrap_or_default().to_string();
    let mut range = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
        let Some((name, value)) = header.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("range") {
            range = parse_range(value.trim());
        }
    }
    Ok(Request {
        head: method == "HEAD",
        target,
        range,
    })
}

fn parse_range(value: &str) -> Option<(u64, Option<u64>)> {
    let (start, end) = value.strip_prefix("bytes=")?.split_once('-')?;
    let start = start.trim().parse().ok()?;
    let end = end.trim();
    let end = if end.is_empty() { None } else { Some(end.parse().ok()?) };
    Some((start, end))
}

fn from_of(target: &str) -> f64 {
    target
        .split_once('?')
        .and_then(|(_, query)| query.split('&').find_map(|pair| pair.strip_prefix("from=")))
        .and_then(|from| from.parse().ok())
        .unwrap_or(0.0)
}

fn pump(mut from: impl Read, to: &mut TcpStream, mut left: u64, stop: &AtomicBool) -> io::Result<()> {
    let mut buffer = vec![0; CHUNK];
    while left > 0 && !stop.load(Ordering::SeqCst) {
        let want = usize::try_from(left).unwrap_or(CHUNK).min(CHUNK);
        let read = from.read(&mut buffer[..want])?;
        if read == 0 {
            break;
        }
        to.write_all(&buffer[..read])?;
        left -= read as u64;
    }
    Ok(())
}

fn answer(mut stream: TcpStream, served: &Served, stop: &AtomicBool) -> io::Result<()> {
    let request = read_request(&mut BufReader::new(stream.try_clone()?))?;
    let mut head = format!(
        "Content-Type: {}\r\ntransferMode.dlna.org: Streaming\r\ncontentFeatures.dlna.org: {}\r\nConnection: close\r\n",
        served.mime,
        features(served.feed)
    );
    if !request.target.starts_with(&format!("/{}/", served.token)) {
        return stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    }
    if served.feed != Feed::File {
        stream.write_all(format!("HTTP/1.1 200 OK\r\n{head}\r\n").as_bytes())?;
        if request.head {
            return Ok(());
        }
        let mut child = Process::new("ffmpeg")
            .args(remux_args(&served.path, from_of(&request.target), served.feed))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()?;
        let streamed = pump(child.stdout.take().expect("piped"), &mut stream, u64::MAX, stop);
        let _ = child.kill();
        let _ = child.wait();
        return streamed;
    }
    let mut file = File::open(&served.path)?;
    let length = file.metadata()?.len();
    head.push_str("Accept-Ranges: bytes\r\n");
    let (status, start, end) = match request.range {
        Some((start, end)) if start < length => {
            let end = end.unwrap_or(length - 1).min(length - 1);
            head.push_str(&format!("Content-Range: bytes {start}-{end}/{length}\r\n"));
            ("206 Partial Content", start, end)
        }
        Some(_) => {
            let refused =
                format!("HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{length}\r\n{head}\r\n");
            return stream.write_all(refused.as_bytes());
        }
        None => ("200 OK", 0, length.saturating_sub(1)),
    };
    let size = if length == 0 { 0 } else { end - start + 1 };
    stream.write_all(format!("HTTP/1.1 {status}\r\n{head}Content-Length: {size}\r\n\r\n").as_bytes())?;
    if request.head {
        return Ok(());
    }
    file.seek(SeekFrom::Start(start))?;
    pump(file, &mut stream, size, stop)
}

struct Server {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
}

impl Server {
    fn start(ip: IpAddr, port: u16, served: Served) -> io::Result<Self> {
        let listener = TcpListener::bind((ip, port))?;
        let address = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let served = Arc::new(served);
        thread::spawn(move || {
            for connection in listener.incoming() {
                if stopping.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = connection else {
                    continue;
                };
                let served = served.clone();
                let stopping = stopping.clone();
                thread::spawn(move || {
                    if let Err(error) = answer(stream, &served, &stopping) {
                        if error.kind() != io::ErrorKind::BrokenPipe && error.kind() != io::ErrorKind::ConnectionReset {
                            eprintln!("tv player: serving {}: {error}", served.path.display());
                        }
                    }
                });
            }
        });
        Ok(Self { address, stop })
    }

    fn url(&self, served_name: &str, token: &str) -> String {
        format!("http://{}/{token}/{served_name}", self.address)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_secs(1));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transport {
    Playing,
    Paused,
    Stopped,
    Busy,
}

impl Transport {
    fn parse(state: &str) -> Self {
        match state.trim() {
            "PLAYING" => Self::Playing,
            "PAUSED_PLAYBACK" => Self::Paused,
            "STOPPED" | "NO_MEDIA_PRESENT" => Self::Stopped,
            _ => Self::Busy,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Snapshot {
    transport: Transport,
    position: f64,
    duration: Option<f64>,
}

/// What the session knows of the TV's player, from its answers alone.
#[derive(Clone, Copy, Debug)]
struct Watch {
    opened: Instant,
    heard: Instant,
    started: bool,
    paused: bool,
    position: f64,
    duration: f64,
    played: f64,
    offset: f64,
    reloading_until: Option<Instant>,
}

impl Watch {
    fn new(now: Instant, duration: f64, offset: f64) -> Self {
        Self {
            opened: now,
            heard: now,
            started: false,
            paused: false,
            position: offset,
            duration,
            played: 0.0,
            offset,
            reloading_until: None,
        }
    }

    fn reload(&mut self, offset: f64, now: Instant) {
        self.offset = offset;
        self.position = offset;
        self.reloading_until = Some(now + RELOAD_GRACE);
    }

    fn observe(&mut self, seen: Option<Snapshot>, now: Instant) -> Option<Finish> {
        let Some(seen) = seen else {
            if now - self.heard >= GONE_AFTER {
                return Some(if self.started { Finish::Stopped } else { Finish::Failed });
            }
            return self.waited_too_long(now);
        };
        self.heard = now;
        let reloading = self.reloading_until.is_some_and(|until| now < until);
        match seen.transport {
            Transport::Playing | Transport::Paused => {
                self.reloading_until = None;
                let position = self.offset + seen.position;
                let step = position - self.position;
                if self.started && !self.paused && step > 0.0 && step <= 2.0 * POLL.as_secs_f64() + 1.0 {
                    self.played += step;
                }
                self.started = true;
                self.paused = seen.transport == Transport::Paused;
                self.position = position;
                if let Some(duration) = seen.duration.filter(|duration| *duration > 0.0 && self.offset == 0.0) {
                    self.duration = duration;
                }
            }
            Transport::Stopped if self.started && !reloading => {
                let ended = self.duration > 0.0 && self.position >= self.duration - END_MARGIN;
                return Some(if ended { Finish::Ended } else { Finish::Stopped });
            }
            Transport::Stopped | Transport::Busy => {}
        }
        self.waited_too_long(now)
    }

    fn waited_too_long(&self, now: Instant) -> Option<Finish> {
        (!self.started && now - self.opened >= START_WAIT).then_some(Finish::Failed)
    }
}

/// What the steering thread last saw of the TV set's player.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Seen {
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    pub started: bool,
    pub muted: bool,
    pub played: f64,
    pub jump: Option<i32>,
}

impl Seen {
    fn place(&self) -> Place {
        Place {
            position: self.position,
            duration: self.duration,
            credits: None,
        }
    }

    /// The report a tv would send for `play` if it had seen this.
    pub fn report(&self, play: &str, tracks: Tracks) -> Report {
        Report {
            play: Some(play.to_string()),
            state: State {
                position: self.position,
                duration: self.duration,
                paused: self.paused,
                muted: self.muted,
                speed: 1.0,
                subtitle: None,
                subtitle_delay: 0.0,
                audio: tracks.audio,
                started: self.started,
                jump: self.jump,
                skip: None,
            },
            place: self.place(),
            tracks,
            played: self.played,
            subtitles: None,
        }
    }

    /// How `play` ended if this was the last thing seen of it.
    pub fn ended(&self, play: &str, file: &Path, owner: ProfileId, finish: Finish, tracks: Tracks) -> Ended {
        Ended {
            play: play.to_string(),
            file: file.to_path_buf(),
            owner,
            finish,
            place: self.place(),
            tracks,
            played: self.played,
        }
    }
}

/// What a session tells the core, in the order it happens: that the file is
/// served or why it can't be, what it sees every `REPORT_EVERY` while the
/// TV set plays, and how it ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Told {
    Started,
    Failed(String),
    Seen(Seen),
    Ended { finish: Finish, seen: Seen },
}

enum Order {
    Pause(bool),
    Seek(f64),
    SeekBy(f64),
    Key(Command),
    Mute(bool),
}

pub struct Start {
    pub renderer: Renderer,
    pub port: u16,
    pub path: PathBuf,
    pub media: Media,
    pub feed: Feed,
    pub from: f64,
    pub name: String,
}

/// The core's end of one hand-off. Its orders never wait, and dropping it
/// stops the TV set.
pub struct Session {
    orders: SyncSender<Order>,
}

struct Steering {
    renderer: Renderer,
    url: String,
    name: String,
    mime: &'static str,
    feed: Feed,
    duration: Option<f64>,
}

impl Steering {
    fn url_from(&self, from: f64) -> String {
        match self.feed {
            Feed::File => self.url.clone(),
            Feed::Remux { .. } => format!("{}?from={from:.3}", self.url),
        }
    }

    fn load(&self, from: f64) -> Result<(), String> {
        let url = self.url_from(from);
        let metadata = didl(&self.name, &url, self.mime, self.feed, self.duration);
        let _ = self.renderer.transport("Stop", "");
        self.renderer
            .transport(
                "SetAVTransportURI",
                &format!(
                    "<CurrentURI>{}</CurrentURI><CurrentURIMetaData>{}</CurrentURIMetaData>",
                    escape(&url),
                    escape(&metadata)
                ),
            )
            .and_then(|_| self.renderer.transport("Play", "<Speed>1</Speed>"))
            .map(drop)
            .map_err(|failure| format!("{failure:?}"))
    }

    fn seek(&self, to: f64) -> Result<(), String> {
        self.renderer
            .transport("Seek", &format!("<Unit>REL_TIME</Unit><Target>{}</Target>", clock(to)))
            .map(drop)
            .map_err(|failure| format!("{failure:?}"))
    }

    fn snapshot(&self) -> Option<Snapshot> {
        let transport = self.renderer.transport("GetTransportInfo", "").ok()?;
        let state = Transport::parse(element(&transport, "CurrentTransportState")?);
        let position = self.renderer.transport("GetPositionInfo", "").ok()?;
        Some(Snapshot {
            transport: state,
            position: element(&position, "RelTime").and_then(seconds).unwrap_or(0.0),
            duration: element(&position, "TrackDuration").and_then(seconds),
        })
    }
}

/// Serves the file where the TV set can reach it, and what steers the TV set
/// to it.
fn serve(start: &Start) -> io::Result<(Server, Steering)> {
    let ip = local_address(start.renderer.host())?;
    let token = crate::auth::token();
    let mime = mime(&start.path, start.feed);
    let served_name = match start.feed {
        Feed::File => format!(
            "video.{}",
            start
                .path
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("mkv")
        ),
        Feed::Remux { .. } => "video.ts".to_string(),
    };
    let server = Server::start(
        ip,
        start.port,
        Served {
            path: start.path.clone(),
            token: token.clone(),
            feed: start.feed,
            mime,
        },
    )?;
    let steering = Steering {
        url: server.url(&served_name, &token),
        renderer: start.renderer.clone(),
        name: start.name.clone(),
        mime,
        feed: start.feed,
        duration: start.media.duration,
    };
    Ok((server, steering))
}

impl Session {
    /// Hands the file to the TV set from a thread of its own, which serves
    /// it, runs `prepare`, then steers the TV set until it ends or the
    /// session is dropped. Everything it learns goes to `tell`.
    pub fn start(start: Start, prepare: impl FnOnce() + Send + 'static, tell: impl Fn(Told) + Send + 'static) -> Self {
        let (orders, received) = mpsc::sync_channel(QUEUE);
        thread::spawn(move || hand_off(&start, prepare, &tell, &received));
        Self { orders }
    }

    /// Carries out the core's order for this play, as a tv would. Orders a
    /// TV set has no use for are ignored.
    pub fn take(&self, order: ToTv) {
        let order = match order {
            ToTv::Command { command, .. } => Order::Key(command),
            ToTv::Stop { .. } => Order::Key(Command::Back),
            ToTv::Pause { paused, .. } => Order::Pause(paused),
            ToTv::Mute { muted, .. } => Order::Mute(muted),
            ToTv::Seek {
                target: Target::To(seconds),
                ..
            } => Order::Seek(seconds),
            ToTv::Seek {
                target: Target::By(seconds),
                ..
            } => Order::SeekBy(seconds),
            _ => return,
        };
        self.order(order);
    }

    pub fn set_paused(&self, paused: bool) {
        self.order(Order::Pause(paused));
    }

    pub fn seek_to(&self, seconds: f64) {
        self.order(Order::Seek(seconds));
    }

    pub fn key(&self, command: Command) {
        self.order(Order::Key(command));
    }

    fn order(&self, order: Order) {
        if let Err(TrySendError::Full(_)) = self.orders.try_send(order) {
            eprintln!("tv player: the TV set is not keeping up, dropped an order");
        }
    }
}

fn hand_off(start: &Start, prepare: impl FnOnce(), tell: &dyn Fn(Told), received: &Receiver<Order>) {
    let (server, steering) = match serve(start) {
        Ok(served) => served,
        Err(error) => {
            eprintln!("tv player: serving {}: {error}", start.path.display());
            tell(Told::Failed(error.to_string()));
            return;
        }
    };
    tell(Told::Started);
    prepare();
    let seen = Seen {
        position: start.from,
        duration: start.media.duration.unwrap_or(0.0),
        ..Seen::default()
    };
    let (finish, seen) = steer(&steering, received, seen, start.from, tell);
    drop(server);
    eprintln!(
        "tv player: {} {finish:?} after {} played",
        start.name,
        clock(seen.played)
    );
    tell(Told::Ended { finish, seen });
}

fn load_until_started(steering: &Steering, from: f64, received: &Receiver<Order>) -> Result<(), String> {
    let deadline = Instant::now() + START_WAIT;
    loop {
        match steering.load(from) {
            Ok(()) => return Ok(()),
            Err(error) if Instant::now() + RETRY >= deadline => return Err(error),
            Err(_) => {}
        }
        if let Err(RecvTimeoutError::Disconnected) = received.recv_timeout(RETRY) {
            return Err("stopped before the TV took it".to_string());
        }
    }
}

fn steer(
    steering: &Steering,
    received: &Receiver<Order>,
    mut seen: Seen,
    from: f64,
    tell: &dyn Fn(Told),
) -> (Finish, Seen) {
    let resume_by_seek = steering.feed == Feed::File && from > 0.0;
    let loaded_from = if resume_by_seek { 0.0 } else { from };
    if let Err(error) = load_until_started(steering, loaded_from, received) {
        eprintln!("tv player: the TV did not take {}: {error}", steering.name);
        return (Finish::Failed, seen);
    }
    let mut watch = Watch::new(Instant::now(), seen.duration, loaded_from);
    let mut scrub = Scrub::default();
    let mut resumed = !resume_by_seek;
    let mut next_poll = Instant::now();
    let mut next_report = Instant::now() + REPORT_EVERY;
    loop {
        let now = Instant::now();
        let order = match received.recv_timeout(TICK) {
            Ok(order) => Some(order),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                let _ = steering.renderer.transport("Stop", "");
                return (Finish::Stopped, seen);
            }
        };
        let ordered = order.is_some();
        let mut seek_to = None;
        match order {
            Some(Order::Pause(true)) => {
                let _ = steering.renderer.transport("Pause", "");
            }
            Some(Order::Pause(false)) => {
                let _ = steering.renderer.transport("Play", "<Speed>1</Speed>");
            }
            Some(Order::Seek(to)) => {
                scrub.cancel();
                seek_to = Some(to);
            }
            Some(Order::SeekBy(by)) => {
                seek_to = Some(scrub.target().unwrap_or(watch.position) + by);
                scrub.cancel();
            }
            Some(Order::Mute(muted)) => {
                seen.muted = muted;
                if let Err(failure) = steering.renderer.set_mute(muted) {
                    eprintln!("tv player: mute: {failure:?}");
                }
            }
            Some(Order::Key(command)) => match command {
                Command::Back => {
                    let _ = steering.renderer.transport("Stop", "");
                    return (Finish::Stopped, seen);
                }
                Command::Select | Command::PlayPause => {
                    let action = if watch.paused { "Play" } else { "Pause" };
                    let _ = steering.renderer.transport(action, "<Speed>1</Speed>");
                }
                Command::Left | Command::Right if watch.started => {
                    let direction = if command == Command::Left {
                        Direction::Back
                    } else {
                        Direction::Forward
                    };
                    seek_to = scrub
                        .step(direction, watch.position, watch.duration, now)
                        .filter(|order| order.exact)
                        .map(|order| order.to);
                }
                Command::Up | Command::Down if watch.started => {
                    let seconds = if command == Command::Up { LONG_JUMP } else { -LONG_JUMP };
                    seek_to = scrub
                        .jump(seconds, watch.position, watch.duration, now)
                        .filter(|order| order.exact)
                        .map(|order| order.to);
                }
                _ => {}
            },
            None => {}
        }
        if seek_to.is_none() {
            seek_to = scrub.poll(now).filter(|order| order.exact).map(|order| order.to);
        }
        if let Some(to) = seek_to {
            let to = crate::seek::within(to, watch.duration);
            let sought = match steering.feed {
                Feed::File => steering.seek(to).map(|()| watch.position = to),
                Feed::Remux { .. } => steering.load(to).map(|()| watch.reload(to, now)),
            };
            if let Err(error) = sought {
                eprintln!("tv player: seeking to {}: {error}", clock(to));
            }
        }
        if now >= next_poll {
            next_poll = now + POLL;
            let finish = watch.observe(steering.snapshot(), now);
            if watch.started && !resumed {
                resumed = true;
                match steering.seek(from) {
                    Ok(()) => watch.position = from,
                    Err(error) => eprintln!("tv player: resuming at {}: {error}", clock(from)),
                }
            }
            if let Some(finish) = finish {
                return (finish, seen);
            }
        }
        seen.position = scrub.target().unwrap_or(watch.position);
        seen.duration = watch.duration;
        seen.paused = watch.paused || !watch.started;
        seen.started = watch.started;
        seen.played = watch.played;
        seen.jump = scrub.jump_shown(now);
        if ordered || now >= next_report {
            next_report = now + REPORT_EVERY;
            tell(Told::Seen(seen));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::probe::{Audio, Video};
    use crate::tv::upnp::tests::{Tv, fake};

    fn audio(index: usize, codec: &str, default: bool) -> Audio {
        Audio {
            index,
            codec: codec.to_string(),
            channels: 6,
            language: None,
            name: String::new(),
            default,
        }
    }

    fn hevc(audio: Vec<Audio>) -> Media {
        Media {
            video: Some(Video {
                codec: "hevc".to_string(),
                profile: None,
                ten_bit: true,
                width: 1920,
                height: 1080,
            }),
            audio,
            duration: Some(3000.0),
            ..Media::default()
        }
    }

    #[test]
    fn the_tv_takes_only_the_codecs_it_was_given() {
        let handoff = Handoff {
            codecs: vec!["hevc".to_string()],
            port: 0,
        };
        assert!(handoff.takes(&hevc(Vec::new())));
        let mut h264 = hevc(Vec::new());
        let video = h264.video.as_mut().unwrap();
        video.codec = "h264".to_string();
        video.ten_bit = false;
        assert!(!handoff.takes(&h264));
        h264.video.as_mut().unwrap().ten_bit = true;
        assert!(handoff.takes(&h264), "10-bit H.264 is beyond the Pi's decoder");
        h264.video.as_mut().unwrap().ten_bit = false;
        h264.video.as_mut().unwrap().height = 2160;
        assert!(handoff.takes(&h264), "so is 4K");
        assert!(!handoff.takes(&Media::default()));
    }

    #[test]
    fn the_file_goes_as_it_is_unless_the_tv_would_play_the_wrong_or_an_unplayable_track() {
        let plain = hevc(vec![audio(0, "eac3", true), audio(1, "aac", false)]);
        assert_eq!(feed(&plain, None), Feed::File);
        assert_eq!(feed(&plain, Some(1)), Feed::File);
        assert_eq!(
            feed(&plain, Some(2)),
            Feed::Remux {
                audio: 1,
                channels: 6,
                convert: false
            }
        );
        let dts = hevc(vec![audio(0, "dts", false), audio(1, "truehd", true)]);
        assert_eq!(
            feed(&dts, None),
            Feed::Remux {
                audio: 1,
                channels: 6,
                convert: true
            }
        );
        assert_eq!(feed(&dts, Some(9)), feed(&dts, None));
        assert_eq!(feed(&hevc(Vec::new()), Some(1)), Feed::File);
    }

    #[test]
    fn ffmpeg_copies_the_video_and_converts_only_unplayable_audio() {
        let path = Path::new("/library/Film.mkv");
        let convert = Feed::Remux {
            audio: 1,
            channels: 8,
            convert: true,
        };
        assert_eq!(
            remux_args(path, 61.5, convert).join(" "),
            "-nostdin -v error -ss 61.500 -i /library/Film.mkv -map 0:v:0 -map 0:a:1 -c:v copy \
             -c:a ac3 -b:a 640k -ac 6 -f mpegts pipe:1"
        );
        let copy = Feed::Remux {
            audio: 0,
            channels: 2,
            convert: false,
        };
        assert!(remux_args(path, 0.0, copy).join(" ").contains("-c:a copy -f mpegts"));
    }

    #[test]
    fn clocks_read_and_write_as_upnp_writes_them() {
        assert_eq!(clock(3725.4), "1:02:05");
        assert_eq!(seconds("1:02:05"), Some(3725.0));
        assert_eq!(seconds("0:00:07.500"), Some(7.5));
        assert_eq!(seconds("NOT_IMPLEMENTED"), None);
        assert_eq!(parse_range("bytes=100-"), Some((100, None)));
        assert_eq!(parse_range("bytes=5-9"), Some((5, Some(9))));
        assert_eq!(from_of("/abc/video.ts?from=12.500"), 12.5);
        assert_eq!(from_of("/abc/video.mkv"), 0.0);
    }

    #[test]
    fn the_metadata_names_the_file_and_escapes_it() {
        let text = didl(
            "Tom & Jerry",
            "http://pi:8097/t/video.ts?from=1&x=2",
            "video/mp2t",
            Feed::Remux {
                audio: 0,
                channels: 2,
                convert: false,
            },
            Some(3725.0),
        );
        assert!(text.contains("<dc:title>Tom &amp; Jerry</dc:title>"));
        assert!(text.contains("http-get:*:video/mp2t:DLNA.ORG_OP=00"));
        assert!(text.contains("duration=\"1:02:05.000\""));
        assert!(text.contains(">http://pi:8097/t/video.ts?from=1&amp;x=2</res>"));
    }

    fn snapshot(transport: Transport, position: f64) -> Option<Snapshot> {
        Some(Snapshot {
            transport,
            position,
            duration: Some(3000.0),
        })
    }

    #[test]
    fn the_watch_counts_steady_play_and_tells_an_end_from_a_stop() {
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        let mut watch = Watch::new(start, 3000.0, 0.0);
        assert_eq!(watch.observe(snapshot(Transport::Stopped, 0.0), at(1)), None);
        assert_eq!(watch.observe(snapshot(Transport::Playing, 1.0), at(2)), None);
        assert_eq!(watch.observe(snapshot(Transport::Playing, 2.0), at(3)), None);
        assert_eq!(watch.observe(snapshot(Transport::Playing, 900.0), at(4)), None);
        assert!((watch.played - 1.0).abs() < 1e-9);
        assert_eq!(
            watch.observe(snapshot(Transport::Stopped, 0.0), at(5)),
            Some(Finish::Stopped)
        );
        let mut ending = Watch::new(start, 3000.0, 0.0);
        ending.observe(snapshot(Transport::Playing, 2985.0), at(1));
        assert_eq!(
            ending.observe(snapshot(Transport::Stopped, 0.0), at(2)),
            Some(Finish::Ended)
        );
    }

    #[test]
    fn a_reload_after_a_seek_is_no_stop_and_a_silent_tv_is() {
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        let mut watch = Watch::new(start, 3000.0, 0.0);
        watch.observe(snapshot(Transport::Playing, 10.0), at(1));
        watch.reload(1200.0, at(2));
        assert_eq!(watch.observe(snapshot(Transport::Stopped, 0.0), at(3)), None);
        assert_eq!(watch.observe(snapshot(Transport::Playing, 5.0), at(4)), None);
        assert_eq!(watch.position, 1205.0);
        assert_eq!(watch.observe(None, at(10)), None);
        assert_eq!(watch.observe(None, at(30)), Some(Finish::Stopped));
        let mut never = Watch::new(start, 3000.0, 0.0);
        assert_eq!(
            never.observe(snapshot(Transport::Busy, 0.0), at(46)),
            Some(Finish::Failed)
        );
    }

    fn get(address: SocketAddr, target: &str, range: Option<&str>) -> (String, Vec<u8>) {
        let mut stream = TcpStream::connect(address).unwrap();
        let range = range.map_or_else(String::new, |range| format!("Range: {range}\r\n"));
        write!(stream, "GET {target} HTTP/1.1\r\nHost: pi\r\n{range}\r\n").unwrap();
        let mut reply = Vec::new();
        stream.read_to_end(&mut reply).unwrap();
        let split = reply.windows(4).position(|window| window == b"\r\n\r\n").unwrap();
        (
            String::from_utf8_lossy(&reply[..split]).to_string(),
            reply[split + 4..].to_vec(),
        )
    }

    #[test]
    fn the_pi_serves_the_file_in_ranges_and_only_under_its_token() {
        let path = std::env::temp_dir().join(format!("semmeltv-handoff-{}.mkv", std::process::id()));
        std::fs::write(&path, b"0123456789").unwrap();
        let server = Server::start(
            "127.0.0.1".parse().unwrap(),
            0,
            Served {
                path: path.clone(),
                token: "secret".to_string(),
                feed: Feed::File,
                mime: "video/x-mkv",
            },
        )
        .unwrap();
        let (head, body) = get(server.address, "/secret/video.mkv", None);
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert!(head.contains("Content-Type: video/x-mkv"));
        assert!(head.contains("contentFeatures.dlna.org: DLNA.ORG_OP=01"));
        assert_eq!(body, b"0123456789");
        let (head, body) = get(server.address, "/secret/video.mkv", Some("bytes=3-5"));
        assert!(head.starts_with("HTTP/1.1 206"), "{head}");
        assert!(head.contains("Content-Range: bytes 3-5/10"));
        assert_eq!(body, b"345");
        let (head, body) = get(server.address, "/secret/video.mkv", Some("bytes=7-"));
        assert!(head.contains("Content-Range: bytes 7-9/10"));
        assert_eq!(body, b"789");
        let (head, _) = get(server.address, "/guess/video.mkv", None);
        assert!(head.starts_with("HTTP/1.1 404"), "{head}");
        std::fs::remove_file(path).unwrap();
    }

    fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "waited for {what}");
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn film(renderer: Renderer, path: PathBuf, from: f64) -> Start {
        Start {
            renderer,
            port: 0,
            path,
            media: hevc(vec![audio(0, "eac3", true)]),
            feed: Feed::File,
            from,
            name: "Outlander S08E01".to_string(),
        }
    }

    /// Starts a session and returns it with what it tells the core.
    fn started(start: Start, prepare: impl FnOnce() + Send + 'static) -> (Session, Receiver<Told>) {
        let (telling, told) = mpsc::channel();
        let session = Session::start(start, prepare, move |said| {
            let _ = telling.send(said);
        });
        (session, told)
    }

    /// Waits until the session tells what `wanted` looks for, and returns it.
    fn told_until<T>(told: &Receiver<Told>, what: &str, wanted: impl Fn(Told) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match told.recv_timeout(left) {
                Ok(said) => {
                    if let Some(found) = wanted(said) {
                        return found;
                    }
                }
                Err(_) => panic!("waited for {what}"),
            }
        }
    }

    fn seen_until(told: &Receiver<Told>, what: &str, wanted: impl Fn(&Seen) -> bool) -> Seen {
        told_until(told, what, |said| match said {
            Told::Seen(seen) if wanted(&seen) => Some(seen),
            _ => None,
        })
    }

    fn ended(told: &Receiver<Told>) -> (Finish, Seen) {
        told_until(told, "the end", |said| match said {
            Told::Ended { finish, seen } => Some((finish, seen)),
            _ => None,
        })
    }

    #[test]
    fn a_session_hands_the_file_to_the_tv_resumes_steers_and_ends_with_it() {
        let path = std::env::temp_dir().join(format!("semmeltv-session-{}.mkv", std::process::id()));
        std::fs::write(&path, b"film").unwrap();
        let tv = Tv::default();
        tv.player.lock().unwrap().duration = 3000;
        let fake = fake(tv);
        let prepared = Arc::new(AtomicBool::new(false));
        let preparing = prepared.clone();
        let (session, told) = started(film(fake.renderer(), path.clone(), 600.0), move || {
            preparing.store(true, Ordering::SeqCst);
        });
        assert_eq!(told.recv_timeout(Duration::from_secs(10)), Ok(Told::Started));
        let player = fake.tv.player.clone();
        wait_for("the resume seek", || player.lock().unwrap().position == 600);
        assert!(prepared.load(Ordering::SeqCst));
        let uri = player.lock().unwrap().uri.clone();
        assert!(
            uri.starts_with("http://127.0.0.1:") && uri.ends_with("/video.mkv"),
            "{uri}"
        );
        assert!(player.lock().unwrap().metadata.contains("Outlander S08E01"));
        let token_path = uri.trim_start_matches("http://").split_once('/').unwrap().1.to_string();
        let address: SocketAddr = uri
            .trim_start_matches("http://")
            .split_once('/')
            .unwrap()
            .0
            .parse()
            .unwrap();
        let (_, body) = get(address, &format!("/{token_path}"), None);
        assert_eq!(body, b"film");
        seen_until(&told, "the position", |seen| seen.position == 600.0 && seen.started);
        let play = "a-play".to_string();
        session.take(ToTv::Pause {
            play: play.clone(),
            paused: true,
        });
        seen_until(&told, "the pause", |seen| seen.paused);
        let muting = Instant::now();
        session.take(ToTv::Mute {
            play: play.clone(),
            muted: true,
        });
        seen_until(&told, "the mute", |seen| seen.muted);
        assert!(muting.elapsed() < REPORT_EVERY, "told only on the next report");
        assert!(*fake.tv.muted.lock().unwrap());
        session.take(ToTv::Seek {
            play: play.clone(),
            target: Target::To(1500.0),
        });
        wait_for("the seek", || player.lock().unwrap().position == 1500);
        session.take(ToTv::Seek {
            play: play.clone(),
            target: Target::By(-100.0),
        });
        wait_for("the step back", || player.lock().unwrap().position == 1400);
        session.take(ToTv::Stop { play });
        let (finish, seen) = ended(&told);
        assert_eq!((finish, seen.position), (Finish::Stopped, 1400.0));
        assert_eq!(player.lock().unwrap().state, "STOPPED");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn the_tv_ending_the_film_ends_the_session() {
        let tv = Tv::default();
        tv.player.lock().unwrap().duration = 3000;
        let fake = fake(tv);
        let (_session, told) = started(film(fake.renderer(), PathBuf::from("/nonexistent.mkv"), 0.0), || {});
        let player = fake.tv.player.clone();
        wait_for("playing", || player.lock().unwrap().state == "PLAYING");
        thread::sleep(Duration::from_millis(1200));
        player.lock().unwrap().position = 2990;
        thread::sleep(Duration::from_millis(1200));
        player.lock().unwrap().state = "STOPPED".to_string();
        assert_eq!(ended(&told).0, Finish::Ended);
    }

    #[test]
    fn dropping_the_session_stops_the_tv_set() {
        let tv = Tv::default();
        tv.player.lock().unwrap().duration = 3000;
        let fake = fake(tv);
        let (session, told) = started(film(fake.renderer(), PathBuf::from("/nonexistent.mkv"), 0.0), || {});
        seen_until(&told, "the start", |seen| seen.started);
        drop(session);
        assert_eq!(ended(&told).0, Finish::Stopped);
        assert_eq!(fake.tv.player.lock().unwrap().state, "STOPPED");
    }

    #[test]
    fn a_tv_set_that_stops_answering_costs_the_core_nothing() {
        let tv = Tv::default();
        tv.player.lock().unwrap().duration = 3000;
        let fake = fake(tv);
        let (session, told) = started(film(fake.renderer(), PathBuf::from("/nonexistent.mkv"), 0.0), || {});
        seen_until(&told, "the start", |seen| seen.started);
        let stuck = fake.tv.player.lock().unwrap();
        thread::sleep(POLL + TICK);
        let ordering = Instant::now();
        for _ in 0..QUEUE * 4 {
            session.take(ToTv::Command {
                play: "a-play".to_string(),
                command: Command::Left,
            });
        }
        assert!(
            ordering.elapsed() < Duration::from_millis(100),
            "{:?}",
            ordering.elapsed()
        );
        drop(stuck);
        drop(session);
        assert_eq!(ended(&told).0, Finish::Stopped);
    }

    #[test]
    fn a_file_the_pi_cannot_serve_fails_the_hand_off_and_says_why() {
        let taken = TcpListener::bind("127.0.0.1:0").unwrap();
        let start = Start {
            port: taken.local_addr().unwrap().port(),
            ..film(
                Renderer::new("http://127.0.0.1:9/description.xml".to_string()),
                PathBuf::from("/nonexistent.mkv"),
                0.0,
            )
        };
        let (_session, told) = started(start, || panic!("prepared a hand-off that failed"));
        match told.recv_timeout(Duration::from_secs(10)) {
            Ok(Told::Failed(error)) => assert!(error.contains("in use"), "{error}"),
            other => panic!("the session told {other:?}"),
        }
        assert!(told.recv_timeout(Duration::from_millis(200)).is_err());
    }
}
