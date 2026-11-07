use regex::Regex;
use serde::Deserialize;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::UNIX_EPOCH;

const SERIES_DIR: &str = "Series";
const MOVIES_DIR: &str = "Movies";
const VIDEO_EXTENSIONS: [&str; 7] = ["mkv", "mp4", "avi", "m4v", "mov", "webm", "ts"];
const SKIPPED_DIRS: [&str; 7] = [
    "sample",
    "samples",
    "extras",
    "featurettes",
    "subs",
    "trailers",
    "screens",
];
const POSTER_NAMES: [&str; 2] = ["poster.jpg", "poster.png"];

static EPISODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:^|[^a-z0-9])S(\d{1,2})[ ._-]?E(\d{1,3})(?:[^a-z0-9]|$)").unwrap());
static EPISODE_DASHED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ ._-](\d{2})-(\d{2})$").unwrap());
static SEASON_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)[ ._-]+(?:S\d{1,2}|Season[ ._-]*\d{1,2})$").unwrap());
static BRACKETS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[[^\]]*\]").unwrap());
static PAREN_YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\((19\d\d|20\d\d)\)").unwrap());
static YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\(?(19\d\d|20\d\d)\)?$").unwrap());
static PLAIN_EPISODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^episode \d+$").unwrap());
static RELEASE_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?ix)^\(?(?:
            \d{3,4}p | 4k | uhd | web | web-?dl | web-?rip | br-?rip | bd-?rip | blu-?ray | hdtv | hdrip | dvdrip
            | x26[45] | h26[45] | hevc | av1 | 10-?bits? | amzn | hmax | dsnp | atvp | repack | proper | remastered
            | extended | uncut | imax | restored | unrated | internal | multisubs | spanish | slosubs
            | ddp?\d.* | aac.* | ac3 | eac3.* | atmos | yify
        )\)?$",
    )
    .unwrap()
});

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
    pub modified: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Library {
    pub shows: Vec<Show>,
    pub movies: Vec<Movie>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Show {
    pub key: String,
    pub title: String,
    pub year: Option<u16>,
    pub tmdb: Option<u64>,
    pub poster: Option<PathBuf>,
    pub dirs: Vec<PathBuf>,
    pub seasons: Vec<Season>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Season {
    pub number: u32,
    pub episodes: Vec<Episode>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Episode {
    pub season: u32,
    pub number: u32,
    pub title: Option<String>,
    pub path: PathBuf,
    size: u64,
    modified: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Movie {
    pub key: String,
    pub title: String,
    pub file_title: String,
    pub year: Option<u16>,
    pub tmdb: Option<u64>,
    pub poster: Option<PathBuf>,
    pub dir: Option<PathBuf>,
    pub path: PathBuf,
    size: u64,
    modified: u64,
}

impl Library {
    pub fn find_episode(&self, path: &Path) -> Option<(&Show, &Episode)> {
        self.shows.iter().find_map(|show| {
            show.seasons
                .iter()
                .flat_map(|season| &season.episodes)
                .find(|episode| episode.path == path)
                .map(|episode| (show, episode))
        })
    }
}

impl Show {
    pub fn episode(&self, season: u32, number: u32) -> Option<&Episode> {
        self.seasons
            .iter()
            .find(|candidate| candidate.number == season)?
            .episodes
            .iter()
            .find(|episode| episode.number == number)
    }

    fn regular_episodes(&self) -> impl Iterator<Item = &Episode> {
        self.seasons
            .iter()
            .filter(|season| season.number > 0)
            .flat_map(|season| &season.episodes)
    }

    pub fn first_episode(&self) -> Option<&Episode> {
        self.regular_episodes()
            .next()
            .or_else(|| self.seasons.first()?.episodes.first())
    }

    pub fn episode_after(&self, season: u32, number: u32) -> Option<&Episode> {
        self.regular_episodes()
            .find(|episode| (episode.season, episode.number) > (season, number))
    }

    pub fn last_season(&self) -> u32 {
        self.seasons.iter().map(|season| season.number).max().unwrap_or(0)
    }

    pub fn episode_count(&self) -> usize {
        self.seasons.iter().map(|season| season.episodes.len()).sum()
    }

    fn latest_addition(&self) -> u64 {
        self.seasons
            .iter()
            .flat_map(|season| &season.episodes)
            .map(|episode| episode.modified)
            .max()
            .unwrap_or(0)
    }

    pub fn only_episode(&self) -> Option<&Episode> {
        match self.seasons.as_slice() {
            [Season { episodes, .. }] if episodes.len() == 1 => episodes.first(),
            _ => None,
        }
    }

    pub fn episode_key(&self, episode: &Episode) -> String {
        format!("{}-s{:02}e{:02}", self.key, episode.season, episode.number)
    }

    pub fn subtitle(&self) -> String {
        let seasons = self.seasons.iter().filter(|season| season.number > 0).count();
        match seasons {
            0 | 1 => plural(self.episode_count(), "episode"),
            _ => format!(
                "{} · {}",
                plural(seasons, "season"),
                plural(self.episode_count(), "episode")
            ),
        }
    }
}

impl Season {
    pub fn label(&self) -> String {
        match self.number {
            0 => "Specials".to_string(),
            number => format!("Season {number}"),
        }
    }
}

impl Episode {
    pub fn label(&self) -> String {
        match &self.title {
            Some(title) if self.season > 0 => format!("{}. {title}", self.number),
            Some(title) => title.clone(),
            None => format!("Episode {}", self.number),
        }
    }
}

fn plural(count: usize, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
}

#[derive(Debug, Default, Deserialize)]
struct Meta {
    title: Option<String>,
    year: Option<u16>,
    tmdb: Option<u64>,
}

pub fn scan(root: &Path) -> Library {
    let mut entries = Vec::new();
    for dir in [SERIES_DIR, MOVIES_DIR] {
        walk(root, Path::new(dir), &mut entries);
    }
    let mut library = build(root, entries);
    for show in &mut library.shows {
        apply_show_files(show);
    }
    for movie in &mut library.movies {
        apply_movie_files(movie);
    }
    sort(&mut library);
    library
}

fn walk(root: &Path, relative: &Path, entries: &mut Vec<FileEntry>) {
    let Ok(listing) = fs::read_dir(root.join(relative)) else {
        return;
    };
    for entry in listing.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let path = relative.join(&name);
        let Ok(metadata) = fs::metadata(root.join(&path)) else {
            continue;
        };
        if metadata.is_dir() {
            walk(root, &path, entries);
        } else if is_video(&path) {
            let modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok());
            entries.push(FileEntry {
                path,
                size: metadata.len(),
                modified: modified.map_or(0, |elapsed| elapsed.as_secs()),
            });
        }
    }
}

fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| VIDEO_EXTENSIONS.contains(&extension.to_lowercase().as_str()))
}

