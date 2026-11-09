use crate::library::normalize;
use crate::parse::parse_release;
use crate::tags::{self, Tags};
use regex::Regex;
use std::sync::LazyLock;

static SEASONS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:^|[^a-z0-9])S(\d{1,2})[ ._-]*-[ ._-]*S(\d{1,2})(?:[^a-z0-9]|$)").unwrap());
static EPISODES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[^a-z0-9])S(\d{1,2})[ ._-]?E(\d{1,3})(?:[ ._]?-[ ._]?E?(\d{1,3}))?(?:[^a-z0-9]|$)").unwrap()
});
static SEASON: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(?:^|[^a-z0-9])S(\d{1,2})(?:[^a-z0-9]|$)").unwrap());
static SEASON_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:^|[^a-z0-9])season[ ._-]*(\d{1,2})(?:[^a-z0-9]|$)").unwrap());
static COMPLETE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(?:^|[^a-z0-9])complete(?:[^a-z0-9]|$)").unwrap());
static BAD_SOURCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:cam|camrip|hdcam|ts|hdts|telesync|tc|hdtc|telecine|scr|screener|dvdscr|r5)\b").unwrap()
});
static SAMPLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(?:sample|trailer)\b").unwrap());
static SOURCE: LazyLock<Vec<(Regex, Source)>> = LazyLock::new(|| {
    [
        (r"\bremux\b", Source::Remux),
        (r"\b(?:blu ?ray|bdrip|brrip|bd)\b", Source::BluRay),
        (r"\bweb ?dl\b", Source::WebDl),
        (r"\bweb ?rip\b", Source::WebRip),
        (r"\bweb\b", Source::WebDl),
        (r"\bhdtv\b", Source::Hdtv),
    ]
    .into_iter()
    .map(|(pattern, source)| (Regex::new(pattern).unwrap(), source))
    .collect()
});
static DUBBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:ita|italian|french|truefrench|vff|vfq|german|spanish|castellano|latino|multi|dubbed|rus|hindi)\b")
        .unwrap()
});
static INFO_START: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[^a-z0-9])((?:2160|1080|720|576|480)[pi]|[xh]\.?26[45]|hevc|av1)(?:[^a-z0-9]|$)").unwrap()
});
static DECIMAL_DOT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d)\.(\d)").unwrap());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coverage {
    Film,
    Season(u32),
    Seasons(u32, u32),
    Episodes { season: u32, first: u32, last: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Remux,
    BluRay,
    WebDl,
    WebRip,
    Hdtv,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Self::Remux => "Remux",
            Self::BluRay => "BluRay",
            Self::WebDl => "WEB-DL",
            Self::WebRip => "WEBRip",
            Self::Hdtv => "HDTV",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub title: String,
    pub year: Option<u16>,
    pub coverage: Coverage,
    pub tags: Tags,
    pub source: Option<Source>,
    pub bad_source: bool,
    pub sample: bool,
    pub dubbed: bool,
    pub info: Option<String>,
}

impl Release {
    pub fn parse(name: &str) -> Self {
        let (coverage, start) = coverage(name);
        let (title, year) = parse_release(start.map_or(name, |start| &name[..start]));
        let words = normalize(name);
        Self {
            title,
            year,
            coverage,
            tags: tags::from_name(name),
            source: SOURCE
                .iter()
                .find(|(pattern, _)| pattern.is_match(&words))
                .map(|(_, source)| *source),
            bad_source: BAD_SOURCE.is_match(&words),
            sample: SAMPLE.is_match(&words),
            dubbed: DUBBED.is_match(&words),
            info: info(name),
        }
    }

    pub fn matches_title(&self, titles: &[String]) -> bool {
        let found = normalize(&deunicode::deunicode(&self.title));
        titles.iter().any(|title| *title == found)
    }
}

pub fn comparable(title: &str) -> String {
    normalize(&deunicode::deunicode(&title.replace('&', "and")))
}

fn coverage(name: &str) -> (Coverage, Option<usize>) {
    let number = |found: &regex::Captures, index: usize| found.get(index).and_then(|m| m.as_str().parse().ok());
    if let Some(found) = SEASONS.captures(name)
        && let (Some(first), Some(last)) = (number(&found, 1), number(&found, 2))
    {
        return (Coverage::Seasons(first, last), Some(found.get(0).unwrap().start()));
    }
    if let Some(found) = EPISODES.captures(name)
        && let (Some(season), Some(first)) = (number(&found, 1), number(&found, 2))
    {
        let last = number(&found, 3).filter(|last| *last >= first).unwrap_or(first);
        let start = found.get(0).unwrap().start();
        return (Coverage::Episodes { season, first, last }, Some(start));
    }
    for pattern in [&*SEASON, &*SEASON_WORD] {
        if let Some(found) = pattern.captures(name)
            && let Some(season) = number(&found, 1)
        {
            return (Coverage::Season(season), Some(found.get(0).unwrap().start()));
        }
    }
    if let Some(found) = COMPLETE.find(name) {
        return (Coverage::Seasons(1, u32::MAX), Some(found.start()));
    }
    (Coverage::Film, None)
}

