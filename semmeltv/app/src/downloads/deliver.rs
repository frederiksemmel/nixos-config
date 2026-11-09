use super::release::{Coverage, Release};
use super::resolve::Target;
use crate::artwork;
use crate::import::{self, Identity, Query};
use crate::library::normalize;
use crate::manifest::{self, Date, Manifest};
use crate::mdblist;
use crate::parse;
use crate::tmdb::{Client, Kind};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

const ATTEMPTS: usize = 3;

pub struct Shelf {
    pub remote: String,
    pub meta: PathBuf,
    pub work: PathBuf,
    pub tmdb: Client,
    pub mdblist: Option<mdblist::Client>,
    pub mount_rc: PathBuf,
    pub sync_unit: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    pub local: PathBuf,
    pub destination: String,
    pub size: u64,
}

fn safe(text: &str) -> String {
    let replaced: String = text
        .chars()
        .filter_map(|c| match c {
            ':' => Some('꞉'),
            '/' | '\\' => Some('-'),
            '?' | '*' | '"' | '<' | '>' | '|' => None,
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect();
    replaced.split_whitespace().collect::<Vec<_>>().join(" ").trim_end_matches('.').to_string()
}

fn named(title: &str, year: Option<u16>) -> String {
    match year {
        Some(year) => format!("{} ({year})", safe(title)),
        None => safe(title),
    }
}

fn with_info(stem: String, info: Option<&str>, extension: &str) -> String {
    match info {
        Some(info) => format!("{stem} ({}).{extension}", safe(info)),
        None => format!("{stem}.{extension}"),
    }
}

pub fn episode_of(file: &Path) -> Option<(u32, u32)> {
    let stem = file.file_stem()?.to_string_lossy();
    match Release::parse(&stem).coverage {
        Coverage::Episodes { season, first, last } if first == last => Some((season, first)),
        _ => None,
    }
}

pub fn destination(target: &Target, file: &Path, part_info: Option<&str>) -> Option<String> {
    let extension = file.extension()?.to_string_lossy().to_lowercase();
    let stem = file.file_stem()?.to_string_lossy();
    let own = Release::parse(&stem).info;
    let info = own.as_deref().or(part_info);
    match target.kind {
        Kind::Movie => {
            let name = named(&target.title, target.year);
            Some(format!("Movies/{name}/{}", with_info(name.clone(), info, &extension)))
        }
        Kind::Tv => {
            let (season, number) = match (episode_of(file), target.season, target.episode) {
                (Some(found), _, _) => found,
                (None, Some(season), Some(episode)) => (season, episode),
                (None, _, _) => return None,
            };
            let folder = target.folder.clone().unwrap_or_else(|| safe(&target.title));
            let mut stem = format!("{} - S{season:02}E{number:02}", named(&target.title, target.year));
            if let Some(name) = target
                .episodes
                .iter()
                .find(|episode| episode.number == number)
                .and_then(|episode| episode.name.as_deref())
                .filter(|name| !name.is_empty())
            {
                stem.push_str(&format!(" - {}", safe(name)));
            }
            Some(format!(
                "Series/{folder}/Season {season:02}/{}",
                with_info(stem, info, &extension)
            ))
        }
    }
}

pub fn folders(placed: &[Placed]) -> Vec<String> {
    let mut folders: Vec<String> = Vec::new();
    for place in placed {
        let parts: Vec<&str> = place.destination.split('/').collect();
        for depth in 1..parts.len() {
            let folder = parts[..depth].join("/");
            if !folders.contains(&folder) {
                folders.push(folder);
            }
        }
    }
    folders
}

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("{program}: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let error = String::from_utf8_lossy(&output.stderr);
        let last = error.lines().rev().find(|line| !line.trim().is_empty()).unwrap_or("");
        Err(format!("{program} {}: {last}", args.first().unwrap_or(&"")))
    }
}

fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn today() -> Date {
    let elapsed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    Date::from_unix(elapsed.map_or(0, |elapsed| elapsed.as_secs()))
}

impl Shelf {
    fn remote(&self, path: &str) -> String {
        format!("{}/{path}", self.remote)
    }

    fn manifest_hash(&self) -> Result<String, String> {
        let listing = run(
            "rclone",
            &["hashsum", "sha1", &self.remote, "--include", "/library.toml"],
        )?;
        listing
            .split_whitespace()
            .next()
            .map(str::to_string)
            .ok_or_else(|| format!("no library.toml in {}", self.remote))
    }

    fn remote_size(&self, path: &str) -> Result<Option<u64>, String> {
        let listing = run("rclone", &["lsjson", "--files-only", &self.remote(path)])?;
        let entries: Vec<serde_json::Value> = serde_json::from_str(&listing).unwrap_or_default();
        Ok(entries.first().and_then(|entry| entry["Size"].as_u64()))
    }

    pub fn upload(&self, placed: &Placed) -> Result<(), String> {
        let local = placed.local.to_string_lossy();
        run("rclone", &["copyto", &local, &self.remote(&placed.destination)])?;
        match self.remote_size(&placed.destination)? {
            Some(size) if size == placed.size => Ok(()),
            Some(size) => Err(format!(
                "{} is {size} bytes on pCloud, {} here",
                placed.destination, placed.size
            )),
            None => Err(format!("{} is missing on pCloud after the upload", placed.destination)),
        }
    }

    pub fn import(&self, target: &Target, placed: &[Placed], log: &dyn Fn(String)) -> Result<(), String> {
        for attempt in 1..=ATTEMPTS {
            let before = self.manifest_hash()?;
            let manifest = self.prepare(target, placed, log)?;
            let after = self.manifest_hash()?;
            if after != before {
                log(format!(
                    "library.toml changed on pCloud while importing (attempt {attempt}), starting over"
                ));
                continue;
            }
            run(
                "rclone",
                &["copy", &self.work.join(artwork::DIR).to_string_lossy(), &self.remote(artwork::DIR)],
            )?;
            if self.manifest_hash()? != before {
                log(format!("library.toml changed on pCloud before the upload (attempt {attempt}), starting over"));
                continue;
            }
            run("rclone", &["copyto", &manifest.to_string_lossy(), &self.remote(manifest::FILE_NAME)])?;
            log("library.toml uploaded".to_string());
            return Ok(());
        }
        Err(format!("library.toml kept changing on pCloud; tried {ATTEMPTS} times"))
    }

    fn prepare(&self, target: &Target, placed: &[Placed], log: &dyn Fn(String)) -> Result<PathBuf, String> {
        let _ = fs::remove_dir_all(&self.work);
        fs::create_dir_all(&self.work).map_err(|error| format!("{}: {error}", self.work.display()))?;
        let manifest_path = self.work.join(manifest::FILE_NAME);
        run(
            "rclone",
            &["copyto", &self.remote(manifest::FILE_NAME), &manifest_path.to_string_lossy()],
        )?;
        let art = self.meta.join(artwork::DIR);
        if art.is_dir() {
            copy_dir(&art, &self.work.join(artwork::DIR)).map_err(|error| format!("artwork copy: {error}"))?;
        }
        let listing = run(
            "rclone",
            &[
                "lsf",
                "-R",
                "--files-only",
                "--format",
                "tsp",
                "--include",
                "/Series/**",
                "--include",
                "/Movies/**",
                &self.remote,
            ],
        )?;
        let files = parse::read_listing(&listing)?;
        let existing = Manifest::read(&manifest_path).map_err(|_| "library.toml on pCloud is unreadable".to_string())?;
        let parsed = parse::build(&files);
        let identity = Identity {
            id: target.tmdb,
            name: target.title.clone(),
            date: target.year.map(|year| format!("{year}-01-01")).unwrap_or_default(),
            inferred: false,
        };
        let names: Vec<String> = [Some(target.title.as_str()), target.folder.as_deref()]
            .into_iter()
            .flatten()
            .map(normalize)
            .collect();
        let identify = |query: &Query| {
            let ours = query.kind == target.kind
                && (names.contains(&normalize(query.title)) || names.contains(&normalize(query.file_title)));
            if ours {
                Some(identity.clone())
            } else {
                import::identify(Some(&self.tmdb), query)
            }
        };
        let (manifest, report) = import::merge(Some(existing), &parsed, &files, today(), identify);
        for line in &report.added {
            log(format!("added: {line}"));
        }
        for line in &report.review {
            log(format!("review: {line}"));
        }
        let listed: Vec<&str> = manifest
            .show
            .iter()
            .flat_map(|show| show.episodes.iter().map(|episode| episode.path.as_str()))
            .chain(manifest.movie.iter().map(|movie| movie.path.as_str()))
            .collect();
        let unlisted: Vec<&str> = placed
            .iter()
            .map(|placed| placed.destination.as_str())
            .filter(|destination| !listed.contains(destination))
            .collect();
        if !unlisted.is_empty() {
            return Err(format!("the import left out {}", unlisted.join(", ")));
        }
        manifest
            .write(&manifest_path)
            .map_err(|error| format!("{}: {error}", manifest_path.display()))?;
        let ratings = self.mdblist.as_ref().map(|client| client as &dyn artwork::Ratings);
        let summary = artwork::run(&manifest_path, &self.tmdb, ratings, false)?;
        log(format!(
            "artwork: {} images, {} titles rated, {} problems",
            summary.images,
            summary.rated,
            summary.problems.len()
        ));
        Ok(manifest_path)
    }

    pub fn show_in_library(&self, placed: &[Placed], log: &dyn Fn(String)) {
        let socket = self.mount_rc.to_string_lossy();
        for dir in folders(placed) {
            let argument = format!("dir={dir}");
            match run("rclone", &["rc", "--unix-socket", &socket, "vfs/refresh", &argument]) {
                Ok(_) => log(format!("refreshed {dir} on the mount")),
                Err(error) => log(format!("could not refresh {dir} on the mount: {error}")),
            }
        }
        match run("systemctl", &["start", "--no-block", &self.sync_unit]) {
            Ok(_) => log(format!("started {}", self.sync_unit)),
            Err(error) => log(format!("could not start {}: {error}", self.sync_unit)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::resolve::EpisodeInfo;
    use super::*;
    use crate::parse::FileEntry;

    fn outlander() -> Target {
        Target {
            kind: Kind::Tv,
            tmdb: 56570,
            imdb: None,
            title: "Outlander".to_string(),
            original: None,
            year: Some(2014),
            season: Some(8),
            episode: None,
            runtime: None,
            episodes: vec![EpisodeInfo {
                number: 1,
                name: Some("Hello: Goodbye?".to_string()),
                runtime: Some(60),
            }],
            folder: None,
        }
    }

    #[test]
    fn episodes_go_to_the_canonical_place_and_keep_their_release_facts() {
        let file = Path::new("Outlander (2014) S08 (1080p)/Outlander (2014) - S08E01 (1080p AMZN WEB-DL x265 10bit EAC3 5.1 Silence).mkv");
        let place = destination(&outlander(), file, Some("1080p AMZN WEB-DL x265 10bit EAC3 5.1 Silence")).unwrap();
        assert_eq!(
            place,
            "Series/Outlander/Season 08/Outlander (2014) - S08E01 - Hello꞉ Goodbye (1080p AMZN WEB-DL x265 10bit EAC3 5.1 Silence).mkv"
        );
        let dotted = Path::new("Outlander.S08E02.1080p.WEB.h264-GRACE.mkv");
        assert_eq!(
            destination(&outlander(), dotted, None).unwrap(),
            "Series/Outlander/Season 08/Outlander (2014) - S08E02 (1080p WEB h264-GRACE).mkv"
        );
    }

    #[test]
    fn only_the_folders_above_new_files_are_refreshed_from_the_top_down() {
        let place = |destination: &str| Placed {
            local: PathBuf::new(),
            destination: destination.to_string(),
            size: 1,
        };
        let placed = [
            place("Series/Bridgerton/Season 01/Bridgerton (2020) - S01E01.mp4"),
            place("Series/Bridgerton/Season 01/Bridgerton (2020) - S01E02.mp4"),
        ];
        assert_eq!(
            folders(&placed),
            ["Series", "Series/Bridgerton", "Series/Bridgerton/Season 01"]
        );
        assert_eq!(
            folders(&[place("Movies/Up (2009)/Up (2009).mkv")]),
            ["Movies", "Movies/Up (2009)"]
        );
    }

    #[test]
    fn a_known_show_keeps_its_folder() {
        let target = Target {
            folder: Some("Outlander Folder".to_string()),
            ..outlander()
        };
        let place = destination(&target, Path::new("Outlander.S08E01.1080p.x265.mkv"), None).unwrap();
        assert!(place.starts_with("Series/Outlander Folder/Season 08/"));
    }

    #[test]
    fn films_go_into_a_title_and_year_folder() {
        let target = Target {
            kind: Kind::Movie,
            tmdb: 693134,
            imdb: None,
            title: "Dune: Part Two".to_string(),
            original: None,
            year: Some(2024),
            season: None,
            episode: None,
            runtime: Some(167),
            episodes: Vec::new(),
            folder: None,
        };
        let file = Path::new("Dune.Part.Two.2024.1080p.WEB-DL.DDP5.1.Atmos.H.265-FLUX.mkv");
        assert_eq!(
            destination(&target, file, None).unwrap(),
            "Movies/Dune꞉ Part Two (2024)/Dune꞉ Part Two (2024) (1080p WEB-DL DDP5.1 Atmos H 265-FLUX).mkv"
        );
    }

    #[test]
    fn the_importer_reads_the_canonical_names_back() {
        let paths = [
            "Series/Outlander/Season 08/Outlander (2014) - S08E01 - Hello꞉ Goodbye (1080p AMZN WEB-DL x265 10bit EAC3 5.1 Silence).mkv",
            "Movies/Dune꞉ Part Two (2024)/Dune꞉ Part Two (2024) (1080p WEB-DL DDP5.1 Atmos H 265-FLUX).mkv",
        ];
        let entries: Vec<FileEntry> = paths
            .iter()
            .map(|path| FileEntry {
                path: path.into(),
                size: 1,
                modified: Date::default(),
            })
            .collect();
        let parsed = parse::build(&entries);
        let show = &parsed.shows[0];
        assert_eq!((show.title.as_str(), show.year), ("Outlander", Some(2014)));
        assert_eq!(
            (show.episodes[0].season, show.episodes[0].number),
            (8, 1)
        );
        assert_eq!(show.episodes[0].title.as_deref(), Some("Hello: Goodbye"));
        let movie = &parsed.movies[0];
        assert_eq!((movie.title.as_str(), movie.year), ("Dune: Part Two", Some(2024)));
        let tags = crate::tags::from_path(paths[0]);
        assert_eq!(tags.video.as_deref(), Some("HEVC 10-bit"));
        assert_eq!(tags.resolution.as_deref(), Some("1080p"));
    }
}
