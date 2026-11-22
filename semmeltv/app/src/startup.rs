//! The steps a process takes before it serves, each named in the log with
//! how long it took, and named again every few seconds while it is still
//! waiting, so a start that hangs says where.

use std::future::Future;
use std::time::{Duration, Instant};

/// How often a step still waiting is named again.
pub const STILL_WAITING: Duration = Duration::from_secs(5);

/// Runs `work`, logging `name` when it takes longer than a moment and every
/// `STILL_WAITING` while it is still pending.
pub async fn step<T>(name: &str, work: impl Future<Output = T>) -> T {
    let started = Instant::now();
    tokio::pin!(work);
    let mut nagging = tokio::time::interval_at(tokio::time::Instant::now() + STILL_WAITING, STILL_WAITING);
    let done = loop {
        tokio::select! {
            done = &mut work => break done,
            _ = nagging.tick() => eprintln!("start: still waiting for {name} after {:.0?}", started.elapsed()),
        }
    };
    let took = started.elapsed();
    if took >= Duration::from_millis(500) {
        eprintln!("start: {name} took {took:.1?}");
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_step_hands_back_what_its_work_gave() {
        assert_eq!(step("a quick step", async { 7 }).await, 7);
        let slow = step("a slow step", async {
            tokio::time::sleep(STILL_WAITING * 3).await;
            "done"
        });
        assert_eq!(slow.await, "done");
    }
}
