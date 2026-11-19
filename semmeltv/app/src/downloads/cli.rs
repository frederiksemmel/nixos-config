use super::release::{self, Release};
use super::resolve::Mode;
use super::torznab::{Offer, Search, Torznab};
use super::{FetchBody, Pick, score};
use crate::tmdb::Kind;
use serde::Serialize;
use std::env;
use std::fs;
use std::time::Duration;

pub const USAGE: &str = "semmeltv get search TITLE [--year YEAR] [--imdb ID] [--season N [--episode N]]
semmeltv get fetch movie|show TMDB_ID [--season N] [--episode N] [--replace] [--by NAME] OFFER_JSON
semmeltv get jobs";

const SERVICE: &str = "http://127.0.0.1:8092";
const TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, PartialEq, Serialize)]
pub struct Found {
    pub title: String,
    pub year: Option<u16>,
    pub resolution: Option<String>,
    pub source: Option<&'static str>,
    pub video: Option<String>,
    pub hdr: Option<String>,
    pub audio: Option<String>,
    pub group: Option<String>,
    pub warnings: Vec<&'static str>,
    #[serde(flatten)]
    pub offer: Offer,
}

pub fn describe(offer: Offer) -> Found {
    let parsed = Release::parse(&offer.name);
    let mut warnings = Vec::new();
    if parsed.bad_source {
        warnings.push("cam or telesync");
    }
    if parsed.sample {
        warnings.push("sample or trailer");
    }
    if parsed.dubbed {
        warnings.push("tagged with another language");
    }
    Found {
        title: parsed.title,
        year: parsed.year,
        resolution: parsed.tags.resolution,
        source: parsed.source.map(|source| source.label()),
        video: parsed.tags.video,
        hdr: release::hdr(&offer.name),
        audio: parsed.tags.audio,
        group: release::group(&offer.name),
        warnings,
        offer,
    }
}

fn number<T: std::str::FromStr>(name: &str, value: Option<String>) -> Result<T, String> {
    let value = value.ok_or(format!("{name} needs a value"))?;
    value.parse().map_err(|_| format!("{name}: not a number: {value}"))
}

pub fn search_of(args: &[String]) -> Result<Search, String> {
    let mut words = Vec::new();
    let (mut year, mut imdb, mut season, mut episode) = (None, None, None, None);
    let mut args = args.iter().cloned();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--year" => year = Some(number("--year", args.next())?),
            "--imdb" => imdb = Some(args.next().ok_or("--imdb needs a value")?),
            "--season" => season = Some(number("--season", args.next())?),
            "--episode" => episode = Some(number("--episode", args.next())?),
            flag if flag.starts_with("--") => return Err(format!("unknown argument {flag}\n{USAGE}")),
            _ => words.push(argument),
        }
    }
    let title = words.join(" ");
    if title.is_empty() {
        return Err(format!("search needs a title\n{USAGE}"));
    }
    match (season, episode) {
        (Some(season), episode) => Ok(Search::Tv { title, season, episode }),
        (None, Some(_)) => Err("--episode needs --season".to_string()),
        (None, None) => Ok(Search::Movie { title, year, imdb }),
    }
}

pub fn fetch_of(args: &[String]) -> Result<FetchBody, String> {
    let mut args = args.iter().cloned();
    let kind = match args.next().as_deref() {
        Some("movie") => Kind::Movie,
        Some("show") => Kind::Tv,
        _ => return Err(format!("fetch needs movie or show\n{USAGE}")),
    };
    let tmdb = number("TMDB_ID", args.next())?;
    let (mut season, mut episode, mut mode, mut offer, mut by) = (None, None, Mode::Add, None, None);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--season" => season = Some(number("--season", args.next())?),
            "--episode" => episode = Some(number("--episode", args.next())?),
            "--replace" => mode = Mode::Replace,
            "--by" => by = Some(args.next().ok_or("--by needs a name")?),
            flag if flag.starts_with("--") => return Err(format!("unknown argument {flag}\n{USAGE}")),
            json => {
                let parsed: Offer =
                    serde_json::from_str(json).map_err(|error| format!("OFFER_JSON is not an offer: {error}"))?;
                offer = Some(parsed);
            }
        }
    }
    let offer = offer.ok_or(format!("fetch needs OFFER_JSON, a line from search\n{USAGE}"))?;
    Ok(FetchBody {
        pick: Pick {
            kind,
            tmdb,
            season,
            episode,
        },
        offer,
        mode,
        by,
    })
}

fn torznab() -> Result<Torznab, String> {
    let endpoints: Vec<String> = env::var("SEMMELTV_TORZNAB")
        .map_err(|_| "search needs SEMMELTV_TORZNAB".to_string())?
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let key = match env::var_os("SEMMELTV_TORZNAB_KEY_FILE") {
        Some(file) => Some(
            fs::read_to_string(&file)
                .map_err(|error| format!("{}: {error}", file.to_string_lossy()))?
                .trim()
                .to_string(),
        ),
        None => None,
    };
    Ok(Torznab::new(endpoints, key))
}

