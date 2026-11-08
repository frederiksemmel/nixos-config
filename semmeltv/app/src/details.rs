use crate::manifest::Scores;
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
}

impl From<&Scores> for Ratings {
    fn from(scores: &Scores) -> Self {
        Self {
            imdb: scores.imdb.map(|score| score.to_string()).unwrap_or_default(),
            rt: scores.rt.map(|score| format!("{score}%")).unwrap_or_default(),
            fresh: scores.rt.is_some_and(|score| score >= FRESH),
        }
    }
}

fn minutes(seconds: f64) -> u64 {
    seconds.max(0.0) as u64 / SECONDS_PER_MINUTE
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
    fn ratings_read_as_imdb_tenths_and_rotten_tomatoes_percent() {
        let scores = Scores {
            imdb: Some(Tenths(80)),
            rt: Some(87),
            metacritic: Some(74),
        };
        let ratings = Ratings::from(&scores);
        assert_eq!(ratings.imdb, "8.0");
        assert_eq!(ratings.rt, "87%");
        assert!(ratings.fresh);
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
    }

    #[test]
    fn durations_round_and_stopping_points_round_down() {
        assert_eq!(clock(minutes(2729.0 + 30.0)), "45m");
        assert_eq!(clock(minutes(2730.0 + 30.0)), "46m");
        assert_eq!(clock(minutes(3599.0)), "59m");
        assert_eq!(clock(minutes(36000.0)), "10h 00m");
    }
}
