use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Arc;
use std::time::Duration;

const API: &str = "https://api.theintrodb.org/v4/media";
const TIMEOUT: Duration = Duration::from_secs(8);
const LONGEST_OPENING_INTRO: f64 = 95.0;
const SAME_TITLES: f64 = 8.0;

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

impl Range {
    fn seconds(&self) -> Option<(f64, f64)> {
        let start = self.start_ms.unwrap_or(0) as f64 / 1000.0;
        let end = self.end_ms? as f64 / 1000.0;
        (start < end).then_some((start, end))
    }
}

impl Media {
    fn intro_lengths(&self) -> Vec<f64> {
        self.intro
            .iter()
            .filter_map(Range::seconds)
            .map(|(start, end)| end - start)
            .collect()
    }

    /// Whether an intro starts at 0:00 and runs too long to be only the
    /// titles: a cold open, unless other episodes have titles that long.
    fn long_opening_intro(&self) -> bool {
        self.intro
            .iter()
            .filter_map(Range::seconds)
            .any(|(start, end)| start == 0.0 && end > LONGEST_OPENING_INTRO)
    }
}

/// `titles` are the intro lengths of the show's neighbouring episodes.
fn first(kind: Kind, ranges: &[Range], duration: f64, titles: &[f64]) -> Option<Segment> {
    ranges
        .iter()
        .filter_map(|range| {
            let (start, end) = range.seconds()?;
            let repeated = titles
                .iter()
                .any(|length| (length - (end - start)).abs() <= SAME_TITLES);
            let cold_open = kind == Kind::Intro && start == 0.0 && end > LONGEST_OPENING_INTRO && !repeated;
            (end < duration && !cold_open).then_some(Segment { kind, start, end })
        })
        .min_by(|a, b| a.start.total_cmp(&b.start))
}

fn segments(media: &Media, duration: f64, titles: &[f64]) -> Vec<Segment> {
    [
        first(Kind::Recap, &media.recap, duration, &[]),
        first(Kind::Intro, &media.intro, duration, titles),
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

fn found(media: &Media, duration: f64, titles: &[f64]) -> Found {
    Found {
        segments: segments(media, duration, titles),
        credits: credits(media, duration),
    }
}

/// Finds a title's segments with `lookup`, which answers a title's media,
/// for a file of the given duration when there is one. An intro from 0:00
/// that runs long is kept only when the episode before or after has an
/// intro as long; otherwise it is taken for a cold open.
fn find<E>(
    lookup: impl Fn(Title, Option<f64>) -> Result<Option<Media>, E>,
    title: Title,
    duration: f64,
) -> Result<Found, E> {
    let Some(media) = lookup(title, Some(duration))? else {
        return Ok(Found::default());
    };
    let titles = match title.episode {
        Some((season, episode)) if media.long_opening_intro() => [episode.checked_sub(1), episode.checked_add(1)]
            .into_iter()
            .flatten()
            .filter(|&episode| episode > 0)
            .filter_map(|episode| {
                let neighbour = Title {
                    episode: Some((season, episode)),
                    ..title
                };
                lookup(neighbour, None).ok().flatten()
            })
            .flat_map(|media| media.intro_lengths())
            .collect(),
        _ => Vec::new(),
    };
    Ok(found(&media, duration, &titles))
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
    let lookup = |title: Title, duration: Option<f64>| {
        let mut request = agent
            .get(api.as_deref().unwrap_or(API))
            .query("tmdb_id", title.tmdb.to_string());
        if let Some(duration) = duration {
            request = request.query("duration_ms", ((duration * 1000.0) as u64).to_string());
        }
        if let Some((season, episode)) = title.episode {
            request = request
                .query("season", season.to_string())
                .query("episode", episode.to_string());
        }
        match request.call() {
            Ok(mut response) => Ok(Some(response.body_mut().read_json()?)),
            Err(ureq::Error::StatusCode(404)) => Ok(None),
            Err(error) => Err(error),
        }
    };
    find(lookup, title, duration)
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
        segments(&serde_json::from_str(json).unwrap(), HOUR, &[])
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
        let credits = |json: &str| found(&serde_json::from_str(json).unwrap(), HOUR, &[]).credits;
        assert_eq!(
            credits(r#"{"credits":[{"start_ms":3485348,"end_ms":null}]}"#),
            Some(3485.348)
        );
        assert_eq!(credits(r#"{"credits":[{"start_ms":3700000,"end_ms":null}]}"#), None);
        assert_eq!(credits(r#"{"credits":[{"start_ms":null,"end_ms":3600000}]}"#), None);
        assert_eq!(credits(r#"{"intro":[]}"#), None);
    }

    #[test]
    fn a_long_intro_from_the_start_counts_when_a_neighbouring_episode_has_titles_as_long() {
        let intro = |start: Option<u64>, end: u64| {
            format!(
                r#"{{"intro":[{{"start_ms":{},"end_ms":{end}}}]}}"#,
                start.map_or("null".to_string(), |start| start.to_string())
            )
        };
        let thrones = |episodes: Vec<(u32, String)>| {
            move |title: Title, _: Option<f64>| -> Result<Option<Media>, ()> {
                Ok(episodes
                    .iter()
                    .find(|(episode, _)| title.episode == Some((1, *episode)))
                    .map(|(_, json)| serde_json::from_str(json).unwrap()))
            }
        };
        let episode = |number| Title {
            tmdb: 1399,
            episode: Some((1, number)),
        };
        let intro_of = |found: Found| {
            found
                .segments
                .iter()
                .find(|segment| segment.kind == Kind::Intro)
                .map(|segment| segment.end)
        };
        let got = thrones(vec![
            (4, intro(None, 106434)),
            (5, intro(None, 111230)),
            (6, intro(Some(8245), 115454)),
        ]);
        assert_eq!(intro_of(find(&got, episode(5), HOUR).unwrap()), Some(111.23));
        assert_eq!(intro_of(find(&got, episode(4), HOUR).unwrap()), Some(106.434));
        let community = thrones(vec![
            (20, intro(Some(193000), 256000)),
            (21, intro(None, 114000)),
            (3, intro(Some(232956), 257917)),
            (4, intro(None, 112000)),
            (5, intro(Some(201000), 299000)),
        ]);
        assert_eq!(intro_of(find(&community, episode(21), HOUR).unwrap()), None);
        assert_eq!(intro_of(find(&community, episode(4), HOUR).unwrap()), None);
        let alone = thrones(vec![(1, intro(None, 112000))]);
        assert_eq!(intro_of(find(&alone, episode(1), HOUR).unwrap()), None);
        assert_eq!(find(&alone, episode(2), HOUR).unwrap(), Found::default());
    }
}
