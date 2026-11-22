//! Pausing and resuming with a short volume fade instead of a hard cut.

use super::commands::Commands;
use serde_json::{Value, json};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

const FULL: u32 = 100;
const STEP: u32 = 10;
const STEP_EVERY: Duration = Duration::from_millis(20);
/// What the HDMI output still holds once the volume is down: 4800 frames at 48 kHz, plus margin.
const DRAIN: Duration = Duration::from_millis(120);

pub struct Fader {
    wanted: Sender<bool>,
}

impl Fader {
    pub fn start(commands: Commands) -> Self {
        let (wanted, requests) = mpsc::channel();
        thread::spawn(move || run(&commands, &requests));
        Self { wanted }
    }

    pub fn set_paused(&self, paused: bool) {
        let _ = self.wanted.send(paused);
    }
}

fn run(commands: &Commands, requests: &Receiver<bool>) {
    let mut fade = Fade::default();
    let Ok(mut paused) = requests.recv() else {
        return;
    };
    loop {
        let request = match fade.step(paused) {
            Some((command, wait)) => {
                if !commands.send(command) {
                    return;
                }
                requests.recv_timeout(wait)
            }
            None => requests.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match request {
            Ok(wanted) => {
                paused = wanted;
                fade.sent = None;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Where a fade stands: mpv's volume, and the pause state sent since the last request. Each
/// request sends its pause state again, so a pause mpv made itself can't put the two out of step.
struct Fade {
    level: u32,
    sent: Option<bool>,
}

impl Default for Fade {
    fn default() -> Self {
        Self {
            level: FULL,
            sent: None,
        }
    }
}

impl Fade {
    /// The next command toward `paused`, and how long to wait after it.
    fn step(&mut self, paused: bool) -> Option<(Value, Duration)> {
        if !paused && self.sent != Some(false) {
            self.sent = Some(false);
            return Some((json!(["set_property", "pause", false]), STEP_EVERY));
        }
        if !paused && self.level < FULL {
            self.level = (self.level + STEP).min(FULL);
            return Some((volume(self.level), STEP_EVERY));
        }
        if paused && self.level > 0 {
            self.level = self.level.saturating_sub(STEP);
            let wait = if self.level == 0 { DRAIN } else { STEP_EVERY };
            return Some((volume(self.level), wait));
        }
        if paused && self.sent != Some(true) {
            self.sent = Some(true);
            return Some((json!(["set_property", "pause", true]), Duration::ZERO));
        }
        None
    }
}

fn volume(level: u32) -> Value {
    json!(["set_property", "volume", level])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::net::UnixStream;
    use std::time::Instant;

    fn plan(level: u32, paused: bool) -> Vec<(Value, Duration)> {
        let mut fade = Fade { level, sent: None };
        std::iter::from_fn(|| fade.step(paused)).take(50).collect()
    }

    #[test]
    fn pausing_lowers_the_volume_in_steps_lets_the_output_drain_then_pauses() {
        let steps = plan(FULL, true);
        let volumes: Vec<_> = steps
            .iter()
            .filter(|(c, _)| c[1] == "volume")
            .map(|(c, _)| c[2].clone())
            .collect();
        assert_eq!(volumes, (0..10).rev().map(|n| json!(n * 10)).collect::<Vec<_>>());
        let (last_volume, drain) = &steps[steps.len() - 2];
        assert_eq!((last_volume, *drain), (&volume(0), DRAIN));
        assert_eq!(steps.last().unwrap().0, json!(["set_property", "pause", true]));
        let total: Duration = steps.iter().map(|(_, wait)| *wait).sum();
        assert!(total <= Duration::from_millis(350), "{total:?}");
    }

    #[test]
    fn resuming_unpauses_at_once_then_raises_the_volume_to_full() {
        let steps = plan(0, false);
        assert_eq!(steps[0].0, json!(["set_property", "pause", false]));
        assert_eq!(steps.last().unwrap().0, volume(FULL));
        assert_eq!(steps.len(), 11);
    }

    #[test]
    fn a_request_always_sends_the_pause_state_even_when_the_volume_is_already_there() {
        assert_eq!(
            plan(FULL, false),
            [(json!(["set_property", "pause", false]), STEP_EVERY)]
        );
        assert_eq!(
            plan(0, true),
            [(json!(["set_property", "pause", true]), Duration::ZERO)]
        );
    }

    #[test]
    fn resuming_halfway_through_a_fade_out_turns_back_up_without_pausing() {
        let (stream, mut mpv) = UnixStream::pair().unwrap();
        let fader = Fader::start(Commands::new(stream));
        let started = Instant::now();
        fader.set_paused(true);
        thread::sleep(STEP_EVERY * 3 + STEP_EVERY / 2);
        fader.set_paused(false);
        thread::sleep(STEP_EVERY * 6 + DRAIN);
        drop(fader);
        thread::sleep(STEP_EVERY * 2);
        assert!(started.elapsed() < Duration::from_secs(2));
        mpv.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        let mut sent = String::new();
        let _ = mpv.read_to_string(&mut sent);
        let sent: Vec<Value> = sent
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap()["command"].clone())
            .collect();
        assert!(!sent.contains(&json!(["set_property", "pause", true])), "{sent:?}");
        assert!(sent.contains(&json!(["set_property", "pause", false])), "{sent:?}");
        assert_eq!(sent.last(), Some(&volume(FULL)), "{sent:?}");
        let lowest = sent
            .iter()
            .filter(|c| c[1] == "volume")
            .map(|c| c[2].as_u64().unwrap())
            .min();
        assert!(
            lowest.is_some_and(|lowest| lowest > 0 && lowest < FULL.into()),
            "{sent:?}"
        );
    }
}