fn service() -> String {
    env::var("SEMMELTV_DOWNLOADS")
        .ok()
        .filter(|address| !address.is_empty())
        .unwrap_or_else(|| SERVICE.to_string())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build()
        .new_agent()
}

fn answer(mut response: ureq::http::Response<ureq::Body>) -> Result<serde_json::Value, String> {
    let status = response.status();
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|error| error.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {status}: {body}"));
    }
    serde_json::from_str(&body).map_err(|error| format!("{error}: {body}"))
}

fn search(args: &[String]) -> Result<(), String> {
    let (offers, problems) = torznab()?.search(&search_of(args)?);
    for problem in problems {
        eprintln!("problem: {problem}");
    }
    let mut found: Vec<Found> = score::merge(&offers).into_iter().map(describe).collect();
    found.sort_by(|a, b| {
        b.offer
            .seeders
            .cmp(&a.offer.seeders)
            .then(b.offer.size.cmp(&a.offer.size))
    });
    for one in found {
        println!("{}", serde_json::to_string(&one).map_err(|error| error.to_string())?);
    }
    Ok(())
}

fn fetch(args: &[String]) -> Result<(), String> {
    let body = fetch_of(args)?;
    let response = agent()
        .post(&format!("{}/api/downloads/fetch", service()))
        .send_json(&body)
        .map_err(|error| error.to_string())?;
    let outcome = answer(response)?;
    println!("{outcome}");
    if outcome["ok"].as_bool() == Some(true) {
        Ok(())
    } else {
        Err(outcome["message"].as_str().unwrap_or("refused").to_string())
    }
}

fn jobs() -> Result<(), String> {
    let response = agent()
        .get(&format!("{}/api/downloads", service()))
        .call()
        .map_err(|error| error.to_string())?;
    let overview = answer(response)?;
    for job in overview["jobs"].as_array().into_iter().flatten() {
        let line = serde_json::json!({
            "id": job["id"],
            "label": job["label"],
            "mode": job["mode"],
            "stage": job["stage"],
            "message": job["message"],
            "release": job["choice"]["label"],
            "placed": job["placed"],
            "log": job["log"].as_array().map(|log| log.iter().rev().take(5).rev().cloned().collect::<Vec<_>>()),
        });
        println!("{line}");
    }
    println!(
        "{}",
        serde_json::json!({ "room": overview["room"], "quota": overview["quota"], "free": overview["free"] })
    );
    Ok(())
}

pub fn run(args: &[String]) -> Result<(), String> {
    match args.split_first() {
        Some((command, rest)) if command == "search" => search(rest),
        Some((command, rest)) if command == "fetch" => fetch(rest),
        Some((command, _)) if command == "jobs" => jobs(),
        _ => Err(USAGE.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn a_search_is_a_film_unless_a_season_is_named() {
        let cases = [
            (
                "View from a Blue Moon --year 2015 --imdb tt5199588",
                Ok(Search::Movie {
                    title: "View from a Blue Moon".to_string(),
                    year: Some(2015),
                    imdb: Some("tt5199588".to_string()),
                }),
            ),
            (
                "Outlander --season 8 --episode 2",
                Ok(Search::Tv {
                    title: "Outlander".to_string(),
                    season: 8,
                    episode: Some(2),
                }),
            ),
            ("Outlander --episode 2", Err("--episode needs --season".to_string())),
            ("--year 2015", Err(format!("search needs a title\n{USAGE}"))),
            ("Up --year soon", Err("--year: not a number: soon".to_string())),
        ];
        for (line, want) in cases {
            assert_eq!(search_of(&words(line)), want, "{line}");
        }
    }

    #[test]
    fn a_search_line_fed_back_to_fetch_names_the_same_release() {
        let offer = Offer {
            name: "View from a Blue Moon 2015 1080p BluRay x264-OFT".to_string(),
            size: 2_746_743_783,
            seeders: 2,
            peers: 4,
            infohash: "3617abc53248dd75a24619e8886a82d571f071a2".to_string(),
            magnet: "magnet:?xt=urn:btih:3617ABC53248DD75A24619E8886A82D571F071A2".to_string(),
            indexer: "Knaben".to_string(),
        };
        let found = describe(offer.clone());
        assert_eq!(
            (
                found.resolution.as_deref(),
                found.source,
                found.video.as_deref(),
                found.group.as_deref()
            ),
            (Some("1080p"), Some("BluRay"), Some("H.264"), Some("OFT"))
        );
        let line = serde_json::to_string(&found).unwrap();
        let mut args = words("movie 368239 --replace");
        args.push(line);
        let body = fetch_of(&args).unwrap();
        assert_eq!(body.offer, offer);
        assert_eq!(
            (body.pick.kind, body.pick.tmdb, body.mode, body.by),
            (Kind::Movie, 368239, Mode::Replace, None)
        );
        let mut asked = words("movie 368239 --by Mami");
        asked.push(serde_json::to_string(&found).unwrap());
        assert_eq!(fetch_of(&asked).unwrap().by.as_deref(), Some("Mami"));
        assert!(fetch_of(&words("movie 368239")).is_err());
        assert!(fetch_of(&words("film 368239 {}")).is_err());
    }
}
