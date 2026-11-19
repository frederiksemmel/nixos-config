use crate::library::{normalize, sort_title};
use crate::manifest::{About, Date, EpisodeEntry, Manifest, MovieEntry, ReadError, Scores, ShowEntry};
use crate::parse::{self, FileEntry, Parsed, ParsedEpisode, ParsedMovie, ParsedShow};
use crate::tags;
use crate::tmdb::{Client, Found, Kind, Lookup};
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct Query<'a> {
    pub kind: Kind,
    pub title: &'a str,
    pub file_title: &'a str,
    pub year: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub id: u64,
    pub name: String,
    pub date: String,
    pub inferred: bool,
}

impl Identity {
    fn year(&self) -> Option<u16> {
        self.date.get(..4)?.parse().ok()
    }
}

impl From<Found> for Identity {
    fn from(found: Found) -> Self {
        Self {
            id: found.id,
            name: found.name,
            date: found.date,
            inferred: false,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub added: Vec<String>,
    pub review: Vec<String>,
    pub missing: Vec<String>,
    pub ignored: Vec<String>,
}

pub struct Sources<'a> {
    pub library: &'a Path,
    pub manifest: &'a Path,
    pub listing: Option<&'a Path>,
    pub client: Option<Client>,
}

pub fn run(sources: Sources) -> Result<(), String> {
    let files = match sources.listing {
        Some(listing) => {
            let text = fs::read_to_string(listing).map_err(|error| format!("{}: {error}", listing.display()))?;
            parse::read_listing(&text)?
        }
        None => parse::scan(sources.library),
    };
    if files.is_empty() {
        return Err("no videos found under Series/ or Movies/".to_string());
    }
    let existing = match Manifest::read(sources.manifest) {
        Ok(manifest) => Some(manifest),
        Err(ReadError::Unreadable(_)) if !sources.manifest.exists() => None,
        Err(ReadError::Unreadable(error)) => return Err(format!("{}: {error}", sources.manifest.display())),
        Err(ReadError::Invalid(error)) => return Err(format!("{}: {error}", sources.manifest.display())),
    };
    let parsed = parse::build(&files);
    let client = sources.client;
    let identify = |query: &Query| identify(client.as_ref(), query);
    let (manifest, report) = merge(existing, &parsed, &files, today(), identify);
    manifest
        .write(sources.manifest)
        .map_err(|error| format!("{}: {error}", sources.manifest.display()))?;
    print_report(&report, manifest.review_count());
    Ok(())
}

fn today() -> Date {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH);
    Date::from_unix(elapsed.map_or(0, |elapsed| elapsed.as_secs()))
}

fn print_report(report: &Report, review_total: usize) {
    let sections = [
        ("added", &report.added),
        ("review", &report.review),
        ("file missing, entry kept", &report.missing),
        ("video not imported", &report.ignored),
    ];
    for (label, lines) in sections {
        for line in lines {
            println!("{label}: {line}");
        }
    }
    println!(
        "{} added, {} marked for review now, {review_total} marked in total",
        report.added.len(),
        report.review.len()
    );
}

pub fn identify(client: Option<&Client>, query: &Query) -> Option<Identity> {
    let client = client?;
    let lookup = match query.kind {
        Kind::Tv => client.search_tv(query.title, query.year),
        Kind::Movie => match client.search_movie(query.title, query.year) {
            Ok(found @ (Lookup::Match(_) | Lookup::Likely(_))) => Ok(found),
            first if normalize(query.title) == normalize(query.file_title) => first,
            _ => client.search_movie(query.file_title, query.year),
        },
    };
    match lookup {
        Ok(Lookup::Match(found)) => Some(found.into()),
        Ok(Lookup::Likely(found)) => {
            println!(
                "unsure: {}: chose {} {} ({})",
                query.title, found.id, found.name, found.date
            );
            Some(Identity {
                inferred: true,
                ..found.into()
            })
        }
        Ok(Lookup::Unsure(candidates)) => {
            let list: Vec<_> = candidates
                .iter()
                .map(|c| format!("{} {} ({})", c.id, c.name, c.date))
                .collect();
            println!("unsure: {}: {}", query.title, list.join("; "));
            None
        }
        Ok(Lookup::Nothing) => {
            println!("unsure: {}: nothing on TMDB", query.title);
            None
        }
        Err(error) => {
            println!("unsure: {}: lookup failed: {error}", query.title);
            None
        }
    }
}

