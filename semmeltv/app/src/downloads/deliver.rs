use super::release::{Coverage, Release};
use super::resolve::{Mode, Target};
use crate::artwork;
use crate::import::{self, Identity, Query};
use crate::library::normalize;
use crate::manifest::{self, Date, Manifest};
use crate::mdblist;
use crate::parse::{self, FileEntry};
use crate::tmdb::{Client, Kind};
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const ATTEMPTS: usize = 3;
pub const REPLACED: &str = "Replaced";

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
    replaced
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches('.')
        .to_string()
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

fn swap(path: &mut String, facts: [&mut Option<String>; 3], new: &str) -> Option<String> {
    if path == new {
        return None;
    }
    for fact in facts {
        *fact = None;
    }
    Some(std::mem::replace(path, new.to_string()))
}

pub fn repoint(manifest: &mut Manifest, target: &Target, placed: &[Placed]) -> Vec<String> {
    let mut old = Vec::new();
    match target.kind {
        Kind::Movie => {
            let movie = manifest.movie.iter_mut().find(|movie| movie.tmdb == Some(target.tmdb));
            if let (Some(movie), Some(place)) = (movie, placed.first()) {
                let facts = [&mut movie.video, &mut movie.resolution, &mut movie.audio];
                old.extend(swap(&mut movie.path, facts, &place.destination));
            }
        }
        Kind::Tv => {
            let Some(show) = manifest.show.iter_mut().find(|show| show.tmdb == Some(target.tmdb)) else {
                return old;
            };
            for place in placed {
                let Some(number) = episode_of(Path::new(&place.destination)) else {
                    continue;
                };
                let found = show
                    .episodes
                    .iter_mut()
                    .find(|episode| (episode.season, episode.episode) == number);
                if let Some(episode) = found {
                    let facts = [&mut episode.video, &mut episode.resolution, &mut episode.audio];
                    old.extend(swap(&mut episode.path, facts, &place.destination));
                }
            }
        }
    }
    old
}

pub fn companions(video: &str, files: &[FileEntry]) -> Vec<String> {
    let path = Path::new(video);
    let (Some(folder), Some(stem)) = (path.parent(), path.file_stem()) else {
        return Vec::new();
    };
    let prefix = format!("{}.", stem.to_string_lossy());
    files
        .iter()
        .filter(|file| file.path.parent() == Some(folder) && !parse::is_video(&file.path))
        .filter(|file| {
            file.path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(&prefix))
        })
        .filter_map(|file| file.path.to_str().map(str::to_string))
        .collect()
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

fn remote_size(remote: &str, path: &str) -> Result<Option<u64>, String> {
    let listing = run("rclone", &["lsjson", "--files-only", &format!("{remote}/{path}")])?;
    let entries: Vec<serde_json::Value> = serde_json::from_str(&listing).unwrap_or_default();
    Ok(entries.first().and_then(|entry| entry["Size"].as_u64()))
}

fn sent(line: &str) -> Option<u64> {
    let entry: serde_json::Value = serde_json::from_str(line).ok()?;
    entry["stats"]["bytes"].as_u64()
}

fn complaint(line: &str) -> Option<String> {
    let entry: serde_json::Value = serde_json::from_str(line).ok()?;
    matches!(entry["level"].as_str(), Some("error" | "critical"))
        .then(|| entry["msg"].as_str().unwrap_or_default().trim().to_string())
}

