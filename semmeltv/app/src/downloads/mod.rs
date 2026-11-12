mod deliver;
pub mod release;
pub mod remote;
mod resolve;
mod rqbit;
mod score;
mod sidecars;
mod torznab;

use crate::intent::{self, Action};
use crate::library::Library;
use crate::manifest::{self, Date, Manifest};
use crate::parse;
use crate::progress::Progress;
use crate::tmdb::{self, Kind};
use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use deliver::{Placed, Shelf};
use resolve::{Candidate, Resolved, Target, Wish};
use rqbit::Rqbit;
use score::{Choice, Part};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const POLL: Duration = Duration::from_secs(2);
const PROPOSAL_LIFE: u64 = 30 * 60;
const KEPT_LOG: usize = 60;
const GIB: u64 = 1024 * 1024 * 1024;
const VIDEO_EXTENSIONS: [&str; 6] = ["mkv", "mp4", "avi", "m4v", "mov", "webm"];

pub struct Config {
    pub listen: String,
    pub state: PathBuf,
    pub downloads: PathBuf,
    pub torznab: Vec<String>,
    pub torznab_key_file: Option<PathBuf>,
    pub rqbit: String,
    pub quota: u64,
    pub reserve: u64,
    pub meta: PathBuf,
    pub remote: String,
    pub mount_rc: PathBuf,
    pub sync_unit: String,
    pub tmdb_token: String,
    pub mdblist: Option<crate::mdblist::Client>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Question {
    pub text: String,
    pub options: Vec<Candidate>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub asked: String,
    pub from: String,
    pub created: u64,
    pub ok: bool,
    pub message: String,
    pub label: Option<String>,
    pub target: Option<Target>,
    pub question: Option<Question>,
    pub choices: Vec<Choice>,
    pub dropped: usize,
    pub problems: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    Checking,
    Downloading,
    Uploading,
    Done,
    Failed,
    Stopped,
}

impl Stage {
    fn active(self) -> bool {
        matches!(self, Self::Checking | Self::Downloading | Self::Uploading)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileState {
    pub index: usize,
    pub path: String,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PartState {
    pub part: Part,
    pub folder: String,
    pub files: Vec<FileState>,
    pub added: bool,
    pub progress: u64,
    pub total: u64,
    pub uploaded: u64,
    pub finished: bool,
    pub paused: bool,
    pub speed: f64,
    pub peers: Option<u64>,
    pub seen: Option<u64>,
    #[serde(default)]
    pub checking: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub label: String,
    pub created: u64,
    pub target: Target,
    pub choice: Choice,
    pub stage: Stage,
    pub message: String,
    pub parts: Vec<PartState>,
    pub placed: Vec<String>,
    pub log: Vec<String>,
}

impl Job {
    fn note(&mut self, line: String) {
        eprintln!("download {}: {line}", self.id);
        self.log.push(format!("{} {line}", clock()));
        if self.log.len() > KEPT_LOG {
            self.log.remove(0);
        }
    }

    fn remaining(&self) -> u64 {
        if !self.stage.active() {
            return 0;
        }
        self.parts
            .iter()
            .map(|part| {
                let total = if part.total > 0 { part.total } else { part.part.size };
                total.saturating_sub(part.progress)
            })
            .sum()
    }

    fn held(&self) -> u64 {
        if !self.stage.active() {
            return 0;
        }
        self.parts
            .iter()
            .map(|part| if part.total > 0 { part.total } else { part.part.size })
            .sum()
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    jobs: Vec<Job>,
}

#[derive(Default)]
struct Book {
    proposals: Vec<Proposal>,
    jobs: Vec<Job>,
}

struct Service {
    book: Mutex<Book>,
    state: PathBuf,
    downloads: PathBuf,
    torznab: torznab::Torznab,
    rqbit: Rqbit,
    quota: u64,
    reserve: u64,
    meta: PathBuf,
    tmdb: tmdb::Client,
    shelf: Mutex<Shelf>,
    sidecars: sidecars::Sidecars,
    deliveries: Mutex<Sender<String>>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn clock() -> String {
    let seconds = now();
    let date = Date::from_unix(seconds);
    let day = seconds % 86_400;
    format!(
        "{date} {:02}:{:02}:{:02}",
        day / 3600,
        day % 3600 / 60,
        day % 60
    )
}

fn new_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!("{:x}", nanos)
}

fn free_bytes(dir: &Path) -> Option<u64> {
    let output = Command::new("stat")
        .args(["-f", "-c", "%a %S"])
        .arg(dir)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut fields = text.split_whitespace();
    let blocks: u64 = fields.next()?.parse().ok()?;
    let size: u64 = fields.next()?.parse().ok()?;
    Some(blocks * size)
}

fn downloading(progress: u64, total: u64, speed: f64) -> String {
    let percent = progress as f64 * 100.0 / total as f64;
    let left = total.saturating_sub(progress) as f64 / (speed * 1024.0 * 1024.0);
    if speed > 0.0 && left.is_finite() {
        let minutes = (left / 60.0).ceil() as u64;
        let time = if minutes >= 60 {
            format!("{}h {:02}m", minutes / 60, minutes % 60)
        } else {
            format!("{minutes} min")
        };
        format!("Downloading, {percent:.0}%, about {time} left")
    } else {
        format!("Downloading, {percent:.0}%")
    }
}

fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| VIDEO_EXTENSIONS.contains(&extension.to_lowercase().as_str()))
}

fn wanted_files(target: &Target, part: &Part, files: &[rqbit::File]) -> Vec<FileState> {
    let candidates: Vec<(usize, &rqbit::File, PathBuf)> = files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            let path: PathBuf = if file.components.is_empty() {
                PathBuf::from(&file.name)
            } else {
                file.components.iter().collect()
            };
            (index, file, path)
        })
        .filter(|(_, _, path)| is_video(path) && !parse::is_extra(path))
        .collect();
    let state = |(index, file, path): &(usize, &rqbit::File, PathBuf)| FileState {
        index: *index,
        path: path.to_string_lossy().into_owned(),
        size: file.length,
    };
    match (target.kind, target.season) {
        (Kind::Movie, _) | (Kind::Tv, None) => candidates
            .iter()
            .max_by_key(|(_, file, _)| file.length)
            .map(state)
            .into_iter()
            .collect(),
        (Kind::Tv, Some(season)) => {
            let wanted: BTreeSet<u32> = if !part.episodes.is_empty() {
                part.episodes.iter().copied().collect()
            } else {
                target
                    .episodes
                    .iter()
                    .map(|episode| episode.number)
                    .filter(|number| target.episode.is_none_or(|wanted| wanted == *number))
                    .collect()
            };
            let mut best: BTreeMap<u32, &(usize, &rqbit::File, PathBuf)> = BTreeMap::new();
            for candidate in &candidates {
                let number = match deliver::episode_of(&candidate.2) {
                    Some((found, number)) if found == season => number,
                    Some(_) => continue,
                    None if wanted.len() == 1 && candidates.len() == 1 => *wanted.first().unwrap(),
                    None => continue,
                };
                if !wanted.contains(&number) {
                    continue;
                }
                match best.get(&number) {
                    Some(known) if known.1.length >= candidate.1.length => {}
                    _ => {
                        best.insert(number, candidate);
                    }
                }
            }
            best.values().map(|candidate| state(candidate)).collect()
        }
    }
}

impl Service {
    fn save(&self, book: &Book) {
        let saved = Saved {
            jobs: book.jobs.clone(),
        };
        let path = self.state.join("jobs.json");
        match serde_json::to_vec_pretty(&saved) {
            Ok(bytes) => {
                if let Err(error) = manifest::write_atomically(&path, &bytes) {
                    eprintln!("{}: {error}", path.display());
                }
            }
            Err(error) => eprintln!("jobs: {error}"),
        }
    }