struct Merger<F> {
    manifest: Manifest,
    known: HashSet<String>,
    first_import: bool,
    today: Date,
    identify: F,
    report: Report,
}

pub fn merge(
    existing: Option<Manifest>,
    parsed: &Parsed,
    files: &[FileEntry],
    today: Date,
    identify: impl FnMut(&Query) -> Option<Identity>,
) -> (Manifest, Report) {
    let first_import = existing.is_none();
    let manifest = existing.unwrap_or_default();
    let known = manifest_paths(&manifest).map(str::to_string).collect();
    let mut merger = Merger {
        manifest,
        known,
        first_import,
        today,
        identify,
        report: Report::default(),
    };
    for show in &parsed.shows {
        merger.merge_show(show);
    }
    for movie in &parsed.movies {
        merger.merge_movie(movie);
    }
    let Merger {
        mut manifest,
        mut report,
        ..
    } = merger;
    fill_tags(&mut manifest);
    sort(&mut manifest);
    let on_disk: BTreeSet<&str> = files.iter().filter_map(|file| file.path.to_str()).collect();
    let listed: BTreeSet<&str> = manifest_paths(&manifest).collect();
    report.missing = listed.difference(&on_disk).map(|path| path.to_string()).collect();
    report.ignored = files
        .iter()
        .filter(|file| !parse::is_extra(&file.path))
        .filter_map(|file| file.path.to_str())
        .filter(|path| !listed.contains(path))
        .map(str::to_string)
        .collect();
    (manifest, report)
}

fn manifest_paths(manifest: &Manifest) -> impl Iterator<Item = &str> {
    let episodes = manifest.show.iter().flat_map(|show| &show.episodes);
    episodes
        .map(|episode| episode.path.as_str())
        .chain(manifest.movie.iter().map(|movie| movie.path.as_str()))
}

fn fill_tags(manifest: &mut Manifest) {
    for episode in manifest.show.iter_mut().flat_map(|show| &mut show.episodes) {
        fill(
            &episode.path,
            &mut episode.video,
            &mut episode.resolution,
            &mut episode.audio,
        );
    }
    for movie in &mut manifest.movie {
        fill(&movie.path, &mut movie.video, &mut movie.resolution, &mut movie.audio);
    }
}

fn fill(path: &str, video: &mut Option<String>, resolution: &mut Option<String>, audio: &mut Option<String>) {
    let tags = tags::from_path(path);
    *video = video.take().or(tags.video);
    *resolution = resolution.take().or(tags.resolution);
    *audio = audio.take().or(tags.audio);
}

fn sort(manifest: &mut Manifest) {
    manifest.show.sort_by_key(|show| sort_title(&show.title));
    for show in &mut manifest.show {
        show.episodes.sort_by_key(|episode| (episode.season, episode.episode));
    }
    manifest
        .movie
        .sort_by_key(|movie| (sort_title(&movie.title), movie.year));
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn better_title(title: &str, identity: Option<&Identity>) -> String {
    match identity {
        Some(identity) if normalize(&identity.name) == normalize(title) => identity.name.clone(),
        _ => title.to_string(),
    }
}

fn needs_review(identity: Option<&Identity>) -> bool {
    identity.is_none_or(|identity| identity.inferred)
}

fn doubt(identity: Option<&Identity>) -> &'static str {
    match identity {
        Some(_) => "best known TMDB match, not certain",
        None => "no TMDB id",
    }
}

