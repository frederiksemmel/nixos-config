use super::release::{Coverage, Release, Source};
use super::resolve::Target;
use super::torznab::Offer;
use crate::tmdb::Kind;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MIN_SEEDERS: u32 = 3;
pub const CHOICES: usize = 3;
const MEGABYTE: f64 = 1_000_000.0;
const RESOLUTION_WEIGHT: f64 = 2.0;
const CODEC_WEIGHT: f64 = 1.5;
const SEEDERS_WEIGHT: f64 = 1.5;
const SIZE_WEIGHT: f64 = 1.0;
const SURROUND_BONUS: f64 = 0.1;
const SOURCE_BONUS: f64 = 0.1;
const DUBBED_PENALTY: f64 = 0.3;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Part {
    pub name: String,
    pub magnet: String,
    pub infohash: String,
    pub size: u64,
    pub seeders: u32,
    pub episodes: Vec<u32>,
    pub info: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    pub label: String,
    pub facts: String,
    pub why: Vec<String>,
    pub score: f64,
    pub size: u64,
    pub seeders: u32,
    pub parts: Vec<Part>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Dropped {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Default, PartialEq, Serialize)]
pub struct Ranking {
    pub choices: Vec<Choice>,
    pub dropped: Vec<Dropped>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unit {
    Whole,
    Episode(u32),
}

#[derive(Clone, Debug)]
struct Rated {
    offer: Offer,
    release: Release,
    unit: Unit,
    score: f64,
    why: Vec<String>,
}

pub fn rank(offers: &[Offer], target: &Target, room: u64) -> Ranking {
    let mut ranking = Ranking::default();
    let mut whole = Vec::new();
    let mut episodes: BTreeMap<u32, Vec<Rated>> = BTreeMap::new();
    for offer in merge(offers) {
        match rate(&offer, target, room) {
            Ok(rated) => match rated.unit {
                Unit::Whole => whole.push(rated),
                Unit::Episode(number) => episodes.entry(number).or_default().push(rated),
            },
            Err(reason) => ranking.dropped.push(Dropped {
                name: offer.name.clone(),
                reason,
            }),
        }
    }
    let best_first = |a: &Rated, b: &Rated| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| b.offer.seeders.cmp(&a.offer.seeders))
    };
    whole.sort_by(best_first);
    ranking.choices = whole.into_iter().take(CHOICES).map(|rated| single(&rated)).collect();
    if ranking.choices.is_empty() && target.season.is_some() && target.episode.is_none() {
        for list in episodes.values_mut() {
            list.sort_by(best_first);
        }
        if let Some(set) = episode_set(&episodes, target, room) {
            ranking.choices.push(set);
        }
    }
    ranking
}

fn merge(offers: &[Offer]) -> Vec<Offer> {
    let mut by_hash: BTreeMap<String, Offer> = BTreeMap::new();
    for offer in offers {
        let key = offer.infohash.to_lowercase();
        match by_hash.get_mut(&key) {
            Some(known) if known.seeders >= offer.seeders => {}
            Some(known) => *known = offer.clone(),
            None => {
                by_hash.insert(key, offer.clone());
            }
        }
    }
    by_hash.into_values().collect()
}

fn unit(release: &Release, target: &Target) -> Result<Unit, String> {
    match (target.kind, target.season, target.episode, release.coverage) {
        (Kind::Movie, _, _, Coverage::Film) => Ok(Unit::Whole),
        (Kind::Movie, _, _, _) => Err("an episode or season, not a film".to_string()),
        (Kind::Tv, _, _, Coverage::Film) => Err("no season or episode in the name".to_string()),
        (Kind::Tv, _, _, Coverage::Seasons(..)) => Err("several seasons in one".to_string()),
        (Kind::Tv, Some(wanted), _, Coverage::Season(found)) if found != wanted => Err(format!("season {found}")),
        (Kind::Tv, Some(wanted), _, Coverage::Episodes { season, .. }) if season != wanted => {
            Err(format!("season {season}"))
        }
        (Kind::Tv, Some(_), None, Coverage::Season(_)) => Ok(Unit::Whole),
        (Kind::Tv, Some(_), None, Coverage::Episodes { first, last, .. }) if first == last => {
            Ok(Unit::Episode(first))
        }
        (Kind::Tv, Some(_), None, Coverage::Episodes { first, last, .. }) => {
            let count = target.episodes.len() as u32;
            if first == 1 && last >= count {
                Ok(Unit::Whole)
            } else {
                Err(format!("only episodes {first} to {last} of {count}"))
            }
        }
        (Kind::Tv, Some(_), Some(wanted), Coverage::Episodes { first, last, .. }) => {
            if first == wanted && last == wanted {
                Ok(Unit::Whole)
            } else {
                Err(format!("episodes {first} to {last}, not {wanted}"))
            }
        }
        (Kind::Tv, Some(_), Some(_), Coverage::Season(_)) => Err("a whole season, not the episode".to_string()),
        (Kind::Tv, None, _, _) => Err("no season asked for".to_string()),
    }
}

