use crate::command::Command;
use crate::display::{self, Mode};
use crate::segments::{self, Kind, Segment, Title};
use serde_json::{Value, json};
use std::env;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Process, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const POLL: Duration = Duration::from_millis(50);
const STOP: &str = "quit-watch-later 10";
const STOPPED_EXIT_CODE: i32 = 10;
const ENDED_EXIT_CODE: i32 = 11;
const SIGNALLED_EXIT_CODE: i32 = 4;
const SAVE_EVERY: f64 = 30.0;
const SELECT_MESSAGE: &str = "semmeltv-select";
const SELECT: &str = "script-message semmeltv-select";
const POSITION_ID: u64 = 1;
const DURATION_ID: u64 = 2;
const ENDED_ID: u64 = 3;
const FRAME_RATE_ID: u64 = 4;
const PROMPT_ID: u64 = 1;
const LOADING_ID: u64 = 2;
const PROMPT_WIDTH: u32 = 1920;
const PROMPT_HEIGHT: u32 = 1080;
const PROMPT_PILL: &str =
    "m 42 0 l 258 0 b 281 0 300 19 300 42 b 300 65 281 84 258 84 l 42 84 b 19 84 0 65 0 42 b 0 19 19 0 42 0";
const LOADING_TEXT: &str = r"{\an5\pos(960,540)\bord0\shad0\b1\fs64\1c&HFFFFFF&}Loading…";
const KEYBINDS: [(&str, &str); 9] = [
    ("ESC", STOP),
    ("BS", STOP),
    ("ENTER", SELECT),
    ("KP_ENTER", SELECT),
    ("SPACE", "cycle pause"),
    ("LEFT", "seek -10"),
    ("RIGHT", "seek 10"),
    ("UP", "seek 60"),
    ("DOWN", "seek -60"),
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
    Select,
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
                _ => None,
            },
            "client-message" => (message["args"][0].as_str()? == SELECT_MESSAGE).then_some(Self::Select),
            "playback-restart" => Some(Self::Started),
            _ => None,
        }
    }
}

#[derive(Default)]
struct Playback {
    stream: Option<UnixStream>,
    position: f64,
    saved: f64,
    ended: bool,
    segments: Vec<Segment>,
    prompt: Option<Kind>,
    started: bool,
    display_mode: Option<Mode>,
}

impl Playback {
    fn send(&mut self, command: Value) {
        if let Some(stream) = self.stream.as_mut() {
            let _ = send(stream, command);
        }
    }

    fn select(&self) -> Value {
        match segments::at(&self.segments, self.position) {
            Some(segment) => json!(["seek", segment.end, "absolute"]),
            None => json!(["keypress", "SPACE"]),
        }
    }

    fn update_prompt(&mut self) {
        let prompt = segments::at(&self.segments, self.position).map(|segment| segment.kind);
        if prompt != self.prompt {
            self.prompt = prompt;
            self.send(prompt_command(prompt));
        }
    }

    fn save_position(&mut self) {
        if !self.ended && save_due(self.saved, self.position) {
            self.saved = self.position;
            self.send(json!(["write-watch-later-config"]));
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

fn prompt_command(prompt: Option<Kind>) -> Value {
    match prompt {
        Some(kind) => {
            let pill = format!(r"{{\an7\pos(1560,930)\bord0\shad0\1c&HFFFFFF&\p1}}{PROMPT_PILL}{{\p0}}");
            let label = format!(
                r"{{\an5\pos(1710,972)\bord0\shad0\b1\fs44\1c&H120D0B&}}{}",
                kind.label()
            );
            json!({
                "name": "osd-overlay",
                "id": PROMPT_ID,
                "format": "ass-events",
                "data": format!("{pill}\n{label}"),
                "res_x": PROMPT_WIDTH,
                "res_y": PROMPT_HEIGHT,
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
        "res_x": PROMPT_WIDTH,
        "res_y": PROMPT_HEIGHT,
    })
}

pub struct Player {
    playback: Shared,
}

impl Player {
    pub fn start(path: &Path, title: Option<Title>, on_exit: impl FnOnce(Finish) + Send + 'static) -> io::Result<Self> {
        let socket_path = socket_path();
        let _ = std::fs::remove_file(&socket_path);
        let mut child = Process::new("mpv")
            .arg("--fs")
            .arg("--pause")
            .arg("--force-window=immediate")
            .arg("--really-quiet")
            .arg("--save-position-on-quit")
            .arg("--keep-open")
            .arg("--hwdec=drm")
            .arg("--gpu-context=wayland")
            .arg("--profile=fast")
            .arg(format!("--input-ipc-server={}", socket_path.display()))
            .arg("--")
            .arg(path)
            .stdin(Stdio::null())
            .spawn()?;
        let playback = Shared::default();
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

    pub fn command(&self, command: Command) {
        let mut playback = self.playback.lock().unwrap();
        let message = match command {
            Command::Back => json!(["quit-watch-later", STOPPED_EXIT_CODE]),
            Command::Select => playback.select(),
            Command::PlayPause => json!(["keypress", "SPACE"]),
            Command::Left => json!(["keypress", "LEFT"]),
            Command::Right => json!(["keypress", "RIGHT"]),
            Command::Up => json!(["keypress", "UP"]),
            Command::Down => json!(["keypress", "DOWN"]),
        };
        playback.send(message);
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
            if title.is_some() {
                let _ = send(&mut stream, json!(["observe_property", DURATION_ID, "duration"]));
            }
            let events = stream.try_clone();
            shared.lock().unwrap().stream = Some(stream);
            if let Ok(events) = events {
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
                playback.position = position;
                playback.update_prompt();
                playback.save_position();
            }
            Some(Event::Duration(duration)) => {
                if let Some(title) = title.take() {
                    let shared = shared.clone();
                    thread::spawn(move || find_segments(title, duration, &shared));
                }
            }
            Some(Event::Ended) => shared.lock().unwrap().end(),
            Some(Event::Select) => {
                let mut playback = shared.lock().unwrap();
                let message = playback.select();
                playback.send(message);
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
                    playback.started = true;
                    playback.send(json!(["set_property", "pause", false]));
                }
            }
            None => {}
        }
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
    writeln!(stream, "{}", json!({ "command": command }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

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
        let key = r#"{"event":"client-message","args":["semmeltv-select"]}"#;
        assert_eq!(Event::parse(position), Some(Event::Position(12.5)));
        assert_eq!(Event::parse(duration), Some(Event::Duration(2730.0)));
        assert_eq!(Event::parse(end), Some(Event::Ended));
        let frame_rate = r#"{"event":"property-change","id":4,"name":"container-fps","data":23.976}"#;
        assert_eq!(Event::parse(frame_rate), Some(Event::FrameRate(23.976)));
        assert_eq!(Event::parse(key), Some(Event::Select));
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
        assert_eq!(playback.select(), json!(["keypress", "SPACE"]));
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
