use crate::command::Command;
use serde_json::{Value, json};
use std::env;
use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Process, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

const POLL: Duration = Duration::from_millis(50);
const STOP: &str = "quit-watch-later 10";
const STOPPED_EXIT_CODE: i32 = 10;
const KEYBINDS: [(&str, &str); 7] = [
    ("ESC", STOP),
    ("BS", STOP),
    ("ENTER", "cycle pause"),
    ("KP_ENTER", "cycle pause"),
    ("SPACE", "cycle pause"),
    ("LEFT", "seek -10"),
    ("RIGHT", "seek 10"),
];

type Socket = Arc<Mutex<Option<UnixStream>>>;

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

pub struct Player {
    socket: Socket,
}

impl Player {
    pub fn start(path: &Path, on_exit: impl FnOnce(Finish) + Send + 'static) -> io::Result<Self> {
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
        let socket = Socket::default();
        let shared = socket.clone();
        thread::spawn(move || {
            connect(&mut child, &socket_path, &shared);
            let finish = Finish::from_exit_code(child.wait().ok().and_then(|status| status.code()));
            let _ = std::fs::remove_file(&socket_path);
            on_exit(finish);
        });
        Ok(Self { socket })
    }

    pub fn command(&self, command: Command) {
        let message = match command {
            Command::Back => json!(["quit-watch-later", STOPPED_EXIT_CODE]),
            Command::Select | Command::PlayPause => json!(["cycle", "pause"]),
            Command::Left => json!(["seek", -10]),
            Command::Right => json!(["seek", 10]),
            Command::Up => json!(["seek", 60]),
            Command::Down => json!(["seek", -60]),
        };
        if let Some(stream) = self.socket.lock().unwrap().as_mut() {
            let _ = send(stream, message);
        }
    }
}

fn socket_path() -> PathBuf {
    env::var_os("XDG_RUNTIME_DIR")
        .map_or_else(env::temp_dir, PathBuf::from)
        .join("semmeltv-mpv.sock")
}

fn connect(child: &mut Child, socket_path: &Path, shared: &Socket) {
    while let Ok(None) = child.try_wait() {
        if let Ok(mut stream) = UnixStream::connect(socket_path) {
            for (key, action) in KEYBINDS {
                let _ = send(&mut stream, json!(["keybind", key, action]));
            }
            *shared.lock().unwrap() = Some(stream);
            return;
        }
        thread::sleep(POLL);
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
}
