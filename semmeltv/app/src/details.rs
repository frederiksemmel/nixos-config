use crate::RatingsData;
use crate::manifest::{Date, Scores};
use crate::tags::Tags;

const SEPARATOR: &str = " · ";
const SECONDS_PER_MINUTE: u64 = 60;
const FRESH: u8 = 60;

#[derive(Debug, Default)]
pub struct Details<'a> {
    pub episode: Option<String>,
    pub tags: Option<&'a Tags>,
    pub duration: Option<f64>,
    pub stopped_at: Option<f64>,
    pub download: &'a str,
}

impl Details<'_> {
    pub fn line(&self) -> String {
        let tag = |read: fn(&Tags) -> &Option<String>| self.tags.and_then(|tags| read(tags).clone());
        let parts = [
            self.episode.clone(),
            tag(|tags| &tags.video),
            tag(|tags| &tags.resolution),
            tag(|tags| &tags.audio),
            self.duration.map(|seconds| clock(minutes(seconds + 30.0))),
            self.stopped_at
                .map(minutes)
                .filter(|minutes| *minutes > 0)
                .map(|minutes| format!("Stopped at {}", clock(minutes))),
            Some(self.download.to_string()),
        ];
        let known: Vec<_> = parts.into_iter().flatten().filter(|part| !part.is_empty()).collect();
        known.join(SEPARATOR)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Ratings {
    pub imdb: String,
    pub rt: String,
    pub fresh: bool,
    pub popcorn: String,
    pub full: bool,
}

fn percent(score: Option<u8>) -> String {
    score.map(|score| format!("{score}%")).unwrap_or_default()
}

impl From<&Scores> for Ratings {
    fn from(scores: &Scores) -> Self {
        Self {
            imdb: scores.imdb.map(|score| score.to_string()).unwrap_or_default(),
            rt: percent(scores.rt),
            fresh: scores.rt.is_some_and(|score| score >= FRESH),
            popcorn: percent(scores.popcorn),
            full: scores.popcorn.is_some_and(|score| score >= FRESH),
        }
    }
}

impl From<Ratings> for RatingsData {
    fn from(ratings: Ratings) -> Self {
        Self {
            imdb: ratings.imdb.into(),
            rt: ratings.rt.into(),
            fresh: ratings.fresh,
            popcorn: ratings.popcorn.into(),
            full: ratings.full,
        }
    }
}

pub fn time_left(seconds: f64) -> String {
    format!("{} left", clock(minutes(seconds + 30.0).max(1)))
}

fn minutes(seconds: f64) -> u64 {
    seconds.max(0.0) as u64 / SECONDS_PER_MINUTE
}

const SECONDS_PER_DAY: i64 = 24 * 60 * 60;
const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

pub fn local_offset(at: u64) -> i64 {
    let time = at as libc::time_t;
    let mut local: libc::tm = unsafe { std::mem::zeroed() };
    let converted = unsafe { libc::localtime_r(&raw const time, &raw mut local) };
    if converted.is_null() { 0 } else { local.tm_gmtoff }
}

pub fn when(now: u64, then: u64) -> String {
    when_local(now as i64 + local_offset(now), then as i64 + local_offset(then))
}

fn when_local(now: i64, then: i64) -> String {
    let day = then.div_euclid(SECONDS_PER_DAY);
    let seconds = then.rem_euclid(SECONDS_PER_DAY);
    let time = format!("{:02}:{:02}", seconds / 3600, seconds % 3600 / 60);
    let date = Date::from_unix((day * SECONDS_PER_DAY).max(0) as u64);
    let month = MONTHS[usize::from(date.month().clamp(1, 12)) - 1];
    let this_year = Date::from_unix(now.max(0) as u64).year();
    match now.div_euclid(SECONDS_PER_DAY) - day {
        0 => format!("today {time}"),
        1 => format!("yesterday {time}"),
        2..=6 => format!("{} {time}", WEEKDAYS[(day + 4).rem_euclid(7) as usize]),
        _ if date.year() == this_year => format!("{} {month} {time}", date.day()),
        _ => format!("{} {month} {}", date.day(), date.year()),
    }
}

fn clock(minutes: u64) -> String {
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes}m"),
        (hours, minutes) => format!("{hours}h {minutes:02}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Tenths;

    fn tags() -> Tags {
        Tags {
            video: Some("HEVC 10-bit".to_string()),
            resolution: Some("1080p".to_string()),
            audio: Some("5.1".to_string()),
        }
    }

    #[test]
    fn a_film_shows_everything_it_knows_in_a_fixed_order() {
        let tags = tags();
        let details = Details {
            tags: Some(&tags),
            duration: Some(7860.0),
            stopped_at: Some(3840.0),
            download: "Downloaded",
            ..Details::default()
        };
        assert_eq!(
            details.line(),
            "HEVC 10-bit · 1080p · 5.1 · 2h 11m · Stopped at 1h 04m · Downloaded"
        );
    }

    #[test]
    fn a_show_names_the_episode_the_values_belong_to() {
        let tags = tags();
        let details = Details {
            episode: Some("S2 E3".to_string()),
            tags: Some(&tags),
            duration: Some(2730.0),
            download: "3 of 10 downloaded",
            ..Details::default()
        };
        assert_eq!(
            details.line(),
            "S2 E3 · HEVC 10-bit · 1080p · 5.1 · 46m · 3 of 10 downloaded"
        );
    }

    #[test]
    fn a_play_reads_as_a_day_and_a_time_close_by_and_a_date_further_back() {
        let now = 1_794_255_000;
        let at = |days: i64, hour: i64, minute: i64| (now / 86_400 - days) * 86_400 + hour * 3600 + minute * 60;
        assert_eq!(when_local(now, at(0, 9, 5)), "today 09:05");
        assert_eq!(when_local(now, at(1, 21, 14)), "yesterday 21:14");
        assert_eq!(when_local(now, at(2, 20, 0)), "Saturday 20:00");
        assert_eq!(when_local(now, at(20, 20, 0)), "20 Oct 20:00");
        assert_eq!(when_local(now, at(400, 20, 0)), "5 Oct 2025");
    }

    #[test]
    fn time_left_rounds_to_the_minute_and_never_reads_zero() {
        assert_eq!(time_left(2700.0), "45m left");
        assert_eq!(time_left(4350.0), "1h 13m left");
        assert_eq!(time_left(5.0), "1m left");
    }

    #[test]
    fn missing_values_are_left_out() {
        assert_eq!(Details::default().line(), "");
        let tags = Tags {
            resolution: Some("720p".to_string()),
            video: Some(String::new()),
            ..Tags::default()
        };
        let details = Details {
            tags: Some(&tags),
            stopped_at: Some(90.0),
            ..Details::default()
        };
        assert_eq!(details.line(), "720p · Stopped at 1m");
    }

    #[test]
    fn a_few_seconds_in_is_not_a_stopping_point() {
        let details = Details {
            duration: Some(3600.0),
            stopped_at: Some(59.0),
            ..Details::default()
        };
        assert_eq!(details.line(), "1h 00m");
    }

    #[test]
    fn ratings_read_as_imdb_tenths_and_rotten_tomatoes_percents() {
        let scores = Scores {
            imdb: Some(Tenths(80)),
            rt: Some(87),
            popcorn: Some(91),
            metacritic: Some(74),
            ..Scores::default()
        };
        let ratings = Ratings::from(&scores);
        assert_eq!(ratings.imdb, "8.0");
        assert_eq!(ratings.rt, "87%");
        assert!(ratings.fresh);
        assert_eq!(ratings.popcorn, "91%");
        assert!(ratings.full);
    }

    #[test]
    fn the_popcorn_bucket_tips_over_below_sixty_percent() {
        let full = |popcorn| {
            Ratings::from(&Scores {
                popcorn: Some(popcorn),
                ..Scores::default()
            })
            .full
        };
        assert!(full(60));
        assert!(!full(59));
    }

    #[test]
    fn rotten_tomatoes_turns_rotten_below_sixty_percent() {
        let fresh = |rt| {
            Ratings::from(&Scores {
                rt: Some(rt),
                ..Scores::default()
            })
            .fresh
        };
        assert!(fresh(60));
        assert!(!fresh(59));
        assert!(!fresh(0));
    }

    #[test]
    fn missing_ratings_stay_empty() {
        assert_eq!(Ratings::from(&Scores::default()), Ratings::default());
        let only_imdb = Ratings::from(&Scores {
            imdb: Some(Tenths(73)),
            metacritic: Some(60),
            ..Scores::default()
        });
        assert_eq!(only_imdb.imdb, "7.3");
        assert_eq!(only_imdb.rt, "");
        assert!(!only_imdb.fresh);
        assert_eq!(only_imdb.popcorn, "");
        assert!(!only_imdb.full);
    }

    #[test]
    fn durations_round_and_stopping_points_round_down() {
        assert_eq!(clock(minutes(2729.0 + 30.0)), "45m");
        assert_eq!(clock(minutes(2730.0 + 30.0)), "46m");
        assert_eq!(clock(minutes(3599.0)), "59m");
        assert_eq!(clock(minutes(36000.0)), "10h 00m");
    }
}