fn copy_with_progress(local: &str, remote: &str, progress: &dyn Fn(u64)) -> Result<(), String> {
    let mut child = Command::new("rclone")
        .args([
            "copyto",
            local,
            remote,
            "--use-json-log",
            "--stats",
            "5s",
            "--stats-log-level",
            "NOTICE",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("rclone: {error}"))?;
    let mut last = String::new();
    if let Some(stderr) = child.stderr.take() {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Some(bytes) = sent(&line) {
                progress(bytes);
            } else if let Some(said) = complaint(&line) {
                last = said;
            } else if !line.trim().is_empty() && !line.starts_with('{') {
                last = line;
            }
        }
    }
    let status = child.wait().map_err(|error| format!("rclone: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("rclone copyto: {last}"))
    }
}

pub fn upload_to(remote: &str, placed: &Placed, progress: &dyn Fn(u64)) -> Result<(), String> {
    copy_with_progress(
        &placed.local.to_string_lossy(),
        &format!("{remote}/{}", placed.destination),
        progress,
    )?;
    match remote_size(remote, &placed.destination)? {
        Some(size) if size == placed.size => Ok(()),
        Some(size) => Err(format!(
            "{} is {size} bytes on pCloud, {} here",
            placed.destination, placed.size
        )),
        None => Err(format!("{} is missing on pCloud after the upload", placed.destination)),
    }
}

pub fn refresh(mount_rc: &Path, dir: &str) -> Result<(), String> {
    let socket = mount_rc.to_string_lossy();
    let argument = format!("dir={dir}");
    run("rclone", &["rc", "--unix-socket", &socket, "vfs/refresh", &argument]).map(|_| ())
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
            &[
                "hashsum",
                "sha1",
                &self.remote,
                "--include",
                &format!("/{}", manifest::FILE_NAME),
            ],
        )?;
        listing
            .split_whitespace()
            .next()
            .map(str::to_string)
            .ok_or_else(|| format!("no library.db in {}", self.remote))
    }

    pub fn upload(&self, placed: &Placed, progress: &dyn Fn(u64)) -> Result<(), String> {
        upload_to(&self.remote, placed, progress)
    }

    pub fn import(&self, target: &Target, placed: &[Placed], mode: Mode, log: &dyn Fn(String)) -> Result<(), String> {
        for attempt in 1..=ATTEMPTS {
            let before = self.manifest_hash()?;
            let (manifest, retired) = self.prepare(target, placed, mode, log)?;
            let after = self.manifest_hash()?;
            if after != before {
                log(format!(
                    "library.db changed on pCloud while importing (attempt {attempt}), starting over"
                ));
                continue;
            }
            run(
                "rclone",
                &[
                    "copy",
                    &self.work.join(artwork::DIR).to_string_lossy(),
                    &self.remote(artwork::DIR),
                ],
            )?;
            if self.manifest_hash()? != before {
                log(format!(
                    "library.db changed on pCloud before the upload (attempt {attempt}), starting over"
                ));
                continue;
            }
            run(
                "rclone",
                &["copyto", &manifest.to_string_lossy(), &self.remote(manifest::FILE_NAME)],
            )?;
            log("library.db uploaded".to_string());
            self.retire(&retired, log);
            return Ok(());
        }
        Err(format!("library.db kept changing on pCloud; tried {ATTEMPTS} times"))
    }

    fn retire(&self, retired: &[String], log: &dyn Fn(String)) {
        for path in retired {
            let kept = format!("{REPLACED}/{path}");
            match run("rclone", &["moveto", &self.remote(path), &self.remote(&kept)]) {
                Ok(_) => log(format!("moved the replaced {path} to {kept}")),
                Err(error) => log(format!("could not move the replaced {path} aside: {error}")),
            }
        }
    }

    fn prepare(
        &self,
        target: &Target,
        placed: &[Placed],
        mode: Mode,
        log: &dyn Fn(String),
    ) -> Result<(PathBuf, Vec<String>), String> {
        let _ = fs::remove_dir_all(&self.work);
        fs::create_dir_all(&self.work).map_err(|error| format!("{}: {error}", self.work.display()))?;
        let manifest_path = self.work.join(manifest::FILE_NAME);
        run(
            "rclone",
            &[
                "copyto",
                &self.remote(manifest::FILE_NAME),
                &manifest_path.to_string_lossy(),
            ],
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
        let mut existing =
            Manifest::read(&manifest_path).map_err(|_| "library.db on pCloud is unreadable".to_string())?;
        let retired: Vec<String> = match mode {
            Mode::Add => Vec::new(),
            Mode::Replace => repoint(&mut existing, target, placed)
                .into_iter()
                .flat_map(|video| {
                    log(format!("replaces {video}"));
                    let companions = companions(&video, &files);
                    std::iter::once(video).chain(companions)
                })
                .collect(),
        };
        let files: Vec<FileEntry> = files
            .into_iter()
            .filter(|file| !retired.iter().any(|path| file.path == Path::new(path)))
            .collect();
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
        Ok((manifest_path, retired))
    }

    pub fn show_in_library(&self, placed: &[Placed], log: &dyn Fn(String)) {
        for dir in folders(placed) {
            match refresh(&self.mount_rc, &dir) {
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

    #[test]
    fn rclone_stats_lines_give_the_bytes_sent_and_error_lines_the_reason() {
        let stats = r#"{"time":"2026-11-17T07:37:52+01:00","level":"notice","msg":"\nTransferred: 5.996 MiB / 19.073 MiB, 31%\n","stats":{"bytes":6287360,"totalBytes":20000000,"transferring":[{"bytes":6287360,"name":"a.mkv","size":20000000}]},"source":"accounting/stats.go:554"}"#;
        assert_eq!(sent(stats), Some(6_287_360));
        assert_eq!(complaint(stats), None);
        let failed = r#"{"time":"2026-11-17T07:38:00+01:00","level":"error","msg":"Failed to copyto: quota exceeded\n","source":"cmd/cmd.go:1"}"#;
        assert_eq!(sent(failed), None);
        assert_eq!(complaint(failed).as_deref(), Some("Failed to copyto: quota exceeded"));
        assert_eq!(sent("2026/11/17 07:38:00 NOTICE: plain text"), None);
    }

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
    fn files_go_to_their_canonical_place() {
        let dune = Target {
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
        let known = Target {
            folder: Some("Outlander Folder".to_string()),
            ..outlander()
        };
        let cases = [
            (
                outlander(),
                "Outlander (2014) S08 (1080p)/Outlander (2014) - S08E01 (1080p AMZN WEB-DL x265 10bit EAC3 5.1 Silence).mkv",
                Some("1080p AMZN WEB-DL x265 10bit EAC3 5.1 Silence"),
                "Series/Outlander/Season 08/Outlander (2014) - S08E01 - Hello꞉ Goodbye (1080p AMZN WEB-DL x265 10bit EAC3 5.1 Silence).mkv",
            ),
            (
                outlander(),
                "Outlander.S08E02.1080p.WEB.h264-GRACE.mkv",
                None,
                "Series/Outlander/Season 08/Outlander (2014) - S08E02 (1080p WEB h264-GRACE).mkv",
            ),
            (
                known,
                "Outlander.S08E01.1080p.x265.mkv",
                None,
                "Series/Outlander Folder/Season 08/Outlander (2014) - S08E01 - Hello꞉ Goodbye (1080p x265).mkv",
            ),
            (
                dune,
                "Dune.Part.Two.2024.1080p.WEB-DL.DDP5.1.Atmos.H.265-FLUX.mkv",
                None,
                "Movies/Dune꞉ Part Two (2024)/Dune꞉ Part Two (2024) (1080p WEB-DL DDP5.1 Atmos H 265-FLUX).mkv",
            ),
        ];
        for (target, file, release, want) in cases {
            assert_eq!(destination(&target, Path::new(file), release).unwrap(), want, "{file}");
        }
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
        assert_eq!((show.episodes[0].season, show.episodes[0].number), (8, 1));
        assert_eq!(show.episodes[0].title.as_deref(), Some("Hello: Goodbye"));
        let movie = &parsed.movies[0];
        assert_eq!((movie.title.as_str(), movie.year), ("Dune: Part Two", Some(2024)));
        let tags = crate::tags::from_path(paths[0]);
        assert_eq!(tags.video.as_deref(), Some("HEVC 10-bit"));
        assert_eq!(tags.resolution.as_deref(), Some("1080p"));
    }

    #[test]
    fn a_replacement_takes_over_the_entry_and_its_id_and_drops_the_old_facts() {
        let mut manifest = Manifest::parse(
            r#"
[[movie]]
title = "View from a Blue Moon"
year = 2015
tmdb = 368239
path = "Movies/View From A Blue Moon (2015)/View.From.A.Blue.Moon.2015.1080p.BluRay.x264-[YTS.AG].mp4"
added = 2021-10-01
video = "H.264"
resolution = "1080p"

[[show]]
title = "Outlander"
year = 2014
tmdb = 56570
[[show.episodes]]
season = 8
episode = 1
path = "Series/Outlander/Season 08/Outlander.S08E01.720p.mkv"
added = 2025-01-01
resolution = "720p"
"#,
        )
        .unwrap();
        manifest.movie[0].id = Some(79);
        manifest.show[0].episodes[0].id = Some(7);
        let place = |destination: &str| Placed {
            local: PathBuf::new(),
            destination: destination.to_string(),
            size: 1,
        };
        let film = Target {
            kind: Kind::Movie,
            tmdb: 368239,
            title: "View from a Blue Moon".to_string(),
            year: Some(2015),
            ..outlander()
        };
        let new_film = "Movies/View from a Blue Moon (2015)/View from a Blue Moon (2015) (1080p BluRay x264-OFT).mkv";
        assert_eq!(
            repoint(&mut manifest, &film, &[place(new_film)]),
            ["Movies/View From A Blue Moon (2015)/View.From.A.Blue.Moon.2015.1080p.BluRay.x264-[YTS.AG].mp4"]
        );
        let movie = &manifest.movie[0];
        assert_eq!(
            (movie.id, movie.path.as_str(), &movie.video),
            (Some(79), new_film, &None)
        );
        assert!(repoint(&mut manifest, &film, &[place(new_film)]).is_empty());

        let new_episodes = [
            place("Series/Outlander/Season 08/Outlander (2014) - S08E01 (1080p).mkv"),
            place("Series/Outlander/Season 08/Outlander (2014) - S08E02 (1080p).mkv"),
        ];
        assert_eq!(
            repoint(&mut manifest, &outlander(), &new_episodes),
            ["Series/Outlander/Season 08/Outlander.S08E01.720p.mkv"]
        );
        let episode = &manifest.show[0].episodes[0];
        assert_eq!(
            (episode.id, episode.path.as_str(), &episode.resolution),
            (Some(7), new_episodes[0].destination.as_str(), &None)
        );
    }

    #[test]
    fn subtitles_named_after_a_replaced_video_go_with_it() {
        let entries: Vec<FileEntry> = [
            "Movies/Up (2009)/Up.2009.mp4",
            "Movies/Up (2009)/Up.2009.en.srt",
            "Movies/Up (2009)/Up.2009.de.srt",
            "Movies/Up (2009)/WWW.YTS.AG.jpg",
            "Movies/Up (2009)/Up.2009.Extended.mp4",
            "Movies/Other/Up.2009.en.srt",
        ]
        .iter()
        .map(|path| FileEntry {
            path: path.into(),
            size: 1,
            modified: Date::default(),
        })
        .collect();
        assert_eq!(
            companions("Movies/Up (2009)/Up.2009.mp4", &entries),
            ["Movies/Up (2009)/Up.2009.en.srt", "Movies/Up (2009)/Up.2009.de.srt"]
        );
    }
}