fn is_extra(path: &Path) -> bool {
    let in_skipped_dir = path.parent().is_some_and(|parent| {
        parent.components().any(|part| {
            let part = part.as_os_str().to_string_lossy().to_lowercase();
            SKIPPED_DIRS.contains(&part.as_str())
        })
    });
    let stem = stem(path).to_lowercase();
    in_skipped_dir
        || stem.contains("trailer")
        || stem.ends_with("sample")
        || stem.ends_with("_stereo")
        || stem.ends_with(" stereo")
}

fn stem(path: &Path) -> String {
    path.file_stem().unwrap_or_default().to_string_lossy().into_owned()
}

fn read_meta(dir: &Path) -> Meta {
    fs::read_to_string(dir.join("meta.toml"))
        .ok()
        .and_then(|text| match toml::from_str(&text) {
            Ok(meta) => Some(meta),
            Err(error) => {
                eprintln!("{}: {error}", dir.join("meta.toml").display());
                None
            }
        })
        .unwrap_or_default()
}

fn find_poster(dir: &Path) -> Option<PathBuf> {
    POSTER_NAMES
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())
}

fn apply_show_files(show: &mut Show) {
    for dir in show.dirs.clone() {
        let meta = read_meta(&dir);
        show.title = meta.title.unwrap_or_else(|| show.title.clone());
        show.year = meta.year.or(show.year);
        show.tmdb = meta.tmdb.or(show.tmdb);
        show.poster = show.poster.take().or_else(|| find_poster(&dir));
    }
}

