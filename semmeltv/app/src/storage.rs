use crate::api::{self, Ask, Request};
use crate::library::{Item, Library};
use crate::offline;
use crate::progress::Progress;
use crate::watching;
use axum::extract::State;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::CString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::UNIX_EPOCH;

const GIB: u64 = 1 << 30;
pub const FLOOR: u64 = 10 * GIB;
const ST_BLOCK_BYTES: u64 = 512;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "key", rename_all = "lowercase")]
pub enum Title {
    Movie(String),
    Series(String),
    Other(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Keep {
    Playing,
    Watching,
    UpNext,
    Started,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Marks {
    pub high: u64,
    pub low: u64,
    pub floor: u64,
}

impl Marks {
    pub fn new(high: u64) -> Self {
        Self {
            high,
            low: high / 10 * 9,
            floor: FLOOR,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Disk {
    pub total: u64,
    pub free: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stored {
    pub path: PathBuf,
    pub size: u64,
    pub bytes: u64,
    pub cached_at: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Cached {
    #[serde(flatten)]
    pub title: Title,
    pub name: String,
    pub bytes: u64,
    pub complete: usize,
    pub total: usize,
    pub last_watched: Option<u64>,
    pub cached_at: u64,
    pub keep: Option<Keep>,
    #[serde(skip)]
    pub files: Vec<Stored>,
}

impl Cached {
    pub fn age(&self) -> u64 {
        self.last_watched.unwrap_or(self.cached_at)
    }

    pub fn describe(&self, now: u64) -> String {
        let (kind, key) = match &self.title {
            Title::Movie(key) => ("movie", key),
            Title::Series(key) => ("series", key),
            Title::Other(path) => ("not in the library", path),
        };
        let days = now.saturating_sub(self.age()) / 86_400;
        let when = match self.last_watched {
            Some(_) => format!("last watched {days} days ago"),
            None => format!("downloaded {days} days ago, never watched"),
        };
        format!("{} ({kind} {key}, {when})", self.name)
    }
}

pub struct Activity<'a> {
    pub playing: Option<&'a Path>,
    pub queued: &'a [PathBuf],
}

pub fn excess(used: u64, free: u64, marks: Marks) -> u64 {
    if used <= marks.high && free >= marks.floor {
        return 0;
    }
    let size = used.saturating_sub(marks.low);
    let space = (marks.floor + (marks.high - marks.low)).saturating_sub(free);
    size.max(space)
}

pub fn order(titles: &[Cached]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..titles.len())
        .filter(|&index| titles[index].keep.is_none() && titles[index].bytes > 0)
        .collect();
    order.sort_by_key(|&index| {
        let title = &titles[index];
        (
            !matches!(title.title, Title::Other(_)),
            title.age(),
            title.title.clone(),
        )
    });
    order
}

pub fn plan(titles: &[Cached], used: u64, free: u64, marks: Marks) -> Vec<usize> {
    let mut need = excess(used, free, marks);
    let mut chosen = Vec::new();
    for index in order(titles) {
        if need == 0 {
            break;
        }
        chosen.push(index);
        need = need.saturating_sub(titles[index].bytes);
    }
    chosen
}

fn relative(root: &Path, path: &Path) -> Option<PathBuf> {
    path.strip_prefix(root).ok().map(Path::to_path_buf)
}

fn other(root: &Path, path: &Path) -> (Title, String) {
    let parts: Vec<String> = relative(root, path)
        .unwrap_or_else(|| path.to_path_buf())
        .components()
        .filter_map(|part| match part {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .take(2)
        .collect();
    let name = parts.last().cloned().unwrap_or_default();
    (Title::Other(parts.join("/")), name)
}

struct Owner {
    title: Title,
    name: String,
}

fn owners(library: &Library) -> HashMap<&Path, Owner> {
    let mut owners = HashMap::new();
    for show in &library.shows {
        for episode in show.episodes() {
            let owner = Owner {
                title: Title::Series(show.key.clone()),
                name: show.title.clone(),
            };
            owners.insert(episode.path.as_path(), owner);
        }
    }
    for movie in &library.movies {
        let owner = Owner {
            title: Title::Movie(movie.key.clone()),
            name: movie.title.clone(),
        };
        owners.insert(movie.path.as_path(), owner);
    }
    owners
}

fn folders(root: &Path, owners: &HashMap<&Path, Owner>) -> HashMap<PathBuf, Option<Title>> {
    let mut folders: HashMap<PathBuf, Option<Title>> = HashMap::new();
    for (path, owner) in owners {
        let Some(folder) = path.parent() else { continue };
        if relative(root, folder).is_none_or(|folder| folder.components().count() < 2) {
            continue;
        }
        folders
            .entry(folder.to_path_buf())
            .and_modify(|seen| {
                if seen.as_ref() != Some(&owner.title) {
                    *seen = None;
                }
            })
            .or_insert_with(|| Some(owner.title.clone()));
    }
    folders
}

fn title_of(library: &Library, item: Item) -> Title {
    match item {
        Item::Show(index) => Title::Series(library.shows[index].key.clone()),
        Item::Movie(index) => Title::Movie(library.movies[index].key.clone()),
    }
}

pub fn titles(
    root: &Path,
    library: &Library,
    progress: &Progress,
    stored: Vec<Stored>,
    activity: &Activity,
) -> Vec<Cached> {
    let owners = owners(library);
    let folders = folders(root, &owners);
    let names: HashMap<&Title, &str> = owners
        .values()
        .map(|owner| (&owner.title, owner.name.as_str()))
        .collect();
    let mut titles: Vec<Cached> = Vec::new();
    let mut index_of: HashMap<Title, usize> = HashMap::new();
    for file in stored {
        let (title, name) = match owners.get(file.path.as_path()) {
            Some(owner) => (owner.title.clone(), owner.name.clone()),
            None => match file
                .path
                .parent()
                .and_then(|folder| folders.get(folder))
                .cloned()
                .flatten()
            {
                Some(title) => {
                    let name = names.get(&title).copied().unwrap_or_default().to_string();
                    (title, name)
                }
                None => other(root, &file.path),
            },
        };
        let index = *index_of.entry(title.clone()).or_insert_with(|| {
            titles.push(Cached {
                title,
                name,
                bytes: 0,
                complete: 0,
                total: 0,
                last_watched: None,
                cached_at: 0,
                keep: None,
                files: Vec::new(),
            });
            titles.len() - 1
        });
        let cached = &mut titles[index];
        cached.bytes += file.bytes;
        cached.cached_at = cached.cached_at.max(file.cached_at);
        cached.files.push(file);
    }

    let keep = |title: Title, reason: Keep, titles: &mut Vec<Cached>| {
        if let Some(&index) = index_of.get(&title) {
            let cached = &mut titles[index];
            cached.keep = Some(cached.keep.map_or(reason, |kept| kept.min(reason)));
        }
    };
    for current in watching::current(library, progress) {
        let title = title_of(library, current.item);
        if let Some(&index) = index_of.get(&title) {
            let cached = &mut titles[index];
            cached.last_watched = cached.last_watched.max(Some(current.last_played));
        }
        keep(title, Keep::Watching, &mut titles);
    }
    for (path, owner) in &owners {
        let Some(&index) = index_of.get(&owner.title) else {
            continue;
        };
        titles[index].total += 1;
        let watched = progress.files.get(*path);
        if let Some(played) = watched.and_then(|watched| watched.last_played) {
            titles[index].last_watched = titles[index].last_watched.max(Some(played));
        }
        if watched.is_some_and(|watched| watched.stopped_at.is_some()) {
            keep(owner.title.clone(), Keep::Started, &mut titles);
        }
        let titles_now = &mut titles[index];
        if titles_now
            .files
            .iter()
            .any(|file| file.path == *path && offline::covers(file.size, file.bytes))
        {
            titles_now.complete += 1;
        }
    }
    for path in activity.queued {
        if let Some(owner) = owners.get(path.as_path()) {
            keep(owner.title.clone(), Keep::UpNext, &mut titles);
        }
    }
    if let Some(owner) = activity.playing.and_then(|path| owners.get(path)) {
        keep(owner.title.clone(), Keep::Playing, &mut titles);
    }
    for cached in &mut titles {
        for file in &cached.files {
            let reason = if activity.playing == Some(file.path.as_path()) {
                Keep::Playing
            } else if activity.queued.contains(&file.path) {
                Keep::UpNext
            } else if progress
                .files
                .get(&file.path)
                .is_some_and(|watched| watched.stopped_at.is_some())
            {
                Keep::Started
            } else {
                continue;
            };
            cached.keep = Some(cached.keep.map_or(reason, |kept| kept.min(reason)));
        }
    }
    titles
}

#[derive(Clone, Debug)]
pub struct Cache {
    pub library: PathBuf,
    pub manifest: PathBuf,
    pub data: PathBuf,
    pub meta: PathBuf,
    pub marks: Marks,
}

fn created(metadata: &fs::Metadata) -> u64 {
    metadata
        .created()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or_else(|| u64::try_from(metadata.ctime()).unwrap_or(0), |since| since.as_secs())
}

fn walk(dir: &Path, found: &mut Vec<(PathBuf, fs::Metadata)>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else { continue };
        if metadata.is_dir() {
            walk(&entry.path(), found);
        } else if metadata.is_file() {
            found.push((entry.path(), metadata));
        }
    }
}

impl Cache {
    pub fn scan(&self) -> Vec<Stored> {
        let mut found = Vec::new();
        walk(&self.data, &mut found);
        found
            .into_iter()
            .filter_map(|(path, metadata)| {
                Some(Stored {
                    path: self.library.join(path.strip_prefix(&self.data).ok()?),
                    size: metadata.len(),
                    bytes: metadata.blocks() * ST_BLOCK_BYTES,
                    cached_at: created(&metadata),
                })
            })
            .collect()
    }

    pub fn disk(&self) -> io::Result<Disk> {
        let path = CString::new(self.data.as_os_str().as_bytes()).map_err(io::Error::other)?;
        let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let stats = unsafe { stats.assume_init() };
        let unit = stats.f_frsize;
        Ok(Disk {
            total: stats.f_blocks * unit,
            free: stats.f_bavail * unit,
        })
    }

    pub fn remove(&self, files: &[Stored]) -> io::Result<()> {
        for file in files {
            let Some(relative) = relative(&self.library, &file.path) else {
                continue;
            };
            for path in [self.data.join(&relative), self.meta.join(&relative)] {
                match fs::remove_file(&path) {
                    Err(error) if error.kind() != io::ErrorKind::NotFound => {
                        return Err(io::Error::new(error.kind(), format!("{}: {error}", path.display())));
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Report {
    pub disk: Disk,
    pub used: u64,
    pub marks: Option<Marks>,
    pub titles: Vec<Cached>,
    pub next: Vec<Title>,
}

impl Report {
    pub fn new(disk: Disk, marks: Marks, mut titles: Vec<Cached>) -> Self {
        let used = titles.iter().map(|title| title.bytes).sum();
        titles.retain(|title| title.bytes > 0);
        let next = order(&titles)
            .into_iter()
            .map(|index| titles[index].title.clone())
            .collect();
        Self {
            disk,
            used,
            marks: Some(marks),
            titles,
            next,
        }
    }
}

#[derive(Clone, Default)]
pub struct Board(Arc<RwLock<Option<Report>>>);

impl Board {
    pub fn set(&self, report: Report) {
        *self.0.write().unwrap() = Some(report);
    }

    fn get(&self) -> Option<Report> {
        self.0.read().unwrap().clone()
    }
}

#[derive(Debug, PartialEq)]
pub enum Change {
    Remove(Title),
}

#[derive(Clone)]
struct Endpoint {
    board: Board,
    app: Ask,
}

pub fn routes<S: Clone + Send + Sync + 'static>(board: Board, app: Ask) -> Router<S> {
    Router::new()
        .route("/api/storage", get(show))
        .route("/api/storage/remove", post(remove))
        .with_state(Endpoint { board, app })
}

async fn show(State(endpoint): State<Endpoint>) -> Response {
    use axum::response::IntoResponse;
    match endpoint.board.get() {
        Some(report) => Json(report).into_response(),
        None => api::problem(
            axum::http::StatusCode::NOT_FOUND,
            "the TV has no offline cache".to_string(),
        ),
    }
}

async fn remove(State(endpoint): State<Endpoint>, Json(title): Json<Title>) -> Response {
    api::ask_app(&endpoint.app, Request::Storage(Change::Remove(title))).await
}

pub fn size(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{Episode, Movie, Season, Show};
    use crate::progress::{Place, Watched};

    const G: u64 = 1_000_000_000;

    fn cached(key: &str, bytes: u64, last_watched: Option<u64>, cached_at: u64) -> Cached {
        Cached {
            title: Title::Movie(key.to_string()),
            name: key.to_string(),
            bytes,
            complete: 1,
            total: 1,
            last_watched,
            cached_at,
            keep: None,
            files: Vec::new(),
        }
    }

    fn keys(titles: &[Cached], indices: &[usize]) -> Vec<String> {
        indices.iter().map(|&index| titles[index].name.clone()).collect()
    }

    const MARKS: Marks = Marks {
        high: 80 * G,
        low: 72 * G,
        floor: 10 * G,
    };

    #[test]
    fn nothing_goes_below_the_high_water_mark_with_room_on_the_disk() {
        assert_eq!(excess(80 * G, 30 * G, MARKS), 0);
        let titles = [cached("up", 5 * G, Some(1), 1)];
        assert!(plan(&titles, 80 * G, 30 * G, MARKS).is_empty());
    }

    #[test]
    fn past_the_high_water_mark_it_frees_down_to_the_low_one() {
        assert_eq!(excess(81 * G, 30 * G, MARKS), 9 * G);
        let titles = [
            cached("new", 6 * G, Some(300), 10),
            cached("old", 4 * G, Some(100), 10),
            cached("middle", 3 * G, Some(200), 10),
            cached("newest", 5 * G, Some(400), 10),
        ];
        assert_eq!(
            keys(&titles, &plan(&titles, 81 * G, 30 * G, MARKS)),
            ["old", "middle", "new"]
        );
    }

    #[test]
    fn a_full_disk_frees_space_even_under_the_high_water_mark() {
        assert_eq!(excess(50 * G, 9 * G, MARKS), 9 * G);
        assert_eq!(excess(75 * G, 9 * G, MARKS), 9 * G);
        let titles = [cached("old", 4 * G, Some(1), 1), cached("new", 6 * G, Some(2), 1)];
        assert_eq!(keys(&titles, &plan(&titles, 50 * G, 9 * G, MARKS)), ["old", "new"]);
    }

    #[test]
    fn titles_never_watched_count_from_when_they_were_cached() {
        let titles = [
            cached("watched-last-week", G, Some(700), 100),
            cached("cached-long-ago", G, None, 50),
            cached("cached-today", G, None, 900),
        ];
        assert_eq!(
            keys(&titles, &order(&titles)),
            ["cached-long-ago", "watched-last-week", "cached-today"]
        );
    }

    #[test]
    fn kept_titles_are_never_chosen_and_files_outside_the_library_go_first() {
        let mut titles = vec![
            cached("playing", 20 * G, Some(1), 1),
            cached("watching", 20 * G, Some(2), 1),
            cached("old", G, Some(3), 1),
            cached("stray", G, None, 999),
        ];
        titles[0].keep = Some(Keep::Playing);
        titles[1].keep = Some(Keep::Watching);
        titles[3].title = Title::Other("Movies/Extras".to_string());
        assert_eq!(keys(&titles, &plan(&titles, 100 * G, 30 * G, MARKS)), ["stray", "old"]);
    }

    #[test]
    fn the_low_water_mark_is_a_tenth_under_the_high_one() {
        let marks = Marks::new(80 * GIB);
        assert_eq!(marks.low, 72 * GIB);
        assert_eq!(marks.floor, 10 * GIB);
    }

    fn library() -> Library {
        let episode = |n: u32| Episode {
            season: 1,
            number: n,
            path: PathBuf::from(format!("/lib/Series/Dark/S01/E0{n}.mkv")),
            ..Episode::default()
        };
        let show = Show {
            key: "dark".to_string(),
            title: "Dark".to_string(),
            seasons: vec![Season {
                number: 1,
                episodes: (1..=3).map(episode).collect(),
            }],
            ..Show::default()
        };
        let movie = |key: &str, path: &str| Movie {
            key: key.to_string(),
            title: key.to_string(),
            path: PathBuf::from(path),
            ..Movie::default()
        };
        Library {
            shows: vec![show],
            movies: vec![
                movie("up", "/lib/Movies/Up (2009)/Up.mkv"),
                movie("heat", "/lib/Movies/Heat.mkv"),
            ],
        }
    }

    fn stored(path: &str, size: u64, bytes: u64, cached_at: u64) -> Stored {
        Stored {
            path: PathBuf::from(path),
            size,
            bytes,
            cached_at,
        }
    }

    fn find<'a>(titles: &'a [Cached], title: &Title) -> &'a Cached {
        titles.iter().find(|cached| cached.title == *title).unwrap()
    }

    #[test]
    fn cached_files_group_into_their_titles() {
        let files = vec![
            stored("/lib/Series/Dark/S01/E01.mkv", 100, 100, 5),
            stored("/lib/Series/Dark/S01/E02.mkv", 100, 40, 9),
            stored("/lib/Movies/Up (2009)/Up.mkv", 100, 100, 3),
            stored("/lib/Movies/Up (2009)/Up.srt", 10, 10, 3),
            stored("/lib/Movies/Gone (2001)/Gone.mkv", 50, 50, 2),
            stored("/lib/Movies/Loose.mkv", 50, 50, 2),
        ];
        let none = Activity {
            playing: None,
            queued: &[],
        };
        let titles = titles(Path::new("/lib"), &library(), &Progress::default(), files, &none);
        let dark = find(&titles, &Title::Series("dark".to_string()));
        assert_eq!((dark.bytes, dark.complete, dark.total, dark.cached_at), (140, 1, 3, 9));
        let up = find(&titles, &Title::Movie("up".to_string()));
        assert_eq!((up.bytes, up.complete, up.total), (110, 1, 1));
        let gone = find(&titles, &Title::Other("Movies/Gone (2001)".to_string()));
        assert_eq!((gone.name.as_str(), gone.bytes), ("Gone (2001)", 50));
        assert!(
            titles
                .iter()
                .any(|title| title.title == Title::Other("Movies/Loose.mkv".to_string()))
        );
        assert_eq!(titles.len(), 4);
    }

    #[test]
    fn playing_watching_up_next_and_started_titles_are_kept() {
        let library = library();
        let mut progress = Progress::default();
        let dark = PathBuf::from("/lib/Series/Dark/S01/E01.mkv");
        let up = PathBuf::from("/lib/Movies/Up (2009)/Up.mkv");
        let heat = PathBuf::from("/lib/Movies/Heat.mkv");
        progress.record(
            &up,
            Place {
                position: 600.0,
                duration: 6000.0,
                credits: None,
            },
        );
        progress.played(&up, 777);
        progress.files.insert(
            heat.clone(),
            Watched {
                last_played: Some(555),
                ..Watched::default()
            },
        );
        let files = vec![
            stored("/lib/Series/Dark/S01/E01.mkv", 10, 10, 1),
            stored("/lib/Movies/Up (2009)/Up.mkv", 10, 10, 1),
            stored("/lib/Movies/Heat.mkv", 10, 10, 1),
        ];
        let queued = [dark.clone()];
        let activity = Activity {
            playing: None,
            queued: &queued,
        };
        let titles = titles(Path::new("/lib"), &library, &progress, files.clone(), &activity);
        assert_eq!(find(&titles, &Title::Series("dark".into())).keep, Some(Keep::UpNext));
        let up_title = find(&titles, &Title::Movie("up".into()));
        assert_eq!((up_title.keep, up_title.last_watched), (Some(Keep::Started), Some(777)));
        let heat_title = find(&titles, &Title::Movie("heat".into()));
        assert_eq!((heat_title.keep, heat_title.last_watched), (None, Some(555)));

        let playing = Activity {
            playing: Some(&dark),
            queued: &queued,
        };
        let titles = super::titles(Path::new("/lib"), &library, &progress, files, &playing);
        assert_eq!(find(&titles, &Title::Series("dark".into())).keep, Some(Keep::Playing));
    }

    #[test]
    fn files_in_use_are_kept_even_when_the_library_does_not_list_them() {
        let progress = Progress {
            files: [(
                PathBuf::from("/lib/Movies/Gone/Gone.mkv"),
                Watched {
                    stopped_at: Some(60.0),
                    ..Watched::default()
                },
            )]
            .into(),
            ..Progress::default()
        };
        let files = vec![
            stored("/lib/Movies/Playing/Playing.mkv", 10, 10, 1),
            stored("/lib/Movies/Queued/Queued.mkv", 10, 10, 1),
            stored("/lib/Movies/Gone/Gone.mkv", 10, 10, 1),
            stored("/lib/Movies/Stale/Stale.mkv", 10, 10, 1),
        ];
        let playing = PathBuf::from("/lib/Movies/Playing/Playing.mkv");
        let queued = [PathBuf::from("/lib/Movies/Queued/Queued.mkv")];
        let activity = Activity {
            playing: Some(&playing),
            queued: &queued,
        };
        let titles = titles(Path::new("/lib"), &Library::default(), &progress, files, &activity);
        let keep = |key: &str| find(&titles, &Title::Other(format!("Movies/{key}"))).keep;
        assert_eq!(keep("Playing"), Some(Keep::Playing));
        assert_eq!(keep("Queued"), Some(Keep::UpNext));
        assert_eq!(keep("Gone"), Some(Keep::Started));
        assert_eq!(keep("Stale"), None);
        assert_eq!(keys(&titles, &order(&titles)), ["Stale"]);
    }

    #[test]
    fn removing_a_title_deletes_its_data_and_metadata_files_only() {
        let root = std::env::temp_dir().join(format!("semmeltv-storage-{}", std::process::id()));
        let cache = Cache {
            library: PathBuf::from("/lib"),
            manifest: root.clone(),
            data: root.join("vfs"),
            meta: root.join("vfsMeta"),
            marks: MARKS,
        };
        for tree in [&cache.data, &cache.meta] {
            fs::create_dir_all(tree.join("Movies/Up")).unwrap();
            fs::write(tree.join("Movies/Up/Up.mkv"), vec![1; 4096]).unwrap();
            fs::write(tree.join("Movies/Up/Up.srt"), b"x").unwrap();
        }
        fs::remove_file(cache.meta.join("Movies/Up/Up.srt")).unwrap();
        let scanned = cache.scan();
        assert_eq!(scanned.len(), 2);
        let movie: Vec<Stored> = scanned
            .into_iter()
            .filter(|file| file.path == Path::new("/lib/Movies/Up/Up.mkv"))
            .collect();
        assert_eq!(movie[0].size, 4096);
        cache.remove(&movie).unwrap();
        assert!(!cache.data.join("Movies/Up/Up.mkv").exists());
        assert!(!cache.meta.join("Movies/Up/Up.mkv").exists());
        assert!(cache.data.join("Movies/Up/Up.srt").exists());
        assert!(cache.disk().unwrap().total > 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_report_lists_what_goes_next_and_skips_empty_placeholders() {
        let mut kept = cached("kept", 5, Some(1), 1);
        kept.keep = Some(Keep::Watching);
        let titles = vec![
            cached("b", 3, Some(20), 1),
            cached("empty", 0, None, 0),
            kept,
            cached("a", 2, Some(10), 1),
        ];
        let report = Report::new(Disk::default(), MARKS, titles);
        assert_eq!(report.used, 10);
        assert_eq!(report.titles.len(), 3);
        assert_eq!(report.next, [Title::Movie("a".into()), Title::Movie("b".into())]);
        let json = serde_json::to_value(&report.titles[0]).unwrap();
        assert_eq!(json["kind"], "movie");
        assert_eq!(json["key"], "b");
    }
}