    fn load(&self) {
        let path = self.state.join("jobs.json");
        let saved: Saved = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        self.book.lock().unwrap().jobs = saved.jobs.into_iter().filter(|job| job.stage != Stage::Stopped).collect();
    }

    fn update(&self, id: &str, change: impl FnOnce(&mut Job)) {
        let mut book = self.book.lock().unwrap();
        if let Some(job) = book.jobs.iter_mut().find(|job| job.id == id) {
            change(job);
        }
        self.save(&book);
    }

    fn room(&self) -> u64 {
        let book = self.book.lock().unwrap();
        let held: u64 = book.jobs.iter().map(Job::held).sum();
        let remaining: u64 = book.jobs.iter().map(Job::remaining).sum();
        drop(book);
        let quota_left = self.quota.saturating_sub(held);
        let disk_left = free_bytes(&self.downloads)
            .unwrap_or(0)
            .saturating_sub(remaining)
            .saturating_sub(self.reserve);
        quota_left.min(disk_left)
    }

    fn manifest(&self) -> Manifest {
        Manifest::read(&self.meta.join(manifest::FILE_NAME)).unwrap_or_default()
    }

    fn wish_from_text(&self, text: &str) -> Result<Wish, String> {
        let manifest = self.manifest();
        let library = Library::from_manifest(Path::new("/"), &manifest, |_| true);
        match intent::understand(&format!("download {text}"), &library, &Progress::default()) {
            Action::Download { title, season, episode } => Ok(Wish {
                title,
                year: None,
                season,
                episode,
            }),
            Action::Refuse(reason) => Err(reason),
            _ => Err(format!("Could not understand \"{text}\" as something to download")),
        }
    }