impl<F: FnMut(&Query) -> Option<Identity>> Merger<F> {
    fn added_date(&self, modified: Date) -> Date {
        if self.first_import { modified } else { self.today }
    }

    fn mark_review(&mut self, line: String) {
        self.report.review.push(line);
    }

    fn show_index(&self, parsed: &ParsedShow) -> Option<usize> {
        let by_path = self.manifest.show.iter().position(|show| {
            show.episodes
                .iter()
                .any(|episode| parsed.episodes.iter().any(|e| path_text(&e.path) == episode.path))
        });
        by_path.or_else(|| {
            let title = normalize(&parsed.title);
            self.manifest
                .show
                .iter()
                .position(|show| normalize(&show.title) == title)
        })
    }

    fn merge_show(&mut self, parsed: &ParsedShow) {
        let new: Vec<&ParsedEpisode> = parsed
            .episodes
            .iter()
            .filter(|episode| !self.known.contains(&path_text(&episode.path)))
            .collect();
        if new.is_empty() {
            return;
        }
        let index = match self.show_index(parsed) {
            Some(index) => index,
            None => self.add_show(parsed),
        };
        for episode in new {
            self.add_episode(index, episode);
        }
    }

    fn add_show(&mut self, parsed: &ParsedShow) -> usize {
        let identity = (self.identify)(&Query {
            kind: Kind::Tv,
            title: &parsed.title,
            file_title: &parsed.title,
            year: parsed.year,
        });
        let review = needs_review(identity.as_ref());
        if review {
            self.mark_review(format!("{}: {}", parsed.title, doubt(identity.as_ref())));
        }
        self.manifest.show.push(ShowEntry {
            id: None,
            title: better_title(&parsed.title, identity.as_ref()),
            year: parsed.year.or(identity.as_ref().and_then(Identity::year)),
            last_aired: None,
            running: false,
            tmdb: identity.map(|identity| identity.id),
            imdb: None,
            ratings: Scores::default(),
            rated: None,
            review,
            episodes: Vec::new(),
            about: About::default(),
        });
        self.manifest.show.len() - 1
    }

    fn add_episode(&mut self, index: usize, parsed: &ParsedEpisode) {
        let added = self.added_date(parsed.modified);
        let path = path_text(&parsed.path);
        let show = &self.manifest.show[index];
        let taken = |number: u32| {
            show.episodes
                .iter()
                .any(|episode| (episode.season, episode.episode) == (parsed.season, number))
        };
        let number = if parsed.guessed {
            (parsed.number..)
                .find(|number| !taken(*number))
                .unwrap_or(parsed.number)
        } else {
            parsed.number
        };
        let duplicate = taken(number);
        let review = parsed.guessed || duplicate;
        let line = format!("{} S{:02}E{:02}", show.title, parsed.season, number);
        if parsed.guessed {
            self.mark_review(format!("{line}: no season or episode in the name"));
        } else if duplicate {
            self.mark_review(format!("{line}: already in the library, {path}"));
        }
        self.report.added.push(line);
        self.known.insert(path.clone());
        self.manifest.show[index].episodes.push(EpisodeEntry {
            id: None,
            season: parsed.season,
            episode: number,
            title: parsed.title.clone(),
            path,
            added,
            runtime: None,
            ratings: Scores::default(),
            video: None,
            resolution: None,
            audio: None,
            review,
            downloaded_by: None,
        });
    }