fn apply_movie_files(movie: &mut Movie) {
    let Some(dir) = movie.dir.clone() else {
        return;
    };
    let meta = read_meta(&dir);
    movie.title = meta.title.unwrap_or_else(|| movie.title.clone());
    movie.year = meta.year.or(movie.year);
    movie.tmdb = meta.tmdb;
    movie.poster = find_poster(&dir);
}

pub fn build(root: &Path, entries: Vec<FileEntry>) -> Library {
    let mut shows: BTreeMap<String, Show> = BTreeMap::new();
    let mut misfiled = Vec::new();
    let mut movie_files: BTreeMap<PathBuf, Vec<FileEntry>> = BTreeMap::new();

    for entry in entries {
        if is_extra(&entry.path) {
            continue;
        }
        let parts: Vec<String> = entry
            .path
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect();
        let [section, top, ..] = parts.as_slice() else {
            continue;
        };
        let marker = find_episode(&entry.path);
        if section == SERIES_DIR {
            let top_is_file = parts.len() == 2;
            let name = if top_is_file {
                show_name_before_marker(&entry.path)
            } else {
                top.clone()
            };
            let (title, year) = clean_show(&name);
            let show = shows.entry(normalize(&title)).or_insert_with(|| Show {
                title,
                ..Show::default()
            });
            show.year = show.year.or(year).or_else(|| parse_release(&stem(&entry.path)).1);
            if !top_is_file {
                push_dir(show, root.join(section).join(top));
            }
            add_episode(show, root, &entry, marker);
        } else if marker.is_some() {
            misfiled.push(entry);
        } else {
            movie_files.entry(PathBuf::from(top)).or_default().push(entry);
        }
    }

    for entry in misfiled {
        let (title, year) = clean_show(&show_name_before_marker(&entry.path));
        let normalized = normalize(&title);
        let key = shows
            .keys()
            .find(|key| **key == normalized || is_word_suffix(key, &normalized) || is_word_suffix(&normalized, key))
            .cloned()
            .unwrap_or(normalized);
        let show = shows.entry(key).or_insert_with(|| Show {
            title,
            ..Show::default()
        });
        show.year = show.year.or(year).or_else(|| parse_release(&stem(&entry.path)).1);
        let marker = find_episode(&entry.path);
        add_episode(show, root, &entry, marker);
    }

    let mut library = Library {
        shows: shows.into_values().map(finish_show).collect(),
        movies: build_movies(root, movie_files),
    };
    sort(&mut library);
    library
}

fn push_dir(show: &mut Show, dir: PathBuf) {
    if !show.dirs.contains(&dir) {
        show.dirs.push(dir);
    }
}

fn is_word_suffix(long: &str, short: &str) -> bool {
    long.strip_suffix(short).is_some_and(|rest| rest.ends_with(' '))
}

fn add_episode(show: &mut Show, root: &Path, entry: &FileEntry, marker: Option<Marker>) {
    let (season_number, number, title) = match marker {
        Some(marker) => (marker.season, marker.episode, marker.title),
        None => (0, next_special(show), Some(parse_release(&stem(&entry.path)).0)),
    };
    let episode = Episode {
        season: season_number,
        number,
        title,
        path: root.join(&entry.path),
        size: entry.size,
        modified: entry.modified,
    };
    let season = match show.seasons.iter_mut().find(|season| season.number == season_number) {
        Some(season) => season,
        None => {
            show.seasons.push(Season {
                number: season_number,
                episodes: Vec::new(),
            });
            show.seasons.last_mut().unwrap()
        }
    };
    match season.episodes.iter_mut().find(|existing| existing.number == number) {
        Some(existing) if existing.size >= episode.size => {}
        Some(existing) => *existing = episode,
        None => season.episodes.push(episode),
    }
}