    fn propose(&self, asked: String, from: String, resolved: Result<Resolved, String>) -> Proposal {
        let mut proposal = Proposal {
            id: new_id(),
            asked,
            from,
            created: now(),
            ok: false,
            message: String::new(),
            label: None,
            target: None,
            question: None,
            choices: Vec::new(),
            dropped: 0,
            problems: Vec::new(),
        };
        match resolved {
            Err(error) => proposal.message = error,
            Ok(Resolved::Refused(reason)) => proposal.message = reason,
            Ok(Resolved::Question(text, options)) => {
                proposal.message = text.clone();
                proposal.question = Some(Question { text, options });
            }
            Ok(Resolved::Target(target)) => {
                let search = match (target.kind, target.season) {
                    (Kind::Movie, _) | (Kind::Tv, None) => torznab::Search::Movie {
                        title: target.search_title(),
                        year: target.year,
                        imdb: target.imdb.clone(),
                    },
                    (Kind::Tv, Some(season)) => torznab::Search::Tv {
                        title: target.search_title(),
                        season,
                        episode: target.episode,
                    },
                };
                let (offers, problems) = self.torznab.search(&search);
                let room = self.room();
                let ranking = score::rank(&offers, &target, room);
                proposal.label = Some(target.label());
                proposal.problems = problems;
                proposal.dropped = ranking.dropped.len();
                for dropped in &ranking.dropped {
                    eprintln!("dropped: {} ({})", dropped.name, dropped.reason);
                }
                proposal.choices = ranking.choices;
                proposal.message = match proposal.choices.first() {
                    Some(best) => {
                        proposal.ok = true;
                        format!("{}: {}", target.label(), best.facts)
                    }
                    None if offers.is_empty() => format!("No releases of {} found", target.label()),
                    None => format!(
                        "None of the {} releases of {} passed the checks ({} free for downloads)",
                        offers.len(),
                        target.label(),
                        score::gb(room)
                    ),
                };
                proposal.target = Some(target);
            }
        }
        let mut book = self.book.lock().unwrap();
        let cutoff = now().saturating_sub(PROPOSAL_LIFE);
        book.proposals.retain(|known| known.created >= cutoff);
        book.proposals.push(proposal.clone());
        proposal
    }

    fn search(&self, request: SearchBody) -> Proposal {
        let today = Date::from_unix(now());
        let manifest = self.manifest();
        let from = request.from.unwrap_or_else(|| "web".to_string());
        if let Some(pick) = request.pick {
            let asked = request.text.unwrap_or_else(|| pick.tmdb.to_string());
            let resolved = resolve::resolve_id(
                &self.tmdb,
                pick.kind,
                pick.tmdb,
                pick.season,
                pick.episode,
                &manifest,
                today,
            );
            return self.propose(asked, from, resolved);
        }
        let (asked, wish) = match (request.wish, request.text) {
            (Some(wish), text) => (text.unwrap_or_else(|| wish.title.clone()), Ok(wish)),
            (None, Some(text)) => {
                let wish = self.wish_from_text(&text);
                (text, wish)
            }
            (None, None) => (String::new(), Err("Say what to download".to_string())),
        };
        let resolved = wish.and_then(|wish| resolve::resolve(&self.tmdb, &wish, &manifest, today));
        self.propose(asked, from, resolved)
    }

