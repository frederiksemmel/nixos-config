use crate::library::normalize;
use crate::manifest::Date;
use regex::Regex;
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
    pub modified: Date,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    pub shows: Vec<ParsedShow>,
    pub movies: Vec<ParsedMovie>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedShow {
    pub title: String,
    pub year: Option<u16>,
    pub episodes: Vec<ParsedEpisode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedEpisode {
    pub season: u32,
    pub number: u32,
    pub title: Option<String>,
    pub path: PathBuf,
    pub guessed: bool,
    pub modified: Date,
    size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedMovie {
    pub title: String,
    pub file_title: String,
    pub year: Option<u16>,
    pub path: PathBuf,
    pub modified: Date,
    size: u64,
}

pub fn scan(root: &Path) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    for dir in [SERIES_DIR, MOVIES_DIR] {
        walk(root, Path::new(dir), &mut entries);
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries
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
                modified: Date::from_unix(modified.map_or(0, |elapsed| elapsed.as_secs())),
            });
        }
    }
}

pub fn read_listing(text: &str) -> Result<Vec<FileEntry>, String> {
    let mut entries = Vec::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let invalid = || format!("not a line of rclone lsf --format tsp: {line}");
        let mut fields = line.splitn(3, ';');
        let (Some(time), Some(size), Some(path)) = (fields.next(), fields.next(), fields.next()) else {
            return Err(invalid());
        };
        let modified = time.get(..10).and_then(Date::parse).ok_or_else(invalid)?;
        let size = size.parse().map_err(|_| invalid())?;
        let path = PathBuf::from(path);
        if is_video(&path) && (path.starts_with(SERIES_DIR) || path.starts_with(MOVIES_DIR)) {
            entries.push(FileEntry { path, size, modified });
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| VIDEO_EXTENSIONS.contains(&extension.to_lowercase().as_str()))
}

pub fn is_extra(path: &Path) -> bool {
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

pub fn build(entries: &[FileEntry]) -> Parsed {
    let mut shows: BTreeMap<String, ParsedShow> = BTreeMap::new();
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
            let show = shows.entry(normalize(&title)).or_insert_with(|| ParsedShow {
                title,
                ..ParsedShow::default()
            });
            show.year = show.year.or(year).or_else(|| parse_release(&stem(&entry.path)).1);
            add_episode(show, entry, marker);
        } else if marker.is_some() {
            misfiled.push(entry);
        } else {
            movie_files.entry(PathBuf::from(top)).or_default().push(entry.clone());
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
        let show = shows.entry(key).or_insert_with(|| ParsedShow {
            title,
            ..ParsedShow::default()
        });
        show.year = show.year.or(year).or_else(|| parse_release(&stem(&entry.path)).1);
        add_episode(show, entry, find_episode(&entry.path));
    }

    let mut shows: Vec<ParsedShow> = shows.into_values().collect();
    for show in &mut shows {
        show.episodes.sort_by_key(|episode| (episode.season, episode.number));
    }
    Parsed {
        shows,
        movies: build_movies(movie_files),
    }
}

fn is_word_suffix(long: &str, short: &str) -> bool {
    long.strip_suffix(short).is_some_and(|rest| rest.ends_with(' '))
}

fn add_episode(show: &mut ParsedShow, entry: &FileEntry, marker: Option<Marker>) {
    let guessed = marker.is_none();
    let (season, number, title) = match marker {
        Some(marker) => (marker.season, marker.episode, marker.title),
        None => (0, next_special(show), Some(parse_release(&stem(&entry.path)).0)),
    };
    let episode = ParsedEpisode {
        season,
        number,
        title,
        path: entry.path.clone(),
        guessed,
        modified: entry.modified,
        size: entry.size,
    };
    let existing = show
        .episodes
        .iter_mut()
        .find(|existing| (existing.season, existing.number) == (season, number));
    match existing {
        Some(existing) if existing.size >= episode.size => {}
        Some(existing) => *existing = episode,
        None => show.episodes.push(episode),
    }
}

fn next_special(show: &ParsedShow) -> u32 {
    show.episodes.iter().filter(|episode| episode.season == 0).count() as u32 + 1
}

