use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Arc;
use std::time::Duration;

const API: &str = "https://api.theintrodb.org/v4/media";
const TIMEOUT: Duration = Duration::from_secs(8);
const LONGEST_OPENING_INTRO: f64 = 95.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Title {
    pub tmdb: u64,
    pub episode: Option<(u32, u32)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Recap,
    Intro,
    Next,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Recap => "Skip recap",
            Self::Intro => "Skip intro",
            Self::Next => "Play next",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
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
            let cold_open = kind == Kind::Intro && start == 0.0 && end > LONGEST_OPENING_INTRO;
            (start < end && end < duration && !cold_open).then_some(Segment { kind, start, end })
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

/// Looks up a title's segments for a video of the given duration.
pub type Finder = Arc<dyn Fn(Title, f64) -> Result<Found, String> + Send + Sync>;

/// The finder that asks theintrodb.org.
pub fn finder() -> Finder {
    Arc::new(|title, duration| fetch(title, duration).map_err(|error| error.to_string()))
}

pub fn fetch(title: Title, duration: f64) -> Result<Found, ureq::Error> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    let api = env::var("SEMMELTV_INTRODB").ok().filter(|url| !url.is_empty());
    let response = agent
        .get(api.as_deref().unwrap_or(API))
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
    fn theintrodb_ranges_become_skippable_segments() {
        let segment = |kind, start, end| Segment { kind, start, end };
        let cases = [
            (
                r#"{"tvdb_id":270408,"type":"tv","season":1,"episode":2,
                    "intro":[{"start_ms":47595,"end_ms":120300}],
                    "recap":[{"start_ms":0,"end_ms":45795}],
                    "credits":[{"start_ms":3485348,"end_ms":null}]}"#,
                vec![segment(Kind::Recap, 0.0, 45.795), segment(Kind::Intro, 47.595, 120.3)],
            ),
            (
                r#"{"intro":[{"start_ms":null,"end_ms":71496}]}"#,
                vec![segment(Kind::Intro, 0.0, 71.496)],
            ),
            (r#"{"recap":[{"start_ms":0,"end_ms":0}]}"#, vec![]),
            (r#"{"intro":[{"start_ms":10000,"end_ms":null}]}"#, vec![]),
            (
                r#"{"intro":[{"start_ms":152018,"end_ms":252035},{"start_ms":null,"end_ms":90869}]}"#,
                vec![segment(Kind::Intro, 0.0, 90.869)],
            ),
            (r#"{"intro":[{"start_ms":3590000,"end_ms":3700000}]}"#, vec![]),
            (r#"{"intro":[{"start_ms":null,"end_ms":117000}]}"#, vec![]),
            (
                r#"{"intro":[{"start_ms":0,"end_ms":130000},{"start_ms":150000,"end_ms":175000}]}"#,
                vec![segment(Kind::Intro, 150.0, 175.0)],
            ),
            (
                r#"{"recap":[{"start_ms":null,"end_ms":120000}]}"#,
                vec![segment(Kind::Recap, 0.0, 120.0)],
            ),
        ];
        for (json, expected) in cases {
            assert_eq!(parse(json), expected, "{json}");
        }
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
}
