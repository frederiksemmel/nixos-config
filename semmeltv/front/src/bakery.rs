use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

pub const SECOND: u64 = 1000;
const MINUTE: u64 = 60 * SECOND;
const REMEMBERED: usize = 20;
const FIRST_GUESS: u64 = MINUTE;
const SHORTEST_STRETCH: u64 = 30 * SECOND;
const FAILURES_FOR_DOWN: u32 = 3;
const ANNOUNCED_GRACE: u64 = 30 * SECOND;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Upstream {
    Core,
    Web,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Api,
    Pages,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    Answered,
    Refused,
    NoAnswer,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Planned {
    pub reason: Option<String>,
    pub since: u64,
    pub until: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Outage {
    pub since: u64,
    pub announced: bool,
    pub seen_down: bool,
    pub until: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Watch {
    pub outage: Option<Outage>,
    pub history: VecDeque<u64>,
    #[serde(skip)]
    failures: u32,
    #[serde(skip)]
    failing_since: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Bakery {
    pub planned: Option<Planned>,
    pub core: Watch,
    pub web: Watch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    Planned,
    Restarting,
    Down,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Closed {
    pub cause: Cause,
    pub reason: Option<String>,
    pub since: u64,
    pub promised: u64,
    pub until: u64,
    pub now: u64,
    pub overdue: bool,
    pub retry_after: u64,
    pub headline: String,
    pub note: String,
    pub late: Option<String>,
    pub eta: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    Open,
    Closed(Closed),
}

impl Watch {
    pub fn guess(&self) -> u64 {
        if self.history.is_empty() {
            return FIRST_GUESS;
        }
        let mut sorted: Vec<u64> = self.history.iter().copied().collect();
        sorted.sort_unstable();
        let fifth = 5 * SECOND;
        let p80 = sorted[(sorted.len() * 4).div_ceil(5) - 1];
        p80.div_ceil(fifth).max(1) * fifth
    }

    pub fn is_down(&self) -> bool {
        self.outage.is_some()
    }

    fn announce(&mut self, now: u64) -> bool {
        if self.outage.is_some() {
            return false;
        }
        self.outage = Some(Outage {
            since: now,
            announced: true,
            seen_down: false,
            until: now + self.guess(),
        });
        true
    }

    fn observe(&mut self, probe: Probe, now: u64) -> bool {
        match probe {
            Probe::Answered => {
                self.failures = 0;
                self.failing_since = None;
                let Some(outage) = &self.outage else { return false };
                if !outage.seen_down && now < outage.since + ANNOUNCED_GRACE {
                    return false;
                }
                if outage.seen_down {
                    self.history.push_back(now.saturating_sub(outage.since));
                    while self.history.len() > REMEMBERED {
                        self.history.pop_front();
                    }
                }
                self.outage = None;
                true
            }
            Probe::Refused => {
                self.failing_since.get_or_insert(now);
                self.down(now)
            }
            Probe::NoAnswer => {
                self.failures += 1;
                let since = *self.failing_since.get_or_insert(now);
                if self.failures >= FAILURES_FOR_DOWN {
                    self.down(since)
                } else {
                    false
                }
            }
        }
    }

    fn down(&mut self, since: u64) -> bool {
        let guess = self.guess();
        match &mut self.outage {
            Some(outage) if outage.seen_down => false,
            Some(outage) => {
                outage.seen_down = true;
                true
            }
            None => {
                self.outage = Some(Outage {
                    since,
                    announced: false,
                    seen_down: true,
                    until: since + guess,
                });
                true
            }
        }
    }
}

impl Bakery {
    pub fn plan(&mut self, now: u64, duration: u64, reason: Option<String>) {
        self.planned = Some(Planned {
            reason: reason.filter(|reason| !reason.trim().is_empty()),
            since: now,
            until: now + duration,
        });
    }

    pub fn reopen(&mut self) -> bool {
        self.planned.take().is_some()
    }

    pub fn announce(&mut self, now: u64) -> bool {
        self.core.announce(now)
    }

    pub fn observe(&mut self, upstream: Upstream, probe: Probe, now: u64) -> bool {
        self.watch_mut(upstream).observe(probe, now)
    }

    pub fn watch(&self, upstream: Upstream) -> &Watch {
        match upstream {
            Upstream::Core => &self.core,
            Upstream::Web => &self.web,
        }
    }

    fn watch_mut(&mut self, upstream: Upstream) -> &mut Watch {
        match upstream {
            Upstream::Core => &mut self.core,
            Upstream::Web => &mut self.web,
        }
    }

    pub fn status(&self, now: u64, scope: Scope) -> Status {
        if let Some(planned) = &self.planned {
            return Status::Closed(closed(
                Cause::Planned,
                planned.reason.clone(),
                planned.since,
                planned.until,
                now,
            ));
        }
        if let Some(outage) = &self.core.outage {
            let cause = if outage.announced {
                Cause::Restarting
            } else {
                Cause::Down
            };
            return Status::Closed(closed(cause, None, outage.since, outage.until, now));
        }
        if let (Scope::Pages, Some(outage)) = (scope, &self.web.outage) {
            return Status::Closed(closed(Cause::Restarting, None, outage.since, outage.until, now));
        }
        Status::Open
    }
}

pub fn stretched(since: u64, promised: u64, now: u64) -> u64 {
    if now < promised {
        return promised;
    }
    let stretch = (promised.saturating_sub(since) / 2).max(SHORTEST_STRETCH);
    promised + ((now - promised) / stretch + 1) * stretch
}

fn closed(cause: Cause, reason: Option<String>, since: u64, promised: u64, now: u64) -> Closed {
    let until = stretched(since, promised, now);
    let overdue = until != promised;
    let remaining = until - now;
    let (headline, note) = match cause {
        Cause::Planned => (
            "Die Backstube ist kurz zu",
            match &reason {
                Some(reason) => format!("Wir backen gerade etwas Neues: {reason}."),
                None => "Wir backen gerade etwas Neues.".to_string(),
            },
        ),
        Cause::Restarting => (
            "Der Ofen wird neu angeheizt",
            "SemmelTV startet neu, gleich gibt es wieder frische Semmeln.".to_string(),
        ),
        Cause::Down => (
            "Wir haben es versemmelt",
            "Wir kriegen es gerade nicht gebacken: SemmelTV antwortet nicht. Wir heizen den Ofen wieder an."
                .to_string(),
        ),
    };
    Closed {
        cause,
        reason,
        since,
        promised,
        until,
        now,
        overdue,
        retry_after: remaining.div_ceil(SECOND).max(1),
        headline: headline.to_string(),
        note,
        late: overdue.then(|| "Dauert länger als gedacht.".to_string()),
        eta: format!("Wieder da {}", eta(remaining)),
    }
}

pub fn eta(remaining: u64) -> String {
    if remaining <= 15 * SECOND {
        return "in wenigen Sekunden".to_string();
    }
    if remaining <= 50 * SECOND {
        return format!("in etwa {} Sekunden", remaining.div_ceil(10 * SECOND) * 10);
    }
    if remaining < 90 * MINUTE {
        let minutes = ((remaining + MINUTE / 2) / MINUTE).max(1);
        return if minutes == 1 {
            "in etwa 1 Minute".to_string()
        } else {
            format!("in etwa {minutes} Minuten")
        };
    }
    let hour = 60 * MINUTE;
    let hours = (remaining + hour / 2) / hour;
    if hours == 1 {
        "in etwa 1 Stunde".to_string()
    } else {
        format!("in etwa {hours} Stunden")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: u64 = 1_800_000_000_000;

    fn closed_at(bakery: &Bakery, now: u64, scope: Scope) -> Closed {
        match bakery.status(now, scope) {
            Status::Closed(closed) => closed,
            Status::Open => panic!("the bakery is open at {now}"),
        }
    }

    fn restarted(bakery: &mut Bakery, at: u64, took: u64) {
        bakery.announce(at);
        bakery.observe(Upstream::Core, Probe::Refused, at + SECOND);
        bakery.observe(Upstream::Core, Probe::Answered, at + took);
    }

    #[test]
    fn a_bakery_with_everything_answering_is_open() {
        let mut bakery = Bakery::default();
        bakery.observe(Upstream::Core, Probe::Answered, T);
        bakery.observe(Upstream::Web, Probe::Answered, T);
        assert_eq!(bakery.status(T, Scope::Pages), Status::Open);
    }

    #[test]
    fn planned_work_closes_until_the_given_time_with_its_reason() {
        let mut bakery = Bakery::default();
        bakery.plan(T, 20 * MINUTE, Some("neue Streaming-Limits".to_string()));
        let closed = closed_at(&bakery, T + MINUTE, Scope::Api);
        assert_eq!(closed.cause, Cause::Planned);
        assert_eq!(closed.until, T + 20 * MINUTE);
        assert_eq!(closed.note, "Wir backen gerade etwas Neues: neue Streaming-Limits.");
        assert_eq!(closed.eta, "Wieder da in etwa 19 Minuten");
        assert_eq!(closed.retry_after, 19 * 60);
        assert!(!closed.overdue);
    }

    #[test]
    fn planned_work_passing_its_time_says_so_and_guesses_again() {
        let mut bakery = Bakery::default();
        bakery.plan(T, 20 * MINUTE, None);
        let late = closed_at(&bakery, T + 21 * MINUTE, Scope::Pages);
        assert!(late.overdue);
        assert_eq!(late.late.as_deref(), Some("Dauert länger als gedacht."));
        assert_eq!(late.until, T + 30 * MINUTE);
        assert_eq!(late.eta, "Wieder da in etwa 9 Minuten");
        let later = closed_at(&bakery, T + 31 * MINUTE, Scope::Pages);
        assert_eq!(later.until, T + 40 * MINUTE);
    }

    #[test]
    fn the_new_guess_is_never_in_the_past() {
        for elapsed in (0..600).map(|seconds| seconds * SECOND) {
            let until = stretched(T, T + 20 * SECOND, T + elapsed);
            assert!(until > T + elapsed, "{elapsed} ms in, the guess was {until}");
        }
    }

    #[test]
    fn switching_planned_work_off_reopens() {
        let mut bakery = Bakery::default();
        bakery.plan(T, MINUTE, None);
        assert!(bakery.reopen());
        assert_eq!(bakery.status(T, Scope::Pages), Status::Open);
    }

    #[test]
    fn an_announced_restart_closes_at_once_and_guesses_a_minute_without_history() {
        let mut bakery = Bakery::default();
        bakery.announce(T);
        let closed = closed_at(&bakery, T, Scope::Api);
        assert_eq!(closed.cause, Cause::Restarting);
        assert_eq!(closed.until, T + MINUTE);
        assert_eq!(closed.headline, "Der Ofen wird neu angeheizt");
    }

    #[test]
    fn a_core_still_answering_right_after_the_announcement_keeps_the_bakery_closed() {
        let mut bakery = Bakery::default();
        bakery.announce(T);
        bakery.observe(Upstream::Core, Probe::Answered, T + SECOND);
        assert!(bakery.core.is_down());
    }

    #[test]
    fn an_announcement_the_core_never_follows_lapses_unrecorded() {
        let mut bakery = Bakery::default();
        bakery.announce(T);
        bakery.observe(Upstream::Core, Probe::Answered, T + ANNOUNCED_GRACE);
        assert_eq!(bakery.status(T + ANNOUNCED_GRACE, Scope::Api), Status::Open);
        assert!(bakery.core.history.is_empty());
    }

    #[test]
    fn a_restart_is_measured_from_the_announcement_to_the_first_answer() {
        let mut bakery = Bakery::default();
        restarted(&mut bakery, T, 42 * SECOND);
        assert_eq!(bakery.core.history, [42 * SECOND]);
        assert_eq!(bakery.status(T + 42 * SECOND, Scope::Api), Status::Open);
    }

    #[test]
    fn the_guess_is_the_80th_percentile_of_past_outages_rounded_up_to_5_seconds() {
        let mut bakery = Bakery::default();
        for (n, seconds) in [10, 12, 14, 16, 18, 20, 22, 24, 26, 90].into_iter().enumerate() {
            restarted(&mut bakery, T + n as u64 * 10 * MINUTE, seconds * SECOND);
        }
        assert_eq!(bakery.core.guess(), 25 * SECOND);
        bakery.announce(T + 200 * MINUTE);
        assert_eq!(
            closed_at(&bakery, T + 200 * MINUTE, Scope::Api).until,
            T + 200 * MINUTE + 25 * SECOND
        );
    }

    #[test]
    fn only_the_last_20_outages_count() {
        let mut bakery = Bakery::default();
        for n in 0..30 {
            restarted(
                &mut bakery,
                T + n * 10 * MINUTE,
                if n < 10 { 5 * MINUTE } else { 10 * SECOND },
            );
        }
        assert_eq!(bakery.core.history.len(), 20);
        assert_eq!(bakery.core.guess(), 10 * SECOND);
    }

    #[test]
    fn a_refused_connection_closes_at_once_as_an_outage() {
        let mut bakery = Bakery::default();
        bakery.observe(Upstream::Core, Probe::Refused, T);
        let closed = closed_at(&bakery, T, Scope::Api);
        assert_eq!(closed.cause, Cause::Down);
        assert_eq!(closed.headline, "Wir haben es versemmelt");
    }

    #[test]
    fn a_core_that_stops_answering_closes_after_three_probes_counted_from_the_first() {
        let mut bakery = Bakery::default();
        bakery.observe(Upstream::Core, Probe::NoAnswer, T);
        bakery.observe(Upstream::Core, Probe::NoAnswer, T + 5 * SECOND);
        assert_eq!(bakery.status(T + 5 * SECOND, Scope::Api), Status::Open);
        bakery.observe(Upstream::Core, Probe::NoAnswer, T + 6 * SECOND);
        assert_eq!(closed_at(&bakery, T + 6 * SECOND, Scope::Api).since, T);
    }

    #[test]
    fn the_web_app_restarting_closes_pages_but_not_the_api() {
        let mut bakery = Bakery::default();
        bakery.observe(Upstream::Web, Probe::Refused, T);
        assert_eq!(closed_at(&bakery, T, Scope::Pages).cause, Cause::Restarting);
        assert_eq!(bakery.status(T, Scope::Api), Status::Open);
    }

    #[test]
    fn planned_work_wins_over_a_restart() {
        let mut bakery = Bakery::default();
        bakery.plan(T, 10 * MINUTE, Some("Tests".to_string()));
        bakery.announce(T + MINUTE);
        assert_eq!(closed_at(&bakery, T + MINUTE, Scope::Api).cause, Cause::Planned);
    }

    #[test]
    fn an_empty_reason_counts_as_none() {
        let mut bakery = Bakery::default();
        bakery.plan(T, MINUTE, Some("  ".to_string()));
        assert_eq!(closed_at(&bakery, T, Scope::Api).note, "Wir backen gerade etwas Neues.");
    }

    #[test]
    fn the_bakery_survives_a_restart_of_the_front_through_its_state_file() {
        let mut bakery = Bakery::default();
        restarted(&mut bakery, T, 30 * SECOND);
        bakery.plan(T + MINUTE, MINUTE, Some("Tests".to_string()));
        let kept: Bakery = serde_json::from_str(&serde_json::to_string(&bakery).unwrap()).unwrap();
        assert_eq!(kept.planned, bakery.planned);
        assert_eq!(kept.core.history, [30 * SECOND]);
    }

    #[test]
    fn the_time_left_reads_in_seconds_minutes_and_hours() {
        assert_eq!(eta(4 * SECOND), "in wenigen Sekunden");
        assert_eq!(eta(31 * SECOND), "in etwa 40 Sekunden");
        assert_eq!(eta(50 * SECOND), "in etwa 50 Sekunden");
        assert_eq!(eta(59 * SECOND), "in etwa 1 Minute");
        assert_eq!(eta(100 * SECOND), "in etwa 2 Minuten");
        assert_eq!(eta(89 * MINUTE), "in etwa 89 Minuten");
        assert_eq!(eta(90 * MINUTE), "in etwa 2 Stunden");
        assert_eq!(eta(70 * MINUTE + 60 * MINUTE), "in etwa 2 Stunden");
    }
}