    fn confirm(&self, proposal: &str, choice: usize) -> Outcome {
        let book = self.book.lock().unwrap();
        let Some(index) = book.proposals.iter().position(|known| known.id == proposal) else {
            return Outcome::no("That search has expired; search again");
        };
        let found = book.proposals[index].clone();
        let (Some(target), Some(picked)) = (found.target.clone(), found.choices.get(choice).cloned()) else {
            return Outcome::no("Nothing to download in that search");
        };
        let duplicate = book.jobs.iter().any(|job| {
            job.stage.active()
                && job
                    .parts
                    .iter()
                    .any(|part| picked.parts.iter().any(|new| new.infohash == part.part.infohash))
        });
        if duplicate {
            return Outcome::no("That is already downloading");
        }
        drop(book);
        let room = self.room();
        if picked.size > room {
            return Outcome::no(&format!(
                "{} is more than the {} free for downloads now",
                score::gb(picked.size),
                score::gb(room)
            ));
        }
        let id = new_id();
        let folder = self.downloads.join(&id);
        let mut job = Job {
            id: id.clone(),
            label: target.label(),
            created: now(),
            target,
            choice: picked.clone(),
            stage: Stage::Checking,
            message: "Finding the files".to_string(),
            parts: picked
                .parts
                .iter()
                .enumerate()
                .map(|(number, part)| PartState {
                    part: part.clone(),
                    folder: folder.join(number.to_string()).to_string_lossy().into_owned(),
                    files: Vec::new(),
                    added: false,
                    progress: 0,
                    total: 0,
                    uploaded: 0,
                    finished: false,
                    paused: false,
                    speed: 0.0,
                    peers: None,
                    seen: None,
                    checking: false,
                })
                .collect(),
            placed: Vec::new(),
            log: Vec::new(),
        };
        job.note(format!("confirmed: {} ({})", picked.label, picked.facts));
        let label = job.label.clone();
        let mut book = self.book.lock().unwrap();
        book.proposals.retain(|known| known.id != proposal);
        book.jobs.push(job);
        self.save(&book);
        Outcome {
            message: format!("Downloading {label}"),
            ok: true,
            job: Some(id),
        }
    }

    fn dismiss(&self, proposal: &str) {
        self.book.lock().unwrap().proposals.retain(|known| known.id != proposal);
    }

    fn stop(&self, id: &str) -> Outcome {
        let parts: Vec<String> = {
            let book = self.book.lock().unwrap();
            match book.jobs.iter().find(|job| job.id == id) {
                Some(job) if job.stage.active() && job.stage != Stage::Uploading => {
                    job.parts.iter().map(|part| part.part.infohash.clone()).collect()
                }
                Some(_) => return Outcome::no("That download can't be stopped now"),
                None => return Outcome::no("No such download"),
            }
        };
        for hash in &parts {
            if let Err(error) = self.rqbit.delete(hash) {
                eprintln!("stop {id}: {error}");
            }
        }
        let folder = self.downloads.join(id);
        if folder.exists()
            && let Err(error) = fs::remove_dir_all(&folder)
        {
            eprintln!("stop {id}: {}: {error}", folder.display());
        }
        let mut book = self.book.lock().unwrap();
        book.jobs.retain(|job| job.id != id);
        self.save(&book);
        Outcome {
            message: "Stopped, and its files deleted".to_string(),
            ok: true,
            job: Some(id.to_string()),
        }
    }

