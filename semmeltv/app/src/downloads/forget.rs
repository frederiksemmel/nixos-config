use super::deliver::{refresh, run};
use crate::artwork;
use crate::library::{movie_key, show_key};
use crate::manifest::{self, Manifest};
use crate::storage::Title;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const ATTEMPTS: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    pub path: String,
    pub size: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Space {
    pub total: u64,
    pub used: u64,
    pub free: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Held {
    #[serde(flatten)]
    pub title: Title,
    pub name: String,
    pub videos: usize,
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Doomed {
    pub name: String,
    pub videos: usize,
    pub files: Vec<String>,
    pub bytes: u64,
    pub folders: Vec<String>,
}

pub trait Pcloud {
    fn manifest_hash(&self) -> Result<String, String>;
    fn fetch_manifest(&self, to: &Path) -> Result<(), String>;
    fn put_manifest(&self, from: &Path) -> Result<(), String>;
    fn files(&self) -> Result<Vec<Remote>, String>;
    fn delete(&self, files: &[String]) -> Result<(), String>;
    fn prune(&self, folder: &str) -> Result<(), String>;
}

pub fn space(about: &str) -> Result<Space, String> {
    serde_json::from_str(about).map_err(|error| format!("rclone about: {error}"))
}

pub fn listing(text: &str) -> Vec<Remote> {
    text.lines()
        .filter_map(|line| {
            let (size, path) = line.split_once(';')?;
            Some(Remote {
                path: path.to_string(),
                size: size.parse().ok()?,
            })
        })
        .collect()
}

struct Entry<'a> {
    title: Title,
    name: String,
    videos: Vec<&'a str>,
    art: Option<String>,
}

fn entries(manifest: &Manifest) -> Vec<Entry<'_>> {
    let shows = manifest.show.iter().map(|show| Entry {
        title: Title::Series(show_key(&show.title)),
        name: show.title.clone(),
        videos: show.episodes.iter().map(|episode| episode.path.as_str()).collect(),
        art: show.tmdb.map(|tmdb| format!("{}/tv/{tmdb}", artwork::DIR)),
    });
    let movies = manifest.movie.iter().map(|movie| Entry {
        title: Title::Movie(movie_key(&movie.title, movie.year)),
        name: match movie.year {
            Some(year) => format!("{} ({year})", movie.title),
            None => movie.title.clone(),
        },
        videos: vec![movie.path.as_str()],
        art: movie.tmdb.map(|tmdb| format!("{}/movie/{tmdb}", artwork::DIR)),
    });
    shows.chain(movies).collect()
}

fn own_folder<'a>(video: &'a str, others: &[&str]) -> Option<&'a str> {
    video
        .match_indices('/')
        .skip(1)
        .map(|(at, _)| &video[..at])
        .find(|folder| !others.iter().any(|other| inside(other, folder)))
}

fn inside(path: &str, folder: &str) -> bool {
    path.strip_prefix(folder).is_some_and(|rest| rest.starts_with('/'))
}

fn companion(path: &str, video: &str) -> bool {
    let (Some((folder, name)), Some((video_folder, video_name))) = (path.rsplit_once('/'), video.rsplit_once('/'))
    else {
        return false;
    };
    let stem = video_name.rsplit_once('.').map_or(video_name, |(stem, _)| stem);
    folder == video_folder
        && name.strip_prefix(stem).is_some_and(|rest| rest.starts_with('.'))
        && !crate::parse::is_video(Path::new(name))
}

pub fn doomed(manifest: &Manifest, title: &Title, files: &[Remote]) -> Option<Doomed> {
    let all = entries(manifest);
    let (ours, others): (Vec<&Entry>, Vec<&Entry>) = all.iter().partition(|entry| entry.title == *title);
    let first = ours.first()?;
    let videos: Vec<&str> = ours.iter().flat_map(|entry| entry.videos.iter().copied()).collect();
    let other_videos: Vec<&str> = others.iter().flat_map(|entry| entry.videos.iter().copied()).collect();
    let mut wholes: BTreeSet<String> = BTreeSet::new();
    let mut folders: BTreeSet<String> = BTreeSet::new();
    let mut singles: Vec<&str> = Vec::new();
    for video in &videos {
        match own_folder(video, &other_videos) {
            Some(folder) => {
                wholes.insert(folder.to_string());
            }
            None => {
                singles.push(video);
                if let Some((parent, _)) = video.rsplit_once('/')
                    && parent.contains('/')
                {
                    folders.insert(parent.to_string());
                }
            }
        }
    }
    for art in ours.iter().filter_map(|entry| entry.art.as_deref()) {
        if !others.iter().any(|other| other.art.as_deref() == Some(art)) {
            wholes.insert(art.to_string());
        }
    }
    let gone: Vec<&Remote> = files
        .iter()
        .filter(|file| {
            wholes.iter().any(|folder| inside(&file.path, folder))
                || singles
                    .iter()
                    .any(|video| file.path == *video || companion(&file.path, video))
        })
        .collect();
    let mut paths: Vec<String> = gone.iter().map(|file| file.path.clone()).collect();
    paths.sort();
    paths.dedup();
    folders.extend(wholes);
    Some(Doomed {
        name: first.name.clone(),
        videos: videos.len(),
        files: paths,
        bytes: gone.iter().map(|file| file.size).sum(),
        folders: folders.into_iter().collect(),
    })
}

