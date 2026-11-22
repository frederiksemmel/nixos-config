//! The seam between the core and the tv.

pub mod client;
pub mod front;
pub mod local;
pub mod messages;
pub mod socket;

use messages::ToTv;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};

/// How many orders wait for a tv that is slow to take them before new ones
/// are dropped.
pub const QUEUE: usize = 256;

/// The longest line either side reads; a longer one ends the connection.
pub const LINE_LIMIT: usize = 16 << 20;

/// Where the core listens for the tv unless `SEMMELTV_TV_SOCKET` says
/// otherwise.
pub const SOCKET: &str = "/run/semmeltv/tv.sock";

/// What a core and a tv compare to know they are the same build: the path
/// of the running program, which in the Nix store names the build.
pub fn revision() -> String {
    std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .unwrap_or_else(|_| PathBuf::from("unknown"))
        .display()
        .to_string()
}

/// One message as a line of JSON.
pub fn line_of(message: &impl serde::Serialize) -> Vec<u8> {
    let mut line = serde_json::to_vec(message).expect("messages serialise");
    line.push(b'\n');
    line
}

/// The core's end of the link to a tv. Sending never waits: an order the tv
/// has no room for is dropped and logged.
#[derive(Clone)]
pub struct Link {
    orders: SyncSender<ToTv>,
}

impl Link {
    /// A link and the tv's end of it.
    pub fn pair() -> (Self, Receiver<ToTv>) {
        let (orders, received) = mpsc::sync_channel(QUEUE);
        (Self { orders }, received)
    }

    pub fn send(&self, order: ToTv) {
        match self.orders.try_send(order) {
            Ok(()) => {}
            Err(TrySendError::Full(order)) => eprintln!("tv: not reading, dropped {}", order.name()),
            Err(TrySendError::Disconnected(order)) => eprintln!("tv: gone, dropped {}", order.name()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn a_tv_that_never_reads_costs_the_sender_nothing() {
        let (link, _never_read) = Link::pair();
        let started = Instant::now();
        for _ in 0..QUEUE * 4 {
            link.send(ToTv::Message(String::new()));
        }
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn orders_reach_the_tv_in_the_order_they_were_sent() {
        let (link, received) = Link::pair();
        link.send(ToTv::Message("one".to_string()));
        link.send(ToTv::Message("two".to_string()));
        assert_eq!(received.try_recv(), Ok(ToTv::Message("one".to_string())));
        assert_eq!(received.try_recv(), Ok(ToTv::Message("two".to_string())));
    }
}
