use crate::command::Command;
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
const SELECT_MESSAGE: &str = "semmeltv-select";
const SELECT: &str = "script-message semmeltv-select";
const POSITION_ID: u64 = 1;
const DURATION_ID: u64 = 2;
const PROMPT_ID: u64 = 1;
const PROMPT_WIDTH: u32 = 1920;
const PROMPT_HEIGHT: u32 = 1080;
const PROMPT_PILL: &str =
    "m 42 0 l 258 0 b 281 0 300 19 300 42 b 300 65 281 84 258 84 l 42 84 b 19 84 0 65 0 42 b 0 19 19 0 42 0";
const KEYBINDS: [(&str, &str); 7] = [
    ("ESC", STOP),
    ("BS", STOP),
    ("ENTER", SELECT),
    ("KP_ENTER", SELECT),
    ("SPACE", "cycle pause"),
    ("LEFT", "seek -10"),
    ("RIGHT", "seek 10"),
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
            Some(0) => Self::Ended,
            Some(STOPPED_EXIT_CODE) => Self::Stopped,
            _ => Self::Failed,
        }
    }
}

#[derive(Debug, PartialEq)]
enum Event {
    Position(f64),
    Duration(f64),
    Select,
}

impl Event {
    fn parse(line: &str) -> Option<Self> {
        let message: Value = serde_json::from_str(line).ok()?;
        match message["event"].as_str()? {
            "property-change" => {
                let value = message["data"].as_f64()?;
                match message["id"].as_u64()? {
                    POSITION_ID => Some(Self::Position(value)),
                    DURATION_ID => Some(Self::Duration(value)),
                    _ => None,
                }
            }
            "client-message" => (message["args"][0].as_str()? == SELECT_MESSAGE).then_some(Self::Select),
            _ => None,
        }
    }
}

#[derive(Default)]
struct Playback {
    stream: Option<UnixStream>,
    position: f64,
    segments: Vec<Segment>,
    prompt: Option<Kind>,
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

pub struct Player {
    playback: Shared,
}

impl Player {
    pub fn start(path: &Path, title: Option<Title>, on_exit: impl FnOnce(Finish) + Send + 'static) -> io::Result<Self> {
        let socket_path = socket_path();
        let _ = std::fs::remove_file(&socket_path);
        let mut child = Process::new("mpv")
            .arg("--fs")
            .arg("--force-window=immediate")
            .arg("--no-terminal")
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
            Command::PlayPause => json!(["cycle", "pause"]),
            Command::Left => json!(["seek", -10]),
            Command::Right => json!(["seek", 10]),
            Command::Up => json!(["seek", 60]),
            Command::Down => json!(["seek", -60]),
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
            if title.is_some() {
                let _ = send(&mut stream, json!(["observe_property", POSITION_ID, "time-pos"]));
                let _ = send(&mut stream, json!(["observe_property", DURATION_ID, "duration"]));
            }
            if let Ok(events) = stream.try_clone() {
                let shared = shared.clone();
                thread::spawn(move || listen(events, &shared, title));
            }
            shared.lock().unwrap().stream = Some(stream);
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
            }
            Some(Event::Duration(duration)) => {
                if let Some(title) = title.take() {
                    let shared = shared.clone();
                    thread::spawn(move || find_segments(title, duration, &shared));
                }
            }
            Some(Event::Select) => {
                let mut playback = shared.lock().unwrap();
                let message = playback.select();
                playback.send(message);
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

    #[test]
    fn the_exit_code_tells_an_ended_video_from_a_stopped_or_broken_one() {
        assert_eq!(Finish::from_exit_code(Some(0)), Finish::Ended);
        assert_eq!(Finish::from_exit_code(Some(STOPPED_EXIT_CODE)), Finish::Stopped);
        assert_eq!(Finish::from_exit_code(Some(2)), Finish::Failed);
        assert_eq!(Finish::from_exit_code(None), Finish::Failed);
    }

    #[test]
    fn mpv_reports_the_position_the_duration_and_a_pressed_key() {
        let position = r#"{"event":"property-change","id":1,"name":"time-pos","data":12.5}"#;
        let duration = r#"{"event":"property-change","id":2,"name":"duration","data":2730.0}"#;
        let key = r#"{"event":"client-message","args":["semmeltv-select"]}"#;
        assert_eq!(Event::parse(position), Some(Event::Position(12.5)));
        assert_eq!(Event::parse(duration), Some(Event::Duration(2730.0)));
        assert_eq!(Event::parse(key), Some(Event::Select));
    }

    #[test]
    fn other_mpv_messages_are_ignored() {
        for line in [
            r#"{"request_id":0,"error":"success"}"#,
            r#"{"event":"property-change","id":1,"name":"time-pos"}"#,
            r#"{"event":"client-message","args":["something-else"]}"#,
            r#"{"event":"playback-restart"}"#,
            "not json",
        ] {
            assert_eq!(Event::parse(line), None, "{line}");
        }
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