pub fn holdings(manifest: &Manifest, files: &[Remote]) -> Vec<Held> {
    let mut seen: BTreeSet<Title> = BTreeSet::new();
    let mut held: Vec<Held> = entries(manifest)
        .into_iter()
        .filter(|entry| seen.insert(entry.title.clone()))
        .filter_map(|entry| {
            let doomed = doomed(manifest, &entry.title, files)?;
            Some(Held {
                title: entry.title,
                name: doomed.name,
                videos: doomed.videos,
                bytes: doomed.bytes,
            })
        })
        .collect();
    held.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
    held
}

fn without(manifest: &mut Manifest, title: &Title) {
    match title {
        Title::Movie(key) => manifest
            .movie
            .retain(|movie| movie_key(&movie.title, movie.year) != *key),
        Title::Series(key) => manifest.show.retain(|show| show_key(&show.title) != *key),
        Title::Other(_) => {}
    }
}

pub fn forget(pcloud: &dyn Pcloud, work: &Path, title: &Title, log: &dyn Fn(String)) -> Result<String, String> {
    if matches!(title, Title::Other(_)) {
        return Err("Only a whole film or series can be deleted".to_string());
    }
    for attempt in 1..=ATTEMPTS {
        let before = pcloud.manifest_hash()?;
        let _ = fs::remove_dir_all(work);
        fs::create_dir_all(work).map_err(|error| format!("{}: {error}", work.display()))?;
        let path = work.join(manifest::FILE_NAME);
        pcloud.fetch_manifest(&path)?;
        let mut library = Manifest::read(&path).map_err(|_| "library.db on pCloud is unreadable".to_string())?;
        let files = pcloud.files()?;
        let doomed = doomed(&library, title, &files).ok_or("That title is no longer in the library")?;
        without(&mut library, title);
        library
            .write(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if pcloud.manifest_hash()? != before {
            log(format!(
                "library.db changed on pCloud while deleting {} (attempt {attempt}), starting over",
                doomed.name
            ));
            continue;
        }
        pcloud.put_manifest(&path)?;
        log(format!("took {} out of library.db", doomed.name));
        pcloud.delete(&doomed.files).map_err(|error| {
            format!(
                "{} is out of the library, but its files are still on pCloud: {error}",
                doomed.name
            )
        })?;
        log(format!(
            "moved {} files of {} to pCloud's trash: {}",
            doomed.files.len(),
            doomed.name,
            doomed.files.join(", ")
        ));
        for folder in &doomed.folders {
            if let Err(error) = pcloud.prune(folder) {
                log(format!("could not remove the empty folder {folder}: {error}"));
            }
        }
        return Ok(format!(
            "Deleted {} from the library; its {} went to pCloud's trash",
            doomed.name,
            crate::storage::size(doomed.bytes)
        ));
    }
    Err(format!("library.db kept changing on pCloud; tried {ATTEMPTS} times"))
}

pub struct Rclone {
    pub remote: String,
    pub work: PathBuf,
    pub mount_rc: PathBuf,
    pub sync_unit: String,
}

impl Rclone {
    fn at(&self, path: &str) -> String {
        format!("{}/{path}", self.remote)
    }

    pub fn space(&self) -> Result<Space, String> {
        space(&run("rclone", &["about", "--json", &self.remote])?)
    }

    pub fn announce(&self, log: &dyn Fn(String)) {
        for dir in ["Movies", "Series"] {
            if let Err(error) = refresh(&self.mount_rc, dir) {
                log(format!("could not refresh {dir} on the mount: {error}"));
            }
        }
        if let Err(error) = run("systemctl", &["start", "--no-block", &self.sync_unit]) {
            log(format!("could not start {}: {error}", self.sync_unit));
        }
    }
}

impl Pcloud for Rclone {
    fn manifest_hash(&self) -> Result<String, String> {
        let include = format!("/{}", manifest::FILE_NAME);
        let listing = run("rclone", &["hashsum", "sha1", &self.remote, "--include", &include])?;
        listing
            .split_whitespace()
            .next()
            .map(str::to_string)
            .ok_or_else(|| format!("no library.db in {}", self.remote))
    }

    fn fetch_manifest(&self, to: &Path) -> Result<(), String> {
        run(
            "rclone",
            &["copyto", &self.at(manifest::FILE_NAME), &to.to_string_lossy()],
        )
        .map(|_| ())
    }

    fn put_manifest(&self, from: &Path) -> Result<(), String> {
        run(
            "rclone",
            &["copyto", &from.to_string_lossy(), &self.at(manifest::FILE_NAME)],
        )
        .map(|_| ())
    }

    fn files(&self) -> Result<Vec<Remote>, String> {
        let text = run(
            "rclone",
            &[
                "lsf",
                "-R",
                "--fast-list",
                "--files-only",
                "--format",
                "sp",
                &self.remote,
            ],
        )?;
        Ok(listing(&text))
    }

    fn delete(&self, files: &[String]) -> Result<(), String> {
        if files.is_empty() {
            return Ok(());
        }
        fs::create_dir_all(&self.work).map_err(|error| format!("{}: {error}", self.work.display()))?;
        let list = self.work.join("delete.txt");
        fs::write(&list, files.join("\n") + "\n").map_err(|error| format!("{}: {error}", list.display()))?;
        run(
            "rclone",
            &["delete", &self.remote, "--files-from-raw", &list.to_string_lossy()],
        )
        .map(|_| ())
    }

    fn prune(&self, folder: &str) -> Result<(), String> {
        run("rclone", &["rmdirs", &self.at(folder)]).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    const LIBRARY: &str = r#"
[[show]]
title = "Dark"
year = 2017
tmdb = 70523
episodes = [
  { season = 1, episode = 1, path = "Series/Dark/Season 01/Dark - S01E01.mkv", added = 2024-05-01 },
  { season = 1, episode = 2, path = "Series/Dark/Season 01/Dark - S01E02.mkv", added = 2024-05-01 },
]

[[movie]]
title = "Up"
year = 2009
tmdb = 14160
path = "Movies/Up (2009)/Up (2009).mkv"
added = 2024-05-01

[[movie]]
title = "Wind"
path = "Movies/Shorts/Wind.avi"
added = 2024-05-01

[[movie]]
title = "Rain"
tmdb = 14160
path = "Movies/Shorts/Rain.avi"
added = 2024-05-01
"#;

    fn files() -> Vec<Remote> {
        [
            ("Series/Dark/Season 01/Dark - S01E01.mkv", 400),
            ("Series/Dark/Season 01/Dark - S01E02.mkv", 500),
            ("Series/Dark/Season 01/Dark - S01E02.en.srt", 1),
            ("Series/Dark/poster.jpg", 2),
            ("Movies/Up (2009)/Up (2009).mkv", 900),
            ("Movies/Up (2009)/Up (2009).de.srt", 3),
            ("Movies/Up (2009)/sample.mkv", 4),
            ("Movies/Shorts/Wind.avi", 70),
            ("Movies/Shorts/Wind.en.srt", 5),
            ("Movies/Shorts/Windy.avi", 6),
            ("Movies/Shorts/Rain.avi", 80),
            ("Artwork/tv/70523/poster.jpg", 7),
            ("Artwork/tv/70523/s01e01.jpg", 8),
            ("Artwork/movie/14160/poster.jpg", 9),
            ("Artwork/person/1.jpg", 10),
        ]
        .into_iter()
        .map(|(path, size)| Remote {
            path: path.to_string(),
            size,
        })
        .collect()
    }

    fn library() -> Manifest {
        Manifest::parse(LIBRARY).unwrap()
    }

    #[test]
    fn pcloud_says_how_much_room_it_has() {
        let about = r#"{"total": 2199023255552, "used": 825724082158, "free": 1373299173394}"#;
        let room = space(about).unwrap();
        assert_eq!(
            room,
            Space {
                total: 2_199_023_255_552,
                used: 825_724_082_158,
                free: 1_373_299_173_394,
            }
        );
        assert!(space("{}").is_err());
    }

    #[test]
    fn the_listing_reads_size_then_path_even_with_semicolons_in_names() {
        let read = listing("12;Movies/Up/Up.mkv\n3;Movies/A; B/A; B.srt\n\nnot a line\n");
        assert_eq!(
            read,
            [
                Remote {
                    path: "Movies/Up/Up.mkv".to_string(),
                    size: 12,
                },
                Remote {
                    path: "Movies/A; B/A; B.srt".to_string(),
                    size: 3,
                },
            ]
        );
    }

    #[test]
    fn a_series_takes_its_whole_folder_and_its_artwork() {
        let dark = doomed(&library(), &Title::Series(show_key("Dark")), &files()).unwrap();
        assert_eq!(dark.name, "Dark");
        assert_eq!(dark.videos, 2);
        assert_eq!(
            dark.files,
            [
                "Artwork/tv/70523/poster.jpg",
                "Artwork/tv/70523/s01e01.jpg",
                "Series/Dark/Season 01/Dark - S01E01.mkv",
                "Series/Dark/Season 01/Dark - S01E02.en.srt",
                "Series/Dark/Season 01/Dark - S01E02.mkv",
                "Series/Dark/poster.jpg",
            ]
        );
        assert_eq!(dark.bytes, 400 + 500 + 1 + 2 + 7 + 8);
        assert_eq!(dark.folders, ["Artwork/tv/70523", "Series/Dark"]);
    }

    #[test]
    fn a_film_in_a_shared_folder_takes_only_its_own_files_and_shared_artwork_stays() {
        let up = doomed(&library(), &Title::Movie(movie_key("Up", Some(2009))), &files()).unwrap();
        assert_eq!(up.name, "Up (2009)");
        assert_eq!(
            up.files,
            [
                "Movies/Up (2009)/Up (2009).de.srt",
                "Movies/Up (2009)/Up (2009).mkv",
                "Movies/Up (2009)/sample.mkv",
            ]
        );
        assert_eq!(up.folders, ["Movies/Up (2009)"]);
        let wind = doomed(&library(), &Title::Movie(movie_key("Wind", None)), &files()).unwrap();
        assert_eq!(wind.files, ["Movies/Shorts/Wind.avi", "Movies/Shorts/Wind.en.srt"]);
        assert_eq!(wind.bytes, 75);
        assert_eq!(wind.folders, ["Movies/Shorts"]);
        assert_eq!(doomed(&library(), &Title::Movie("heat".to_string()), &files()), None);
        assert_eq!(
            doomed(&library(), &Title::Other("Movies/Up".to_string()), &files()),
            None
        );
    }

    #[test]
    fn a_film_in_a_collection_takes_its_own_subfolder() {
        let collection = Manifest::parse(
            r#"
[[movie]]
title = "One"
path = "Movies/Potter/1 One/One.mkv"
added = 2024-05-01

[[movie]]
title = "Two"
path = "Movies/Potter/2 Two/Two.mkv"
added = 2024-05-01
"#,
        )
        .unwrap();
        let files: Vec<Remote> = [
            "Movies/Potter/1 One/One.mkv",
            "Movies/Potter/1 One/Subs/2_English.srt",
            "Movies/Potter/1 One/RARBG.txt",
            "Movies/Potter/2 Two/Two.mkv",
            "Movies/Potter/cover.jpg",
        ]
        .into_iter()
        .map(|path| Remote {
            path: path.to_string(),
            size: 1,
        })
        .collect();
        let one = doomed(&collection, &Title::Movie(movie_key("One", None)), &files).unwrap();
        assert_eq!(
            one.files,
            [
                "Movies/Potter/1 One/One.mkv",
                "Movies/Potter/1 One/RARBG.txt",
                "Movies/Potter/1 One/Subs/2_English.srt",
            ]
        );
        assert_eq!(one.folders, ["Movies/Potter/1 One"]);
    }

    #[test]
    fn holdings_list_every_title_biggest_first() {
        let held = holdings(&library(), &files());
        let names: Vec<(&str, u64, usize)> = held
            .iter()
            .map(|held| (held.name.as_str(), held.bytes, held.videos))
            .collect();
        assert_eq!(
            names,
            [
                ("Dark", 918, 2),
                ("Up (2009)", 907, 1),
                ("Rain", 80, 1),
                ("Wind", 75, 1)
            ]
        );
        assert_eq!(held[0].title, Title::Series("dark".to_string()));
        let json = serde_json::to_value(&held[1]).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "kind": "movie", "key": "up-2009", "name": "Up (2009)", "videos": 1, "bytes": 907 })
        );
    }

    struct Fake {
        dir: PathBuf,
        version: Cell<u32>,
        hashes: Cell<u32>,
        meddle_at: Option<u32>,
        puts: Cell<u32>,
        deleted: RefCell<Vec<String>>,
        pruned: RefCell<Vec<String>>,
    }

    impl Fake {
        fn new(name: &str, meddle_at: Option<u32>) -> Self {
            let dir = std::env::temp_dir().join(format!("semmeltv-forget-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            library().write(&dir.join(manifest::FILE_NAME)).unwrap();
            Self {
                dir,
                version: Cell::new(0),
                hashes: Cell::new(0),
                meddle_at,
                puts: Cell::new(0),
                deleted: RefCell::new(Vec::new()),
                pruned: RefCell::new(Vec::new()),
            }
        }

        fn remote(&self) -> PathBuf {
            self.dir.join(manifest::FILE_NAME)
        }
    }

    impl Pcloud for Fake {
        fn manifest_hash(&self) -> Result<String, String> {
            self.hashes.set(self.hashes.get() + 1);
            if Some(self.hashes.get()) == self.meddle_at {
                self.version.set(self.version.get() + 1);
            }
            Ok(self.version.get().to_string())
        }

        fn fetch_manifest(&self, to: &Path) -> Result<(), String> {
            fs::copy(self.remote(), to)
                .map(|_| ())
                .map_err(|error| error.to_string())
        }

        fn put_manifest(&self, from: &Path) -> Result<(), String> {
            self.puts.set(self.puts.get() + 1);
            self.version.set(self.version.get() + 1);
            fs::copy(from, self.remote())
                .map(|_| ())
                .map_err(|error| error.to_string())
        }

        fn files(&self) -> Result<Vec<Remote>, String> {
            Ok(files())
        }

        fn delete(&self, files: &[String]) -> Result<(), String> {
            self.deleted.borrow_mut().extend(files.iter().cloned());
            Ok(())
        }

        fn prune(&self, folder: &str) -> Result<(), String> {
            self.pruned.borrow_mut().push(folder.to_string());
            Ok(())
        }
    }

    #[test]
    fn deleting_a_title_takes_it_out_of_library_db_then_deletes_its_files() {
        let fake = Fake::new("plain", None);
        let ids: Vec<Option<i64>> = Manifest::read(&fake.remote())
            .unwrap()
            .movie
            .iter()
            .map(|movie| movie.id)
            .collect();
        let said = forget(
            &fake,
            &fake.dir.join("work"),
            &Title::Movie(movie_key("Up", Some(2009))),
            &|_| {},
        )
        .unwrap();
        assert_eq!(
            said,
            "Deleted Up (2009) from the library; its 0.0 GB went to pCloud's trash"
        );
        let left = Manifest::read(&fake.remote()).unwrap();
        let titles: Vec<(&str, Option<i64>)> = left
            .movie
            .iter()
            .map(|movie| (movie.title.as_str(), movie.id))
            .collect();
        assert_eq!(titles, [("Wind", ids[1]), ("Rain", ids[2])]);
        assert_eq!(left.show.len(), 1);
        assert_eq!(fake.puts.get(), 1);
        assert_eq!(fake.deleted.borrow().len(), 3);
        assert_eq!(*fake.pruned.borrow(), ["Movies/Up (2009)"]);
        fs::remove_dir_all(&fake.dir).unwrap();
    }

    #[test]
    fn a_library_db_changed_meanwhile_on_pcloud_is_read_again() {
        let fake = Fake::new("changed", Some(2));
        let mut lines = RefCell::new(Vec::new());
        let log = |line: String| lines.borrow_mut().push(line);
        forget(&fake, &fake.dir.join("work"), &Title::Series(show_key("Dark")), &log).unwrap();
        assert_eq!(fake.puts.get(), 1);
        assert!(lines.get_mut()[0].contains("changed on pCloud"));
        assert!(Manifest::read(&fake.remote()).unwrap().show.is_empty());
        fs::remove_dir_all(&fake.dir).unwrap();
    }

    #[test]
    fn nothing_is_deleted_for_a_title_the_library_lacks() {
        let fake = Fake::new("lacking", None);
        let error = forget(
            &fake,
            &fake.dir.join("work"),
            &Title::Movie("heat".to_string()),
            &|_| {},
        )
        .unwrap_err();
        assert_eq!(error, "That title is no longer in the library");
        let refused = forget(
            &fake,
            &fake.dir.join("work"),
            &Title::Other("Movies/Up".to_string()),
            &|_| {},
        );
        assert_eq!(refused.unwrap_err(), "Only a whole film or series can be deleted");
        assert_eq!(fake.puts.get(), 0);
        assert!(fake.deleted.borrow().is_empty());
        fs::remove_dir_all(&fake.dir).unwrap();
    }
}