fn next_special(show: &Show) -> u32 {
    show.seasons
        .iter()
        .find(|season| season.number == 0)
        .map_or(1, |season| season.episodes.len() as u32 + 1)
}

fn finish_show(mut show: Show) -> Show {
    show.key = slug(&show.title);
    show.seasons
        .sort_by_key(|season| if season.number == 0 { u32::MAX } else { season.number });
    for season in &mut show.seasons {
        season.episodes.sort_by_key(|episode| episode.number);
    }
    show
}

fn build_movies(root: &Path, groups: BTreeMap<PathBuf, Vec<FileEntry>>) -> Vec<Movie> {
    let mut movies: BTreeMap<(String, Option<u16>), Movie> = BTreeMap::new();
    for (top, files) in groups {
        for movie in movies_in_group(root, &top, files) {
            let key = (normalize(&movie.title), movie.year);
            match movies.get(&key) {
                Some(existing) if existing.size >= movie.size => {}
                _ => {
                    movies.insert(key, movie);
                }
            }
        }
    }
    movies.into_values().collect()
}

fn movies_in_group(root: &Path, top: &Path, files: Vec<FileEntry>) -> Vec<Movie> {
    let is_loose = files.iter().all(|file| file.path.components().count() == 2);
    if is_loose {
        return files.iter().map(|file| movie_from_file(root, file, None)).collect();
    }

    let mut leaves: BTreeMap<PathBuf, Vec<FileEntry>> = BTreeMap::new();
    for file in files {
        leaves
            .entry(file.path.parent().unwrap_or(Path::new("")).to_path_buf())
            .or_default()
            .push(file);
    }
    let top_depth = 2;
    let mut movies = Vec::new();
    for (leaf, mut files) in leaves {
        let dir = root.join(&leaf);
        if leaf.components().count() > top_depth {
            let largest = files.iter().max_by_key(|file| file.size).unwrap();
            movies.push(movie_from_file(root, largest, Some(dir)));
        } else if let [file] = files.as_mut_slice() {
            let mut movie = movie_from_file(root, file, Some(dir));
            let (title, year) = clean_folder_title(&top.to_string_lossy(), movie.year);
            movie.title = title;
            movie.year = year;
            movies.push(movie);
        } else {
            movies.extend(files.iter().map(|file| movie_from_file(root, file, None)));
        }
    }
    movies
}

fn movie_from_file(root: &Path, file: &FileEntry, dir: Option<PathBuf>) -> Movie {
    let (title, year) = parse_release(&stem(&file.path));
    Movie {
        title: title.clone(),
        file_title: title,
        year,
        dir,
        path: root.join(&file.path),
        size: file.size,
        modified: file.modified,
        ..Movie::default()
    }
}

fn clean_folder_title(name: &str, file_year: Option<u16>) -> (String, Option<u16>) {
    let name = fix_punctuation(name);
    if let Some(found) = PAREN_YEAR.captures(&name) {
        let title = trim_title(&name[..found.get(0).unwrap().start()]);
        return (title, found[1].parse().ok());
    }
    let (title, year) = parse_release(&name);
    match (year, file_year) {
        (Some(folder), Some(file)) if folder != file => (trim_title(&format!("{title} {folder}")), Some(file)),
        _ => (title, year.or(file_year)),
    }
}

fn clean_show(name: &str) -> (String, Option<u16>) {
    let name = fix_punctuation(name);
    let name = SEASON_SUFFIX.replace(&name, "");
    let (title, year) = parse_release(&name);
    (title, year)
}

struct Marker {
    season: u32,
    episode: u32,
    title: Option<String>,
}

