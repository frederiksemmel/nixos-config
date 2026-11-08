use crate::command::Command;
use crate::display::{self, Mode};
use crate::overlay::{self, View};
use crate::progress::Place;
use crate::seek::{Direction, Order, Scrub};
use crate::segments::{self, Kind, Segment, Title};
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
const FRAME_RATE_ID: u64 = 4;
const PAUSE_ID: u64 = 5;
const PROMPT_ID: u64 = 1;
const LOADING_ID: u64 = 2;
const OVERLAY_ID: u64 = 3;
const TOAST_ID: u64 = 4;
const LOADING_TEXT: &str = r"{\an5\pos(960,540)\bord0\shad0\b1\fs64\1c&HFFFFFF&}Loading…";
const KEYBINDS: [(&str, &str); 10] = [
    ("ESC", "script-message semmeltv-back"),
    ("BS", "script-message semmeltv-back"),
    ("v", "script-message semmeltv-listen"),
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
    FrameRate(f64),
    Paused(bool),
    Key(Command),
    Started,
}

impl Event {
    fn parse(line: &str) -> Option<Self> {
        let message: Value = serde_json::from_str(line).ok()?;
        match message["event"].as_str()? {
            "property-change" => match message["id"].as_u64()? {
                POSITION_ID => Some(Self::Position(message["data"].as_f64()?)),
                DURATION_ID => Some(Self::Duration(message["data"].as_f64()?)),
                FRAME_RATE_ID => Some(Self::FrameRate(message["data"].as_f64()?)),
                ENDED_ID => message["data"].as_bool()?.then_some(Self::Ended),
                PAUSE_ID => Some(Self::Paused(message["data"].as_bool()?)),
                _ => None,
            },
            "client-message" => key_command(message["args"][0].as_str()?).map(Self::Key),
            "playback-restart" => Some(Self::Started),
            _ => None,
        }
    }
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
        _ => None,
    }
}