fn info(name: &str) -> Option<String> {
    let start = INFO_START.captures(name)?.get(1)?.start();
    let rest = &name[start..];
    let spaced = if rest.contains(' ') {
        rest.to_string()
    } else {
        DECIMAL_DOT.replace_all(rest, "$1\u{0}$2").replace('.', " ").replace('\0', ".")
    };
    let cleaned: String = spaced
        .chars()
        .filter(|c| !matches!(c, '(' | ')' | '[' | ']' | '{' | '}'))
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    (!cleaned.is_empty()).then_some(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(name: &str) -> (String, Option<u16>, Coverage) {
        let release = Release::parse(name);
        (release.title, release.year, release.coverage)
    }

    #[test]
    fn season_packs_and_episodes_are_told_apart() {
        assert_eq!(
            parsed("Outlander (2014) S08 (1080p AMZN WEB-DL x265 10bit EAC3 5.1 Silence)"),
            ("Outlander".to_string(), Some(2014), Coverage::Season(8))
        );
        assert_eq!(
            parsed("Outlander.S08E10.1080p.WEB.h264-GRACE"),
            (
                "Outlander".to_string(),
                None,
                Coverage::Episodes {
                    season: 8,
                    first: 10,
                    last: 10
                }
            )
        );
        assert_eq!(
            parsed("Outlander S08e01-10 [1080p Ita Eng Spa h265 10bit SubS] byMe7alh [MIRCrew]"),
            (
                "Outlander".to_string(),
                None,
                Coverage::Episodes {
                    season: 8,
                    first: 1,
                    last: 10
                }
            )
        );
        assert_eq!(
            parsed("Outlander S05 1080p BluRay AV1 PTNX"),
            ("Outlander".to_string(), None, Coverage::Season(5))
        );
        assert_eq!(
            parsed("Outlander Season 8 Complete 1080p"),
            ("Outlander".to_string(), None, Coverage::Season(8))
        );
        assert_eq!(
            parsed("Outlander S01-S07 1080p BluRay x265"),
            ("Outlander".to_string(), None, Coverage::Seasons(1, 7))
        );
    }

    #[test]
    fn a_spin_off_keeps_its_own_title() {
        let release = Release::parse("Outlander Blood of My Blood S02E08 1080p HEVC x265 MeGusta");
        assert_eq!(release.title, "Outlander Blood of My Blood");
        assert!(!release.matches_title(&[comparable("Outlander")]));
        assert!(Release::parse("Outlander.S08E10.1080p.x265-ELiTE").matches_title(&[comparable("Outlander")]));
    }

    #[test]
    fn films_keep_title_and_year() {
        assert_eq!(
            parsed("Dune.Part.Two.2024.1080p.WEB-DL.DDP5.1.Atmos.H.265-FLUX"),
            ("Dune Part Two".to_string(), Some(2024), Coverage::Film)
        );
        assert_eq!(
            parsed("Blade Runner 2049 (2017) 1080p BrRip x264 - YIFY"),
            ("Blade Runner 2049".to_string(), Some(2017), Coverage::Film)
        );
    }

    #[test]
    fn quality_facts_come_from_the_name() {
        let release = Release::parse("Outlander (2014) S08 (1080p AMZN WEB-DL x265 10bit EAC3 5.1 Silence)");
        assert_eq!(release.tags.video.as_deref(), Some("HEVC 10-bit"));
        assert_eq!(release.tags.resolution.as_deref(), Some("1080p"));
        assert_eq!(release.tags.audio.as_deref(), Some("5.1"));
        assert_eq!(release.source, Some(Source::WebDl));
        assert!(!release.dubbed && !release.bad_source && !release.sample);
        assert_eq!(
            release.info.as_deref(),
            Some("1080p AMZN WEB-DL x265 10bit EAC3 5.1 Silence")
        );
        let dotted = Release::parse("Outlander.S08E10.1080p.WEB.h264-GRACE");
        assert_eq!(dotted.tags.video.as_deref(), Some("H.264"));
        assert_eq!(dotted.info.as_deref(), Some("1080p WEB h264-GRACE"));
        let dubbed = Release::parse("Outlander S08e01-10 [1080p Ita Eng Spa h265 10bit SubS] byMe7alh [MIRCrew]");
        assert!(dubbed.dubbed);
        assert_eq!(
            Release::parse("Dune.Part.Two.2024.1080p.WEB-DL.DDP5.1.Atmos.H.265-FLUX").info.as_deref(),
            Some("1080p WEB-DL DDP5.1 Atmos H 265-FLUX")
        );
    }

    #[test]
    fn cams_and_samples_are_flagged() {
        assert!(Release::parse("Dune Part Two 2024 HDCAM x264").bad_source);
        assert!(Release::parse("Dune.Part.Two.2024.TS.x264").bad_source);
        assert!(!Release::parse("Dune.Part.Two.2024.1080p.DTS-HD.MA.5.1").bad_source);
        assert!(Release::parse("Dune.Part.Two.2024.1080p.Sample").sample);
    }
}
