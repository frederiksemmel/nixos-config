pub mod cli;
mod deliver;
mod forget;
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
use crate::ready::{self, Ready};
use crate::tmdb::{self, Kind};
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use deliver::{Placed, Shelf};
use resolve::{Candidate, Mode, Resolved, Target, Wish};
use rqbit::Rqbit;
use score::{Choice, Part};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const POLL: Duration = Duration::from_secs(2);
const PROPOSAL_LIFE: u64 = 30 * 60;
const SEASONS_LIFE: u64 = 6 * 60 * 60;
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
    pub season: Option<u32>,
    pub episode: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Searched {
    pub label: String,
    pub target: Target,
    pub message: String,
    pub choices: Vec<Choice>,
    pub dropped: usize,
    pub problems: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum Answer {
    Refused { message: String },
    Question(Question),
    Searched(Searched),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub asked: String,
    pub from: String,
    #[serde(default)]
    pub by: Option<String>,
    pub created: u64,
    #[serde(flatten)]
    pub answer: Answer,
}

impl Proposal {
    pub fn heading(&self) -> &str {
        match &self.answer {
            Answer::Searched(searched) => &searched.label,
            Answer::Refused { .. } | Answer::Question(_) => &self.asked,
        }
    }

    pub fn message(&self) -> &str {
        match &self.answer {
            Answer::Refused { message } | Answer::Searched(Searched { message, .. }) => message,
            Answer::Question(question) => &question.text,
        }
    }

    pub fn best(&self) -> Option<&Choice> {
        match &self.answer {
            Answer::Searched(searched) => searched.choices.first(),
            Answer::Refused { .. } | Answer::Question(_) => None,
        }
    }
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
    pub progress: u64,
}

impl FileState {
    fn fetched(&self) -> bool {
        self.progress >= self.size
    }

    fn episode(&self) -> Option<String> {
        deliver::episode_of(Path::new(&self.path)).map(|(season, episode)| format!("S{season:02}E{episode:02}"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PartPhase {
    New,
    Checking,
    Downloading,
    Done,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PartState {
    pub part: Part,
    pub folder: String,
    pub files: Vec<FileState>,
    pub phase: PartPhase,
    pub progress: u64,
    pub total: u64,
    pub uploaded: u64,
    pub speed: f64,
    pub peers: Option<u64>,
    pub seen: Option<u64>,
    pub asked: Option<usize>,
}

impl PartState {
    fn next_file(&self) -> Option<&FileState> {
        self.files.iter().find(|file| !file.fetched())
    }

    fn record(&mut self, file_progress: &[u64]) -> Vec<String> {
        let mut fetched = Vec::new();
        for file in self.files.iter_mut().filter(|file| !file.fetched()) {
            if let Some(&have) = file_progress.get(file.index) {
                file.progress = have.min(file.size);
                if file.fetched() {
                    fetched.push(file.path.clone());
                }
            }
        }
        self.progress = self.files.iter().map(|file| file.progress).sum();
        fetched
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sending {
    pub sent: u64,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub label: String,
    pub created: u64,
    pub target: Target,
    pub choice: Choice,
    pub mode: Mode,
    pub stage: Stage,
    pub message: String,
    pub parts: Vec<PartState>,
    pub placed: Vec<String>,
    pub log: Vec<String>,
    #[serde(default)]
    pub sending: Option<Sending>,
    #[serde(default)]
    pub by: Option<String>,
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

    fn current(&self) -> Option<&FileState> {
        self.parts
            .iter()
            .find(|part| part.phase != PartPhase::Done)
            .and_then(PartState::next_file)
    }

    fn count(&self) -> usize {
        self.parts
            .iter()
            .map(|part| part.files.len().max(part.part.episodes.len()).max(1))
            .sum()
    }

    fn status(&self) -> String {
        if self.parts.iter().any(|part| part.phase == PartPhase::Checking) {
            return "Checking the pieces already downloaded".to_string();
        }
        let progress: u64 = self.parts.iter().map(|part| part.progress).sum();
        let total: u64 = self
            .parts
            .iter()
            .map(|part| if part.total > 0 { part.total } else { part.part.size })
            .sum();
        let speed: f64 = self.parts.iter().map(|part| part.speed).sum();
        let count = self.count();
        let what = match self.current().and_then(FileState::episode) {
            Some(episode) if count > 1 => format!("Downloading {episode}"),
            _ => "Downloading".to_string(),
        };
        let line = if total > 0 {
            downloading(&what, progress, total, speed)
        } else {
            what
        };
        if count > 1 {
            format!("{line} · {} of {count} in the library", self.placed.len())
        } else {
            line
        }
    }

    fn deliverable(&self) -> Result<Option<Placed>, String> {
        for part in &self.parts {
            for file in part.files.iter().filter(|file| file.fetched()) {
                let destination = deliver::destination(&self.target, Path::new(&file.path), part.part.info.as_deref())
                    .ok_or_else(|| format!("No place in the library for {}", file.path))?;
                if !self.placed.contains(&destination) {
                    return Ok(Some(Placed {
                        local: Path::new(&part.folder).join(&file.path),
                        destination,
                        size: file.size,
                    }));
                }
            }
        }
        Ok(None)
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

impl Book {
    fn picked(
        &self,
        proposal: &str,
        choice: usize,
        by: Option<String>,
    ) -> Result<(Target, Choice, Option<String>), &'static str> {
        let known = self
            .proposals
            .iter()
            .find(|known| known.id == proposal)
            .ok_or("That search has expired; search again")?;
        let Answer::Searched(Searched { target, choices, .. }) = &known.answer else {
            return Err("Nothing to download in that search");
        };
        let picked = choices.get(choice).ok_or("Nothing to download in that search")?;
        Ok((target.clone(), picked.clone(), by.or_else(|| known.by.clone())))
    }
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
    seasons: Mutex<HashMap<u64, (u64, Vec<tmdb::SeasonSummary>)>>,
    pcloud: forget::Rclone,
    forgetting: Mutex<()>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissingSeason {
    pub number: u32,
    pub downloading: bool,
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
    format!("{date} {:02}:{:02}:{:02}", day / 3600, day % 3600 / 60, day % 60)
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

fn downloading(what: &str, progress: u64, total: u64, speed: f64) -> String {
    let percent = progress as f64 * 100.0 / total as f64;
    let left = total.saturating_sub(progress) as f64 / (speed * 1024.0 * 1024.0);
    if speed > 0.0 && left.is_finite() {
        let minutes = (left / 60.0).ceil() as u64;
        let time = if minutes >= 60 {
            format!("{}h {:02}m", minutes / 60, minutes % 60)
        } else {
            format!("{minutes} min")
        };
        format!("{what}, {percent:.0}%, about {time} left")
    } else {
        format!("{what}, {percent:.0}%")
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
        progress: 0,
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
        self.book.lock().unwrap().jobs = saved
            .jobs
            .into_iter()
            .filter(|job| job.stage != Stage::Stopped)
            .map(|job| Job { sending: None, ..job })
            .collect();
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
        match intent::understand_typed_download(text, &library) {
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

    fn suggest(&self, text: &str) -> Result<Vec<Suggestion>, String> {
        let (title, season) = match self.wish_from_text(text) {
            Ok(wish) => (wish.title, wish.season),
            Err(_) => (text.trim().to_string(), None),
        };
        if title.is_empty() {
            return Ok(Vec::new());
        }
        let found = self
            .tmdb
            .search_titles(&title)
            .map_err(|error| format!("TMDB: {error}"))?;
        Ok(suggestions(found, season, &self.manifest()))
    }

    fn propose(&self, asked: String, from: String, by: Option<String>, resolved: Result<Resolved, String>) -> Proposal {
        let answer = match resolved {
            Err(message) | Ok(Resolved::Refused(message)) => Answer::Refused { message },
            Ok(Resolved::Question {
                text,
                options,
                season,
                episode,
            }) => Answer::Question(Question {
                text,
                options,
                season,
                episode,
            }),
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
                for dropped in &ranking.dropped {
                    eprintln!("dropped: {} ({})", dropped.name, dropped.reason);
                }
                let message = match ranking.choices.first() {
                    Some(best) => format!("{}: {}", target.label(), best.facts),
                    None if offers.is_empty() => format!("No releases of {} found", target.label()),
                    None => format!(
                        "None of the {} releases of {} passed the checks ({} free for downloads)",
                        offers.len(),
                        target.label(),
                        score::gb(room)
                    ),
                };
                Answer::Searched(Searched {
                    label: target.label(),
                    target,
                    message,
                    choices: ranking.choices,
                    dropped: ranking.dropped.len(),
                    problems,
                })
            }
        };
        let proposal = Proposal {
            id: new_id(),
            asked,
            from,
            by,
            created: now(),
            answer,
        };
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
        let by = request.by;
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
                Mode::Add,
            );
            return self.propose(asked, from, by, resolved);
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
        self.propose(asked, from, by, resolved)
    }

    fn confirm(&self, proposal: &str, choice: usize, by: Option<String>) -> Outcome {
        let picked = self.book.lock().unwrap().picked(proposal, choice, by);
        let (target, picked, by) = match picked {
            Ok(picked) => picked,
            Err(reason) => return Outcome::no(reason),
        };
        let outcome = self.start(target, picked, Mode::Add, by);
        if outcome.ok {
            self.dismiss(proposal);
        }
        outcome
    }

    fn fetch(&self, body: FetchBody) -> Outcome {
        let FetchBody { pick, offer, mode, by } = body;
        let resolved = resolve::resolve_id(
            &self.tmdb,
            pick.kind,
            pick.tmdb,
            pick.season,
            pick.episode,
            &self.manifest(),
            Date::from_unix(now()),
            mode,
        );
        match resolved {
            Ok(Resolved::Target(target)) => self.start(target, score::by_hand(&offer), mode, by),
            Ok(Resolved::Refused(message)) | Err(message) => Outcome::no(&message),
            Ok(Resolved::Question { text, .. }) => Outcome::no(&text),
        }
    }

    fn start(&self, target: Target, picked: Choice, mode: Mode, by: Option<String>) -> Outcome {
        let duplicate = self.book.lock().unwrap().jobs.iter().any(|job| {
            job.stage.active()
                && job
                    .parts
                    .iter()
                    .any(|part| picked.parts.iter().any(|new| new.infohash == part.part.infohash))
        });
        if duplicate {
            return Outcome::no("That is already downloading");
        }
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
            mode,
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
                    phase: PartPhase::New,
                    progress: 0,
                    total: 0,
                    uploaded: 0,
                    speed: 0.0,
                    peers: None,
                    seen: None,
                    asked: None,
                })
                .collect(),
            placed: Vec::new(),
            log: Vec::new(),
            sending: None,
            by,
        };
        let verb = match mode {
            Mode::Add => "Downloading",
            Mode::Replace => "Downloading a replacement for",
        };
        job.note(format!("confirmed: {} ({}), {mode:?}", picked.label, picked.facts));
        let label = job.label.clone();
        let mut book = self.book.lock().unwrap();
        book.jobs.push(job);
        self.save(&book);
        Outcome {
            message: format!("{verb} {label}"),
            ok: true,
            job: Some(id),
        }
    }

    fn missing_seasons(&self, show: u64) -> Result<Vec<MissingSeason>, String> {
        let cached = self
            .seasons
            .lock()
            .unwrap()
            .get(&show)
            .filter(|(fetched, _)| now().saturating_sub(*fetched) < SEASONS_LIFE)
            .map(|(_, seasons)| seasons.clone());
        let seasons = match cached {
            Some(seasons) => seasons,
            None => {
                let seasons = self.tmdb.seasons(show).map_err(|error| format!("TMDB: {error}"))?;
                self.seasons.lock().unwrap().insert(show, (now(), seasons.clone()));
                seasons
            }
        };
        let missing = resolve::missing_seasons(&seasons, &self.manifest(), show, Date::from_unix(now()));
        let book = self.book.lock().unwrap();
        Ok(missing
            .into_iter()
            .map(|number| MissingSeason {
                number,
                downloading: book.jobs.iter().any(|job| {
                    job.stage.active()
                        && job.target.kind == Kind::Tv
                        && job.target.tmdb == show
                        && job.target.season == Some(number)
                }),
            })
            .collect())
    }

    fn pcloud(&self) -> Result<Pcloud, String> {
        let space = self.pcloud.space()?;
        let files = forget::Pcloud::files(&self.pcloud)?;
        let titles = forget::holdings(&self.manifest(), &files);
        Ok(Pcloud { space, titles })
    }

    fn forget(&self, title: &crate::storage::Title) -> Outcome {
        let _one_at_a_time = self.forgetting.lock().unwrap();
        let log = |line: String| eprintln!("delete: {line}");
        match forget::forget(&self.pcloud, &self.pcloud.work, title, &log) {
            Ok(message) => {
                log(message.clone());
                self.pcloud.announce(&log);
                Outcome {
                    message,
                    ok: true,
                    job: None,
                }
            }
            Err(error) => {
                log(error.clone());
                Outcome::no(&error)
            }
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
        let kept = book
            .jobs
            .iter()
            .find(|job| job.id == id)
            .map_or(0, |job| job.placed.len());
        book.jobs.retain(|job| job.id != id);
        self.save(&book);
        let message = match kept {
            0 => "Stopped, and its files deleted".to_string(),
            1 => "Stopped, and its files deleted; the episode already in the library stays".to_string(),
            kept => format!("Stopped, and its files deleted; the {kept} episodes already in the library stay"),
        };
        Outcome {
            message,
            ok: true,
            job: Some(id.to_string()),
        }
    }

    fn begin(&self, target: &Target, part: &mut PartState) -> Result<String, String> {
        if part.files.is_empty() {
            let details = self
                .rqbit
                .files(&part.part.magnet)
                .map_err(|error| format!("Could not read the torrent: {error}"))?;
            part.files = wanted_files(target, &part.part, &details.files.unwrap_or_default());
        }
        let Some(first) = part.next_file().cloned() else {
            return Err(format!("{} has no video file to use", part.part.name));
        };
        self.rqbit
            .add(&part.part.magnet, &part.folder, &[first.index])
            .map_err(|error| format!("rqbit did not take the torrent: {error}"))?;
        part.asked = Some(first.index);
        part.phase = PartPhase::Downloading;
        part.total = part.files.iter().map(|file| file.size).sum();
        let files: Vec<&str> = part.files.iter().map(|file| file.path.as_str()).collect();
        Ok(format!(
            "{}: {} files: {}; fetching {} first",
            part.part.name,
            files.len(),
            files.join(", "),
            first.path
        ))
    }

    fn check(&self, job: &Job) {
        let mut parts = job.parts.clone();
        let Some(part) = parts.iter_mut().find(|part| part.phase == PartPhase::New) else {
            return;
        };
        let note = match self.begin(&job.target, part) {
            Ok(note) => note,
            Err(reason) => return self.fail(&job.id, reason),
        };
        self.update(&job.id, |job| {
            job.note(note);
            job.parts = parts;
            job.stage = Stage::Downloading;
            job.message = job.status();
        });
    }

    fn fail(&self, id: &str, reason: String) {
        let running: Vec<String> = {
            let book = self.book.lock().unwrap();
            book.jobs
                .iter()
                .filter(|job| job.id == id)
                .flat_map(|job| &job.parts)
                .filter(|part| matches!(part.phase, PartPhase::Checking | PartPhase::Downloading))
                .map(|part| part.part.infohash.clone())
                .collect()
        };
        for hash in &running {
            if let Err(error) = self.rqbit.pause(hash) {
                eprintln!("download {id}: {error}");
            }
        }
        self.update(id, |job| {
            job.stage = Stage::Failed;
            job.note(reason.clone());
            job.message = reason;
        });
    }

    fn follow(&self, part: &mut PartState, notes: &mut Vec<String>) -> Vec<String> {
        let hash = part.part.infohash.clone();
        let stats = match self.rqbit.stats(&hash) {
            Ok(stats) => stats,
            Err(error) => {
                notes.push(format!("{}: {error}", part.part.name));
                return Vec::new();
            }
        };
        let checking = stats.state == "initializing";
        let fetched = if checking {
            Vec::new()
        } else {
            part.record(&stats.file_progress)
        };
        part.uploaded = stats.uploaded_bytes;
        part.speed = stats.live.as_ref().map_or(0.0, |live| live.download_speed.mbps);
        part.peers = stats.peers();
        part.seen = stats.seen_peers();
        if let Some(error) = &stats.error {
            notes.push(format!("{}: rqbit: {error}", part.part.name));
        }
        notes.extend(fetched.iter().map(|path| format!("fetched {path}")));
        match part.next_file().cloned() {
            _ if checking => part.phase = PartPhase::Checking,
            Some(next) => {
                part.phase = PartPhase::Downloading;
                if part.asked != Some(next.index) {
                    match self.rqbit.select(&hash, &[next.index]) {
                        Ok(()) => {
                            part.asked = Some(next.index);
                            notes.push(format!("now fetching {}", next.path));
                        }
                        Err(error) => notes.push(format!("could not move on to {}: {error}", next.path)),
                    }
                }
            }
            None => {
                let paused = self.rqbit.pause(&hash);
                let after = self
                    .rqbit
                    .stats(&hash)
                    .map(|stats| stats.uploaded_bytes)
                    .unwrap_or(part.uploaded);
                part.uploaded = after;
                if paused.is_ok() {
                    part.phase = PartPhase::Done;
                }
                notes.push(match paused {
                    Ok(()) => format!(
                        "{}: complete, paused at once; {} bytes uploaded in all",
                        part.part.name, after
                    ),
                    Err(error) => format!("{}: complete, but pausing failed: {error}", part.part.name),
                });
            }
        }
        fetched
    }

    fn poll(&self, job: &Job) {
        let mut parts = job.parts.clone();
        let mut notes = Vec::new();
        let mut fetched = Vec::new();
        if let Some(part) = parts.iter_mut().find(|part| part.phase != PartPhase::Done) {
            if part.phase == PartPhase::New {
                match self.begin(&job.target, part) {
                    Ok(note) => notes.push(note),
                    Err(reason) => return self.fail(&job.id, reason),
                }
            } else {
                fetched = self.follow(part, &mut notes);
            }
        }
        let done = parts.iter().all(|part| part.phase == PartPhase::Done);
        self.update(&job.id, |job| {
            for note in notes {
                job.note(note);
            }
            job.parts = parts;
            if done {
                job.stage = Stage::Uploading;
                job.message = "Adding the last files to the library".to_string();
            } else {
                job.message = job.status();
            }
        });
        if done || !fetched.is_empty() {
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
        loop {
            let Some(job) = self.book.lock().unwrap().jobs.iter().find(|job| job.id == id).cloned() else {
                return;
            };
            if !matches!(job.stage, Stage::Downloading | Stage::Uploading) {
                return;
            }
            let place = match job.deliverable() {
                Ok(Some(place)) => place,
                Ok(None) if job.stage == Stage::Uploading => return self.finish(&job),
                Ok(None) => return,
                Err(reason) => return self.fail(id, reason),
            };
            if let Err(reason) = self.place(&job, &place) {
                return self.fail(id, reason);
            }
        }
    }

    fn place(&self, job: &Job, place: &Placed) -> Result<(), String> {
        let id = job.id.as_str();
        let log = |line: String| self.update(id, |job| job.note(line));
        let shelf = self.shelf.lock().unwrap();
        if job.stage == Stage::Uploading {
            self.update(id, |job| {
                job.message = format!("Uploading to pCloud, file {} of {}", job.placed.len() + 1, job.count());
            });
        }
        self.update(id, |job| {
            job.sending = Some(Sending {
                sent: 0,
                size: place.size,
            });
        });
        let ready = self.downloads.join(ready::FILE_NAME);
        let plays_first = job.mode == Mode::Add;
        if plays_first {
            if let Err(error) = Ready::add(&ready, &place.destination, &place.local) {
                log(format!(
                    "could not offer {} before its upload: {error}",
                    place.destination
                ));
            }
            shelf
                .import(
                    &job.target,
                    std::slice::from_ref(place),
                    job.mode,
                    job.by.as_deref(),
                    &log,
                )
                .map_err(|error| format!("Import failed: {error}"))?;
            shelf.sync(&log);
            log(format!("plays here while it uploads: {}", place.destination));
        }
        let uploaded = shelf.upload(place, &|sent| {
            self.update(id, |job| {
                job.sending = Some(Sending { sent, size: place.size });
            });
        });
        self.update(id, |job| job.sending = None);
        uploaded.map_err(|error| format!("Upload failed: {error}"))?;
        log(format!("uploaded {}", place.destination));
        if !plays_first {
            shelf
                .import(
                    &job.target,
                    std::slice::from_ref(place),
                    job.mode,
                    job.by.as_deref(),
                    &log,
                )
                .map_err(|error| format!("Import failed: {error}"))?;
        }
        shelf.show_in_library(std::slice::from_ref(place), &log);
        if let Err(error) = Ready::remove(&ready, &place.destination) {
            log(format!(
                "could not drop {} from the ready list: {error}",
                place.destination
            ));
        }
        self.update(id, |job| {
            job.placed.push(place.destination.clone());
            job.note(format!("in the library: {}", place.destination));
        });
        Ok(())
    }

    fn finish(&self, job: &Job) {
        let id = job.id.as_str();
        let log = |line: String| self.update(id, |job| job.note(line));
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Suggestion {
    #[serde(with = "resolve::kind_name")]
    pub kind: Kind,
    pub tmdb: u64,
    pub title: String,
    pub year: Option<u16>,
    pub poster: Option<String>,
    pub season: Option<u32>,
    pub owned: bool,
}

fn suggestions(found: Vec<tmdb::Match>, season: Option<u32>, manifest: &Manifest) -> Vec<Suggestion> {
    found
        .into_iter()
        .filter(|found| season.is_none() || found.kind == Kind::Tv)
        .map(|found| Suggestion {
            owned: resolve::in_library(manifest, found.kind, found.id),
            kind: found.kind,
            tmdb: found.id,
            title: found.name,
            year: found.year,
            poster: found.poster,
            season: season.filter(|_| found.kind == Kind::Tv),
        })
        .collect()
}

#[derive(Deserialize)]
struct SuggestQuery {
    text: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchBody {
    pub text: Option<String>,
    pub wish: Option<Wish>,
    pub pick: Option<Pick>,
    pub from: Option<String>,
    #[serde(default)]
    pub by: Option<String>,
}

#[derive(Deserialize)]
struct ConfirmBody {
    proposal: String,
    #[serde(default)]
    choice: usize,
    #[serde(default)]
    by: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FetchBody {
    pub pick: Pick,
    pub offer: torznab::Offer,
    pub mode: Mode,
    #[serde(default)]
    pub by: Option<String>,
}

#[derive(Deserialize)]
struct DismissBody {
    proposal: String,
}

#[derive(Serialize)]
struct Pcloud {
    space: forget::Space,
    titles: Vec<forget::Held>,
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

async fn suggest(State(service): State<Shared>, Query(query): Query<SuggestQuery>) -> Response {
    match blocking(move || service.suggest(&query.text)).await {
        Ok(Ok(found)) => Json(found).into_response(),
        Ok(Err(error)) => crate::api::problem(StatusCode::BAD_GATEWAY, error),
        Err(response) => response,
    }
}

async fn confirm(State(service): State<Shared>, Json(body): Json<ConfirmBody>) -> Response {
    match blocking(move || service.confirm(&body.proposal, body.choice, body.by)).await {
        Ok(outcome) => Json(outcome).into_response(),
        Err(response) => response,
    }
}

async fn fetch(State(service): State<Shared>, Json(body): Json<FetchBody>) -> Response {
    match blocking(move || service.fetch(body)).await {
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

async fn missing_seasons(State(service): State<Shared>, UrlPath(show): UrlPath<u64>) -> Response {
    match blocking(move || service.missing_seasons(show)).await {
        Ok(Ok(missing)) => Json(missing).into_response(),
        Ok(Err(error)) => crate::api::problem(StatusCode::BAD_GATEWAY, error),
        Err(response) => response,
    }
}

async fn pcloud(State(service): State<Shared>) -> Response {
    match blocking(move || service.pcloud()).await {
        Ok(Ok(found)) => Json(found).into_response(),
        Ok(Err(error)) => crate::api::problem(StatusCode::BAD_GATEWAY, error),
        Err(response) => response,
    }
}

async fn delete_title(State(service): State<Shared>, Json(title): Json<crate::storage::Title>) -> Response {
    match blocking(move || service.forget(&title)).await {
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
        .route("/api/downloads/suggest", get(suggest))
        .route("/api/downloads/confirm", post(confirm))
        .route("/api/downloads/fetch", post(fetch))
        .route("/api/downloads/dismiss", post(dismiss))
        .route("/api/downloads/jobs/{id}/stop", post(stop))
        .route("/api/downloads/subtitles", post(subtitles))
        .route("/api/downloads/shows/{tmdb}/missing", get(missing_seasons))
        .route("/api/downloads/pcloud", get(pcloud))
        .route("/api/downloads/pcloud/delete", post(delete_title))
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
        pcloud: forget::Rclone {
            remote: config.remote.clone(),
            work: config.state.join("forget"),
            mount_rc: config.mount_rc.clone(),
            sync_unit: config.sync_unit.clone(),
        },
        forgetting: Mutex::new(()),
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
        seasons: Mutex::new(HashMap::new()),
    });
    service.load();
    let waiting: Vec<String> = service
        .book
        .lock()
        .unwrap()
        .jobs
        .iter()
        .filter(|job| matches!(job.stage, Stage::Downloading | Stage::Uploading))
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
    use resolve::EpisodeInfo;

    #[test]
    fn suggestions_mark_owned_titles_and_a_typed_season_keeps_only_series() {
        let manifest = Manifest::parse(
            r#"
[[movie]]
title = "Dune"
year = 2021
tmdb = 438631
path = "Movies/Dune (2021)/Dune.mkv"
added = 2025-01-01
"#,
        )
        .unwrap();
        let found = |kind, id| tmdb::Match {
            kind,
            id,
            name: "Dune".to_string(),
            year: Some(2021),
            poster: None,
        };
        let all = suggestions(
            vec![found(Kind::Movie, 438631), found(Kind::Tv, 90228)],
            None,
            &manifest,
        );
        assert_eq!(
            all.iter()
                .map(|found| (found.tmdb, found.owned, found.season))
                .collect::<Vec<_>>(),
            vec![(438631, true, None), (90228, false, None)]
        );
        let second = suggestions(
            vec![found(Kind::Movie, 438631), found(Kind::Tv, 90228)],
            Some(2),
            &manifest,
        );
        assert_eq!(
            second
                .iter()
                .map(|found| (found.tmdb, found.season))
                .collect::<Vec<_>>(),
            vec![(90228, Some(2))]
        );
        let sent = serde_json::to_value(&second[0]).unwrap();
        assert_eq!(sent["kind"], "show");
    }

    #[test]
    fn the_progress_line_says_how_long_is_left() {
        let mib = 1024 * 1024;
        assert_eq!(
            downloading("Downloading", 10 * 1024 * mib, 20 * 1024 * mib, 8.0),
            "Downloading, 50%, about 22 min left"
        );
        assert_eq!(
            downloading("Downloading S01E02", 0, 40 * 1024 * mib, 8.0),
            "Downloading S01E02, 0%, about 1h 26m left"
        );
        assert_eq!(downloading("Downloading", 5, 10, 0.0), "Downloading, 50%");
    }

    fn file(index: usize, path: &str, size: u64) -> FileState {
        FileState {
            index,
            path: path.to_string(),
            size,
            progress: 0,
        }
    }

    fn pack(files: Vec<FileState>) -> PartState {
        PartState {
            part: Part {
                name: "Dark S01".to_string(),
                magnet: "magnet:?xt=urn:btih:ab".to_string(),
                infohash: "ab".to_string(),
                size: files.iter().map(|file| file.size).sum(),
                seeders: 10,
                episodes: Vec::new(),
                info: None,
            },
            folder: "/downloads/1/0".to_string(),
            total: files.iter().map(|file| file.size).sum(),
            files,
            phase: PartPhase::Downloading,
            progress: 0,
            uploaded: 0,
            speed: 0.0,
            peers: None,
            seen: None,
            asked: Some(2),
        }
    }

    fn dark(parts: Vec<PartState>) -> Job {
        Job {
            id: "1".to_string(),
            label: "Dark season 1".to_string(),
            created: 1,
            target: Target {
                kind: Kind::Tv,
                tmdb: 70523,
                imdb: None,
                title: "Dark".to_string(),
                original: None,
                year: Some(2017),
                season: Some(1),
                episode: None,
                runtime: None,
                episodes: (1..=3)
                    .map(|number| EpisodeInfo {
                        number,
                        name: None,
                        runtime: Some(50),
                    })
                    .collect(),
                folder: None,
            },
            choice: Choice {
                label: "Dark S01".to_string(),
                facts: String::new(),
                why: Vec::new(),
                score: 0.0,
                size: 300,
                seeders: 10,
                parts: Vec::new(),
            },
            mode: Mode::Add,
            stage: Stage::Downloading,
            message: String::new(),
            parts,
            placed: Vec::new(),
            log: Vec::new(),
            sending: None,
            by: None,
        }
    }

    #[test]
    fn a_job_saved_without_upload_progress_still_loads_and_the_upload_shows_its_bytes() {
        let mut saved = serde_json::to_value(dark(vec![season()])).unwrap();
        saved.as_object_mut().unwrap().remove("sending");
        let job: Job = serde_json::from_value(saved).unwrap();
        assert_eq!(job.sending, None);
        let mut job = job;
        job.sending = Some(Sending { sent: 40, size: 100 });
        let shown = serde_json::to_value(&job).unwrap();
        assert_eq!(shown["sending"], serde_json::json!({ "sent": 40, "size": 100 }));
    }

    #[test]
    fn a_job_saved_before_it_knew_who_asked_still_loads_and_keeps_the_person_once_known() {
        let mut saved = serde_json::to_value(dark(vec![season()])).unwrap();
        saved.as_object_mut().unwrap().remove("by");
        let job: Job = serde_json::from_value(saved).unwrap();
        assert_eq!(job.by, None);
        let asked = Job {
            by: Some("Mami".to_string()),
            ..job
        };
        let again: Job = serde_json::from_value(serde_json::to_value(&asked).unwrap()).unwrap();
        assert_eq!(again.by.as_deref(), Some("Mami"));
    }

    #[test]
    fn whoever_confirms_starts_the_download_and_a_voice_offer_keeps_the_person_who_asked() {
        let mut book = Book::default();
        let searched = |id: &str, by: Option<&str>| Proposal {
            id: id.to_string(),
            asked: "dark".to_string(),
            from: "voice".to_string(),
            by: by.map(str::to_string),
            created: now(),
            answer: Answer::Searched(Searched {
                label: "Dark season 1".to_string(),
                target: dark(Vec::new()).target,
                message: String::new(),
                choices: vec![dark(Vec::new()).choice],
                dropped: 0,
                problems: Vec::new(),
            }),
        };
        book.proposals.push(searched("tv", Some("Mami")));
        book.proposals.push(searched("nobody", None));
        let by = |picked: Result<(Target, Choice, Option<String>), &'static str>| picked.map(|(_, _, by)| by);
        assert_eq!(by(book.picked("tv", 0, None)), Ok(Some("Mami".to_string())));
        assert_eq!(
            by(book.picked("tv", 0, Some("Frederik".to_string()))),
            Ok(Some("Frederik".to_string()))
        );
        assert_eq!(by(book.picked("nobody", 0, None)), Ok(None));
        assert_eq!(
            by(book.picked("tv", 1, None)),
            Err("Nothing to download in that search")
        );
        assert_eq!(
            by(book.picked("gone", 0, None)),
            Err("That search has expired; search again")
        );
    }

    #[test]
    fn a_search_from_an_older_caller_names_nobody() {
        let body: SearchBody = serde_json::from_value(serde_json::json!({ "text": "dark", "from": "web" })).unwrap();
        assert_eq!(body.by, None);
        let fetch = serde_json::json!({
            "pick": { "kind": "movie", "tmdb": 1, "season": null, "episode": null },
            "offer": { "name": "Up.2009.1080p", "size": 1, "seeders": 1, "peers": 1, "infohash": "0", "magnet": "magnet:?xt=urn:btih:0", "indexer": "x" },
            "mode": "add",
        });
        let fetch: FetchBody = serde_json::from_value(fetch).unwrap();
        assert_eq!(fetch.by, None);
    }

    fn season() -> PartState {
        pack(vec![
            file(2, "Dark S01/Dark.S01E01.mkv", 100),
            file(0, "Dark S01/Dark.S01E02.mkv", 100),
            file(3, "Dark S01/Dark.S01E03.mkv", 100),
        ])
    }

    #[test]
    fn a_file_counts_as_fetched_once_rqbit_has_all_its_bytes_and_stays_fetched() {
        let mut part = season();
        assert_eq!(part.next_file().unwrap().index, 2);
        assert!(part.record(&[40, 0, 99, 0]).is_empty());
        assert_eq!(part.progress, 139);
        assert_eq!(part.record(&[40, 0, 100, 0]), ["Dark S01/Dark.S01E01.mkv"]);
        assert_eq!(part.next_file().unwrap().index, 0);
        assert!(part.record(&[50, 0, 0, 0]).is_empty());
        assert_eq!(
            part.progress, 150,
            "a fetched file keeps its bytes while rqbit rechecks"
        );
        assert!(part.record(&[]).is_empty());
        part.record(&[100, 0, 100, 100]);
        assert!(part.next_file().is_none());
    }

    #[test]
    fn episodes_go_to_the_library_in_order_as_soon_as_each_is_fetched() {
        let mut job = dark(vec![season()]);
        assert_eq!(job.deliverable(), Ok(None));
        job.parts[0].record(&[0, 0, 100, 0]);
        let first = job.deliverable().unwrap().unwrap();
        assert_eq!(first.destination, "Series/Dark/Season 01/Dark (2017) - S01E01.mkv");
        assert_eq!(first.local, Path::new("/downloads/1/0/Dark S01/Dark.S01E01.mkv"));
        job.placed.push(first.destination);
        assert_eq!(job.deliverable(), Ok(None));
        job.parts[0].record(&[100, 0, 100, 100]);
        let second = job.deliverable().unwrap().unwrap();
        assert_eq!(second.destination, "Series/Dark/Season 01/Dark (2017) - S01E02.mkv");
    }

    #[test]
    fn the_status_names_the_episode_being_fetched_and_how_many_are_in_the_library() {
        let mut job = dark(vec![season()]);
        job.parts[0].record(&[0, 0, 100, 0]);
        job.parts[0].speed = 0.0;
        job.placed
            .push("Series/Dark/Season 01/Dark (2017) - S01E01.mkv".to_string());
        assert_eq!(job.status(), "Downloading S01E02, 33% · 1 of 3 in the library");
        job.parts[0].phase = PartPhase::Checking;
        assert_eq!(job.status(), "Checking the pieces already downloaded");
        let mut movie = dark(vec![pack(vec![file(0, "Up.2009.mkv", 100)])]);
        movie.parts[0].record(&[25]);
        assert_eq!(movie.status(), "Downloading, 25%");
    }

    #[test]
    fn episodes_of_separate_torrents_count_before_their_files_are_known() {
        let mut one = pack(Vec::new());
        one.part.episodes = vec![1];
        let mut two = one.clone();
        two.part.episodes = vec![2];
        two.phase = PartPhase::New;
        assert_eq!(dark(vec![one, two]).count(), 2);
    }

    #[test]
    fn a_part_phase_reads_as_kebab_case() {
        assert_eq!(
            serde_json::to_value(PartPhase::Done).unwrap(),
            serde_json::json!("done")
        );
    }

    fn proposal(answer: Answer) -> serde_json::Value {
        serde_json::to_value(Proposal {
            id: "p".to_string(),
            asked: "dark season 2".to_string(),
            from: "voice".to_string(),
            by: None,
            created: 1,
            answer,
        })
        .unwrap()
    }

    #[test]
    fn a_proposal_is_refused_a_question_or_a_search_and_only_a_search_has_choices() {
        let refused = proposal(Answer::Refused {
            message: "Nothing called \"x\" on TMDB".to_string(),
        });
        assert_eq!(refused["outcome"], "refused");
        assert!(refused.get("choices").is_none() && refused.get("target").is_none());
        let question = proposal(Answer::Question(Question {
            text: "Which one: Dark (2017) or Dark (2005)?".to_string(),
            options: Vec::new(),
            season: Some(2),
            episode: None,
        }));
        assert_eq!(
            (&question["outcome"], &question["season"]),
            (&serde_json::json!("question"), &serde_json::json!(2))
        );
        assert!(question.get("choices").is_none());
        let read: Proposal = serde_json::from_value(question).unwrap();
        assert!(read.best().is_none());
        assert_eq!(read.heading(), "dark season 2");
        let mut missing = refused;
        missing["outcome"] = serde_json::json!("searched");
        assert!(serde_json::from_value::<Proposal>(missing).is_err());
    }
}