fn minutes(target: &Target, unit: Unit) -> Option<f64> {
    match unit {
        Unit::Whole => target.minutes().map(f64::from),
        Unit::Episode(number) => target.episode_minutes(number).map(f64::from),
    }
}

struct Band {
    floor: f64,
    low: f64,
    high: f64,
}

fn band(resolution: Option<&str>) -> Band {
    match resolution {
        Some("2160p") => Band {
            floor: 15.0,
            low: 60.0,
            high: 120.0,
        },
        Some("1080p" | "1080i") => Band {
            floor: 8.0,
            low: 25.0,
            high: 40.0,
        },
        Some("720p") => Band {
            floor: 4.0,
            low: 12.0,
            high: 25.0,
        },
        _ => Band {
            floor: 3.0,
            low: 8.0,
            high: 40.0,
        },
    }
}

fn size_fit(per_minute: f64, band: &Band) -> f64 {
    if per_minute < band.low {
        ((per_minute - band.floor) / (band.low - band.floor)).clamp(0.0, 1.0)
    } else if per_minute > band.high {
        band.high / per_minute
    } else {
        1.0
    }
}

fn rate(offer: &Offer, target: &Target, room: u64) -> Result<Rated, String> {
    let release = Release::parse(&offer.name);
    if !release.matches_title(&target.titles()) {
        return Err(format!("title \"{}\" is not {}", release.title, target.title));
    }
    let unit = unit(&release, target)?;
    if target.kind == Kind::Movie
        && let (Some(found), Some(wanted)) = (release.year, target.year)
        && found.abs_diff(wanted) > 1
    {
        return Err(format!("year {found}, not {wanted}"));
    }
    if release.bad_source {
        return Err("a cam or telesync".to_string());
    }
    if release.sample {
        return Err("a sample or trailer".to_string());
    }
    if offer.seeders < MIN_SEEDERS {
        return Err(format!("{} seeders", offer.seeders));
    }
    if offer.size > room {
        return Err(format!("{} is more than the {} free for downloads", gb(offer.size), gb(room)));
    }
    let resolution = release.tags.resolution.as_deref();
    let band = band(resolution);
    let per_minute = minutes(target, unit).map(|minutes| offer.size as f64 / MEGABYTE / minutes);
    if let Some(per_minute) = per_minute
        && per_minute < band.floor
    {
        return Err(format!("only {per_minute:.0} MB a minute"));
    }

    let mut why = Vec::new();
    let resolution_value = match resolution {
        Some("1080p" | "1080i") => 1.0,
        Some("720p") => 0.6,
        Some("2160p") => 0.3,
        _ => 0.1,
    };
    why.push(resolution.unwrap_or("resolution unknown").to_string());
    let video = release.tags.video.as_deref().unwrap_or("");
    let codec_value = if video.starts_with("HEVC") {
        why.push(format!("{video}, decoded in hardware"));
        1.0
    } else if video.starts_with("AV1") {
        why.push(format!("{video}, decoded in software"));
        0.4
    } else if video.starts_with("H.264") {
        why.push(format!("{video}, decoded in software"));
        0.3
    } else {
        why.push(if video.is_empty() { "codec unknown".to_string() } else { video.to_string() });
        0.0
    };
    let seeders_value = ((1.0 + f64::from(offer.seeders)).log10() / 2.0).min(1.0);
    why.push(format!("{} seeders", offer.seeders));
    let size_value = match per_minute {
        Some(per_minute) => {
            why.push(format!(
                "{} at {per_minute:.0} MB a minute ({:.0}–{:.0} is the aim)",
                gb(offer.size),
                band.low,
                band.high
            ));
            size_fit(per_minute, &band)
        }
        None => {
            why.push(format!("{}, runtime unknown", gb(offer.size)));
            0.5
        }
    };
    let mut extras = 0.0;
    if let Some(audio) = release.tags.audio.as_deref()
        && (audio.starts_with("5.1") || audio.starts_with("7.1") || audio.contains("Atmos"))
    {
        why.push(format!("{audio} sound"));
        extras += SURROUND_BONUS;
    }
    if let Some(source) = release.source
        && matches!(source, Source::WebDl | Source::BluRay | Source::Remux)
    {
        why.push(source.label().to_string());
        extras += SOURCE_BONUS;
    }
    if release.dubbed {
        why.push("tagged with another language".to_string());
        extras -= DUBBED_PENALTY;
    }
    let score = RESOLUTION_WEIGHT * resolution_value
        + CODEC_WEIGHT * codec_value
        + SEEDERS_WEIGHT * seeders_value
        + SIZE_WEIGHT * size_value
        + extras;
    Ok(Rated {
        offer: offer.clone(),
        release,
        unit,
        score,
        why,
    })
}