#[derive(Default)]
struct Playback {
    stream: Option<UnixStream>,
    name: String,
    position: f64,
    duration: f64,
    played: f64,
    saved: f64,
    started: bool,
    paused: bool,
    muted: bool,
    ended: bool,
    closed: bool,
    segments: Vec<Segment>,
    prompt: Option<Kind>,
    display_mode: Option<Mode>,
    scrub: Scrub,
    shown_until: Option<Instant>,
    drawn: String,
    toast_until: Option<Instant>,
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
            Command::Back => self.send(json!(["quit-watch-later", STOPPED_EXIT_CODE])),
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
            Command::Listen => {}
        }
        self.wake(now);
        self.present(now);
    }

    fn step(&mut self, direction: Direction, now: Instant) {
        if self.started {
            let order = self.scrub.step(direction, self.position, self.duration, now);
            self.seek(order);
        }
    }

    fn jump(&mut self, seconds: i32, now: Instant) {
        if self.started {
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
        if self.started && !self.paused && step > 0.0 && step <= PLAY_STEP {
            self.played += step;
        }
        self.position = position;
    }

    fn begin(&mut self, now: Instant) {
        if !self.started {
            self.started = true;
            self.paused = false;
            self.wake(now);
        }
    }

    fn wake(&mut self, now: Instant) {
        self.shown_until = Some(now + SHOWN_FOR);
    }

    fn key(&mut self, command: Command, now: Instant) {
        match (command, &self.on_key) {
            (Command::Back | Command::Listen, Some(on_key)) => on_key(command),
            _ => self.command(command, now),
        }
    }

    fn toast(&mut self, heading: &str, message: &str, until: Instant) {
        self.toast_until = Some(until);
        self.send(toast_command(&overlay::toast(heading, message)));
    }

    fn hide_toast(&mut self) {
        if self.toast_until.take().is_some() {
            self.send(toast_command(""));
        }
    }

    fn tick(&mut self, now: Instant) {
        if self.toast_until.is_some_and(|until| now >= until) {
            self.hide_toast();
        }
        if !self.ended {
            let order = self.scrub.poll(now);
            self.seek(order);
            self.present(now);
        }
    }

    fn shown(&self, now: Instant) -> bool {
        self.started && (self.paused || self.shown_until.is_some_and(|until| now < until))
    }

    fn view(&self, now: Instant) -> View<'_> {
        View {
            title: &self.name,
            position: self.scrub.target().unwrap_or(self.position),
            duration: self.duration,
            paused: self.paused,
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
        if !self.ended && save_due(self.saved, self.position) {
            self.saved = self.position;
            self.send(json!(["write-watch-later-config"]));
            if let Some(on_save) = &self.on_save {
                on_save();
            }
        }
    }

    fn end(&mut self) {
        self.ended = true;
        self.send(json!(["set_property", "save-position-on-quit", false]));
        self.send(json!(["delete-watch-later-config"]));
        self.send(json!(["quit", ENDED_EXIT_CODE]));
    }
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

fn loading_command(shown: bool) -> Value {
    let (format, data) = if shown {
        ("ass-events", LOADING_TEXT)
    } else {
        ("none", "")
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
    pub started: bool,
}

impl Player {
    pub fn start(
        path: &Path,
        watch_later_dir: &Path,
        name: String,
        title: Option<Title>,
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
            .arg("--save-position-on-quit")
            .arg("--ignore-path-in-watch-later-config")
            .arg(format!("--watch-later-dir={}", watch_later_dir.display()))
            .arg("--keep-open")
            .arg("--hwdec=drm")
            .arg("--gpu-context=wayland")
            .arg("--profile=fast")
            .arg("--no-osc")
            .arg(format!("--osd-font={OSD_FONT}"))
            .arg(format!("--osd-fonts-dir={OSD_FONTS}"))
            .arg(format!("--input-ipc-server={}", socket_path.display()))
            .arg("--")
            .arg(path)
            .stdin(Stdio::null())
            .spawn()?;
        let playback = Shared::new(Mutex::new(Playback {
            name,
            on_save: Some(Box::new(on_save)),
            on_key: Some(Box::new(on_key)),
            ..Playback::default()
        }));
        let shared = playback.clone();
        thread::spawn(move || {
            connect(&mut child, &socket_path, &shared, title);
            let finish = Finish::from_exit_code(child.wait().ok().and_then(|status| status.code()));
            if let Some(mode) = shared.lock().unwrap().display_mode.take() {
                mode.apply();
            }
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
        }
    }

    pub fn played(&self) -> f64 {
        self.playback.lock().unwrap().played
    }

    pub fn state(&self) -> State {
        let playback = self.playback.lock().unwrap();
        State {
            position: playback.scrub.target().unwrap_or(playback.position),
            duration: playback.duration,
            paused: playback.paused,
            muted: playback.muted,
            started: playback.started,
        }
    }

    pub fn command(&self, command: Command) {
        self.playback.lock().unwrap().command(command, Instant::now());
    }

    pub fn seek_to(&self, seconds: f64) {
        let now = Instant::now();
        let mut playback = self.playback.lock().unwrap();
        if playback.started {
            let to = seconds.clamp(0.0, playback.duration.max(0.0));
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

    pub fn say(&self, text: &str) {
        self.toast("", text, SAY_FOR);
    }

    pub fn hide_toast(&self) {
        self.playback.lock().unwrap().hide_toast();
    }

    pub fn toast(&self, heading: &str, message: &str, time: Duration) {
        self.playback
            .lock()
            .unwrap()
            .toast(heading, message, Instant::now() + time);
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
            let _ = send(&mut stream, loading_command(true));
            let _ = send(&mut stream, json!(["observe_property", FRAME_RATE_ID, "container-fps"]));
            let _ = send(&mut stream, json!(["observe_property", DURATION_ID, "duration"]));
            let _ = send(&mut stream, json!(["observe_property", PAUSE_ID, "pause"]));
            let events = stream.try_clone();
            shared.lock().unwrap().stream = Some(stream);
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
                playback.paused = paused;
                playback.wake(now);
                playback.present(now);
            }
            Some(Event::FrameRate(fps)) => {
                let matched = {
                    let playback = shared.lock().unwrap();
                    playback.started || playback.display_mode.is_some()
                };
                if !matched {
                    let previous = display::match_frame_rate(fps);
                    shared.lock().unwrap().display_mode = previous;
                }
            }
            Some(Event::Started) => {
                let mut playback = shared.lock().unwrap();
                playback.send(loading_command(false));
                if !playback.started {
                    playback.send(json!(["set_property", "pause", false]));
                    playback.begin(Instant::now());
                }
            }
            Some(Event::Key(command)) => shared.lock().unwrap().key(command, Instant::now()),
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
            playback.segments = found;
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
    use std::io::Read;

    #[test]
    fn only_steady_unpaused_playback_counts_as_watched() {
        let mut playback = Playback {
            started: true,
            ..Playback::default()
        };
        for position in [0.5, 1.0, 1.5, 300.0, 300.5, 120.0, 120.4] {
            playback.advance(position);
        }
        playback.paused = true;
        playback.advance(121.0);
        assert!((playback.played - 2.4).abs() < 1e-9);
    }

    #[test]
    fn the_exit_code_tells_an_ended_video_from_a_stopped_or_broken_one() {
        assert_eq!(Finish::from_exit_code(Some(ENDED_EXIT_CODE)), Finish::Ended);
        assert_eq!(Finish::from_exit_code(Some(STOPPED_EXIT_CODE)), Finish::Stopped);
        assert_eq!(Finish::from_exit_code(Some(0)), Finish::Stopped);
        assert_eq!(Finish::from_exit_code(Some(SIGNALLED_EXIT_CODE)), Finish::Stopped);
        assert_eq!(Finish::from_exit_code(Some(2)), Finish::Failed);
        assert_eq!(Finish::from_exit_code(None), Finish::Failed);
    }

    #[test]
    fn mpv_reports_the_position_the_duration_the_end_and_a_pressed_key() {
        let position = r#"{"event":"property-change","id":1,"name":"time-pos","data":12.5}"#;
        let duration = r#"{"event":"property-change","id":2,"name":"duration","data":2730.0}"#;
        let end = r#"{"event":"property-change","id":3,"name":"eof-reached","data":true}"#;
        let pause = r#"{"event":"property-change","id":5,"name":"pause","data":true}"#;
        let key = r#"{"event":"client-message","args":["semmeltv-select"]}"#;
        assert_eq!(Event::parse(position), Some(Event::Position(12.5)));
        assert_eq!(Event::parse(duration), Some(Event::Duration(2730.0)));
        assert_eq!(Event::parse(end), Some(Event::Ended));
        let frame_rate = r#"{"event":"property-change","id":4,"name":"container-fps","data":23.976}"#;
        assert_eq!(Event::parse(frame_rate), Some(Event::FrameRate(23.976)));
        assert_eq!(Event::parse(pause), Some(Event::Paused(true)));
        assert_eq!(Event::parse(key), Some(Event::Key(Command::Select)));
    }

    #[test]
    fn the_first_frame_clears_the_loading_screen() {
        assert_eq!(Event::parse(r#"{"event":"playback-restart"}"#), Some(Event::Started));
    }

    #[test]
    fn other_mpv_messages_are_ignored() {
        for line in [
            r#"{"request_id":0,"error":"success"}"#,
            r#"{"event":"property-change","id":1,"name":"time-pos"}"#,
            r#"{"event":"property-change","id":3,"name":"eof-reached","data":false}"#,
            r#"{"event":"client-message","args":["something-else"]}"#,
            r#"{"event":"property-change","id":5,"name":"pause"}"#,
            "not json",
        ] {
            assert_eq!(Event::parse(line), None, "{line}");
        }
    }

    #[test]
    fn the_position_is_saved_once_it_moved_thirty_seconds_either_way() {
        assert!(!save_due(0.0, 29.9));
        assert!(save_due(0.0, 30.0));
        assert!(save_due(100.0, 60.0));
        assert!(!save_due(100.0, 80.0));
    }

    #[test]
    fn a_resumed_video_saves_its_position_at_once() {
        assert!(save_due(Playback::default().saved, 1200.0));
    }

    #[test]
    fn a_save_is_announced_once() {
        let saves = Arc::new(Mutex::new(0));
        let counter = saves.clone();
        let mut playback = Playback {
            position: 100.0,
            on_save: Some(Box::new(move || *counter.lock().unwrap() += 1)),
            ..Playback::default()
        };
        playback.save_position();
        playback.save_position();
        assert_eq!(*saves.lock().unwrap(), 1);
    }

    #[test]
    fn nothing_is_saved_after_the_end() {
        let mut playback = Playback {
            position: 100.0,
            ended: true,
            ..Playback::default()
        };
        playback.save_position();
        assert_eq!(playback.saved, 0.0);
        playback.ended = false;
        playback.save_position();
        assert_eq!(playback.saved, 100.0);
    }

    #[test]
    fn the_end_forgets_the_position_before_quitting() {
        let (stream, mut mpv) = UnixStream::pair().unwrap();
        let mut playback = Playback {
            stream: Some(stream),
            ..Playback::default()
        };
        playback.end();
        drop(playback);
        let mut sent = String::new();
        mpv.read_to_string(&mut sent).unwrap();
        let commands: Vec<Value> = sent
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap()["command"].clone())
            .collect();
        assert_eq!(
            commands,
            [
                json!(["set_property", "save-position-on-quit", false]),
                json!(["delete-watch-later-config"]),
                json!(["quit", ENDED_EXIT_CODE]),
            ]
        );
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
        assert!(sent.iter().all(|command| command[0] != "quit-watch-later"));
        assert_eq!(
            received.try_iter().collect::<Vec<_>>(),
            [Command::Back, Command::Listen]
        );
        let sent = sent_by(|playback| playback.command(Command::Back, Instant::now()), playing());
        assert_eq!(sent[0], json!(["quit-watch-later", STOPPED_EXIT_CODE]));
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
            started: true,
            ..Playback::default()
        }
    }

    fn is_overlay(command: &Value) -> bool {
        command["name"] == "osd-overlay" && command["id"] == OVERLAY_ID
    }

    #[test]
    fn an_arrow_seeks_and_draws_the_target_and_the_jump() {
        let now = Instant::now();
        let sent = sent_by(|playback| playback.command(Command::Right, now), playing());
        assert_eq!(sent[0], json!(["seek", 110.0, "absolute+exact"]));
        assert!(is_overlay(&sent[1]));
        assert!(sent[1]["data"].as_str().unwrap().contains("+10s"));
    }

    #[test]
    fn up_and_down_jump_a_minute() {
        let now = Instant::now();
        let up = sent_by(|playback| playback.command(Command::Up, now), playing());
        assert_eq!(up[0], json!(["seek", 160.0, "absolute+exact"]));
        let down = sent_by(|playback| playback.command(Command::Down, now), playing());
        assert_eq!(down[0], json!(["seek", 40.0, "absolute+exact"]));
    }

    #[test]
    fn arrows_do_nothing_before_the_first_frame() {
        let now = Instant::now();
        let sent = sent_by(|playback| playback.command(Command::Right, now), Playback::default());
        assert!(sent.is_empty());
    }

    #[test]
    fn the_overlay_hides_after_four_seconds_without_input() {
        let now = Instant::now();
        let sent = sent_by(
            |playback| {
                playback.command(Command::PlayPause, now);
                playback.tick(now + Duration::from_millis(3900));
                playback.tick(now + SHOWN_FOR);
            },
            playing(),
        );
        let overlays: Vec<_> = sent.iter().filter(|command| is_overlay(command)).collect();
        assert_eq!(overlays.len(), 2);
        assert_eq!(overlays[0]["format"], "ass-events");
        assert_eq!(overlays[1]["format"], "none");
    }

    #[test]
    fn the_overlay_stays_while_paused() {
        let now = Instant::now();
        let mut playback = playing();
        playback.paused = true;
        playback.wake(now);
        assert!(playback.shown(now + Duration::from_secs(600)));
        playback.paused = false;
        assert!(!playback.shown(now + Duration::from_secs(600)));
    }

    #[test]
    fn nothing_is_drawn_before_playback_starts() {
        let now = Instant::now();
        let mut playback = Playback::default();
        playback.wake(now);
        assert!(!playback.shown(now));
        playback.begin(now);
        assert!(playback.shown(now));
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
    fn the_tick_sends_nothing_after_the_end() {
        let now = Instant::now();
        let mut ended = playing();
        ended.ended = true;
        ended.wake(now);
        assert!(sent_by(|playback| playback.tick(now), ended).is_empty());
    }

    #[test]
    fn ok_skips_a_section_even_right_after_an_arrow() {
        let now = Instant::now();
        let during = Playback {
            started: true,
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
    fn the_prompt_follows_the_position() {
        let mut playback = during_the_intro();
        playback.update_prompt();
        assert_eq!(playback.prompt, Some(Kind::Intro));
        playback.position = 25.0;
        playback.update_prompt();
        assert_eq!(playback.prompt, None);
    }

    #[test]
    fn the_prompt_names_the_section_and_can_be_removed() {
        let shown = prompt_command(Some(Kind::Recap));
        assert_eq!(shown["format"], "ass-events");
        assert!(shown["data"].as_str().unwrap().ends_with("Skip recap"));
        assert_eq!(prompt_command(None)["format"], "none");
    }
}