    fn merge_movie(&mut self, parsed: &ParsedMovie) {
        let path = path_text(&parsed.path);
        if self.known.contains(&path) {
            return;
        }
        let identity = (self.identify)(&Query {
            kind: Kind::Movie,
            title: &parsed.title,
            file_title: &parsed.file_title,
            year: parsed.year,
        });
        let year = parsed.year.or(identity.as_ref().and_then(Identity::year));
        let title = better_title(&parsed.title, identity.as_ref());
        let duplicate = self
            .manifest
            .movie
            .iter()
            .any(|movie| normalize(&movie.title) == normalize(&title) && movie.year == year);
        let line = match year {
            Some(year) => format!("{title} ({year})"),
            None => title.clone(),
        };
        let unsure = needs_review(identity.as_ref());
        if unsure {
            self.mark_review(format!("{line}: {}", doubt(identity.as_ref())));
        }
        if duplicate {
            self.mark_review(format!("{line}: already in the library, {path}"));
        }
        self.report.added.push(line);
        self.known.insert(path.clone());
        self.manifest.movie.push(MovieEntry {
            id: None,
            title,
            year,
            tmdb: identity.as_ref().map(|identity| identity.id),
            imdb: None,
            ratings: Scores::default(),
            rated: None,
            path,
            added: self.added_date(parsed.modified),
            runtime: None,
            video: None,
            resolution: None,
            audio: None,
            review: unsure || duplicate,
            downloaded_by: None,
            about: About::default(),
        });
    }
}

