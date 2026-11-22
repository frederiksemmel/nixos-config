//! Commands for mpv go through a queue to one writer thread, so whoever sends one never waits
//! on mpv's socket.

use serde_json::Value;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Sender};
use std::thread;

#[derive(Clone)]
pub struct Commands {
    queue: Sender<Value>,
}

impl Commands {
    pub fn new(mut stream: UnixStream) -> Self {
        let (queue, queued) = mpsc::channel::<Value>();
        thread::spawn(move || {
            for command in queued {
                if super::send(&mut stream, command).is_err() {
                    return;
                }
            }
        });
        Self { queue }
    }

    pub fn send(&self, command: Value) -> bool {
        self.queue.send(command).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{BufRead, BufReader};
    use std::time::{Duration, Instant};

    #[test]
    fn sending_returns_at_once_while_mpv_reads_nothing_and_everything_arrives_in_order() {
        let (stream, mpv) = UnixStream::pair().unwrap();
        let commands = Commands::new(stream);
        let frame = "x".repeat(5_000);
        let started = Instant::now();
        for n in 0..1_000 {
            assert!(commands.send(json!(["show-text", frame, n])));
        }
        assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
        drop(commands);
        let numbers: Vec<u64> = BufReader::new(mpv)
            .lines()
            .map(|line| {
                serde_json::from_str::<Value>(&line.unwrap()).unwrap()["command"][2]
                    .as_u64()
                    .unwrap()
            })
            .collect();
        assert_eq!(numbers, (0..1_000).collect::<Vec<_>>());
    }
}
