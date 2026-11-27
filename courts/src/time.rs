//! Dates as GotCourts and the pages use them: YYYY-MM-DD in Zurich.

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{ToSpan, Zoned};

fn zurich() -> TimeZone {
    TimeZone::get("Europe/Zurich").unwrap_or(TimeZone::UTC)
}

fn here() -> Zoned {
    Zoned::now().with_time_zone(zurich())
}

pub fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

pub fn today() -> String {
    here().date().to_string()
}

pub fn seconds_today() -> i64 {
    let time = here().time();
    i64::from(time.hour()) * 3600 + i64::from(time.minute()) * 60 + i64::from(time.second())
}

/// Today and the `count - 1` days after it.
pub fn dates(count: i64) -> Vec<String> {
    let today = here().date();
    (0..count.max(1))
        .filter_map(|n| today.checked_add(n.days()).ok())
        .map(|date| date.to_string())
        .collect()
}

pub fn parse(date: &str) -> Option<Date> {
    date.parse().ok()
}

/// "Today", "Tomorrow", or "Sat 5.12.".
pub fn label(date: &str) -> String {
    let Some(day) = parse(date) else {
        return date.to_string();
    };
    let today = here().date();
    if day == today {
        "Today".into()
    } else if today.checked_add(1.day()).ok() == Some(day) {
        "Tomorrow".into()
    } else {
        short(day)
    }
}

/// "Today", or the weekday: "Sat".
pub fn weekday(date: &str) -> String {
    match parse(date) {
        Some(day) if day == here().date() => "Today".into(),
        Some(day) => day.strftime("%a").to_string(),
        None => date.to_string(),
    }
}

/// The coming Saturday and Sunday among `dates` (today, if it is one of them).
pub fn weekend(dates: &[String]) -> Vec<String> {
    use jiff::civil::Weekday;
    let mut days: Vec<String> = Vec::new();
    for date in dates {
        let Some(day) = parse(date) else { continue };
        match day.weekday() {
            Weekday::Saturday => days.push(date.clone()),
            Weekday::Sunday => {
                days.push(date.clone());
                break;
            }
            _ if !days.is_empty() => break,
            _ => {}
        }
    }
    days
}

/// "28.11.", for under the weekday.
pub fn day_month(date: &str) -> String {
    parse(date).map_or_else(|| date.to_string(), |day| day.strftime("%-d.%-m.").to_string())
}

pub fn short(day: Date) -> String {
    day.strftime("%a %-d.%-m.").to_string()
}

/// "a minute ago", for when something was last asked.
pub fn ago(at: i64) -> String {
    match (now() - at).max(0) {
        0..60 => "just now".into(),
        60..120 => "a minute ago".into(),
        seconds @ 120..3600 => format!("{} minutes ago", seconds / 60),
        3600..7200 => "an hour ago".into(),
        seconds => format!("{} hours ago", seconds / 3600),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_count_from_today_and_read_back() {
        let days = dates(3);
        assert_eq!(days.len(), 3);
        assert_eq!(days[0], today());
        assert_eq!(label(&days[0]), "Today");
        assert_eq!(label(&days[1]), "Tomorrow");
        assert_eq!(short("2026-12-05".parse().unwrap()), "Sat 5.12.");
        let around: Vec<String> = ["2026-11-27", "2026-11-28", "2026-11-29", "2026-11-30"]
            .map(String::from)
            .to_vec();
        assert_eq!(weekend(&around), ["2026-11-28", "2026-11-29"]);
        assert_eq!(weekend(&around[2..]), ["2026-11-29"]);
    }
}