pub fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

fn facts(release: &Release, size: u64, seeders: u32) -> String {
    let tags = &release.tags;
    let mut facts: Vec<String> = [tags.resolution.clone(), tags.video.clone(), tags.audio.clone()]
        .into_iter()
        .flatten()
        .collect();
    facts.push(gb(size));
    facts.push(format!("{seeders} seeders"));
    facts.join(" · ")
}

fn part(rated: &Rated, episodes: Vec<u32>) -> Part {
    Part {
        name: rated.offer.name.clone(),
        magnet: rated.offer.magnet.clone(),
        infohash: rated.offer.infohash.to_lowercase(),
        size: rated.offer.size,
        seeders: rated.offer.seeders,
        episodes,
        info: rated.release.info.clone(),
    }
}

fn single(rated: &Rated) -> Choice {
    Choice {
        label: rated.offer.name.clone(),
        facts: facts(&rated.release, rated.offer.size, rated.offer.seeders),
        why: rated.why.clone(),
        score: rated.score,
        size: rated.offer.size,
        seeders: rated.offer.seeders,
        parts: vec![part(rated, Vec::new())],
    }
}

fn episode_set(episodes: &BTreeMap<u32, Vec<Rated>>, target: &Target, room: u64) -> Option<Choice> {
    let picked: Vec<(u32, &Rated)> = target
        .episodes
        .iter()
        .filter_map(|episode| Some((episode.number, episodes.get(&episode.number)?.first()?)))
        .collect();
    let first = picked.first()?.1;
    let size: u64 = picked.iter().map(|(_, rated)| rated.offer.size).sum();
    if size > room {
        return None;
    }
    let seeders = picked.iter().map(|(_, rated)| rated.offer.seeders).min().unwrap_or(0);
    let numbers: Vec<String> = picked.iter().map(|(number, _)| number.to_string()).collect();
    let mut why = vec![format!(
        "no season pack, so {} of {} episodes one by one",
        picked.len(),
        target.episodes.len()
    )];
    why.extend(first.why.iter().cloned());
    Some(Choice {
        label: format!("Episodes {} as separate torrents", numbers.join(", ")),
        facts: format!(
            "{} episodes · {}",
            picked.len(),
            facts(&first.release, size, seeders)
        ),
        why,
        score: picked.iter().map(|(_, rated)| rated.score).sum::<f64>() / picked.len() as f64,
        size,
        seeders,
        parts: picked.iter().map(|(number, rated)| part(rated, vec![*number])).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::super::resolve::EpisodeInfo;
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    fn outlander_s08() -> Target {
        Target {
            kind: Kind::Tv,
            tmdb: 56570,
            imdb: Some("tt3006802".to_string()),
            title: "Outlander".to_string(),
            original: None,
            year: Some(2014),
            season: Some(8),
            episode: None,
            runtime: None,
            episodes: (1..=10)
                .map(|number| EpisodeInfo {
                    number,
                    name: Some(format!("Episode {number}")),
                    runtime: Some(65),
                })
                .collect(),
            folder: None,
        }
    }

    fn offer(name: &str, size_mib: u64, seeders: u32, hash: &str) -> Offer {
        Offer {
            name: name.to_string(),
            size: size_mib * 1024 * 1024,
            seeders,
            peers: seeders,
            infohash: hash.to_string(),
            magnet: format!("magnet:?xt=urn:btih:{hash}"),
            indexer: "test".to_string(),
        }
    }

    fn real_offers() -> Vec<Offer> {
        vec![
            offer("Outlander Blood of My Blood S02E08 1080p WEB h264 ETHEL", 3409, 555, "a1"),
            offer("Outlander Blood of My Blood S02E08 1080p HEVC x265 MeGusta", 556, 383, "a2"),
            offer("Outlander (2014) S08 (1080p AMZN WEB-DL x265 10bit EAC3 5.1 Silence)", 19896, 253, "a3"),
            offer("Outlander (2014) S08 (1080p AMZN WEB-DL x265 10bit EAC3 5.1 Silence)", 19896, 229, "A3"),
            offer("Outlander.S08E10.1080p.WEB.h264-GRACE", 4812, 142, "a4"),
            offer("Outlander S08E09 1080p WEB h264-GRACE", 3287, 132, "a5"),
            offer("Outlander S08E03 720p 10bit WEBRip 2CH x265 HEVC-PSA", 327, 68, "a6"),
            offer("Outlander.S08E09.1080p.x265-ELiTE", 777, 53, "a7"),
            offer("Outlander S05 1080p BluRay AV1 PTNX", 8366, 19, "a8"),
            offer("Outlander S08e01-10 [1080p Ita Eng Spa h265 10bit SubS] byMe7alh [MIRCrew]", 9054, 17, "a9"),
            offer("Outlander.S08E08.REPACK.MULTi.2160p.SDR.NF.WEB.DDP5.1.H265-SiC", 5242, 18, "a10"),
            offer("Outlander S08 720p HEVC x265-MeGusta", 4300, 2, "a11"),
        ]
    }

    #[test]
    fn outlander_season_8_picks_the_1080p_hevc_season_pack() {
        let ranking = rank(&real_offers(), &outlander_s08(), 30 * GIB);
        let names: Vec<&str> = ranking.choices.iter().map(|choice| choice.label.as_str()).collect();
        assert_eq!(
            names,
            [
                "Outlander (2014) S08 (1080p AMZN WEB-DL x265 10bit EAC3 5.1 Silence)",
                "Outlander S08e01-10 [1080p Ita Eng Spa h265 10bit SubS] byMe7alh [MIRCrew]",
            ]
        );
        let best = &ranking.choices[0];
        assert_eq!(best.seeders, 253);
        assert_eq!(best.facts, "1080p · HEVC 10-bit · 5.1 · 20.9 GB · 253 seeders");
        let reason = |name: &str| {
            ranking
                .dropped
                .iter()
                .find(|dropped| dropped.name.starts_with(name))
                .map(|dropped| dropped.reason.clone())
        };
        assert_eq!(
            reason("Outlander Blood of My Blood S02E08 1080p WEB"),
            Some("title \"Outlander Blood of My Blood\" is not Outlander".to_string())
        );
        assert_eq!(reason("Outlander S05"), Some("season 5".to_string()));
        assert_eq!(reason("Outlander S08 720p"), Some("2 seeders".to_string()));
    }

    #[test]
    fn a_pack_bigger_than_the_room_left_is_dropped() {
        let ranking = rank(&real_offers(), &outlander_s08(), 15 * GIB);
        assert_eq!(ranking.choices.len(), 1);
        assert!(ranking.choices[0].label.contains("MIRCrew"));
    }

    #[test]
    fn without_a_pack_the_best_release_of_each_episode_is_taken() {
        let offers: Vec<Offer> = real_offers()
            .into_iter()
            .filter(|offer| offer.name.contains("E0") || offer.name.contains("E1"))
            .filter(|offer| !offer.name.contains("e01-10"))
            .collect();
        let ranking = rank(&offers, &outlander_s08(), 30 * GIB);
        let [set] = ranking.choices.as_slice() else {
            panic!("{:?}", ranking.choices);
        };
        let episodes: Vec<(u32, &str)> = set
            .parts
            .iter()
            .map(|part| (part.episodes[0], part.name.as_str()))
            .collect();
        assert_eq!(
            episodes,
            [
                (3, "Outlander S08E03 720p 10bit WEBRip 2CH x265 HEVC-PSA"),
                (8, "Outlander.S08E08.REPACK.MULTi.2160p.SDR.NF.WEB.DDP5.1.H265-SiC"),
                (9, "Outlander.S08E09.1080p.x265-ELiTE"),
                (10, "Outlander.S08E10.1080p.WEB.h264-GRACE"),
            ]
        );
        assert!(set.why[0].contains("4 of 10 episodes"));
    }

    #[test]
    fn films_are_checked_against_the_year() {
        let target = Target {
            kind: Kind::Movie,
            tmdb: 693134,
            imdb: Some("tt15239678".to_string()),
            title: "Dune: Part Two".to_string(),
            original: None,
            year: Some(2024),
            season: None,
            episode: None,
            runtime: Some(167),
            episodes: Vec::new(),
            folder: None,
        };
        let offers = [
            offer("Dune.Part.Two.2024.1080p.WEB-DL.DDP5.1.Atmos.H.265-FLUX", 6200, 412, "b1"),
            offer("Dune.Part.Two.2024.1080p.WEBRip.x264.AAC5.1-YTS", 2800, 900, "b2"),
            offer("Dune Part Two 2024 HDCAM x264", 1500, 5000, "b3"),
            offer("Dune.Part.Two.1999.1080p.x265", 3000, 50, "b4"),
            offer("Dune.Part.Two.2024.1080p.x265.Sample", 30, 50, "b5"),
        ];
        let ranking = rank(&offers, &target, 30 * GIB);
        assert_eq!(
            ranking.choices[0].label,
            "Dune.Part.Two.2024.1080p.WEB-DL.DDP5.1.Atmos.H.265-FLUX"
        );
        assert_eq!(ranking.choices.len(), 2);
        assert_eq!(ranking.dropped.len(), 3);
    }
}