fn find_episode(path: &Path) -> Option<Marker> {
    let file = stem(path);
    if let Some(found) = EPISODE.captures(&file) {
        let after = &file[found.get(0).unwrap().end()..];
        let title = parse_release(after.trim_start_matches([' ', '-', '.'])).0;
        return Some(Marker {
            season: found[1].parse().ok()?,
            episode: found[2].parse().ok()?,
            title: (!title.is_empty() && !PLAIN_EPISODE.is_match(&title)).then_some(title),
        });
    }
    if let Some(found) = EPISODE_DASHED.captures(&file) {
        return Some(Marker {
            season: found[1].parse().ok()?,
            episode: found[2].parse().ok()?,
            title: None,
        });
    }
    path.parent().and_then(|parent| {
        parent.components().rev().find_map(|part| {
            let found = EPISODE.captures(part.as_os_str().to_str()?)?;
            Some(Marker {
                season: found[1].parse().ok()?,
                episode: found[2].parse().ok()?,
                title: None,
            })
        })
    })
}

fn show_name_before_marker(path: &Path) -> String {
    let file = stem(path);
    let before = EPISODE
        .find(&file)
        .map_or(file.as_str(), |found| &file[..found.start()]);
    before.to_string()
}

fn fix_punctuation(name: &str) -> String {
    name.replace('꞉', ":").replace('’', "'")
}

fn trim_title(title: &str) -> String {
    title.trim().trim_end_matches([' ', '-', ':']).trim().to_string()
}

pub fn parse_release(name: &str) -> (String, Option<u16>) {
    let name = fix_punctuation(name);
    let name = BRACKETS.replace_all(&name, "");
    let name = name.trim();
    let spaced = if name.contains(' ') {
        name.replace('_', " ")
    } else {
        name.replace(['.', '_'], " ")
    };
    let tokens: Vec<&str> = spaced.split_whitespace().collect();
    let end = tokens
        .iter()
        .position(|token| RELEASE_TAG.is_match(token))
        .unwrap_or(tokens.len());
    let year_at = (1..end).rev().find(|&index| YEAR.is_match(tokens[index]));
    let title = tokens[..year_at.unwrap_or(end)].join(" ");
    let year = year_at.and_then(|index| YEAR.captures(tokens[index])?[1].parse().ok());
    (trim_title(&title), year)
}

