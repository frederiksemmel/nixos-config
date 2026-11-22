//! The seam between the core and the tv.

pub mod local;
#[allow(dead_code)]
pub mod messages;

use messages::ToTv;
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};

/// How many orders wait for a tv that is slow to take them before new ones
/// are dropped.
pub const QUEUE: usize = 256;

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