fn build_movies(groups: BTreeMap<PathBuf, Vec<FileEntry>>) -> Vec<ParsedMovie> {
    let mut movies: BTreeMap<(String, Option<u16>), ParsedMovie> = BTreeMap::new();
    for (top, files) in groups {
        for movie in movies_in_group(&top, files) {
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

fn movies_in_group(top: &Path, files: Vec<FileEntry>) -> Vec<ParsedMovie> {
    let is_loose = files.iter().all(|file| file.path.components().count() == 2);
    if is_loose {
        return files.iter().map(movie_from_file).collect();
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
        if leaf.components().count() > top_depth {
            let largest = files.iter().max_by_key(|file| file.size).unwrap();
            movies.push(movie_from_file(largest));
        } else if let [file] = files.as_mut_slice() {
            let mut movie = movie_from_file(file);
            let (title, year) = clean_folder_title(&top.to_string_lossy(), movie.year);
            movie.title = title;
            movie.year = year;
            movies.push(movie);
        } else {
            movies.extend(files.iter().map(movie_from_file));
        }
    }
    movies
}

fn movie_from_file(file: &FileEntry) -> ParsedMovie {
    let (title, year) = parse_release(&stem(&file.path));
    ParsedMovie {
        title: title.clone(),
        file_title: title,
        year,
        path: file.path.clone(),
        modified: file.modified,
        size: file.size,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(paths: &[(&str, u64)]) -> Vec<FileEntry> {
        paths
            .iter()
            .map(|(path, size)| FileEntry {
                path: path.into(),
                size: *size,
                modified: Date::default(),
            })
            .collect()
    }

    fn library(paths: &[(&str, u64)]) -> Parsed {
        build(&entries(paths))
    }

    fn titles(parsed: &Parsed) -> Vec<(String, Option<u16>)> {
        parsed
            .movies
            .iter()
            .map(|movie| (movie.title.clone(), movie.year))
            .collect()
    }

    fn movie(paths: &[(&str, u64)]) -> (String, Option<u16>) {
        titles(&library(paths)).remove(0)
    }

    fn seasons(show: &ParsedShow) -> Vec<u32> {
        let mut seasons: Vec<_> = show.episodes.iter().map(|episode| episode.season).collect();
        seasons.dedup();
        seasons
    }

    fn episode_titles(show: &ParsedShow) -> Vec<Option<&str>> {
        show.episodes.iter().map(|episode| episode.title.as_deref()).collect()
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
            .map(|show| (show.title.as_str(), seasons(show)))
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
        assert_eq!(
            episode_titles(&library.shows[0]),
            [Some("1 23 45"), Some("Please Remain Calm")]
        );
        assert_eq!(library.shows[2].episodes[0].title, None);
        assert_eq!(episode_titles(&library.shows[3]), [Some("The Emperor's Peace")]);
        assert_eq!(
            (library.shows[4].episodes[0].number, library.shows[4].episodes[0].season),
            (2, 1)
        );
        assert_eq!(episode_titles(&library.shows[5]), [Some("Chapter 1: The Dark Revenge")]);
        assert_eq!(episode_titles(&library.shows[6]), [Some("Balancing")]);
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
                let episodes: Vec<_> = show.episodes.iter().map(|e| (e.season, e.number)).collect();
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
            library.shows[0].episodes[2]
                .path
                .ends_with("Dragon/Dragon.S02E08.1080p.WEBRip.mkv")
        );
        assert!(library.shows[1].episodes[0].path.starts_with("Movies"));
    }

    #[test]
    fn one_off_specials_are_guesses() {
        let library = library(&[("Series/Friends The Reunion 2021/Friends.The.Reunion.2021.1080p.mp4", 1)]);
        let show = &library.shows[0];
        assert_eq!((show.title.as_str(), show.year), ("Friends The Reunion", Some(2021)));
        assert_eq!(show.episodes.len(), 1);
        assert!(show.episodes[0].guessed);
    }

    #[test]
    fn rclone_listings_read_like_the_tree() {
        let listing = "2024-03-29 22:46:13;4943;Movies/Wonka; Director's Cut (2023)/Wonka.2023.mkv\n\
                       2023-11-28 16:25:58;1659;Series/Dark/Dark.S01E01.mkv\n\
                       2023-11-28 16:25:58;12;Series/Dark/RARBG.txt\n\
                       2023-11-28 16:25:58;12;Incoming/New.Film.2026.mkv\n";
        let entries = read_listing(listing).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0].path,
            Path::new("Movies/Wonka; Director's Cut (2023)/Wonka.2023.mkv")
        );
        assert_eq!(entries[0].size, 4943);
        assert_eq!(entries[0].modified, Date::parse("2024-03-29").unwrap());
        assert!(read_listing("nonsense").is_err());
    }

    #[test]
    fn scanning_only_looks_at_series_and_movies() {
        let root = std::env::temp_dir().join(format!("semmeltv-scan-{}", std::process::id()));
        for dir in ["Series/Dark", "Incoming"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        fs::write(root.join("Series/Dark/Dark.S01E01.mkv"), b"x").unwrap();
        fs::write(root.join("Series/Dark/RARBG.txt"), b"x").unwrap();
        fs::write(root.join("Incoming/New.Film.2026.mkv"), b"x").unwrap();
        let found: Vec<_> = scan(&root).into_iter().map(|entry| entry.path).collect();
        assert_eq!(found, [PathBuf::from("Series/Dark/Dark.S01E01.mkv")]);
        fs::remove_dir_all(root).unwrap();
    }
}
