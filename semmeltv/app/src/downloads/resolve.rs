use super::release::comparable;
use crate::manifest::{Date, Manifest};
use crate::tmdb::{Client, Found, Kind, Lookup, SeasonSummary};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

const QUESTION_OPTIONS: usize = 8;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wish {
    pub title: String,
    pub year: Option<u16>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    Add,
    Replace,
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
    #[serde(default)]
    pub season: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    Target(Target),
    Question {
        text: String,
        options: Vec<Candidate>,
        season: Option<u32>,
        episode: Option<u32>,
    },
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
        season: None,
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

pub fn in_library(manifest: &Manifest, kind: Kind, id: u64) -> bool {
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
    let (kind, found) = match decide(wish, sure, &unsure, manifest) {
        Decision::Pick(kind, found) => (kind, *found),
        Decision::Nothing => return Ok(Resolved::Refused(format!("Nothing called \"{}\" on TMDB", wish.title))),
        Decision::Ask(options) => {
            let labels: Vec<&str> = options.iter().map(|option| option.label.as_str()).collect();
            return Ok(Resolved::Question {
                text: format!("Which one: {}?", labels.join(" or ")),
                options,
                season: wish.season,
                episode: wish.episode,
            });
        }
    };
    target(
        client,
        kind,
        &found,
        wish.season,
        wish.episode,
        manifest,
        today,
        Mode::Add,
    )
}

#[derive(Debug, PartialEq)]
enum Decision {
    Pick(Kind, Box<Found>),
    Ask(Vec<Candidate>),
    Nothing,
}

/// What the TMDB lookups for a wish lead to: the one title it surely means,
/// or a question listing the sure ones first. A sure title of one kind still
/// asks when the other kind has titles of exactly that name.
fn decide(wish: &Wish, mut sure: Vec<(Kind, Found)>, unsure: &[(Kind, Found)], manifest: &Manifest) -> Decision {
    let wanted = without_article(&wish.title);
    let named = |found: &Found| {
        without_article(&found.name) == wanted
            && wish
                .year
                .is_none_or(|year| year_of(found).is_some_and(|found| found.abs_diff(year) <= 1))
    };
    if sure.is_empty()
        && let [only] = unsure
            .iter()
            .filter(|(_, found)| named(found))
            .collect::<Vec<_>>()
            .as_slice()
    {
        sure.push((*only).clone());
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
    let asked: Vec<&(Kind, Found)> = match sure.as_slice() {
        [] if unsure.is_empty() => return Decision::Nothing,
        [(kind, found)] => {
            let namesakes: Vec<&(Kind, Found)> = unsure
                .iter()
                .filter(|(other, namesake)| other != kind && named(namesake))
                .collect();
            if namesakes.is_empty() {
                return Decision::Pick(*kind, Box::new(found.clone()));
            }
            sure.iter().chain(namesakes).collect()
        }
        _ => sure.iter().chain(unsure).collect(),
    };
    Decision::Ask(
        asked
            .into_iter()
            .take(QUESTION_OPTIONS)
            .map(|(kind, found)| candidate(*kind, found))
            .collect(),
    )
}

/// The question for a series asked without a season: one option per season
/// the library lacks, or the season itself when only one is missing.
fn season_choice(target: &Target, missing: &[u32]) -> Result<u32, Box<Resolved>> {
    match missing {
        [] => Err(Box::new(Resolved::Refused(format!(
            "The library has every season of {}",
            target.title
        )))),
        [only] => Ok(*only),
        many => Err(Box::new(Resolved::Question {
            text: format!("Which season of {}?", target.title),
            options: many
                .iter()
                .map(|number| Candidate {
                    kind: Kind::Tv,
                    tmdb: target.tmdb,
                    label: format!("Season {number}"),
                    season: Some(*number),
                })
                .collect(),
            season: None,
            episode: target.episode,
        })),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn resolve_id(
    client: &Client,
    kind: Kind,
    id: u64,
    season: Option<u32>,
    episode: Option<u32>,
    manifest: &Manifest,
    today: Date,
    mode: Mode,
) -> Result<Resolved, String> {
    let found = client.details(kind, id).map_err(|error| format!("TMDB: {error}"))?;
    target(client, kind, &found, season, episode, manifest, today, mode)
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

#[allow(clippy::too_many_arguments)]
fn target(
    client: &Client,
    kind: Kind,
    found: &Found,
    season: Option<u32>,
    episode: Option<u32>,
    manifest: &Manifest,
    today: Date,
    mode: Mode,
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
            let details = client
                .details(Kind::Movie, found.id)
                .map_err(|error| format!("TMDB: {error}"))?;
            target.runtime = details.runtime;
            if let Some(refusal) = refusal(&target, mode, in_library(manifest, Kind::Movie, found.id)) {
                return Ok(Resolved::Refused(refusal));
            }
        }
        Kind::Tv => {
            let number = match (season, mode) {
                (Some(number), _) => number,
                (None, Mode::Replace) => {
                    return Ok(Resolved::Refused(format!("Which season of {}?", target.title)));
                }
                (None, Mode::Add) => {
                    let seasons = client.seasons(found.id).map_err(|error| format!("TMDB: {error}"))?;
                    match season_choice(&target, &missing_seasons(&seasons, manifest, found.id, today)) {
                        Ok(number) => number,
                        Err(asked) => return Ok(*asked),
                    }
                }
            };
            target.season = Some(number);
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
                return Ok(Resolved::Refused(format!(
                    "{} has not aired or does not exist",
                    target.label()
                )));
            }
            target.folder = show_folder(manifest, found.id);
            let wanted = target
                .episodes
                .iter()
                .filter(|known| episode.is_none_or(|wanted| wanted == known.number))
                .count();
            let have = missing(manifest, &target).map_or(Have::None, |missing| match missing.len() {
                0 => Have::All,
                gone if gone == wanted => Have::None,
                _ => Have::Some,
            });
            if let Some(refusal) = refusal(&target, mode, have) {
                return Ok(Resolved::Refused(refusal));
            }
        }
    }
    Ok(Resolved::Target(target))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Have {
    None,
    Some,
    All,
}

impl From<bool> for Have {
    fn from(have: bool) -> Self {
        if have { Self::All } else { Self::None }
    }
}

fn refusal(target: &Target, mode: Mode, have: impl Into<Have>) -> Option<String> {
    match (mode, have.into()) {
        (Mode::Add, Have::All) => Some(format!("{} is already in the library", target.label())),
        (Mode::Replace, Have::None) => Some(format!(
            "{} is not in the library, so there is nothing to replace",
            target.label()
        )),
        _ => None,
    }
}

pub fn missing_seasons(seasons: &[SeasonSummary], manifest: &Manifest, show: u64, today: Date) -> Vec<u32> {
    let have = |number: u32| {
        manifest
            .show
            .iter()
            .filter(|known| known.tmdb == Some(show))
            .flat_map(|known| &known.episodes)
            .any(|episode| episode.season == number)
    };
    seasons
        .iter()
        .filter(|season| season.season_number > 0 && season.episode_count > 0)
        .filter(|season| aired(season.air_date.as_deref(), today))
        .map(|season| season.season_number)
        .filter(|number| !have(*number))
        .collect()
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

    fn found(id: u64, name: &str, date: &str) -> Found {
        Found {
            id,
            name: name.to_string(),
            date: date.to_string(),
            ..Found::default()
        }
    }

    fn wish(title: &str) -> Wish {
        Wish {
            title: title.to_string(),
            ..Wish::default()
        }
    }

    fn labels(decision: &Decision) -> Vec<&str> {
        match decision {
            Decision::Ask(options) => options.iter().map(|option| option.label.as_str()).collect(),
            other => panic!("no question: {other:?}"),
        }
    }

    #[test]
    fn a_sure_film_still_asks_when_series_have_exactly_its_name() {
        let film = (Kind::Movie, found(1, "Would I Lie to You?", "1997-03-01"));
        let unsure = [
            (Kind::Tv, found(2, "Would I Lie to You?", "2007-06-16")),
            (Kind::Tv, found(3, "Would I Lie to You?", "2022-01-01")),
            (Kind::Tv, found(4, "Would I Lie to You? at Christmas", "2010-12-01")),
            (Kind::Movie, found(5, "Would I Lie to You? 2", "2001-01-01")),
        ];
        let decision = decide(
            &wish("would i lie to you"),
            vec![film.clone()],
            &unsure,
            &Manifest::default(),
        );
        assert_eq!(
            labels(&decision),
            vec![
                "Would I Lie to You? (1997, film)",
                "Would I Lie to You? (2007, series)",
                "Would I Lie to You? (2022, series)"
            ]
        );
        let alone = decide(
            &wish("would i lie to you"),
            vec![film.clone()],
            &unsure[3..],
            &Manifest::default(),
        );
        assert_eq!(alone, Decision::Pick(film.0, Box::new(film.1)));
    }

    #[test]
    fn an_unsure_search_lists_up_to_eight_titles_sure_ones_first() {
        let unsure: Vec<_> = (1..=10)
            .map(|id| (Kind::Movie, found(id, "Taskmaster", &format!("{}-01-01", 1990 + id))))
            .collect();
        let sure = vec![(Kind::Tv, found(99, "Taskmaster", "2015-07-28"))];
        let decision = decide(&wish("taskmaster"), sure, &unsure, &Manifest::default());
        let labels = labels(&decision);
        assert_eq!(labels.len(), QUESTION_OPTIONS);
        assert_eq!(labels[0], "Taskmaster (2015, series)");
        assert_eq!(
            decide(&wish("x"), Vec::new(), &[], &Manifest::default()),
            Decision::Nothing
        );
    }

    #[test]
    fn a_series_without_a_season_asks_for_one_of_the_missing_seasons() {
        let target = Target {
            kind: Kind::Tv,
            tmdb: 7,
            imdb: None,
            title: "Taskmaster".to_string(),
            original: None,
            year: Some(2015),
            season: None,
            episode: None,
            runtime: None,
            episodes: Vec::new(),
            folder: None,
        };
        assert_eq!(season_choice(&target, &[4]), Ok(4));
        assert_eq!(
            season_choice(&target, &[]),
            Err(Box::new(Resolved::Refused(
                "The library has every season of Taskmaster".to_string()
            )))
        );
        let Err(asked) = season_choice(&target, &[1, 2, 3]) else {
            panic!("no question");
        };
        let Resolved::Question { text, options, .. } = *asked else {
            panic!("no question");
        };
        assert_eq!(text, "Which season of Taskmaster?");
        assert_eq!(
            options
                .iter()
                .map(|option| (option.label.as_str(), option.tmdb, option.season))
                .collect::<Vec<_>>(),
            vec![
                ("Season 1", 7, Some(1)),
                ("Season 2", 7, Some(2)),
                ("Season 3", 7, Some(3))
            ]
        );
    }

    #[test]
    fn a_dropped_article_still_names_the_title() {
        assert_eq!(without_article("The Bear"), without_article("bear"));
        assert_ne!(without_article("Little Bear"), without_article("bear"));
    }

    #[test]
    fn adding_needs_something_missing_and_replacing_needs_something_there() {
        let target = Target {
            kind: Kind::Movie,
            tmdb: 368239,
            imdb: None,
            title: "View from a Blue Moon".to_string(),
            original: None,
            year: Some(2015),
            season: None,
            episode: None,
            runtime: Some(58),
            episodes: Vec::new(),
            folder: None,
        };
        let cases = [
            (Mode::Add, Have::None, None),
            (Mode::Add, Have::Some, None),
            (
                Mode::Add,
                Have::All,
                Some("View from a Blue Moon (2015) is already in the library"),
            ),
            (
                Mode::Replace,
                Have::None,
                Some("View from a Blue Moon (2015) is not in the library, so there is nothing to replace"),
            ),
            (Mode::Replace, Have::Some, None),
            (Mode::Replace, Have::All, None),
        ];
        for (mode, have, want) in cases {
            assert_eq!(refusal(&target, mode, have).as_deref(), want, "{mode:?} {have:?}");
        }
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

    #[test]
    fn a_season_is_missing_when_it_aired_and_the_library_has_none_of_its_episodes() {
        let manifest = Manifest::parse(
            r#"
[[show]]
title = "Foundation"
year = 2021
tmdb = 93740
[[show.episodes]]
season = 1
episode = 4
path = "Series/Foundation/Season 01/Foundation (2021) - S01E04.mkv"
added = 2025-01-01
"#,
        )
        .unwrap();
        let season = |number: u32, air_date: Option<&str>, episodes: u32| SeasonSummary {
            season_number: number,
            air_date: air_date.map(str::to_string),
            episode_count: episodes,
        };
        let seasons = [
            season(0, Some("2021-09-01"), 5),
            season(1, Some("2021-09-24"), 10),
            season(2, Some("2023-07-14"), 10),
            season(3, Some("2025-07-11"), 10),
            season(4, Some("2027-01-01"), 10),
            season(5, None, 0),
        ];
        let today = Date::parse("2026-11-15").unwrap();
        assert_eq!(missing_seasons(&seasons, &manifest, 93740, today), [2, 3]);
        assert_eq!(missing_seasons(&seasons, &manifest, 1, today), [1, 2, 3]);
    }
}