    fn check(&self, job: &Job) {
        let mut parts = job.parts.clone();
        for part in parts.iter_mut().filter(|part| !part.added) {
            if part.files.is_empty() {
                let details = match self.rqbit.files(&part.part.magnet) {
                    Ok(details) => details,
                    Err(error) => {
                        return self.fail(&job.id, format!("Could not read the torrent: {error}"));
                    }
                };
                part.files = wanted_files(&job.target, &part.part, &details.files.unwrap_or_default());
                if part.files.is_empty() {
                    return self.fail(&job.id, format!("{} has no video file to use", part.part.name));
                }
            }
            let only: Vec<usize> = part.files.iter().map(|file| file.index).collect();
            if let Err(error) = self.rqbit.add(&part.part.magnet, &part.folder, &only) {
                return self.fail(&job.id, format!("rqbit did not take the torrent: {error}"));
            }
            part.added = true;
            part.total = part.files.iter().map(|file| file.size).sum();
            let files: Vec<String> = part.files.iter().map(|file| file.path.clone()).collect();
            let name = part.part.name.clone();
            self.update(&job.id, |job| job.note(format!("{name}: {} files: {}", files.len(), files.join(", "))));
        }
        self.update(&job.id, |job| {
            job.parts = parts;
            job.stage = Stage::Downloading;
            job.message = "Downloading".to_string();
        });
    }

    fn fail(&self, id: &str, reason: String) {
        self.update(id, |job| {
            job.stage = Stage::Failed;
            job.note(reason.clone());
            job.message = reason;
        });
    }

    fn poll(&self, job: &Job) {
        let mut parts = job.parts.clone();
        let mut notes = Vec::new();
        for part in parts.iter_mut().filter(|part| !part.paused) {
            let hash = part.part.infohash.clone();
            let stats = match self.rqbit.stats(&hash) {
                Ok(stats) => stats,
                Err(error) => {
                    notes.push(format!("{}: {error}", part.part.name));
                    continue;
                }
            };
            part.checking = stats.state == "initializing";
            if !part.checking {
                part.progress = stats.progress_bytes;
            }
            part.total = stats.total_bytes;
            part.uploaded = stats.uploaded_bytes;
            part.speed = stats.live.as_ref().map_or(0.0, |live| live.download_speed.mbps);
            part.peers = stats.peers();
            part.seen = stats.seen_peers();
            if let Some(error) = &stats.error {
                notes.push(format!("{}: rqbit: {error}", part.part.name));
            }
            if stats.finished {
                let paused = self.rqbit.pause(&hash);
                let after = self.rqbit.stats(&hash).map(|stats| stats.uploaded_bytes).unwrap_or(part.uploaded);
                part.uploaded = after;
                part.finished = true;
                part.paused = paused.is_ok();
                notes.push(match paused {
                    Ok(()) => format!(
                        "{}: complete, paused at once; {} bytes uploaded in all",
                        part.part.name, after
                    ),
                    Err(error) => format!("{}: complete, but pausing failed: {error}", part.part.name),
                });
            }
        }
        let done = parts.iter().all(|part| part.finished && part.paused);
        let checking = parts.iter().any(|part| part.checking);
        let progress: u64 = parts.iter().map(|part| part.progress).sum();
        let total: u64 = parts.iter().map(|part| part.total).sum();
        let speed: f64 = parts.iter().map(|part| part.speed).sum();
        self.update(&job.id, |job| {
            for note in notes {
                job.note(note);
            }
            job.parts = parts;
            if done {
                job.stage = Stage::Uploading;
                job.message = "Uploading to pCloud".to_string();
            } else if checking {
                job.message = "Checking the pieces already downloaded".to_string();
            } else if total > 0 {
                job.message = downloading(progress, total, speed);
            }
        });
        if done {
            let _ = self.deliveries.lock().unwrap().send(job.id.clone());
        }
    }

    fn work(&self) {
        let jobs: Vec<Job> = {
            let book = self.book.lock().unwrap();
            book.jobs
                .iter()
                .filter(|job| matches!(job.stage, Stage::Checking | Stage::Downloading))
                .cloned()
                .collect()
        };
        for job in jobs {
            match job.stage {
                Stage::Checking => self.check(&job),
                Stage::Downloading => self.poll(&job),
                _ => {}
            }
        }
    }

