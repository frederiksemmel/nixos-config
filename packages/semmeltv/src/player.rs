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
const KEYBINDS: [(&str, &str); 7] = [
    ("ESC", "quit"),
    ("BS", "quit"),
    ("ENTER", "cycle pause"),
    ("KP_ENTER", "cycle pause"),
    ("SPACE", "cycle pause"),
    ("LEFT", "seek -10"),
    ("RIGHT", "seek 10"),
];

type Socket = Arc<Mutex<Option<UnixStream>>>;

pub struct Player {
    socket: Socket,
}

impl Player {
    pub fn start(path: &Path, on_exit: impl FnOnce(bool) + Send + 'static) -> io::Result<Self> {
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
            let success = child.wait().is_ok_and(|status| status.success());
            let _ = std::fs::remove_file(&socket_path);
            on_exit(success);
        });
        Ok(Self { socket })
    }

    pub fn command(&self, command: Command) {
        let message = match command {
            Command::Back => json!(["quit"]),
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
