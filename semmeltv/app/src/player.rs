use crate::audio;
use crate::card::{self, Billing};
use crate::command::Command;
use crate::display::{self, Video};
use crate::overlay::{self, View};
use crate::progress::{Place, Tracks};
use crate::seek::{self, Direction, Order, Scrub};
use crate::segments::{self, Kind, Segment, Title};
use crate::sidecar;
use crate::subtitles::{self, Answer, Change, Observed, Subtitles};
use serde_json::{Value, json};
use std::env;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Process, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(50);
const STOPPED_EXIT_CODE: i32 = 10;
const ENDED_EXIT_CODE: i32 = 11;
const SIGNALLED_EXIT_CODE: i32 = 4;
const SAVE_EVERY: f64 = 30.0;
const PLAY_STEP: f64 = 2.0;
const SHOWN_FOR: Duration = Duration::from_secs(4);
const TICK: Duration = Duration::from_millis(100);
const LONG_JUMP: i32 = 60;
const SAY_FOR: Duration = Duration::from_secs(3);
const OSD_FONT: &str = "Inter";
const OSD_FONTS: &str = env!("SEMMELTV_FONT_DIR");
const POSITION_ID: u64 = 1;
const DURATION_ID: u64 = 2;
const ENDED_ID: u64 = 3;
const VIDEO_ID: u64 = 4;
const PAUSE_ID: u64 = 5;
const SUBTITLES_ID: u64 = 6;
const CHAPTERS_ID: u64 = 7;
const PROMPT_ID: u64 = 1;
const LOADING_ID: u64 = 2;
const OVERLAY_ID: u64 = 3;
const TOAST_ID: u64 = 4;
const CARD_FADE: Duration = Duration::from_millis(300);
const KEYBINDS: [(&str, &str); 12] = [
    ("ESC", "script-message semmeltv-back"),
    ("BS", "script-message semmeltv-back"),
    ("v", "script-message semmeltv-listen"),
    ("s", "script-message semmeltv-subtitles"),
    ("d", "script-message semmeltv-dialogue"),
    ("ENTER", "script-message semmeltv-select"),
    ("KP_ENTER", "script-message semmeltv-select"),
    ("SPACE", "script-message semmeltv-play-pause"),
    ("LEFT", "repeatable script-message semmeltv-left"),
    ("RIGHT", "repeatable script-message semmeltv-right"),
    ("UP", "repeatable script-message semmeltv-up"),
    ("DOWN", "repeatable script-message semmeltv-down"),
];

type Shared = Arc<Mutex<Playback>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Finish {
    Ended,
    Stopped,
    Failed,
}

impl Finish {
    fn from_exit_code(code: Option<i32>) -> Self {
        match code {
            Some(ENDED_EXIT_CODE) => Self::Ended,
            Some(0 | SIGNALLED_EXIT_CODE | STOPPED_EXIT_CODE) => Self::Stopped,
            _ => Self::Failed,
        }
    }
}

#[derive(Debug, PartialEq)]
enum Event {
    Position(f64),
    Duration(f64),
    Ended,
    Video(Video),
    Paused(bool),
    Key(Command),
    Started,
    Loaded,
    Subtitles(Observed),
    Chapters(Option<f64>),
}

impl Event {
    fn parse(line: &str) -> Option<Self> {
        let message: Value = serde_json::from_str(line).ok()?;
        match message["event"].as_str()? {
            "property-change" => match message["id"].as_u64()? {
                POSITION_ID => Some(Self::Position(message["data"].as_f64()?)),
                DURATION_ID => Some(Self::Duration(message["data"].as_f64()?)),
                VIDEO_ID => video(&message["data"]).map(Self::Video),
                ENDED_ID => message["data"].as_bool()?.then_some(Self::Ended),
                PAUSE_ID => Some(Self::Paused(message["data"].as_bool()?)),
                SUBTITLES_ID => Observed::parse(&message).map(Self::Subtitles),
                CHAPTERS_ID => Some(Self::Chapters(credits_chapter(&message["data"]))),
                _ => None,
            },
            "client-message" => key_command(message["args"][0].as_str()?).map(Self::Key),
            "playback-restart" => Some(Self::Started),
            "file-loaded" => Some(Self::Loaded),
            _ => None,
        }
    }
}

fn credits_chapter(chapters: &Value) -> Option<f64> {
    chapters.as_array()?.iter().find_map(|chapter| {
        let title = chapter["title"].as_str()?.to_lowercase();
        title.contains("credits").then(|| chapter["time"].as_f64())?
    })
}

fn video(track: &Value) -> Option<Video> {
    let width = track["demux-w"].as_f64()?;
    let height = track["demux-h"].as_f64()?;
    let par = track["demux-par"].as_f64().unwrap_or(1.0);
    let (width, height) = if par > 1.0 {
        (width * par, height)
    } else {
        (width, height / par)
    };
    Some(Video {
        width: width.round() as u64,
        height: height.round() as u64,
        fps: track["demux-fps"].as_f64()?,
    })
}

