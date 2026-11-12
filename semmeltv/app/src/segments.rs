use serde::Deserialize;
use std::time::Duration;

const API: &str = "https://api.theintrodb.org/v4/media";
const TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Title {
    pub tmdb: u64,
    pub episode: Option<(u32, u32)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Recap,
    Intro,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Recap => "Skip recap",
            Self::Intro => "Skip intro",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub kind: Kind,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Deserialize)]
struct Range {
    start_ms: Option<u64>,
    end_ms: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct Media {
    #[serde(default)]
    recap: Vec<Range>,
    #[serde(default)]
    intro: Vec<Range>,
    #[serde(default)]
    credits: Vec<Range>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    pub segments: Vec<Segment>,
    pub credits: Option<f64>,
}

fn first(kind: Kind, ranges: &[Range], duration: f64) -> Option<Segment> {
    ranges
        .iter()
        .filter_map(|range| {
            let start = range.start_ms.unwrap_or(0) as f64 / 1000.0;
            let end = range.end_ms? as f64 / 1000.0;
            (start < end && end < duration).then_some(Segment { kind, start, end })
        })
        .min_by(|a, b| a.start.total_cmp(&b.start))
}

fn segments(media: &Media, duration: f64) -> Vec<Segment> {
    [
        first(Kind::Recap, &media.recap, duration),
        first(Kind::Intro, &media.intro, duration),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn credits(media: &Media, duration: f64) -> Option<f64> {
    media
        .credits
        .iter()
        .filter_map(|range| Some(range.start_ms? as f64 / 1000.0))
        .filter(|start| *start > 0.0 && *start < duration)
        .min_by(f64::total_cmp)
}

fn found(media: &Media, duration: f64) -> Found {
    Found {
        segments: segments(media, duration),
        credits: credits(media, duration),
    }
}

pub fn fetch(title: Title, duration: f64) -> Result<Found, ureq::Error> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    let response = agent
        .get(API)
        .query("tmdb_id", title.tmdb.to_string())
        .query("duration_ms", ((duration * 1000.0) as u64).to_string());
    let response = match title.episode {
        Some((season, episode)) => response
            .query("season", season.to_string())
            .query("episode", episode.to_string()),
        None => response,
    }
    .call();
    match response {
        Ok(mut response) => Ok(found(&response.body_mut().read_json()?, duration)),
        Err(ureq::Error::StatusCode(404)) => Ok(Found::default()),
        Err(error) => Err(error),
    }
}

pub fn at(segments: &[Segment], position: f64) -> Option<Segment> {
    segments
        .iter()
        .find(|segment| (segment.start..segment.end).contains(&position))
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: f64 = 3600.0;

    fn parse(json: &str) -> Vec<Segment> {
        segments(&serde_json::from_str(json).unwrap(), HOUR)
    }

    #[test]
    fn an_episode_gets_its_recap_and_its_intro() {
        let json = r#"{"tvdb_id":270408,"type":"tv","season":1,"episode":2,
            "intro":[{"start_ms":47595,"end_ms":120300}],
            "recap":[{"start_ms":0,"end_ms":45795}],
            "credits":[{"start_ms":3485348,"end_ms":null}]}"#;
        assert_eq!(
            parse(json),
            [
                Segment {
                    kind: Kind::Recap,
                    start: 0.0,
                    end: 45.795
                },
                Segment {
                    kind: Kind::Intro,
                    start: 47.595,
                    end: 120.3
                },
            ]
        );
    }

    #[test]
    fn the_credits_start_at_the_earliest_known_start_inside_this_file() {
        let credits = |json: &str| found(&serde_json::from_str(json).unwrap(), HOUR).credits;
        assert_eq!(
            credits(r#"{"credits":[{"start_ms":3485348,"end_ms":null}]}"#),
            Some(3485.348)
        );
        assert_eq!(credits(r#"{"credits":[{"start_ms":3700000,"end_ms":null}]}"#), None);
        assert_eq!(credits(r#"{"credits":[{"start_ms":null,"end_ms":3600000}]}"#), None);
        assert_eq!(credits(r#"{"intro":[]}"#), None);
    }

    #[test]
    fn a_missing_start_means_the_beginning_of_the_episode() {
        let intro = parse(r#"{"intro":[{"start_ms":null,"end_ms":71496}]}"#);
        assert_eq!(intro[0].start, 0.0);
    }

    #[test]
    fn a_zero_length_range_says_there_is_nothing_to_skip() {
        assert!(parse(r#"{"recap":[{"start_ms":0,"end_ms":0}]}"#).is_empty());
    }

    #[test]
    fn a_range_without_an_end_is_ignored() {
        assert!(parse(r#"{"intro":[{"start_ms":10000,"end_ms":null}]}"#).is_empty());
    }

    #[test]
    fn only_the_earliest_of_several_ranges_counts() {
        let intro = parse(r#"{"intro":[{"start_ms":152018,"end_ms":252035},{"start_ms":null,"end_ms":90869}]}"#);
        assert_eq!(intro.len(), 1);
        assert_eq!(intro[0].end, 90.869);
    }

    #[test]
    fn a_range_past_the_end_of_this_file_is_from_another_release() {
        assert!(parse(r#"{"intro":[{"start_ms":3590000,"end_ms":3700000}]}"#).is_empty());
    }

    #[test]
    fn a_segment_is_active_from_its_start_up_to_its_end() {
        let intro = Segment {
            kind: Kind::Intro,
            start: 10.0,
            end: 20.0,
        };
        assert_eq!(at(&[intro], 9.9), None);
        assert_eq!(at(&[intro], 10.0), Some(intro));
        assert_eq!(at(&[intro], 19.9), Some(intro));
        assert_eq!(at(&[intro], 20.0), None);
    }
}
