//! Who used SemmelTV and what they watched: the rows the app keeps in the
//! `usage` table, and the text `semmeltv stats` prints from them.

use crate::manifest::Date;
use crate::profile::ProfileId;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

const DAY: u64 = 86_400;
const HOUR: u64 = 3_600;
const LISTED_PLAYS: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    /// A new session: a password, a passkey or an invite.
    SignIn,
    /// A signed-in device used the app again, at most once an hour per session.
    Visit,
    /// One play on the TV or a phone, with the time actually watched.
    Play,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub at: u64,
    pub activity: Activity,
    pub profile: ProfileId,
    pub device: String,
    pub title: String,
    pub seconds: u64,
}

#[derive(Default)]
struct Person {
    sign_ins: usize,
    days: BTreeSet<i64>,
    hours: BTreeSet<i64>,
    plays: usize,
    on_tv: u64,
    on_phone: u64,
    last: Option<u64>,
    devices: BTreeSet<String>,
}

fn watched(seconds: u64) -> String {
    let minutes = (seconds + 30) / 60;
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes} min"),
        (hours, minutes) => format!("{hours} h {minutes:02}"),
    }
}

fn stamp(at: u64, offset: &impl Fn(u64) -> i64) -> String {
    let local = (at as i64 + offset(at)).max(0) as u64;
    let day = local % DAY;
    format!("{} {:02}:{:02}", Date::from_unix(local), day / HOUR, day % HOUR / 60)
}

fn work(title: &str) -> &str {
    title.split(" · ").next().unwrap_or(title)
}