fn key_command(message: &str) -> Option<Command> {
    match message {
        "semmeltv-select" => Some(Command::Select),
        "semmeltv-play-pause" => Some(Command::PlayPause),
        "semmeltv-left" => Some(Command::Left),
        "semmeltv-right" => Some(Command::Right),
        "semmeltv-up" => Some(Command::Up),
        "semmeltv-down" => Some(Command::Down),
        "semmeltv-back" => Some(Command::Back),
        "semmeltv-listen" => Some(Command::Listen),
        "semmeltv-subtitles" => Some(Command::Subtitles),
        "semmeltv-dialogue" => Some(Command::Dialogue),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Opening,
    Playing {
        paused: bool,
    },
    Ended,
}

impl Phase {
    fn started(self) -> bool {
        self != Self::Opening
    }

    fn playing(self) -> bool {
        matches!(self, Self::Playing { .. })
    }

    fn paused(self) -> bool {
        match self {
            Self::Opening => true,
            Self::Playing { paused } => paused,
            Self::Ended => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Card {
    #[default]
    Gone,
    Shown {
        opened: Instant,
    },
    Fading {
        opened: Instant,
        since: Instant,
    },
}

#[derive(Default)]
struct Playback {
    stream: Option<UnixStream>,
    name: String,
    position: f64,
    duration: f64,
    played: f64,
    saved: f64,
    phase: Phase,
    muted: bool,
    speed: f64,
    closed: bool,
    segments: Vec<Segment>,
    credits: Option<f64>,
    chapter_credits: Option<f64>,
    prompt: Option<Kind>,
    matched: bool,
    scrub: Scrub,
    shown_until: Option<Instant>,
    drawn: String,
    toast_until: Option<Instant>,
    subtitles: Subtitles,
    tracks: Tracks,
    billing: Billing,
    card: Card,
    on_save: Option<Box<dyn Fn() + Send>>,
    on_key: Option<Box<dyn Fn(Command) + Send>>,
}

impl Playback {
    fn send(&mut self, command: Value) {
        if let Some(stream) = self.stream.as_mut() {
            let _ = send(stream, command);
        }
    }

    fn command(&mut self, command: Command, now: Instant) {
        match command {
            Command::Back => self.send(json!(["quit", STOPPED_EXIT_CODE])),
            Command::Select => {
                self.scrub.cancel();
                let message = self.select();
                self.send(message);
            }
            Command::PlayPause => self.send(json!(["cycle", "pause"])),
            Command::Left => self.step(Direction::Back, now),
            Command::Right => self.step(Direction::Forward, now),
            Command::Up => self.jump(LONG_JUMP, now),
            Command::Down => self.jump(-LONG_JUMP, now),
            Command::Listen | Command::Dialogue => {}
            Command::Subtitles => {
                self.change_subtitles(Change::Toggle, now);
                return;
            }
        }
        self.wake(now);
        self.present(now);
    }

    fn set_audio_device(&mut self, device: &str) {
        self.send(json!(["set_property", "audio-device", device]));
        if let Some(track) = self.subtitles.audio_to_restore() {
            self.send(json!(["set_property", "aid", track]));
        }
    }

    fn step(&mut self, direction: Direction, now: Instant) {
        if self.phase.playing() {
            let order = self.scrub.step(direction, self.position, self.duration, now);
            self.seek(order);
        }
    }

    fn jump(&mut self, seconds: i32, now: Instant) {
        if self.phase.playing() {
            let order = self.scrub.jump(seconds, self.position, self.duration, now);
            self.seek(order);
        }
    }

    fn seek(&mut self, order: Option<Order>) {
        if let Some(order) = order {
            self.send(seek_command(order));
            if order.exact {
                self.position = order.to;
            }
        }
    }

    fn select(&self) -> Value {
        match segments::at(&self.segments, self.position) {
            Some(segment) => json!(["seek", segment.end, "absolute"]),
            None => json!(["cycle", "pause"]),
        }
    }

    fn update_prompt(&mut self) {
        let prompt = segments::at(&self.segments, self.position).map(|segment| segment.kind);
        if prompt != self.prompt {
            self.prompt = prompt;
            self.send(prompt_command(prompt));
        }
    }

    fn advance(&mut self, position: f64) {
        let step = position - self.position;
        if self.phase == (Phase::Playing { paused: false }) && step > 0.0 && step <= PLAY_STEP {
            self.played += step;
        }
        self.position = position;
    }

    fn begin(&mut self, now: Instant) {
        if self.phase == Phase::Opening {
            self.phase = Phase::Playing { paused: false };
            self.wake(now);
        }
    }

    fn pause_changed(&mut self, paused: bool) {
        if let Phase::Playing { .. } = self.phase {
            self.phase = Phase::Playing { paused };
        }
    }

    fn wake(&mut self, now: Instant) {
        self.shown_until = Some(now + SHOWN_FOR);
    }

    fn key(&mut self, command: Command, now: Instant) {
        match (command, &self.on_key) {
            (Command::Back | Command::Listen | Command::Dialogue, Some(on_key)) => on_key(command),
            _ => self.command(command, now),
        }
    }

    fn toast(&mut self, heading: &str, message: &str, until: Instant) {
        self.toast_until = Some(until);
        self.send(toast_command(&overlay::toast(heading, message)));
    }

    fn change_subtitles(&mut self, change: Change, now: Instant) -> Answer {
        let told = change != Change::Restyle;
        let answer = self.subtitles.change(change);
        for command in &answer.commands {
            self.send(command.clone());
        }
        if told {
            self.toast("", &answer.message, now + SAY_FOR);
        }
        answer
    }

    fn hide_toast(&mut self) {
        if self.toast_until.take().is_some() {
            self.send(toast_command(""));
        }
    }

    fn tick(&mut self, now: Instant) {
        self.show_card(now);
        if self.toast_until.is_some_and(|until| now >= until) {
            self.hide_toast();
        }
        if self.phase != Phase::Ended {
            let order = self.scrub.poll(now);
            self.seek(order);
            self.present(now);
        }
    }

    fn open_card(&mut self, now: Instant) {
        self.card = Card::Shown { opened: now };
        self.show_card(now);
    }

    fn show_card(&mut self, now: Instant) {
        let (opened, opacity) = match self.card {
            Card::Gone => return,
            Card::Shown { opened } => (opened, 1.0),
            Card::Fading { opened, since } => (
                opened,
                1.0 - now.duration_since(since).as_secs_f64() / CARD_FADE.as_secs_f64(),
            ),
        };
        if opacity <= 0.0 {
            self.card = Card::Gone;
            self.send(card_command(None));
            return;
        }
        let frame = card::frame(&self.billing, now.duration_since(opened), opacity);
        self.send(card_command(Some(&frame)));
    }

    fn close_card(&mut self, now: Instant) {
        if let Card::Shown { opened } = self.card {
            self.card = Card::Fading { opened, since: now };
            self.send(json!(["set_property", "background-color", "#000000"]));
        }
    }

    fn shown(&self, now: Instant) -> bool {
        match self.phase {
            Phase::Playing { paused: true } => true,
            Phase::Playing { paused: false } => self.shown_until.is_some_and(|until| now < until),
            Phase::Opening | Phase::Ended => false,
        }
    }

    fn view(&self, now: Instant) -> View<'_> {
        View {
            title: &self.name,
            position: self.scrub.target().unwrap_or(self.position),
            duration: self.duration,
            paused: self.phase.paused(),
            jump: self.scrub.jump_shown(now),
        }
    }

    fn present(&mut self, now: Instant) {
        let drawn = if self.shown(now) {
            self.view(now).render()
        } else {
            String::new()
        };
        if drawn != self.drawn {
            self.send(overlay_command(&drawn));
            self.drawn = drawn;
        }
    }

    fn save_position(&mut self) {
        if self.phase != Phase::Ended && save_due(self.saved, self.position) {
            self.saved = self.position;
            if let Some(on_save) = &self.on_save {
                on_save();
            }
        }
    }

    fn end(&mut self) {
        self.phase = Phase::Ended;
        self.send(json!(["quit", ENDED_EXIT_CODE]));
    }
}

pub fn speed_within(speed: f64) -> f64 {
    if speed.is_finite() {
        ((speed * 20.0).round() / 20.0).clamp(0.5, 2.0)
    } else {
        1.0
    }
}

pub fn speed_label(speed: f64) -> String {
    let text = format!("{speed:.2}");
    format!("{}×", text.trim_end_matches('0').trim_end_matches('.'))
}

fn sound_filter(dialogue: bool) -> String {
    format!("lavfi=[{}]", audio::stereo(dialogue))
}

fn resume_args(start: f64, tracks: Tracks) -> Vec<String> {
    let mut args = Vec::new();
    if start > 0.0 {
        args.push(format!("--start={start:.3}"));
    }
    if let Some(audio) = tracks.audio {
        args.push(format!("--aid={audio}"));
    }
    if let Some(subtitle) = tracks.subtitle {
        args.push(format!("--sid={subtitle}"));
    }
    if let Some(shown) = tracks.subtitles_shown {
        args.push(format!("--sub-visibility={}", if shown { "yes" } else { "no" }));
    }
    args
}

fn save_due(saved: f64, position: f64) -> bool {
    (position - saved).abs() >= SAVE_EVERY
}

fn seek_command(order: Order) -> Value {
    let mode = if order.exact {
        "absolute+exact"
    } else {
        "absolute+keyframes"
    };
    json!(["seek", order.to, mode])
}

fn overlay_command(data: &str) -> Value {
    if data.is_empty() {
        return json!({ "name": "osd-overlay", "id": OVERLAY_ID, "format": "none", "data": "" });
    }
    json!({
        "name": "osd-overlay",
        "id": OVERLAY_ID,
        "format": "ass-events",
        "data": data,
        "res_x": overlay::WIDTH,
        "res_y": overlay::HEIGHT,
    })
}

fn toast_command(data: &str) -> Value {
    let format = if data.is_empty() { "none" } else { "ass-events" };
    json!({
        "name": "osd-overlay",
        "id": TOAST_ID,
        "format": format,
        "data": data,
        "res_x": overlay::WIDTH,
        "res_y": overlay::HEIGHT,
        "z": 2,
    })
}

fn prompt_command(prompt: Option<Kind>) -> Value {
    match prompt {
        Some(kind) => {
            let pill = format!(
                r"{{\an7\pos(1524,792)\bord0\shad0\1c&HFFFFFF&\p1}}{}{{\p0}}",
                overlay::pill(300.0, 84.0)
            );
            let label = format!(
                r"{{\an5\pos(1674,834)\bord0\shad0\b1\fs44\1c&H120D0B&}}{}",
                kind.label()
            );
            json!({
                "name": "osd-overlay",
                "id": PROMPT_ID,
                "format": "ass-events",
                "data": format!("{pill}\n{label}"),
                "res_x": overlay::WIDTH,
                "res_y": overlay::HEIGHT,
                "z": 1,
            })
        }
        None => json!({ "name": "osd-overlay", "id": PROMPT_ID, "format": "none", "data": "" }),
    }
}

fn card_command(frame: Option<&str>) -> Value {
    let (format, data) = match frame {
        Some(frame) => ("ass-events", frame),
        None => ("none", ""),
    };
    json!({
        "name": "osd-overlay",
        "id": LOADING_ID,
        "format": format,
        "data": data,
        "res_x": overlay::WIDTH,
        "res_y": overlay::HEIGHT,
    })
}

pub struct Player {
    playback: Shared,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct State {
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    pub muted: bool,
    pub speed: f64,
    pub subtitle: Option<i64>,
    pub subtitle_delay: f64,
    pub audio: Option<i64>,
    pub started: bool,
    pub jump: Option<i32>,
}

impl Player {
    pub fn start(
        path: &Path,
        start: f64,
        tracks: Tracks,
        name: String,
        billing: Billing,
        title: Option<Title>,
        subtitles: Subtitles,
        audio_device: &str,
        dialogue: bool,
        on_save: impl Fn() + Send + 'static,
        on_key: impl Fn(Command) + Send + 'static,
        on_exit: impl FnOnce(Finish) + Send + 'static,
    ) -> io::Result<Self> {
        let socket_path = socket_path();
        let _ = std::fs::remove_file(&socket_path);
        let mut child = Process::new("mpv")
            .arg("--fs")
            .arg("--pause")
            .arg("--force-window=immediate")
            .arg("--really-quiet")
            .arg("--no-resume-playback")
            .arg("--no-save-position-on-quit")
            .arg("--keep-open")
            .args(video_args(env::var("SEMMELTV_DECODER").ok().as_deref()))
            .arg(format!("--audio-device={audio_device}"))
            .arg(format!("--af={}", sound_filter(dialogue)))
            .arg("--no-osc")
            .arg(format!("--background-color={}", card::BACKGROUND))
            .arg(format!("--osd-font={OSD_FONT}"))
            .arg(format!("--osd-fonts-dir={OSD_FONTS}"))
            .arg(format!("--input-ipc-server={}", socket_path.display()))
            .args(subtitles.mpv_args(path))
            .args(resume_args(start, tracks))
            .arg("--")
            .arg(path)
            .stdin(Stdio::null())
            .spawn()?;
        subtitles.started(path, sidecar::find(path));
        let playback = Shared::new(Mutex::new(Playback {
            name,
            subtitles,
            tracks,
            billing,
            speed: 1.0,
            on_save: Some(Box::new(on_save)),
            on_key: Some(Box::new(on_key)),
            ..Playback::default()
        }));
        let shared = playback.clone();
        thread::spawn(move || {
            connect(&mut child, &socket_path, &shared, title);
            let finish = Finish::from_exit_code(child.wait().ok().and_then(|status| status.code()));
            shared.lock().unwrap().subtitles.stopped();
            display::show_ui();
            let _ = std::fs::remove_file(&socket_path);
            on_exit(finish);
        });
        Ok(Self { playback })
    }

    pub fn place(&self) -> Place {
        let playback = self.playback.lock().unwrap();
        Place {
            position: playback.position,
            duration: playback.duration,
            credits: playback.chapter_credits.or(playback.credits),
        }
    }

    pub fn tracks(&self) -> Tracks {
        self.playback.lock().unwrap().tracks
    }

    pub fn played(&self) -> f64 {
        self.playback.lock().unwrap().played
    }

    pub fn state(&self) -> State {
        let playback = self.playback.lock().unwrap();
        let (subtitle, audio) = playback.subtitles.chosen();
        State {
            position: playback.scrub.target().unwrap_or(playback.position),
            duration: playback.duration,
            paused: playback.phase.paused(),
            muted: playback.muted,
            speed: playback.speed,
            subtitle,
            subtitle_delay: playback.subtitles.delay(),
            audio,
            started: playback.phase.started(),
            jump: playback.scrub.jump_shown(Instant::now()),
        }
    }

    pub fn command(&self, command: Command) {
        self.playback.lock().unwrap().command(command, Instant::now());
    }

    pub fn seek_to(&self, seconds: f64) {
        let now = Instant::now();
        let mut playback = self.playback.lock().unwrap();
        if playback.phase.playing() {
            let to = seek::within(seconds, playback.duration);
            playback.scrub.cancel();
            playback.seek(Some(Order { to, exact: true }));
            playback.wake(now);
            playback.present(now);
        }
    }

    pub fn seek_by(&self, seconds: f64) {
        let position = self.state().position;
        self.seek_to(position + seconds);
    }

    pub fn set_paused(&self, paused: bool) {
        self.playback
            .lock()
            .unwrap()
            .send(json!(["set_property", "pause", paused]));
    }

    pub fn muted(&self) -> bool {
        self.playback.lock().unwrap().muted
    }

    pub fn set_muted(&self, muted: bool) {
        let mut playback = self.playback.lock().unwrap();
        playback.muted = muted;
        playback.send(json!(["set_property", "mute", muted]));
    }

    pub fn set_audio_device(&self, device: &str) {
        self.playback.lock().unwrap().set_audio_device(device);
    }

    pub fn set_dialogue(&self, on: bool) {
        self.playback
            .lock()
            .unwrap()
            .send(json!(["set_property", "af", sound_filter(on)]));
    }

    pub fn set_speed(&self, speed: f64) -> f64 {
        let speed = speed_within(speed);
        let mut playback = self.playback.lock().unwrap();
        playback.speed = speed;
        playback.send(json!(["set_property", "speed", speed]));
        speed
    }

    pub fn say(&self, text: &str) {
        self.toast("", text, SAY_FOR);
    }

    pub fn hide_toast(&self) {
        self.playback.lock().unwrap().hide_toast();
    }

    pub fn subtitles(&self, change: Change) -> Answer {
        self.playback.lock().unwrap().change_subtitles(change, Instant::now())
    }

    pub fn toast(&self, heading: &str, message: &str, time: Duration) {
        self.playback
            .lock()
            .unwrap()
            .toast(heading, message, Instant::now() + time);
    }
}

fn video_args(decoder: Option<&str>) -> &'static [&'static str] {
    match decoder {
        Some("v4l2m2m") => &["--hwdec=v4l2m2m", "--vo=dmabuf-wayland", "--video-sync=audio"],
        _ => &[
            "--hwdec=drm",
            "--gpu-context=wayland",
            "--profile=fast",
            "--video-sync=display-resample",
        ],
    }
}

fn socket_path() -> PathBuf {
    env::var_os("XDG_RUNTIME_DIR")
        .map_or_else(env::temp_dir, PathBuf::from)
        .join("semmeltv-mpv.sock")
}

fn connect(child: &mut Child, socket_path: &Path, shared: &Shared, title: Option<Title>) {
    while let Ok(None) = child.try_wait() {
        if let Ok(mut stream) = UnixStream::connect(socket_path) {
            for (key, action) in KEYBINDS {
                let _ = send(&mut stream, json!(["keybind", key, action]));
            }
            let _ = send(&mut stream, json!(["observe_property", POSITION_ID, "time-pos"]));
            let _ = send(&mut stream, json!(["observe_property", ENDED_ID, "eof-reached"]));
            let _ = send(
                &mut stream,
                json!(["observe_property", VIDEO_ID, "current-tracks/video"]),
            );
            let _ = send(&mut stream, json!(["observe_property", DURATION_ID, "duration"]));
            let _ = send(&mut stream, json!(["observe_property", PAUSE_ID, "pause"]));
            let _ = send(&mut stream, json!(["observe_property", CHAPTERS_ID, "chapter-list"]));
            for name in subtitles::OBSERVED {
                let _ = send(&mut stream, json!(["observe_property", SUBTITLES_ID, name]));
            }
            let events = stream.try_clone();
            {
                let mut playback = shared.lock().unwrap();
                playback.stream = Some(stream);
                playback.open_card(Instant::now());
            }
            if let Ok(events) = events {
                let ticking = shared.clone();
                thread::spawn(move || tick(&ticking));
                let shared = shared.clone();
                thread::spawn(move || listen(events, &shared, title));
            }
            return;
        }
        thread::sleep(POLL);
    }
}

fn listen(events: UnixStream, shared: &Shared, mut title: Option<Title>) {
    for line in BufReader::new(events).lines().map_while(Result::ok) {
        match Event::parse(&line) {
            Some(Event::Position(position)) => {
                let mut playback = shared.lock().unwrap();
                playback.advance(position);
                playback.update_prompt();
                playback.save_position();
            }
            Some(Event::Duration(duration)) => {
                shared.lock().unwrap().duration = duration;
                if let Some(title) = title.take() {
                    let shared = shared.clone();
                    thread::spawn(move || find_segments(title, duration, &shared));
                }
            }
            Some(Event::Ended) => shared.lock().unwrap().end(),
            Some(Event::Paused(paused)) => {
                let now = Instant::now();
                let mut playback = shared.lock().unwrap();
                playback.pause_changed(paused);
                playback.wake(now);
                playback.present(now);
            }
            Some(Event::Video(video)) => {
                let settled = {
                    let playback = shared.lock().unwrap();
                    playback.phase.started() || playback.matched
                };
                if !settled {
                    let matched = display::match_video(video);
                    shared.lock().unwrap().matched = matched;
                }
            }
            Some(Event::Started) => {
                let mut playback = shared.lock().unwrap();
                playback.close_card(Instant::now());
                if playback.phase == Phase::Opening {
                    playback.send(json!(["set_property", "pause", false]));
                    playback.begin(Instant::now());
                }
            }
            Some(Event::Key(command)) => shared.lock().unwrap().key(command, Instant::now()),
            Some(Event::Loaded) => {
                let mut playback = shared.lock().unwrap();
                for command in playback.subtitles.loaded() {
                    playback.send(command);
                }
            }
            Some(Event::Subtitles(observed)) => {
                let mut playback = shared.lock().unwrap();
                for command in playback.subtitles.observe(observed) {
                    playback.send(command);
                }
                if let Some(tracks) = playback.subtitles.tracks() {
                    playback.tracks = tracks;
                }
            }
            Some(Event::Chapters(credits)) => shared.lock().unwrap().chapter_credits = credits,
            None => {}
        }
    }
    shared.lock().unwrap().closed = true;
}

fn tick(shared: &Shared) {
    loop {
        {
            let mut playback = shared.lock().unwrap();
            if playback.closed {
                return;
            }
            playback.tick(Instant::now());
        }
        thread::sleep(TICK);
    }
}

fn find_segments(title: Title, duration: f64, shared: &Shared) {
    match segments::fetch(title, duration) {
        Ok(found) => {
            let mut playback = shared.lock().unwrap();
            playback.segments = found.segments;
            playback.credits = found.credits;
            playback.update_prompt();
        }
        Err(error) => eprintln!("segments: {error}"),
    }
}

fn send(stream: &mut UnixStream, command: Value) -> io::Result<()> {
    let line = format!("{}\n", json!({ "command": command }));
    stream.write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speeds_stay_between_half_and_double_in_twentieths() {
        assert_eq!(speed_within(1.25), 1.25);
        assert_eq!(speed_within(1.23), 1.25);
        assert_eq!(speed_within(9.0), 2.0);
        assert_eq!(speed_within(0.1), 0.5);
        assert_eq!(speed_within(f64::NAN), 1.0);
        assert_eq!(speed_label(1.0), "1×");
        assert_eq!(speed_label(1.25), "1.25×");
        assert_eq!(speed_label(1.5), "1.5×");
    }
    use std::io::Read;

    #[test]
    fn only_steady_unpaused_playback_counts_as_watched() {
        let mut playback = Playback {
            phase: Phase::Playing { paused: false },
            ..Playback::default()
        };
        for position in [0.5, 1.0, 1.5, 300.0, 300.5, 120.0, 120.4] {
            playback.advance(position);
        }
        playback.pause_changed(true);
        playback.advance(121.0);
        assert!((playback.played - 2.4).abs() < 1e-9);
    }

    #[test]
    fn an_ended_video_quits_as_ended_and_the_exit_code_says_so() {
        let sent = sent_by(|playback| playback.end(), Playback::default());
        assert_eq!(sent, [json!(["quit", ENDED_EXIT_CODE])]);
        let cases = [
            (Some(ENDED_EXIT_CODE), Finish::Ended),
            (Some(STOPPED_EXIT_CODE), Finish::Stopped),
            (Some(0), Finish::Stopped),
            (Some(SIGNALLED_EXIT_CODE), Finish::Stopped),
            (Some(2), Finish::Failed),
            (None, Finish::Failed),
        ];
        for (code, finish) in cases {
            assert_eq!(Finish::from_exit_code(code), finish, "{code:?}");
        }
    }

    #[test]
    fn a_chapter_named_for_the_credits_says_where_they_start() {
        let chapters = |list: Value| {
            let message = json!({"event": "property-change", "id": CHAPTERS_ID, "name": "chapter-list", "data": list});
            Event::parse(&message.to_string())
        };
        let named = json!([
            {"title": "Scene1", "time": 0.0},
            {"title": "EndCredits", "time": 6828.5},
        ]);
        assert_eq!(chapters(named), Some(Event::Chapters(Some(6828.5))));
        let numbered = json!([{"title": "Chapter 1", "time": 0.0}, {"title": "Chapter 2", "time": 600.0}]);
        assert_eq!(chapters(numbered), Some(Event::Chapters(None)));
        assert_eq!(chapters(json!([])), Some(Event::Chapters(None)));
    }

    #[test]
    fn mpv_messages_become_events() {
        let cases = [
            (
                r#"{"event":"property-change","id":1,"name":"time-pos","data":12.5}"#,
                Some(Event::Position(12.5)),
            ),
            (
                r#"{"event":"property-change","id":2,"name":"duration","data":2730.0}"#,
                Some(Event::Duration(2730.0)),
            ),
            (
                r#"{"event":"property-change","id":3,"name":"eof-reached","data":true}"#,
                Some(Event::Ended),
            ),
            (
                r#"{"event":"property-change","id":5,"name":"pause","data":true}"#,
                Some(Event::Paused(true)),
            ),
            (
                r#"{"event":"client-message","args":["semmeltv-select"]}"#,
                Some(Event::Key(Command::Select)),
            ),
            (r#"{"event":"playback-restart"}"#, Some(Event::Started)),
            (r#"{"request_id":0,"error":"success"}"#, None),
            (r#"{"event":"property-change","id":1,"name":"time-pos"}"#, None),
            (
                r#"{"event":"property-change","id":3,"name":"eof-reached","data":false}"#,
                None,
            ),
            (r#"{"event":"client-message","args":["something-else"]}"#, None),
            (r#"{"event":"property-change","id":5,"name":"pause"}"#, None),
            ("not json", None),
        ];
        for (line, event) in cases {
            assert_eq!(Event::parse(line), event, "{line}");
        }
    }

    #[test]
    fn mpv_reports_the_size_and_rate_of_the_video_track() {
        let film = json!({ "event": "property-change", "id": VIDEO_ID, "name": "current-tracks/video",
            "data": { "id": 1, "type": "video", "demux-w": 1920, "demux-h": 800, "demux-fps": 23.976 } });
        let expected = Video {
            width: 1920,
            height: 800,
            fps: 23.976,
        };
        assert_eq!(Event::parse(&film.to_string()), Some(Event::Video(expected)));
        let anamorphic = json!({ "event": "property-change", "id": VIDEO_ID, "name": "current-tracks/video",
            "data": { "demux-w": 720, "demux-h": 576, "demux-par": 1.4222, "demux-fps": 25.0 } });
        let Some(Event::Video(dvd)) = Event::parse(&anamorphic.to_string()) else {
            panic!("a video event");
        };
        assert_eq!((dvd.width, dvd.height), (1024, 576));
        let none = r#"{"event":"property-change","id":4,"name":"current-tracks/video"}"#;
        assert_eq!(Event::parse(none), None);
    }

    #[test]
    fn the_position_is_saved_after_thirty_seconds_of_movement() {
        let cases = [
            (0.0, 29.9, false),
            (0.0, 30.0, true),
            (100.0, 60.0, true),
            (100.0, 80.0, false),
            (Playback::default().saved, 1200.0, true),
        ];
        for (saved, position, due) in cases {
            let saves = Arc::new(Mutex::new(0));
            let counter = saves.clone();
            let mut playback = Playback {
                saved,
                position,
                on_save: Some(Box::new(move || *counter.lock().unwrap() += 1)),
                ..Playback::default()
            };
            playback.save_position();
            playback.save_position();
            assert_eq!(*saves.lock().unwrap(), usize::from(due), "from {saved} to {position}");
            assert_eq!(
                playback.saved,
                if due { position } else { saved },
                "from {saved} to {position}"
            );
        }
    }

    #[test]
    fn mpv_starts_where_the_file_stopped_with_the_tracks_it_had() {
        let tracks = Tracks {
            audio: Some(2),
            subtitle: Some(3),
            subtitles_shown: Some(false),
        };
        assert_eq!(
            resume_args(1234.5, tracks),
            ["--start=1234.500", "--aid=2", "--sid=3", "--sub-visibility=no"]
        );
        assert!(resume_args(0.0, Tracks::default()).is_empty());
    }

    #[test]
    fn nothing_is_saved_after_the_end() {
        let mut playback = Playback {
            position: 100.0,
            phase: Phase::Playing { paused: false },
            ..Playback::default()
        };
        playback.save_position();
        assert_eq!(playback.saved, 100.0);
        playback.end();
        playback.position = 200.0;
        playback.save_position();
        assert_eq!(playback.saved, 100.0);
    }

    #[test]
    fn a_pause_before_the_start_is_mpvs_own_and_the_start_always_plays() {
        let mut playback = Playback::default();
        playback.pause_changed(true);
        assert_eq!(playback.phase, Phase::Opening);
        assert!(playback.phase.paused() && !playback.phase.started());
        playback.begin(Instant::now());
        assert_eq!(playback.phase, Phase::Playing { paused: false });
        playback.pause_changed(true);
        assert_eq!(playback.phase, Phase::Playing { paused: true });
        playback.begin(Instant::now());
        assert_eq!(playback.phase, Phase::Playing { paused: true });
        playback.end();
        playback.pause_changed(false);
        assert_eq!(playback.phase, Phase::Ended);
        assert!(playback.phase.started() && !playback.phase.playing());
    }

    #[test]
    fn the_card_fades_only_once_it_was_shown_and_is_gone_after_the_fade() {
        let now = Instant::now();
        let mut playback = Playback::default();
        playback.close_card(now);
        assert_eq!(playback.card, Card::Gone);
        playback.open_card(now);
        assert_eq!(playback.card, Card::Shown { opened: now });
        let closed = now + Duration::from_secs(5);
        playback.close_card(closed);
        assert_eq!(
            playback.card,
            Card::Fading {
                opened: now,
                since: closed
            }
        );
        playback.close_card(closed + Duration::from_secs(1));
        assert_eq!(
            playback.card,
            Card::Fading {
                opened: now,
                since: closed
            }
        );
        playback.show_card(closed + CARD_FADE);
        assert_eq!(playback.card, Card::Gone);
    }

    #[test]
    fn every_key_binding_message_reaches_a_command() {
        for (_, action) in KEYBINDS {
            if let Some((_, message)) = action.split_once("script-message ") {
                assert!(key_command(message).is_some(), "{message}");
            }
        }
    }

    #[test]
    fn back_and_listen_keys_go_to_the_app_which_decides() {
        let (keys, received) = std::sync::mpsc::channel();
        let playback = Playback {
            on_key: Some(Box::new(move |command| keys.send(command).unwrap())),
            ..playing()
        };
        let sent = sent_by(
            |playback| {
                playback.key(Command::Back, Instant::now());
                playback.key(Command::Listen, Instant::now());
            },
            playback,
        );
        assert!(sent.iter().all(|command| command[0] != "quit"));
        assert_eq!(
            received.try_iter().collect::<Vec<_>>(),
            [Command::Back, Command::Listen]
        );
        let sent = sent_by(|playback| playback.command(Command::Back, Instant::now()), playing());
        assert_eq!(sent[0], json!(["quit", STOPPED_EXIT_CODE]));
    }

    fn sent_by(act: impl FnOnce(&mut Playback), mut playback: Playback) -> Vec<Value> {
        let (stream, mut mpv) = UnixStream::pair().unwrap();
        playback.stream = Some(stream);
        act(&mut playback);
        drop(playback);
        let mut sent = String::new();
        mpv.read_to_string(&mut sent).unwrap();
        sent.lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap()["command"].clone())
            .collect()
    }

    fn playing() -> Playback {
        Playback {
            name: "Sintel".to_string(),
            position: 100.0,
            duration: 5400.0,
            phase: Phase::Playing { paused: false },
            ..Playback::default()
        }
    }

    fn is_overlay(command: &Value) -> bool {
        command["name"] == "osd-overlay" && command["id"] == OVERLAY_ID
    }

    #[test]
    fn each_arrow_seeks_by_its_step_and_draws_the_jump() {
        let cases = [
            (Command::Right, 110.0, "+10s"),
            (Command::Left, 90.0, "-10s"),
            (Command::Up, 160.0, "+60s"),
            (Command::Down, 40.0, "-60s"),
        ];
        for (command, to, jump) in cases {
            let sent = sent_by(|playback| playback.command(command, Instant::now()), playing());
            assert_eq!(sent[0], json!(["seek", to, "absolute+exact"]), "{command:?}");
            assert!(is_overlay(&sent[1]), "{command:?}");
            assert!(sent[1]["data"].as_str().unwrap().contains(jump), "{command:?}");
        }
    }

    #[test]
    fn arrows_do_nothing_before_the_first_frame() {
        let now = Instant::now();
        let sent = sent_by(|playback| playback.command(Command::Right, now), Playback::default());
        assert!(sent.is_empty());
    }

    #[test]
    fn the_overlay_shows_while_paused_or_four_seconds_after_input() {
        type Act = fn(&mut Playback, Instant);
        let cases: [(&str, Phase, Act, &[&str]); 5] = [
            (
                "playing",
                Phase::Playing { paused: false },
                |playback, now| {
                    playback.command(Command::PlayPause, now);
                    playback.tick(now + Duration::from_millis(3900));
                    playback.tick(now + SHOWN_FOR);
                },
                &["ass-events", "none"],
            ),
            (
                "paused",
                Phase::Playing { paused: true },
                |playback, now| {
                    playback.wake(now);
                    playback.tick(now + Duration::from_secs(600));
                    playback.pause_changed(false);
                    playback.tick(now + Duration::from_secs(600));
                },
                &["ass-events", "none"],
            ),
            (
                "opening",
                Phase::Opening,
                |playback, now| {
                    playback.wake(now);
                    playback.tick(now);
                },
                &[],
            ),
            (
                "first frame",
                Phase::Opening,
                |playback, now| {
                    playback.begin(now);
                    playback.tick(now);
                },
                &["ass-events"],
            ),
            (
                "ended",
                Phase::Ended,
                |playback, now| {
                    playback.wake(now);
                    playback.tick(now);
                },
                &[],
            ),
        ];
        for (name, phase, act, expected) in cases {
            let sent = sent_by(
                |playback| act(playback, Instant::now()),
                Playback { phase, ..playing() },
            );
            let overlays: Vec<_> = sent
                .iter()
                .filter(|command| is_overlay(command))
                .map(|command| command["format"].as_str().unwrap())
                .collect();
            assert_eq!(overlays, expected, "{name}");
            if expected.is_empty() {
                assert!(sent.is_empty(), "{name}");
            }
        }
    }

    #[test]
    fn a_seek_settles_on_its_exact_target_from_the_tick() {
        let now = Instant::now();
        let sent = sent_by(
            |playback| {
                playback.command(Command::Right, now);
                playback.command(Command::Right, now + Duration::from_millis(200));
                playback.tick(now + Duration::from_millis(300));
                playback.tick(now + Duration::from_millis(900));
                assert_eq!(playback.position, 120.0);
            },
            playing(),
        );
        let seeks: Vec<_> = sent.iter().filter(|command| command[0] == "seek").collect();
        assert_eq!(
            seeks,
            [
                &json!(["seek", 110.0, "absolute+exact"]),
                &json!(["seek", 120.0, "absolute+keyframes"]),
                &json!(["seek", 120.0, "absolute+exact"]),
            ]
        );
    }

    #[test]
    fn ok_skips_a_section_even_right_after_an_arrow() {
        let now = Instant::now();
        let during = Playback {
            phase: Phase::Playing { paused: false },
            duration: 2700.0,
            ..during_the_intro()
        };
        let sent = sent_by(
            |playback| {
                playback.command(Command::Right, now);
                playback.position = 15.0;
                playback.command(Command::Select, now + Duration::from_millis(100));
                playback.tick(now + Duration::from_secs(2));
            },
            during,
        );
        let seeks: Vec<_> = sent.iter().filter(|command| command[0] == "seek").collect();
        assert_eq!(seeks.last(), Some(&&json!(["seek", 20.0, "absolute"])));
    }

    #[test]
    fn the_subtitle_key_toggles_and_says_what_it_did() {
        let subtitles = Subtitles::default();
        subtitles.started(Path::new("/lib/Movies/Up/Up.mkv"), Vec::new());
        let tracks = json!({ "event": "property-change", "id": SUBTITLES_ID, "name": "track-list",
            "data": [{ "id": 1, "type": "sub", "lang": "en" }] });
        let Some(Event::Subtitles(observed)) = Event::parse(&tracks.to_string()) else {
            panic!("a subtitle event");
        };
        subtitles.observe(observed);
        let playback = Playback {
            subtitles: subtitles.clone(),
            ..playing()
        };
        let sent = sent_by(
            |playback| playback.command(Command::Subtitles, Instant::now()),
            playback,
        );
        assert_eq!(sent[0], json!(["set_property", "sid", 1]));
        assert_eq!(sent[1], json!(["set_property", "sub-visibility", true]));
        assert!(sent[2]["data"].as_str().unwrap().contains("Subtitles: English"));
        assert!(!sent.iter().any(is_overlay));
        let sent = sent_by(
            |playback| playback.command(Command::Subtitles, Instant::now()),
            playing(),
        );
        assert!(sent[0]["data"].as_str().unwrap().contains("No subtitles in this file"));
    }

    #[test]
    fn moving_the_sound_selects_the_last_audio_track_again() {
        let subtitles = Subtitles::default();
        subtitles.started(Path::new("/lib/Movies/Up/Up.mkv"), Vec::new());
        subtitles.observe(Observed::Audio(Some(2)));
        subtitles.observe(Observed::Audio(None));
        let playback = Playback { subtitles, ..playing() };
        let sent = sent_by(|playback| playback.set_audio_device("auto"), playback);
        assert_eq!(
            sent,
            [
                json!(["set_property", "audio-device", "auto"]),
                json!(["set_property", "aid", 2])
            ]
        );
    }

    fn during_the_intro() -> Playback {
        Playback {
            position: 15.0,
            segments: vec![Segment {
                kind: Kind::Intro,
                start: 10.0,
                end: 20.0,
            }],
            ..Playback::default()
        }
    }

    #[test]
    fn ok_skips_while_a_prompt_shows_and_pauses_otherwise() {
        let mut playback = during_the_intro();
        assert_eq!(playback.select(), json!(["seek", 20.0, "absolute"]));
        playback.position = 20.0;
        assert_eq!(playback.select(), json!(["cycle", "pause"]));
    }

    #[test]
    fn the_prompt_shows_the_section_while_the_position_is_inside_it() {
        for kind in [Kind::Recap, Kind::Intro] {
            let playback = Playback {
                position: 5.0,
                segments: vec![Segment {
                    kind,
                    start: 10.0,
                    end: 20.0,
                }],
                ..Playback::default()
            };
            let sent = sent_by(
                |playback| {
                    for position in [5.0, 10.0, 19.9, 20.0, 25.0] {
                        playback.position = position;
                        playback.update_prompt();
                    }
                },
                playback,
            );
            assert_eq!(sent.len(), 2, "{kind:?}");
            assert_eq!(sent[0]["format"], "ass-events", "{kind:?}");
            assert!(sent[0]["data"].as_str().unwrap().ends_with(kind.label()), "{kind:?}");
            assert_eq!(sent[1]["format"], "none", "{kind:?}");
        }
    }
}
