//! Telling systemd how the process is: ready, and still alive, so its
//! watchdog restarts a process that stopped saying so. Without
//! `NOTIFY_SOCKET` nothing is sent. And the beats of the threads a process
//! cannot do without, for a watchdog of its own.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::sync::Mutex;
use std::time::{Duration, Instant};

static BEATS: Mutex<BTreeMap<&'static str, Instant>> = Mutex::new(BTreeMap::new());

fn send(state: &str) {
    if let Some(path) = std::env::var_os("NOTIFY_SOCKET") {
        tell(&path, state);
    }
}

/// Sends `state` to the socket at `path`, where a leading `@` names an
/// abstract socket.
fn tell(path: &OsStr, state: &str) {
    let address = match path.as_bytes() {
        [b'@', name @ ..] => SocketAddr::from_abstract_name(name),
        _ => SocketAddr::from_pathname(path),
    };
    let sent = UnixDatagram::unbound().and_then(|socket| socket.send_to_addr(state.as_bytes(), &address?));
    if let Err(error) = sent {
        eprintln!("systemd: telling it {state}: {error}");
    }
}

/// The process is up and doing its work.
pub fn ready() {
    send("READY=1");
}

/// The process is still alive.
pub fn alive() {
    send("WATCHDOG=1");
}

/// `thread` is alive now. Once a thread has beaten, `all_beating` waits for
/// it until it `rests`.
pub fn beat(thread: &'static str) {
    BEATS.lock().unwrap().insert(thread, Instant::now());
}

/// `thread` ended on purpose and is no longer waited for.
pub fn rests(thread: &'static str) {
    BEATS.lock().unwrap().remove(thread);
}

/// The threads that beat and did not within `within`.
pub fn silent(now: Instant, within: Duration) -> Vec<&'static str> {
    BEATS
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, beat)| now.saturating_duration_since(**beat) > within)
        .map(|(thread, _)| *thread)
        .collect()
}

/// Watches the beats on a thread of its own and calls `stuck` with the
/// threads that went `within` without one.
pub fn watch(within: Duration, stuck: impl Fn(Vec<&'static str>) + Send + 'static) {
    std::thread::Builder::new()
        .name("watchdog".to_string())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let silent = silent(Instant::now(), within);
                if !silent.is_empty() {
                    stuck(silent);
                }
            }
        })
        .expect("a thread for the watchdog");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thread_that_stopped_beating_is_silent_until_it_rests() {
        let within = Duration::from_secs(15);
        beat("a test thread");
        assert!(!silent(Instant::now(), within).contains(&"a test thread"));
        let later = Instant::now() + within + Duration::from_secs(1);
        assert!(silent(later, within).contains(&"a test thread"));
        rests("a test thread");
        assert!(!silent(later, within).contains(&"a test thread"));
    }

    #[test]
    fn systemd_hears_ready_and_alive_on_its_socket() {
        let dir = std::env::temp_dir().join(format!("semmeltv-notify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("notify");
        let systemd = UnixDatagram::bind(&path).unwrap();
        systemd.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        tell(path.as_os_str(), "READY=1");
        tell(path.as_os_str(), "WATCHDOG=1");
        let mut said = [0; 64];
        let first = systemd.recv(&mut said).unwrap();
        assert_eq!(&said[..first], b"READY=1");
        let second = systemd.recv(&mut said).unwrap();
        assert_eq!(&said[..second], b"WATCHDOG=1");
    }
}
