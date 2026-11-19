//! Who used SemmelTV and what they watched: the rows the app keeps in the
//! `usage` table, and the text `semmeltv stats` prints from them.

use crate::manifest::Date;
use crate::profile::ProfileId;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

const DAY: u64 = 86_400;
const HOUR: u64 = 3_600;
const LISTED_PLAYS: usize = 40;

const LISTED_TITLES: usize = 10;
const RECENT_PLAYS: usize = 30;
const LONGEST_DAILY: u64 = 62;
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Span {
    Day,
    Week,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct Totals {
    pub tv: u64,
    pub phone: u64,
    pub plays: usize,
    pub people: usize,
    pub titles: usize,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Bucket {
    pub label: String,
    pub mark: String,
    pub tv: u64,
    pub phone: u64,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Someone {
    pub name: String,
    pub tv: u64,
    pub phone: u64,
    pub plays: usize,
    pub days: usize,
    pub sign_ins: usize,
    pub favourite: Option<String>,
    pub last_seen: Option<String>,
    pub devices: Vec<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Share {
    pub name: String,
    pub seconds: u64,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Watched {
    pub title: String,
    pub seconds: u64,
    pub plays: usize,
    pub people: Vec<Share>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Played {
    pub at: u64,
    pub when: String,
    pub name: String,
    pub device: String,
    pub tv: bool,
    pub title: String,
    pub seconds: u64,
}

/// What `/api/usage` answers: watched time is in seconds, and days, hours and
/// times are the TV's local ones.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub days: u64,
    pub totals: Totals,
    pub span: Span,
    pub buckets: Vec<Bucket>,
    pub hours: [[u64; 24]; 7],
    pub people: Vec<Someone>,
    pub titles: Vec<Watched>,
    pub plays: Vec<Played>,
}

pub fn summary(
    events: &[Event],
    names: &BTreeMap<ProfileId, String>,
    now: u64,
    days: u64,
    offset: impl Fn(u64) -> i64,
) -> Summary {
    const D: i64 = DAY as i64;
    const H: i64 = HOUR as i64;
    let days = days.max(1);
    let name = |profile: &ProfileId| names.get(profile).cloned().unwrap_or_else(|| format!("#{}", profile.0));
    let local_now = now as i64 + offset(now);
    let last_day = local_now.div_euclid(D);
    let first_day = last_day - (days as i64 - 1);
    let span = if days > LONGEST_DAILY { Span::Week } else { Span::Day };
    let (start, width) = match span {
        Span::Day => (first_day, 1),
        Span::Week => (first_day - weekday(first_day) as i64, 7),
    };
    let count = (last_day - start) / width + 1;
    let mut buckets: Vec<Bucket> = (0..count)
        .map(|index| bucket(span, days, start + index * width))
        .collect();
    let mut hours = [[0; 24]; 7];
    let mut people: BTreeMap<String, Person> = BTreeMap::new();
    let mut works: BTreeMap<&str, (usize, BTreeMap<String, u64>)> = BTreeMap::new();
    let mut plays = Vec::new();
    for event in events {
        let who = name(&event.profile);
        let person = people.entry(who.clone()).or_default();
        let local = event.at as i64 + offset(event.at);
        person.days.insert(local.div_euclid(D));
        person.last = person.last.max(Some(event.at));
        if !event.device.is_empty() {
            person.devices.insert(event.device.clone());
        }
        match event.activity {
            Activity::SignIn => person.sign_ins += 1,
            Activity::Visit => {}
            Activity::Play => {
                let tv = event.device == "TV";
                person.plays += 1;
                if tv {
                    person.on_tv += event.seconds;
                } else {
                    person.on_phone += event.seconds;
                }
                let work = works.entry(work(&event.title)).or_default();
                work.0 += 1;
                *work.1.entry(who.clone()).or_default() += event.seconds;
                let end = local + event.seconds as i64;
                let mut at = local;
                while at < end {
                    let next = ((at.div_euclid(H) + 1) * H).min(end);
                    let seconds = (next - at) as u64;
                    let day = at.div_euclid(D);
                    hours[weekday(day)][(at.rem_euclid(D) / H) as usize] += seconds;
                    let index = (day - start).div_euclid(width);
                    if let Some(bucket) = usize::try_from(index).ok().and_then(|index| buckets.get_mut(index)) {
                        if tv {
                            bucket.tv += seconds;
                        } else {
                            bucket.phone += seconds;
                        }
                    }
                    at = next;
                }
                plays.push(Played {
                    at: event.at,
                    when: crate::details::when_local(local_now, local),
                    name: who,
                    device: event.device.clone(),
                    tv,
                    title: event.title.clone(),
                    seconds: event.seconds,
                });
            }
        }
    }

    let favourite = |who: &str| {
        works
            .iter()
            .filter_map(|(title, (_, by))| by.get(who).map(|seconds| (*seconds, *title)))
            .min_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)))
            .map(|(_, title)| title.to_string())
    };
    let mut someone: Vec<Someone> = people
        .iter()
        .map(|(who, person)| Someone {
            name: who.clone(),
            tv: person.on_tv,
            phone: person.on_phone,
            plays: person.plays,
            days: person.days.len(),
            sign_ins: person.sign_ins,
            favourite: favourite(who),
            last_seen: person
                .last
                .map(|at| crate::details::when_local(local_now, at as i64 + offset(at))),
            devices: person.devices.iter().cloned().collect(),
        })
        .collect();
    someone.sort_by(|a, b| {
        (b.tv + b.phone)
            .cmp(&(a.tv + a.phone))
            .then(b.plays.cmp(&a.plays))
            .then(a.name.cmp(&b.name))
    });

    let mut titles: Vec<Watched> = works
        .iter()
        .map(|(title, (count, by))| {
            let mut shares: Vec<Share> = by
                .iter()
                .map(|(who, seconds)| Share {
                    name: who.clone(),
                    seconds: *seconds,
                })
                .collect();
            shares.sort_by(|a, b| b.seconds.cmp(&a.seconds).then(a.name.cmp(&b.name)));
            Watched {
                title: title.to_string(),
                seconds: by.values().sum(),
                plays: *count,
                people: shares,
            }
        })
        .collect();
    titles.sort_by(|a, b| b.seconds.cmp(&a.seconds).then(a.title.cmp(&b.title)));
    let totals = Totals {
        tv: someone.iter().map(|person| person.tv).sum(),
        phone: someone.iter().map(|person| person.phone).sum(),
        plays: plays.len(),
        people: someone.iter().filter(|person| person.plays > 0).count(),
        titles: titles.len(),
    };
    titles.truncate(LISTED_TITLES);
    plays.sort_by_key(|play| std::cmp::Reverse(play.at));
    plays.truncate(RECENT_PLAYS);
    Summary {
        days,
        totals,
        span,
        buckets,
        hours,
        people: someone,
        titles,
        plays,
    }
}

fn weekday(day: i64) -> usize {
    (day + 3).rem_euclid(7) as usize
}

fn bucket(span: Span, days: u64, day: i64) -> Bucket {
    let date = Date::from_unix((day.max(0) as u64) * DAY);
    let month = crate::details::MONTHS[usize::from(date.month().clamp(1, 12)) - 1];
    let weekday = WEEKDAYS[weekday(day)];
    let (label, mark) = match span {
        Span::Day if days <= 7 => (format!("{weekday} {} {month}", date.day()), weekday.to_string()),
        Span::Day => (format!("{weekday} {} {month}", date.day()), date.day().to_string()),
        Span::Week => (
            format!("Week of {} {month}", date.day()),
            format!("{} {month}", date.day()),
        ),
    };
    Bucket {
        label,
        mark,
        tv: 0,
        phone: 0,
    }
}

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

    fn sample() -> (Vec<Event>, BTreeMap<ProfileId, String>, u64) {
        let tuesday_noon = 1_794_916_800;
        let names = BTreeMap::from([
            (ProfileId(1), "Frederik".to_string()),
            (ProfileId(2), "Mami".to_string()),
        ]);
        let events = vec![
            event(tuesday_noon, Activity::SignIn, 2, "iPad", "", 0),
            event(tuesday_noon + 60, Activity::Play, 2, "iPad", "Dark · S01E01", 3_000),
            event(tuesday_noon + 2 * HOUR, Activity::Visit, 2, "iPad", "", 0),
            event(
                tuesday_noon + 3 * HOUR + 1_800,
                Activity::Play,
                1,
                "TV",
                "Arrival",
                6_900,
            ),
            event(tuesday_noon + DAY + 60, Activity::Play, 9, "Laptop", "Arrival", 20),
            event(
                tuesday_noon + DAY + 11 * HOUR + 1_800,
                Activity::Play,
                1,
                "TV",
                "Dark · S01E02",
                3_300,
            ),
        ];
        (events, names, tuesday_noon + 2 * DAY)
    }

    #[test]
    fn the_summary_adds_up_each_person_and_title() {
        let (events, names, now) = sample();
        let summary = summary(&events, &names, now, 7, |_| 0);
        assert_eq!(
            summary.totals,
            Totals {
                tv: 10_200,
                phone: 3_020,
                plays: 4,
                people: 3,
                titles: 2
            }
        );
        let people: Vec<_> = summary
            .people
            .iter()
            .map(|person| {
                (
                    person.name.as_str(),
                    person.tv,
                    person.phone,
                    person.plays,
                    person.days,
                    person.sign_ins,
                    person.favourite.as_deref(),
                    person.last_seen.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            people,
            [
                ("Frederik", 10_200, 0, 2, 2, 0, Some("Arrival"), Some("yesterday 23:30")),
                ("Mami", 0, 3_000, 1, 1, 1, Some("Dark"), Some("Tuesday 14:00")),
                ("#9", 0, 20, 1, 1, 0, Some("Arrival"), Some("yesterday 12:01")),
            ]
        );
        assert_eq!(summary.people[0].devices, ["TV"]);
        let titles: Vec<_> = summary
            .titles
            .iter()
            .map(|title| {
                (
                    title.title.as_str(),
                    title.seconds,
                    title.plays,
                    title
                        .people
                        .iter()
                        .map(|share| (share.name.as_str(), share.seconds))
                        .collect(),
                )
            })
            .collect();
        assert_eq!(
            titles,
            [
                ("Arrival", 6_920, 2, vec![("Frederik", 6_900), ("#9", 20)]),
                ("Dark", 6_300, 2, vec![("Frederik", 3_300), ("Mami", 3_000)]),
            ]
        );
        let plays: Vec<(&str, &str, &str, bool)> = summary
            .plays
            .iter()
            .map(|play| (play.when.as_str(), play.name.as_str(), play.title.as_str(), play.tv))
            .collect();
        assert_eq!(
            plays,
            [
                ("yesterday 23:30", "Frederik", "Dark · S01E02", true),
                ("yesterday 12:01", "#9", "Arrival", false),
                ("Tuesday 15:30", "Frederik", "Arrival", true),
                ("Tuesday 12:01", "Mami", "Dark · S01E01", false),
            ]
        );
    }

    #[test]
    fn watched_time_spreads_over_the_days_and_hours_it_covered() {
        let (events, names, now) = sample();
        let summary = summary(&events, &names, now, 7, |_| 0);
        assert_eq!(summary.span, Span::Day);
        let days: Vec<(&str, &str, u64, u64)> = summary
            .buckets
            .iter()
            .map(|bucket| (bucket.label.as_str(), bucket.mark.as_str(), bucket.tv, bucket.phone))
            .collect();
        assert_eq!(
            days,
            [
                ("Fri 13 Nov", "Fri", 0, 0),
                ("Sat 14 Nov", "Sat", 0, 0),
                ("Sun 15 Nov", "Sun", 0, 0),
                ("Mon 16 Nov", "Mon", 0, 0),
                ("Tue 17 Nov", "Tue", 6_900, 3_000),
                ("Wed 18 Nov", "Wed", 1_800, 20),
                ("Thu 19 Nov", "Thu", 1_500, 0),
            ]
        );
        let tuesday = summary.hours[1];
        assert_eq!(
            (tuesday[12], tuesday[15], tuesday[16], tuesday[17]),
            (3_000, 1_800, 3_600, 1_500)
        );
        assert_eq!(
            (summary.hours[2][12], summary.hours[2][23], summary.hours[3][0]),
            (20, 1_800, 1_500)
        );
        assert_eq!(summary.hours.iter().flatten().sum::<u64>(), 13_220);

        let local = summary_of(&events, &names, now, 7, 3_600);
        assert_eq!((local.hours[1][12], local.hours[1][13]), (0, 3_000));
        assert_eq!((local.hours[3][0], local.hours[3][1]), (1_800, 1_500));
    }

    fn summary_of(events: &[Event], names: &BTreeMap<ProfileId, String>, now: u64, days: u64, offset: i64) -> Summary {
        summary(events, names, now, days, move |_| offset)
    }

    #[test]
    fn a_long_window_counts_by_the_week_and_a_month_by_the_day() {
        let (events, names, now) = sample();
        let year = summary(&events, &names, now, 365, |_| 0);
        assert_eq!(year.span, Span::Week);
        assert_eq!(year.buckets.len(), 53);
        let last = year.buckets.last().unwrap();
        assert_eq!(
            (last.label.as_str(), last.mark.as_str(), last.tv, last.phone),
            ("Week of 16 Nov", "16 Nov", 10_200, 3_020)
        );
        let month = summary(&events, &names, now, 30, |_| 0);
        assert_eq!(month.span, Span::Day);
        assert_eq!(month.buckets.len(), 30);
        assert_eq!(month.buckets[0].mark, "21");
        let empty = summary(&[], &names, now, 7, |_| 0);
        assert!(empty.people.is_empty() && empty.titles.is_empty() && empty.plays.is_empty());
        assert_eq!(empty.buckets.len(), 7);
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