#[cfg(test)]
pub fn library_from_paths(paths: &[&str]) -> crate::library::Library {
    let files: Vec<_> = paths
        .iter()
        .map(|path| FileEntry {
            path: path.into(),
            size: 1,
            modified: Date::default(),
        })
        .collect();
    let parsed = parse::build(&files);
    let (manifest, _) = merge(None, &parsed, &files, Date::default(), |_| None);
    crate::library::Library::from_manifest("/lib".as_ref(), &manifest, |_| true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY: &str = "2026-11-07";

    fn date(text: &str) -> Date {
        Date::parse(text).unwrap()
    }

    fn files(paths: &[(&str, &str)]) -> Vec<FileEntry> {
        paths
            .iter()
            .map(|(path, modified)| FileEntry {
                path: path.into(),
                size: 1,
                modified: date(modified),
            })
            .collect()
    }

    fn import(existing: Option<Manifest>, files: &[FileEntry]) -> (Manifest, Report) {
        merge(existing, &parse::build(files), files, date(TODAY), known_title)
    }

    fn known_title(query: &Query) -> Option<Identity> {
        match query.title {
            "airplane" => Some(Identity {
                id: 813,
                name: "Airplane!".to_string(),
                date: "1980-06-27".to_string(),
                inferred: false,
            }),
            "Dark" => Some(Identity {
                id: 70523,
                name: "Dark".to_string(),
                date: "2017-12-01".to_string(),
                inferred: false,
            }),
            "Chernobyl" => Some(Identity {
                id: 87108,
                name: "Chernobyl".to_string(),
                date: "2019-05-06".to_string(),
                inferred: true,
            }),
            _ => None,
        }
    }

    #[test]
    fn tmdb_identity_names_and_flags_an_entry() {
        let cases: [(&str, (&str, Option<u16>, Option<u64>, bool), &[&str]); 4] = [
            (
                "Series/Chernobyl/Chernobyl.S01E01.mkv",
                ("Chernobyl", Some(2019), Some(87108), true),
                &["Chernobyl: best known TMDB match, not certain"],
            ),
            (
                "Movies/airplane/airplane.mkv",
                ("Airplane!", Some(1980), Some(813), false),
                &[],
            ),
            (
                "Movies/Wind/WIND.avi",
                ("Wind", None, None, true),
                &["Wind: no TMDB id"],
            ),
            (
                "Series/Nowhere/Nowhere.S01E01.mkv",
                ("Nowhere", None, None, true),
                &["Nowhere: no TMDB id"],
            ),
        ];
        for (path, entry, review) in cases {
            let (manifest, report) = import(None, &files(&[(path, "2021-03-04")]));
            let got = match (manifest.movie.first(), manifest.show.first()) {
                (Some(movie), _) => (movie.title.as_str(), movie.year, movie.tmdb, movie.review),
                (None, Some(show)) => (show.title.as_str(), show.year, show.tmdb, show.review),
                (None, None) => panic!("{path}: nothing imported"),
            };
            assert_eq!(got, entry, "{path}");
            assert_eq!(report.review, review, "{path}");
        }
    }

    #[test]
    fn an_entry_is_dated_by_its_file_only_on_the_first_import() {
        let cases: [(&[(&str, &str)], &[(&str, &str)], &str, &[&str], usize); 2] = [
            (
                &[],
                &[
                    ("Movies/airplane/airplane.mkv", "2021-03-04"),
                    ("Series/Dark/Dark.S01E01.mkv", "2020-05-06"),
                ],
                "2021-03-04",
                &["2020-05-06"],
                2,
            ),
            (
                &[("Series/Dark/Dark.S01E01.mkv", "2020-05-06")],
                &[
                    ("Series/Dark/Dark.S01E01.mkv", "2020-05-06"),
                    ("Series/Dark/Dark.S01E02.mkv", "2015-01-01"),
                    ("Movies/airplane/airplane.mkv", "2015-01-01"),
                ],
                TODAY,
                &["2020-05-06", TODAY],
                2,
            ),
        ];
        for (earlier, tree, movie_added, episodes_added, added) in cases {
            let existing = (!earlier.is_empty()).then(|| import(None, &files(earlier)).0);
            let (manifest, report) = import(existing, &files(tree));
            let episodes: Vec<_> = manifest.show[0].episodes.iter().map(|e| e.added).collect();
            let want: Vec<_> = episodes_added.iter().map(|text| date(text)).collect();
            assert_eq!(manifest.movie[0].added, date(movie_added), "{earlier:?}");
            assert_eq!(episodes, want, "{earlier:?}");
            assert_eq!(report.added.len(), added, "{earlier:?}");
        }
    }

    #[test]
    fn specials_take_season_zero_and_the_next_free_number() {
        let cases: [(&[(&str, &str)], &[(&str, &str)], &[(u32, u32, bool)]); 2] = [
            (
                &[],
                &[("Series/Dark/Dark.Behind.The.Scenes.2021.mp4", "2021-03-04")],
                &[(0, 1, true)],
            ),
            (
                &[("Series/Dark/Dark.One.mp4", "2021-03-04")],
                &[
                    ("Series/Dark/Dark.Two.mp4", "2021-03-04"),
                    ("Series/Dark/Dark.One.mp4", "2021-03-04"),
                ],
                &[(0, 1, true), (0, 2, true)],
            ),
        ];
        for (earlier, tree, want) in cases {
            let existing = (!earlier.is_empty()).then(|| import(None, &files(earlier)).0);
            let (manifest, _) = import(existing, &files(tree));
            let episodes: Vec<_> = manifest.show[0]
                .episodes
                .iter()
                .map(|e| (e.season, e.episode, e.review))
                .collect();
            assert_eq!(episodes, want, "{tree:?}");
        }
    }

    #[test]
    fn edits_to_existing_entries_survive_and_a_second_run_changes_nothing() {
        let tree = files(&[
            ("Movies/airplane/airplane.mkv", "2021-03-04"),
            ("Series/Dark/Dark.S01E01.mkv", "2021-03-04"),
        ]);
        let (mut manifest, _) = import(None, &tree);
        manifest.movie[0].title = "Flying High".to_string();
        manifest.movie[0].tmdb = Some(1);
        manifest.movie[0].added = date("1999-09-09");
        manifest.movie[0].review = true;
        manifest.show[0].title = "Dunkel".to_string();
        manifest.show[0].episodes[0].title = Some("Secrets".to_string());
        manifest.movie[0].imdb = Some("tt0080339".to_string());
        manifest.movie[0].ratings.rt = Some(97);
        manifest.movie[0].rated = Some(date("2026-11-01"));
        manifest.show[0].ratings.imdb = Some(crate::manifest::Tenths(87));
        let edited = manifest.clone();
        let (again, report) = import(Some(manifest), &tree);
        assert_eq!(again.render(), edited.render());
        assert!(report.added.is_empty() && report.review.is_empty());
    }

    #[test]
    fn importing_again_fills_missing_tags_and_keeps_hand_edits() {
        let text = r#"[[show]]
title = "Dark"
episodes = [
  { season = 1, episode = 1, path = "Series/Dark/Dark.S01E01.1080p.x265.DD5.1.mkv", added = 2021-03-04, video = "AV1" },
]

[[movie]]
title = "Gone"
path = "Movies/Gone/Gone.720p.x264.mkv"
added = 2021-03-04
review = true
"#;
        let tree = files(&[("Series/Dark/Dark.S01E01.1080p.x265.DD5.1.mkv", "2021-03-04")]);
        let (first, _) = import(Some(Manifest::parse(text).unwrap()), &tree);
        let episode = first.show[0].episodes[0].tags();
        assert_eq!(episode.video.as_deref(), Some("AV1"));
        assert_eq!(episode.resolution.as_deref(), Some("1080p"));
        assert_eq!(episode.audio.as_deref(), Some("5.1"));
        assert_eq!(first.movie[0].resolution.as_deref(), Some("720p"));
        assert!(first.movie[0].review);
        let (second, report) = import(Some(first.clone()), &tree);
        assert_eq!(second.render(), first.render());
        assert!(report.added.is_empty());
    }

    #[test]
    fn a_new_episode_joins_a_renamed_show_through_its_folder() {
        let (mut manifest, _) = import(None, &files(&[("Series/Dark/Dark.S01E01.mkv", "2021-03-04")]));
        manifest.show[0].title = "Dunkel".to_string();
        let tree = files(&[
            ("Series/Dark/Dark.S01E01.mkv", "2021-03-04"),
            ("Series/Dark/Dark.S01E02.mkv", "2021-03-04"),
        ]);
        let (manifest, _) = import(Some(manifest), &tree);
        assert_eq!(manifest.show.len(), 1);
        assert_eq!(manifest.show[0].episodes.len(), 2);
    }

    #[test]
    fn a_second_copy_of_a_title_is_added_for_review() {
        let (first, _) = import(None, &files(&[("Movies/Up (2009)/Up.2009.mkv", "2021-03-04")]));
        let tree = files(&[
            ("Movies/Up (2009)/Up.2009.mkv", "2021-03-04"),
            ("Movies/Up 2009 better/Up.2009.1080p.mkv", "2021-03-04"),
        ]);
        let parsed = Parsed {
            movies: tree
                .iter()
                .flat_map(|file| parse::build(std::slice::from_ref(file)).movies)
                .collect(),
            ..Parsed::default()
        };
        let (manifest, report) = merge(Some(first), &parsed, &tree, date(TODAY), known_title);
        assert_eq!(manifest.movie.len(), 2);
        assert!(manifest.movie.iter().any(|movie| movie.review));
        assert!(report.review.iter().any(|line| line.contains("already in the library")));
    }

    #[test]
    fn missing_files_keep_their_entry_and_are_reported() {
        let (first, _) = import(None, &files(&[("Movies/Up (2009)/Up.2009.mkv", "2021-03-04")]));
        let tree = files(&[("Movies/Brave (2012)/Brave.2012.mkv", "2021-03-04")]);
        let (manifest, report) = import(Some(first), &tree);
        assert_eq!(manifest.movie.len(), 2);
        assert_eq!(report.missing, ["Movies/Up (2009)/Up.2009.mkv"]);
    }

    #[test]
    fn duplicates_the_parser_dropped_are_reported() {
        let (_, report) = import(
            None,
            &[
                FileEntry {
                    path: "Series/Dark/Dark.S01E01.mkv".into(),
                    size: 9,
                    modified: date("2021-03-04"),
                },
                FileEntry {
                    path: "Movies/Dark.S01E01.mkv".into(),
                    size: 1,
                    modified: date("2021-03-04"),
                },
                FileEntry {
                    path: "Movies/Dark/Trailer.mkv".into(),
                    size: 1,
                    modified: date("2021-03-04"),
                },
            ],
        );
        assert_eq!(report.ignored, ["Movies/Dark.S01E01.mkv"]);
    }
}
