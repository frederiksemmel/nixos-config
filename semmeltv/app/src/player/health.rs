use std::time::{Duration, Instant};

pub const WINDOW: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reading {
    Dropped(u64),
    Late(u64),
    Waiting(bool),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub dropped: u64,
    pub late: u64,
    pub waits: u64,
}

impl Counts {
    fn since(self, earlier: Self) -> Self {
        Self {
            dropped: self.dropped.saturating_sub(earlier.dropped),
            late: self.late.saturating_sub(earlier.late),
            waits: self.waits.saturating_sub(earlier.waits),
        }
    }

    fn is_clean(self) -> bool {
        self == Self::default()
    }

    pub fn trouble(self) -> String {
        let mut parts = Vec::new();
        if self.late > 0 {
            parts.push(format!("{} late", frames(self.late)));
        }
        if self.dropped > 0 {
            parts.push(format!("{} dropped", frames(self.dropped)));
        }
        if self.waits > 0 {
            parts.push(format!("waited for data {}", times(self.waits)));
        }
        parts.join(", ")
    }

    pub fn summary(self) -> String {
        format!(
            "{} late, {} dropped, waited for data {}",
            frames(self.late),
            frames(self.dropped),
            times(self.waits)
        )
    }
}

fn frames(count: u64) -> String {
    match count {
        1 => "1 frame".to_string(),
        count => format!("{count} frames"),
    }
}

fn times(count: u64) -> String {
    match count {
        0 => "never".to_string(),
        1 => "once".to_string(),
        count => format!("{count} times"),
    }
}

pub fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!("{}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
}

#[derive(Debug, Default)]
pub struct Health {
    mpv: Counts,
    waiting: bool,
    begun: Option<Counts>,
    reported: Counts,
    window_from: Option<Instant>,
}

impl Health {
    pub fn observe(&mut self, reading: Reading) {
        match reading {
            Reading::Dropped(count) => self.mpv.dropped = count,
            Reading::Late(count) => self.mpv.late = count,
            Reading::Waiting(waiting) => {
                if waiting && !self.waiting {
                    self.mpv.waits += 1;
                }
                self.waiting = waiting;
            }
        }
    }

    pub fn begin(&mut self, now: Instant) {
        if self.begun.is_none() {
            self.begun = Some(self.mpv);
            self.reported = self.mpv;
            self.window_from = Some(now);
        }
    }

    pub fn trouble_due(&mut self, now: Instant) -> Option<Counts> {
        let from = self.window_from?;
        if now.duration_since(from) < WINDOW {
            return None;
        }
        self.window_from = Some(now);
        let window = self.mpv.since(self.reported);
        self.reported = self.mpv;
        (!window.is_clean()).then_some(window)
    }

    pub fn total(&self) -> Option<Counts> {
        self.begun.map(|begun| self.mpv.since(begun))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_late_before_the_film_starts_do_not_count() {
        let mut health = Health::default();
        health.observe(Reading::Late(3));
        assert_eq!(health.total(), None);
        let now = Instant::now();
        health.begin(now);
        health.observe(Reading::Late(5));
        health.observe(Reading::Dropped(1));
        assert_eq!(
            health.total(),
            Some(Counts {
                dropped: 1,
                late: 2,
                waits: 0
            })
        );
    }

    #[test]
    fn each_window_reports_only_what_went_wrong_in_it() {
        let mut health = Health::default();
        let start = Instant::now();
        health.begin(start);
        health.observe(Reading::Late(2));
        assert_eq!(health.trouble_due(start + Duration::from_secs(5)), None);
        let first = health.trouble_due(start + WINDOW).unwrap();
        assert_eq!(first.trouble(), "2 frames late");
        assert_eq!(health.trouble_due(start + WINDOW * 2), None);
        health.observe(Reading::Dropped(1));
        health.observe(Reading::Waiting(true));
        let third = health.trouble_due(start + WINDOW * 3).unwrap();
        assert_eq!(third.trouble(), "1 frame dropped, waited for data once");
    }

    #[test]
    fn a_wait_counts_once_until_the_data_arrives() {
        let mut health = Health::default();
        health.begin(Instant::now());
        health.observe(Reading::Waiting(true));
        health.observe(Reading::Waiting(true));
        health.observe(Reading::Waiting(false));
        health.observe(Reading::Waiting(true));
        assert_eq!(health.total().unwrap().waits, 2);
    }

    #[test]
    fn a_clean_film_sums_up_as_nothing_late_dropped_or_waited_for() {
        assert_eq!(
            Counts::default().summary(),
            "0 frames late, 0 frames dropped, waited for data never"
        );
        assert_eq!(clock(3723.9), "1:02:03");
    }

    #[test]
    fn nothing_is_due_before_the_film_starts() {
        let mut health = Health::default();
        health.observe(Reading::Dropped(4));
        assert_eq!(health.trouble_due(Instant::now() + WINDOW * 2), None);
    }
}