pub fn normalize(title: &str) -> String {
    let folded: String = title
        .to_lowercase()
        .chars()
        .filter(|c| *c != '\'')
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn slug(title: &str) -> String {
    normalize(title).replace(' ', "-")
}

fn sort_title(title: &str) -> String {
    let normalized = normalize(title);
    normalized
        .strip_prefix("the ")
        .map(str::to_string)
        .unwrap_or(normalized)
}

fn sort(library: &mut Library) {
    library
        .shows
        .sort_by_key(|show| (Reverse(show.latest_addition()), sort_title(&show.title)));
    library
        .movies
        .sort_by_key(|movie| (Reverse(movie.modified), sort_title(&movie.title), movie.year));
    for show in &mut library.shows {
        show.key = slug(&show.title);
    }
    for movie in &mut library.movies {
        movie.key = match movie.year {
            Some(year) => format!("{}-{year}", slug(&movie.title)),
            None => slug(&movie.title),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(paths: &[(&str, u64)]) -> Library {
        let entries = paths
            .iter()
            .map(|(path, size)| FileEntry {
                path: path.into(),
                size: *size,
                modified: 0,
            })
            .collect();
        build("/lib".as_ref(), entries)
    }

    fn titles(library: &Library) -> Vec<(String, Option<u16>)> {
        library
            .movies
            .iter()
            .map(|movie| (movie.title.clone(), movie.year))
            .collect()
    }

    fn movie(library: &[(&str, u64)]) -> (String, Option<u16>) {
        titles(&self::library(library)).remove(0)
    }

    #[test]
    fn release_names_lose_their_tags() {
        let cases = [
            (
                "Anyone.But.You.2023.REPACK.1080p.WEB-DL.DD5.1.H.264-EniaHD",
                "Anyone But You",
                Some(2023),
            ),
            (
                "Barbie 2023 MULTiSUBS 1080p WEB-DL DD5.1 HEVC x265-RMTeam ",
                "Barbie",
                Some(2023),
            ),
            ("Wonka 2023 1080p PROPER WEBRip AV1 Opus 5.1 [981]", "Wonka", Some(2023)),
            (
                "Wonder.Woman.1984.2020.IMAX.1080p.WEBRip.x265-RARBG",
                "Wonder Woman 1984",
                Some(2020),
            ),
            (
                "The.Trial.of.the.Chicago.7.2020.1080p.WEBRip",
                "The Trial of the Chicago 7",
                Some(2020),
            ),
            ("The Order", "The Order", None),
            ("1917.2019.1080p.BluRay", "1917", Some(2019)),
            ("Green.Book.2018.1080p.WEBRip.x264-[YTS.AM]", "Green Book", Some(2018)),
            (
                "Paddington.2.2017.1080p.BluRay.x264-[YTS.AM]",
                "Paddington 2",
                Some(2017),
            ),
        ];
        for (name, title, year) in cases {
            assert_eq!(parse_release(name), (title.to_string(), year), "{name}");
        }
    }

    #[test]
    fn folder_titles_win_over_file_titles() {
        let cases = [
            (
                "Movies/Surf's Up 2007/Surfs.Up.2007.720p.BluRay.mkv",
                "Surf's Up",
                Some(2007),
            ),
            (
                "Movies/Brave (2012) [1080p]/Brave.2012.1080p.BRrip.mp4",
                "Brave",
                Some(2012),
            ),
            (
                "Movies/Green Book/Green.Book.2018.1080p.WEBRip.mp4",
                "Green Book",
                Some(2018),
            ),
            (
                "Movies/Wonder Woman 1984/Wonder.Woman.1984.2020.IMAX.1080p.mp4",
                "Wonder Woman 1984",
                Some(2020),
            ),
            (
                "Movies/Top Gun (1986)/Top.Gun.1986.REMASTERED.1080p.mp4",
                "Top Gun",
                Some(1986),
            ),
            (
                "Movies/Shutter Island (2010) (1080p BluRay x265 Tigole)/Shutter Island (2010) (1080p).mkv",
                "Shutter Island",
                Some(2010),
            ),
            (
                "Movies/Endless (1966) with extras/Endless.1966.1080p.mkv",
                "Endless",
                Some(1966),
            ),
        ];
        for (path, title, year) in cases {
            assert_eq!(movie(&[(path, 1)]), (title.to_string(), year), "{path}");
        }
    }

    #[test]
    fn collections_become_separate_films() {
        let library = library(&[
            ("Movies/Beasts/Beasts Two.mp4", 2),
            ("Movies/Beasts/Beasts One.mp4", 2),
            ("Movies/Potter/1 Potter Stone/Potter.And.The.Stone.2001.1080p.mp4", 2),
            (
                "Movies/Potter/7 Potter Hallows/Potter.And.The.Hallows.2010.1080p.mp4",
                5,
            ),
            ("Movies/Potter/7 Potter Hallows/p1.mp4", 4),
        ]);
        assert_eq!(
            titles(&library),
            [
                ("Beasts One".to_string(), None),
                ("Beasts Two".to_string(), None),
                ("Potter And The Hallows".to_string(), Some(2010)),
                ("Potter And The Stone".to_string(), Some(2001)),
            ]
        );
    }

    #[test]
    fn extras_and_alternate_audio_are_ignored() {
        let library = library(&[
            ("Movies/Wind/WIND.avi", 9),
            ("Movies/Wind/Theatrical Trailer.avi", 1),
            ("Movies/Summer (1966)/Summer.1966.1080p.mkv", 9),
            ("Movies/Summer (1966)/Extras/Introduction.mkv", 1),
            ("Movies/Ip Man 4/Ip Man 4.mp4", 9),
            ("Movies/Ip Man 4/Ip Man 4 stereo.mp4", 8),
            ("Movies/Memento 2000/Memento.2000.1080p.mp4", 9),
            ("Movies/Memento 2000/Memento.2000.1080p_stereo.mp4", 8),
        ]);
        assert_eq!(library.movies.len(), 4);
    }

    #[test]
    fn duplicate_films_keep_the_largest_file() {
        let library = library(&[
            ("Movies/Wonka 2023 1080p PROPER WEBRip AV1 [981].mkv", 5),
            ("Movies/Wonka/Wonka 2023 1080p PROPER WEBRip AV1 [981].mkv", 6),
            ("Movies/Island (2010)/Island.2010.1080p.mp4", 2),
            ("Movies/Island (2010) (1080p BluRay)/Island (2010) (1080p).mkv", 3),
        ]);
        assert_eq!(library.movies.len(), 2);
        assert!(
            library.movies[1]
                .path
                .ends_with("Wonka/Wonka 2023 1080p PROPER WEBRip AV1 [981].mkv")
        );
        assert!(library.movies[0].path.ends_with("Island (2010) (1080p).mkv"));
    }

    #[test]
    fn show_layouts_all_produce_seasons() {
        let library = library(&[
            ("Series/Chernobyl Season 1/Chernobyl S01E02 Please Remain Calm.mkv", 1),
            ("Series/Chernobyl Season 1/Chernobyl S01E01 1 23 45.mkv", 1),
            ("Series/Dark/Dark S02/Dark.S02E01.WEBRip.x264-ION10.mp4", 1),
            ("Series/Dark/Dark S01/DARK.S01E01.WEBRip.x264-ION10.mp4", 1),
            ("Series/Nine/Nine.S08E04/Nine.S08E04.Balancing.1080p.AMZN.WEB-DL.mkv", 1),
            ("Series/Daredevil S01/Daredevil.S01E01.1080p.BluRay.mp4", 1),
            ("Series/Daredevil S02/Daredevil S02E01.mp4", 1),
            ("Series/Genius/genius-01-02.mp4", 1),
            (
                "Series/Foundation/Foundation S01/Foundation (2021) - S01E01 - The Emperor’s Peace (1080p ATVP x265).mkv",
                1,
            ),
            (
                "Series/Maul/Maul S01/Star Wars꞉ Maul (2026) - S01E01 - Chapter 1꞉ The Dark Revenge (1080p DSNP).mkv",
                1,
            ),
        ]);
        let summary: Vec<_> = library
            .shows
            .iter()
            .map(|show| {
                (
                    show.title.as_str(),
                    show.seasons.iter().map(|s| s.number).collect::<Vec<_>>(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("Chernobyl", vec![1]),
                ("Daredevil", vec![1, 2]),
                ("Dark", vec![1, 2]),
                ("Foundation", vec![1]),
                ("Genius", vec![1]),
                ("Maul", vec![1]),
                ("Nine", vec![8]),
            ]
        );
        let labels = |index: usize| -> Vec<String> {
            library.shows[index].seasons[0]
                .episodes
                .iter()
                .map(Episode::label)
                .collect()
        };
        assert_eq!(labels(0), ["1. 1 23 45", "2. Please Remain Calm"]);
        assert_eq!(labels(2), ["Episode 1"]);
        assert_eq!(labels(3), ["1. The Emperor's Peace"]);
        assert_eq!(labels(4), ["Episode 2"]);
        assert_eq!(labels(5), ["1. Chapter 1: The Dark Revenge"]);
        assert_eq!(labels(6), ["4. Balancing"]);
        assert_eq!(library.shows[3].year, Some(2021));
    }

    #[test]
    fn episodes_filed_under_movies_join_their_show() {
        let library = library(&[
            ("Series/Dragon/Dragon S01E01/Dragon.S01E01.Heirs.1080p.mkv", 9),
            (
                "Movies/Dragon/Dragon.S02E01.1080p.MAX.WEB-DL/Dragon.S02E01.1080p.mkv",
                9,
            ),
            ("Movies/Dragon.S02E08.1080p.WEBRip.mkv", 8),
            ("Movies/Dragon/Dragon.S02E08.1080p.WEBRip.mkv", 9),
            (
                "Series/The Rings/The Lord The Rings S02E02/The.Lord.The.Rings.S02E02.1080p.mkv",
                5,
            ),
            (
                "Movies/The Lord The Rings S02E02 Stars 1080p/The Lord The Rings S02E02 Stars 1080p.mkv",
                6,
            ),
            ("Movies/The Rings S02E08/the.lord.the.rings.s02e08.1080p.web.mkv", 5),
        ]);
        assert!(library.movies.is_empty());
        let summary: Vec<_> = library
            .shows
            .iter()
            .map(|show| {
                let episodes: Vec<_> = show
                    .seasons
                    .iter()
                    .flat_map(|s| &s.episodes)
                    .map(|e| (e.season, e.number))
                    .collect();
                (show.title.as_str(), episodes)
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("Dragon", vec![(1, 1), (2, 1), (2, 8)]),
                ("The Rings", vec![(2, 2), (2, 8)])
            ]
        );
        assert!(
            library.shows[0].seasons[1].episodes[1]
                .path
                .ends_with("Dragon/Dragon.S02E08.1080p.WEBRip.mkv")
        );
        assert!(library.shows[1].seasons[0].episodes[0].path.starts_with("/lib/Movies"));
    }

    #[test]
    fn one_off_specials_have_a_single_episode() {
        let library = library(&[("Series/Friends The Reunion 2021/Friends.The.Reunion.2021.1080p.mp4", 1)]);
        let show = &library.shows[0];
        assert_eq!((show.title.as_str(), show.year), ("Friends The Reunion", Some(2021)));
        assert!(show.only_episode().is_some());
    }

    fn dated(paths: &[(&str, u64)]) -> Library {
        let entries = paths
            .iter()
            .map(|(path, modified)| FileEntry {
                path: path.into(),
                size: 1,
                modified: *modified,
            })
            .collect();
        build("/lib".as_ref(), entries)
    }

    #[test]
    fn newest_films_come_first() {
        let library = dated(&[
            ("Movies/Apple (2001)/Apple.2001.mkv", 100),
            ("Movies/Zebra (1960)/Zebra.1960.mkv", 300),
            ("Movies/Mango (2020)/Mango.2020.mkv", 200),
        ]);
        let order: Vec<_> = library.movies.iter().map(|movie| movie.title.as_str()).collect();
        assert_eq!(order, ["Zebra", "Mango", "Apple"]);
    }

    #[test]
    fn a_new_episode_brings_its_show_forward_but_episodes_stay_in_order() {
        let library = dated(&[
            ("Series/Old Show/Old.Show.S01E01.mkv", 100),
            ("Series/Old Show/Old.Show.S01E02.mkv", 900),
            ("Series/Old Show/Old.Show.S02E01.mkv", 110),
            ("Series/New Show/New.Show.S01E01.mkv", 500),
        ]);
        let order: Vec<_> = library.shows.iter().map(|show| show.title.as_str()).collect();
        assert_eq!(order, ["Old Show", "New Show"]);
        let show = &library.shows[0];
        let seasons: Vec<_> = show.seasons.iter().map(|season| season.number).collect();
        let episodes: Vec<_> = show.seasons[0].episodes.iter().map(|episode| episode.number).collect();
        assert_eq!((seasons, episodes), (vec![1, 2], vec![1, 2]));
    }

    #[test]
    fn episodes_filed_under_movies_count_towards_their_show() {
        let library = dated(&[
            ("Series/Alpha/Alpha.S01E01.mkv", 100),
            ("Series/Beta/Beta.S01E01.mkv", 200),
            ("Movies/Alpha.S01E02.1080p.mkv", 300),
        ]);
        assert_eq!(library.shows[0].title, "Alpha");
    }

    #[test]
    fn titles_added_together_fall_back_to_the_alphabet_ignoring_the() {
        let library = library(&[
            ("Movies/The Zebra/The.Zebra.2001.mp4", 1),
            ("Movies/The Apple/The.Apple.2001.mp4", 1),
            ("Movies/Mango/Mango.2001.mp4", 1),
        ]);
        let order: Vec<_> = library.movies.iter().map(|movie| movie.title.as_str()).collect();
        assert_eq!(order, ["The Apple", "Mango", "The Zebra"]);
    }
}
