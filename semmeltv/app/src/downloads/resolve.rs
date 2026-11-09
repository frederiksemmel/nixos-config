use super::release::comparable;
use crate::manifest::{Date, Manifest};
use crate::tmdb::{Client, Found, Kind, Lookup};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

const QUESTION_OPTIONS: usize = 4;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wish {
    pub title: String,
    pub year: Option<u16>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpisodeInfo {
    pub number: u32,
    pub name: Option<String>,
    pub runtime: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    #[serde(with = "kind_name")]
    pub kind: Kind,
    pub tmdb: u64,
    pub imdb: Option<String>,
    pub title: String,
    pub original: Option<String>,
    pub year: Option<u16>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub runtime: Option<u32>,
    pub episodes: Vec<EpisodeInfo>,
    pub folder: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    #[serde(with = "kind_name")]
    pub kind: Kind,
    pub tmdb: u64,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    Target(Target),
    Question(String, Vec<Candidate>),
    Refused(String),
}

pub mod kind_name {
    use crate::tmdb::Kind;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(kind: &Kind, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match kind {
            Kind::Movie => "movie",
            Kind::Tv => "show",
        })
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Kind, D::Error> {
        match String::deserialize(deserializer)?.as_str() {
            "movie" => Ok(Kind::Movie),
            "show" => Ok(Kind::Tv),
            other => Err(serde::de::Error::custom(format!("no kind {other}"))),
        }
    }
}

impl Target {
    pub fn titles(&self) -> Vec<String> {
        let mut titles = vec![comparable(&self.title)];
        if let Some(original) = &self.original {
            titles.push(comparable(original));
        }
        titles.dedup();
        titles
    }

    pub fn minutes(&self) -> Option<u32> {
        match (self.kind, self.episode) {
            (Kind::Movie, _) => self.runtime,
            (Kind::Tv, Some(number)) => self.episode_minutes(number),
            (Kind::Tv, None) => {
                let known: Vec<u32> = self.episodes.iter().filter_map(|episode| episode.runtime).collect();
                if known.is_empty() {
                    return None;
                }
                let average = known.iter().sum::<u32>() / known.len() as u32;
                Some(
                    self.episodes
                        .iter()
                        .map(|episode| episode.runtime.unwrap_or(average))
                        .sum(),
                )
            }
        }
    }

    pub fn episode_minutes(&self, number: u32) -> Option<u32> {
        self.episodes
            .iter()
            .find(|episode| episode.number == number)
            .and_then(|episode| episode.runtime)
    }

    pub fn label(&self) -> String {
        match (self.kind, self.season, self.episode) {
            (Kind::Movie, _, _) => match self.year {
                Some(year) => format!("{} ({year})", self.title),
                None => self.title.clone(),
            },
            (Kind::Tv, Some(season), Some(episode)) => format!("{} S{season:02}E{episode:02}", self.title),
            (Kind::Tv, Some(season), None) => format!("{} season {season}", self.title),
            (Kind::Tv, None, _) => self.title.clone(),
        }
    }

    pub fn search_title(&self) -> String {
        comparable(&self.title)
    }
}

fn year(date: &str) -> Option<u16> {
    date.get(..4)?.parse().ok()
}

fn without_article(title: &str) -> String {
    let title = comparable(title);
    title.strip_prefix("the ").map(str::to_string).unwrap_or(title)
}

fn year_of(found: &Found) -> Option<u16> {
    year(&found.date)
}

fn candidate(kind: Kind, found: &Found) -> Candidate {
    let what = match kind {
        Kind::Movie => "film",
        Kind::Tv => "series",
    };
    let label = match year(&found.date) {
        Some(year) => format!("{} ({year}, {what})", found.name),
        None => format!("{} ({what})", found.name),
    };
    Candidate {
        kind,
        tmdb: found.id,
        label,
    }
}

fn split_year(title: &str) -> Option<(String, u16)> {
    let (rest, last) = title.trim().rsplit_once(' ')?;
    let year: u16 = last.parse().ok().filter(|year| (1900..2100).contains(year))?;
    Some((rest.to_string(), year))
}

fn lookup(client: &Client, kind: Kind, title: &str, year: Option<u16>) -> Result<Lookup, String> {
    let search = |title: &str, year| match kind {
        Kind::Movie => client.search_movie(title, year),
        Kind::Tv => client.search_tv(title, year),
    };
    let first = search(title, year).map_err(|error| format!("TMDB: {error}"))?;
    if year.is_none()
        && !matches!(first, Lookup::Match(_) | Lookup::Likely(_))
        && let Some((rest, year)) = split_year(title)
    {
        let second = search(&rest, Some(year)).map_err(|error| format!("TMDB: {error}"))?;
        if matches!(second, Lookup::Match(_) | Lookup::Likely(_)) {
            return Ok(second);
        }
    }
    if !matches!(first, Lookup::Match(_) | Lookup::Likely(_)) && !comparable(title).starts_with("the ") {
        let with_article = search(&format!("the {title}"), year).map_err(|error| format!("TMDB: {error}"))?;
        if matches!(with_article, Lookup::Match(_) | Lookup::Likely(_)) {
            return Ok(with_article);
        }
    }
    Ok(first)
}

fn in_library(manifest: &Manifest, kind: Kind, id: u64) -> bool {
    match kind {
        Kind::Movie => manifest.movie.iter().any(|movie| movie.tmdb == Some(id)),
        Kind::Tv => manifest.show.iter().any(|show| show.tmdb == Some(id)),
    }
}

pub fn resolve(client: &Client, wish: &Wish, manifest: &Manifest, today: Date) -> Result<Resolved, String> {
    let kinds: &[Kind] = if wish.season.is_some() || wish.episode.is_some() {
        &[Kind::Tv]
    } else {
        &[Kind::Movie, Kind::Tv]
    };
    let mut sure = Vec::new();
    let mut unsure = Vec::new();
    for &kind in kinds {
        match lookup(client, kind, &wish.title, wish.year)? {
            Lookup::Match(found) | Lookup::Likely(found) => sure.push((kind, found)),
            Lookup::Unsure(found) => unsure.extend(found.into_iter().map(|found| (kind, found))),
            Lookup::Nothing => {}
        }
    }
    if sure.is_empty() {
        let wanted = without_article(&wish.title);
        let named: Vec<_> = unsure
            .iter()
            .filter(|(_, found)| {
                without_article(&found.name) == wanted
                    && wish.year.is_none_or(|year| year_of(found).is_some_and(|found| found.abs_diff(year) <= 1))
            })
            .cloned()
            .collect();
        if let [only] = named.as_slice() {
            sure.push(only.clone());
        }
    }
    if sure.len() > 1 {
        let known: Vec<_> = sure
            .iter()
            .filter(|(kind, found)| in_library(manifest, *kind, found.id))
            .cloned()
            .collect();
        if known.len() == 1 {
            sure = known;
        }
    }
    let (kind, found) = match sure.as_slice() {
        [only] => only.clone(),
        [] if unsure.is_empty() => {
            return Ok(Resolved::Refused(format!("Nothing called \"{}\" on TMDB", wish.title)));
        }
        _ => {
            let options: Vec<Candidate> = sure
                .iter()
                .chain(unsure.iter())
                .take(QUESTION_OPTIONS)
                .map(|(kind, found)| candidate(*kind, found))
                .collect();
            let labels: Vec<&str> = options.iter().map(|option| option.label.as_str()).collect();
            return Ok(Resolved::Question(format!("Which one: {}?", labels.join(" or ")), options));
        }
    };
    target(client, kind, &found, wish.season, wish.episode, manifest, today)
}

pub fn resolve_id(
    client: &Client,
    kind: Kind,
    id: u64,
    season: Option<u32>,
    episode: Option<u32>,
    manifest: &Manifest,
    today: Date,
) -> Result<Resolved, String> {
    let found = client.details(kind, id).map_err(|error| format!("TMDB: {error}"))?;
    target(client, kind, &found, season, episode, manifest, today)
}

fn show_folder(manifest: &Manifest, id: u64) -> Option<String> {
    let show = manifest.show.iter().find(|show| show.tmdb == Some(id))?;
    show.episodes.iter().find_map(|episode| {
        let mut parts = Path::new(&episode.path).components();
        match (parts.next(), parts.next(), parts.next()) {
            (Some(Component::Normal(_)), Some(Component::Normal(folder)), Some(_)) => {
                Some(folder.to_string_lossy().into_owned())
            }
            _ => None,
        }
    })
}

fn aired(date: Option<&str>, today: Date) -> bool {
    date.and_then(Date::parse).is_some_and(|date| date <= today)
}

fn target(
    client: &Client,
    kind: Kind,
    found: &Found,
    season: Option<u32>,
    episode: Option<u32>,
    manifest: &Manifest,
    today: Date,
) -> Result<Resolved, String> {
    let imdb = client.imdb(kind, found.id).map_err(|error| format!("TMDB: {error}"))?;
    let mut target = Target {
        kind,
        tmdb: found.id,
        imdb,
        title: found.name.clone(),
        original: found.original.clone().filter(|original| *original != found.name),
        year: year(&found.date),
        season,
        episode,
        runtime: None,
        episodes: Vec::new(),
        folder: None,
    };
    match kind {
        Kind::Movie => {
            let details = client.details(Kind::Movie, found.id).map_err(|error| format!("TMDB: {error}"))?;
            target.runtime = details.runtime;
            if in_library(manifest, Kind::Movie, found.id) {
                return Ok(Resolved::Refused(format!("{} is already in the library", target.label())));
            }
        }
        Kind::Tv => {
            let Some(number) = season else {
                return Ok(Resolved::Refused(format!("Which season of {}?", target.title)));
            };
            let season = match client.season(found.id, number) {
                Ok(season) => season,
                Err(ureq::Error::StatusCode(404)) => {
                    return Ok(Resolved::Refused(format!("{} has no season {number}", target.title)));
                }
                Err(error) => return Err(format!("TMDB: {error}")),
            };
            target.episodes = season
                .episodes()
                .iter()
                .filter(|episode| aired(episode.air_date.as_deref(), today))
                .map(|episode| EpisodeInfo {
                    number: episode.episode_number,
                    name: episode.name.clone(),
                    runtime: episode.runtime,
                })
                .collect();
            if target.episodes.is_empty() {
                return Ok(Resolved::Refused(format!("{} has not aired yet", target.label())));
            }
            if let Some(wanted) = episode
                && !target.episodes.iter().any(|episode| episode.number == wanted)
            {
                return Ok(Resolved::Refused(format!("{} has not aired or does not exist", target.label())));
            }
            target.folder = show_folder(manifest, found.id);
            if let Some(missing) = missing(manifest, &target)
                && missing.is_empty()
            {
                return Ok(Resolved::Refused(format!("{} is already in the library", target.label())));
            }
        }
    }
    Ok(Resolved::Target(target))
}

fn missing(manifest: &Manifest, target: &Target) -> Option<Vec<u32>> {
    let show = manifest.show.iter().find(|show| show.tmdb == Some(target.tmdb))?;
    let season = target.season?;
    let have = |number: u32| {
        show.episodes
            .iter()
            .any(|episode| episode.season == season && episode.episode == number)
    };
    Some(
        target
            .episodes
            .iter()
            .map(|episode| episode.number)
            .filter(|number| target.episode.is_none_or(|wanted| wanted == *number))
            .filter(|number| !have(*number))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dropped_article_still_names_the_title() {
        assert_eq!(without_article("The Bear"), without_article("bear"));
        assert_ne!(without_article("Little Bear"), without_article("bear"));
    }

    #[test]
    fn a_trailing_year_is_split_off() {
        assert_eq!(split_year("dune 2021"), Some(("dune".to_string(), 2021)));
        assert_eq!(split_year("1917"), None);
    }

    #[test]
    fn the_show_folder_comes_from_its_episodes() {
        let manifest = Manifest::parse(
            r#"
[[show]]
title = "Foundation"
year = 2021
tmdb = 93740
[[show.episodes]]
season = 1
episode = 1
path = "Series/Foundation/Foundation S01/Foundation (2021) - S01E01.mkv"
added = 2025-01-01
"#,
        )
        .unwrap();
        assert_eq!(show_folder(&manifest, 93740), Some("Foundation".to_string()));
        assert_eq!(show_folder(&manifest, 1), None);
        let target = Target {
            kind: Kind::Tv,
            tmdb: 93740,
            imdb: None,
            title: "Foundation".to_string(),
            original: None,
            year: Some(2021),
            season: Some(1),
            episode: None,
            runtime: None,
            episodes: (1..=3)
                .map(|number| EpisodeInfo {
                    number,
                    name: None,
                    runtime: Some(60),
                })
                .collect(),
            folder: None,
        };
        assert_eq!(missing(&manifest, &target), Some(vec![2, 3]));
        assert_eq!(target.minutes(), Some(180));
    }
}