    fn deliver(&self, id: &str) {
        let Some(job) = self.book.lock().unwrap().jobs.iter().find(|job| job.id == id).cloned() else {
            return;
        };
        let mut placed = Vec::new();
        for part in &job.parts {
            for file in &part.files {
                let local = Path::new(&part.folder).join(&file.path);
                let Some(destination) = deliver::destination(&job.target, Path::new(&file.path), part.part.info.as_deref())
                else {
                    return self.fail(id, format!("No place in the library for {}", file.path));
                };
                placed.push(Placed {
                    local,
                    destination,
                    size: file.size,
                });
            }
        }
        let log = |line: String| self.update(id, |job| job.note(line));
        let shelf = self.shelf.lock().unwrap();
        for (number, place) in placed.iter().enumerate() {
            self.update(id, |job| {
                job.message = format!("Uploading to pCloud, file {} of {}", number + 1, placed.len());
            });
            if let Err(error) = shelf.upload(place) {
                return self.fail(id, format!("Upload failed: {error}"));
            }
            log(format!("uploaded {}", place.destination));
        }
        self.update(id, |job| {
            job.placed = placed.iter().map(|place| place.destination.clone()).collect();
            job.message = "Adding to the library".to_string();
        });
        if let Err(error) = shelf.import(&job.target, &placed, &log) {
            return self.fail(id, format!("Import failed: {error}"));
        }
        for part in &job.parts {
            match self.rqbit.delete(&part.part.infohash) {
                Ok(()) => log(format!("deleted the local copy of {}", part.part.name)),
                Err(error) => log(format!("could not delete {}: {error}", part.part.name)),
            }
        }
        let folder = self.downloads.join(id);
        if folder.exists() {
            match fs::remove_dir_all(&folder) {
                Ok(()) => log(format!("removed {}", folder.display())),
                Err(error) => log(format!("{}: {error}", folder.display())),
            }
        }
        shelf.show_in_library(&placed, &log);
        let uploaded: u64 = job.parts.iter().map(|part| part.uploaded).sum();
        self.update(id, |job| {
            job.stage = Stage::Done;
            job.message = format!("In the library ({uploaded} bytes uploaded to peers)");
        });
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Outcome {
    pub message: String,
    pub ok: bool,
    pub job: Option<String>,
}

impl Outcome {
    fn no(message: &str) -> Self {
        Self {
            message: message.to_string(),
            ok: false,
            job: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pick {
    #[serde(with = "resolve::kind_name")]
    pub kind: Kind,
    pub tmdb: u64,
    pub season: Option<u32>,
    pub episode: Option<u32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchBody {
    pub text: Option<String>,
    pub wish: Option<Wish>,
    pub pick: Option<Pick>,
    pub from: Option<String>,
}

#[derive(Deserialize)]
struct ConfirmBody {
    proposal: String,
    #[serde(default)]
    choice: usize,
}

#[derive(Deserialize)]
struct DismissBody {
    proposal: String,
}

#[derive(Serialize)]
struct Overview {
    proposals: Vec<Proposal>,
    jobs: Vec<Job>,
    room: u64,
    quota: u64,
    free: Option<u64>,
}

type Shared = Arc<Service>;

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T, Response> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response())
}

async fn overview(State(service): State<Shared>) -> Response {
    let result = blocking(move || {
        let room = service.room();
        let free = free_bytes(&service.downloads);
        let book = service.book.lock().unwrap();
        let cutoff = now().saturating_sub(PROPOSAL_LIFE);
        Overview {
            proposals: book
                .proposals
                .iter()
                .filter(|proposal| proposal.created >= cutoff)
                .cloned()
                .collect(),
            jobs: book.jobs.iter().rev().cloned().collect(),
            room,
            quota: service.quota,
            free,
        }
    })
    .await;
    match result {
        Ok(overview) => Json(overview).into_response(),
        Err(response) => response,
    }
}

async fn search(State(service): State<Shared>, Json(body): Json<SearchBody>) -> Response {
    match blocking(move || service.search(body)).await {
        Ok(proposal) => Json(proposal).into_response(),
        Err(response) => response,
    }
}

async fn confirm(State(service): State<Shared>, Json(body): Json<ConfirmBody>) -> Response {
    match blocking(move || service.confirm(&body.proposal, body.choice)).await {
        Ok(outcome) => Json(outcome).into_response(),
        Err(response) => response,
    }
}

async fn dismiss(State(service): State<Shared>, Json(body): Json<DismissBody>) -> Response {
    service.dismiss(&body.proposal);
    Json(Outcome {
        message: "Dismissed".to_string(),
        ok: true,
        job: None,
    })
    .into_response()
}

async fn stop(State(service): State<Shared>, UrlPath(id): UrlPath<String>) -> Response {
    match blocking(move || service.stop(&id)).await {
        Ok(outcome) => Json(outcome).into_response(),
        Err(response) => response,
    }
}

async fn subtitles(State(service): State<Shared>, Json(wanted): Json<crate::subfetch::Wanted>) -> Response {
    match blocking(move || service.sidecars.fetch(&wanted)).await {
        Ok(Ok(fetched)) => Json(fetched).into_response(),
        Ok(Err(error)) => crate::api::problem(StatusCode::UNPROCESSABLE_ENTITY, error),
        Err(response) => response,
    }
}

fn router(service: Shared) -> Router {
    Router::new()
        .route("/api/downloads", get(overview))
        .route("/api/downloads/search", post(search))
        .route("/api/downloads/confirm", post(confirm))
        .route("/api/downloads/dismiss", post(dismiss))
        .route("/api/downloads/jobs/{id}/stop", post(stop))
        .route("/api/downloads/subtitles", post(subtitles))
        .with_state(service)
}

fn deliveries(service: Shared, received: Receiver<String>) {
    for id in received {
        service.deliver(&id);
    }
}

pub fn run(config: Config) -> Result<(), String> {
    fs::create_dir_all(&config.state).map_err(|error| format!("{}: {error}", config.state.display()))?;
    let api_key = match &config.torznab_key_file {
        Some(file) => Some(
            fs::read_to_string(file)
                .map_err(|error| format!("{}: {error}", file.display()))?
                .trim()
                .to_string(),
        ),
        None => None,
    };
    let (send, received) = mpsc::channel();
    let service = Arc::new(Service {
        book: Mutex::new(Book::default()),
        state: config.state.clone(),
        downloads: config.downloads.clone(),
        torznab: torznab::Torznab::new(config.torznab.clone(), api_key),
        rqbit: Rqbit::new(config.rqbit.clone()),
        quota: config.quota,
        reserve: config.reserve,
        meta: config.meta.clone(),
        tmdb: tmdb::Client::new(config.tmdb_token.clone()),
        sidecars: sidecars::Sidecars {
            client: crate::subfetch::Client::default(),
            remote: config.remote.clone(),
            mount_rc: config.mount_rc.clone(),
            work: config.state.join("subtitles"),
        },
        shelf: Mutex::new(Shelf {
            remote: config.remote,
            meta: config.meta,
            work: config.state.join("work"),
            tmdb: tmdb::Client::new(config.tmdb_token),
            mdblist: config.mdblist,
            mount_rc: config.mount_rc,
            sync_unit: config.sync_unit,
        }),
        deliveries: Mutex::new(send),
    });
    service.load();
    let waiting: Vec<String> = service
        .book
        .lock()
        .unwrap()
        .jobs
        .iter()
        .filter(|job| job.stage == Stage::Uploading)
        .map(|job| job.id.clone())
        .collect();
    for id in waiting {
        let _ = service.deliveries.lock().unwrap().send(id);
    }
    let delivering = service.clone();
    thread::Builder::new()
        .name("deliveries".to_string())
        .spawn(move || deliveries(delivering, received))
        .map_err(|error| error.to_string())?;
    let working = service.clone();
    thread::Builder::new()
        .name("downloads".to_string())
        .spawn(move || {
            loop {
                working.work();
                thread::sleep(POLL);
            }
        })
        .map_err(|error| error.to_string())?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    let quota = config.quota / GIB;
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind(&config.listen)
            .await
            .map_err(|error| format!("{}: {error}", config.listen))?;
        eprintln!("downloads on http://{}/api/downloads, quota {quota} GiB", config.listen);
        axum::serve(listener, router(service))
            .await
            .map_err(|error| error.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_progress_line_says_how_long_is_left() {
        let mib = 1024 * 1024;
        assert_eq!(
            downloading(10 * 1024 * mib, 20 * 1024 * mib, 8.0),
            "Downloading, 50%, about 22 min left"
        );
        assert_eq!(
            downloading(0, 40 * 1024 * mib, 8.0),
            "Downloading, 0%, about 1h 26m left"
        );
        assert_eq!(downloading(5, 10, 0.0), "Downloading, 50%");
    }
}