pub fn report(
    events: &[Event],
    names: &BTreeMap<ProfileId, String>,
    now: u64,
    days: u64,
    offset: impl Fn(u64) -> i64,
) -> String {
    let name = |profile: &ProfileId| names.get(profile).cloned().unwrap_or_else(|| format!("#{}", profile.0));
    let mut people: BTreeMap<String, Person> = BTreeMap::new();
    let mut works: BTreeMap<&str, BTreeMap<String, u64>> = BTreeMap::new();
    for event in events {
        let who = name(&event.profile);
        let person = people.entry(who.clone()).or_default();
        let local = event.at as i64 + offset(event.at);
        person.days.insert(local.div_euclid(DAY as i64));
        person.hours.insert(local.div_euclid(HOUR as i64));
        person.last = person.last.max(Some(event.at));
        if !event.device.is_empty() {
            person.devices.insert(event.device.clone());
        }
        match event.activity {
            Activity::SignIn => person.sign_ins += 1,
            Activity::Visit => {}
            Activity::Play => {
                person.plays += 1;
                if event.device == "TV" {
                    person.on_tv += event.seconds;
                } else {
                    person.on_phone += event.seconds;
                }
                *works.entry(work(&event.title)).or_default().entry(who).or_default() += event.seconds;
            }
        }
    }

    let mut out = String::new();
    let since = now.saturating_sub(days * DAY);
    let _ = writeln!(
        out,
        "SemmelTV over the last {days} days, since {}\n",
        stamp(since, &offset)
    );
    if events.is_empty() {
        out.push_str("Nobody used it.\n");
        return out;
    }

    let _ = writeln!(
        out,
        "{:<10} {:>8} {:>5} {:>6} {:>6} {:>9} {:>9} {:>9}  {:<16}  devices",
        "person", "sign-ins", "days", "hours", "plays", "watched", "on TV", "on phone", "last seen"
    );
    for (who, person) in &people {
        let _ = writeln!(
            out,
            "{:<10} {:>8} {:>5} {:>6} {:>6} {:>9} {:>9} {:>9}  {:<16}  {}",
            who,
            person.sign_ins,
            person.days.len(),
            person.hours.len(),
            person.plays,
            watched(person.on_tv + person.on_phone),
            watched(person.on_tv),
            watched(person.on_phone),
            person.last.map(|at| stamp(at, &offset)).unwrap_or_default(),
            person.devices.iter().cloned().collect::<Vec<_>>().join(", ")
        );
    }

    if !works.is_empty() {
        out.push_str("\nMost watched\n");
        let mut ranked: Vec<(&str, u64, &BTreeMap<String, u64>)> = works
            .iter()
            .map(|(title, by)| (*title, by.values().sum(), by))
            .collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        for (title, total, by) in ranked {
            let by = by
                .iter()
                .map(|(who, seconds)| format!("{who} {}", watched(*seconds)))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "  {:>9}  {title}  ({by})", watched(total));
        }
    }

    let plays: Vec<&Event> = events
        .iter()
        .rev()
        .filter(|event| event.activity == Activity::Play)
        .collect();
    if !plays.is_empty() {
        out.push_str("\nPlays, newest first\n");
        for event in plays.iter().take(LISTED_PLAYS) {
            let _ = writeln!(
                out,
                "  {}  {:<10} {:>9}  {:<16}  {}",
                stamp(event.at, &offset),
                name(&event.profile),
                watched(event.seconds),
                event.device,
                event.title
            );
        }
        if plays.len() > LISTED_PLAYS {
            let _ = writeln!(out, "  … and {} older", plays.len() - LISTED_PLAYS);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(at: u64, activity: Activity, profile: i64, device: &str, title: &str, seconds: u64) -> Event {
        Event {
            at,
            activity,
            profile: ProfileId(profile),
            device: device.to_string(),
            title: title.to_string(),
            seconds,
        }
    }

    #[test]
    fn the_report_counts_each_person_and_ranks_what_was_watched() {
        let noon = 1_794_916_800;
        let names = BTreeMap::from([
            (ProfileId(1), "Frederik".to_string()),
            (ProfileId(2), "Mami".to_string()),
        ]);
        let events = [
            event(noon, Activity::SignIn, 2, "iPad", "", 0),
            event(noon + 60, Activity::Play, 2, "iPad", "Dark · S01E01", 3_000),
            event(noon + 2 * HOUR, Activity::Visit, 2, "iPad", "", 0),
            event(noon + 3 * HOUR, Activity::Play, 1, "TV", "Arrival", 6_900),
            event(noon + DAY, Activity::Play, 1, "TV", "Dark · S01E02", 3_300),
            event(noon + DAY + 60, Activity::Play, 9, "Laptop", "Arrival", 20),
        ];
        let text = report(&events, &names, noon + DAY + HOUR, 7, |_| 0);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "SemmelTV over the last 7 days, since 2026-11-11 13:00");
        let row = |who: &str| {
            lines
                .iter()
                .find(|line| line.starts_with(who))
                .unwrap()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert_eq!(
            row("Frederik"),
            "Frederik 0 2 2 2 2 h 50 2 h 50 0 min 2026-11-18 12:00 TV"
        );
        assert_eq!(row("Mami"), "Mami 1 1 2 1 50 min 0 min 50 min 2026-11-17 14:00 iPad");
        assert_eq!(row("#9"), "#9 0 1 1 1 0 min 0 min 0 min 2026-11-18 12:01 Laptop");
        let ranked: Vec<&str> = text
            .split("Most watched\n")
            .nth(1)
            .unwrap()
            .lines()
            .take_while(|line| !line.is_empty())
            .map(str::trim)
            .collect();
        assert_eq!(
            ranked,
            [
                "1 h 55  Arrival  (#9 0 min, Frederik 1 h 55)",
                "1 h 45  Dark  (Frederik 55 min, Mami 50 min)",
            ]
        );
        assert!(lines.iter().any(|line| {
            line.split_whitespace().collect::<Vec<_>>().join(" ") == "2026-11-17 12:01 Mami 50 min iPad Dark · S01E01"
        }));
    }

    #[test]
    fn an_empty_window_says_so() {
        let text = report(&[], &BTreeMap::new(), 1_794_916_800, 1, |_| 3_600);
        assert_eq!(
            text,
            "SemmelTV over the last 1 days, since 2026-11-16 13:00\n\nNobody used it.\n"
        );
    }
}
