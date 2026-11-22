use super::prepare::{self, Files, Prepared};
use super::tending::{self, Tended};
use super::{Core, Input, Outcome, Request, ScreenCommand, ScreenKind, Seek, Signal, TV_PROFILE, sound};
use crate::audio::{self, Audio};
use crate::bus::{Mate, Notice, NowPlaying, Panel, Playing, Skip, Together};
use crate::challenge::bake::{Input as Pressed, SEMMELS};
use crate::challenge::screen::Answer as Baked;
use crate::challenge::{Seat, Side, View};
use crate::command::Command;
use crate::db::{self, Db};
use crate::downloads::remote::Downloads;
use crate::downloads::{Answer, Proposal};
use crate::fetch;
use crate::hold::{self, Hold};
use crate::intent::{self, Action, Control};
use crate::library::Library;
use crate::link::Link;
use crate::link::messages::{self, Ended, FromTv, Play, Report, Running, Target, ToTv, UiRequest, UpNextCard};
use crate::offline::Store;
use crate::player::renderer::{self, Handoff};
use crate::player::{self, Finish};
use crate::profile::ProfileId;
use crate::progress::{self, Position, Progress};
use crate::screen;
use crate::seek::Direction;
use crate::segments::{self, Kind, Title};
use crate::startup::step;
use crate::storage;
use crate::stream;
use crate::subtitles::{self, SubtitleSettings};
use crate::together;
use crate::tv::{self, Television};
use crate::up_next::{self, Press, UpNext};
use crate::watching;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tokio::task::spawn_blocking;
use tokio::time::MissedTickBehavior;

const NOTICE_TIME: Duration = Duration::from_secs(6);
const OUT_OF_REACH: &str = "Not downloaded, and pCloud can't be reached right now";
const MOUNT_SILENT: &str = "Wir kriegen es gerade nicht gebacken: pCloud antwortet nicht";
const NOTHING_PLAYING: &str = "Nothing is playing";
const PLAYBACK_EVERY: Duration = Duration::from_millis(500);
const SCREEN_EVERY: Duration = Duration::from_secs(2);
const OFFER_TIME: Duration = Duration::from_secs(60);
const TV_WAKE_WAIT: Duration = Duration::from_secs(20);
const TV_PICTURE_SETTLE: Duration = Duration::from_secs(2);
const TV_STALE: Duration = Duration::from_secs(5);
const TV_LOST_FOR: Duration = Duration::from_secs(120);
const PICKED_UP_WITHIN: Duration = Duration::from_secs(600);
const PICKED_UP_AT_MOST: usize = 2;
const PICKED_UP_BEFORE: f64 = 3.0;
const TV_RESTARTING: &str = "The TV app is restarting";
const TV_ANSWER_TIME: Duration = Duration::from_secs(2);
const TV_SILENT: &str = "The TV did not answer";
const LEFT_FOR_UPDATE: &str = "left_for_update";
const UPDATE_SHOWN: &str = "The TV shows that SemmelTV is being updated";
const UPDATE_SHOWN_FOR: Duration = Duration::from_secs(2);
const UPDATE_HIDES_AFTER: Duration = Duration::from_millis(1500);
const NO_RESTART_WITHIN: Duration = Duration::from_secs(if cfg!(test) { 3 } else { 30 });
const PICKED_UP: &str = "Picked up where it stopped";
const PICKED_UP_AFTER_UPDATE: &str = "Wieder gebacken. Weiter geht's!";
const TV_SET_SUBTITLES: &str = "The TV's own player shows the subtitles: choose them with the TV's remote";

type Boxed<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

pub struct Hardware {
    pub store: Store,
    pub storage: Option<storage::Cache>,
    pub handoff: Option<Handoff>,
    pub tv: Link,
    pub segments: segments::Finder,
    pub files: Files,
    pub remove: tending::Remove,
}

pub enum Found {
    Proposal(Proposal),
    Failed(String),
}

/// Where the orders for a playback go: the tv, or the TV set's own player.
enum Backend {
    Tv(Link),
    TvSet(renderer::Session),
}

/// What the TV plays: where its orders go, what it shows, the file, whose
/// progress it moves and the last report about it. One value, so the TV
/// never plays without the rest or the rest without a playback.
struct OnTv {
    backend: Backend,
    shown: NowPlaying,
    file: PathBuf,
    owner: ProfileId,
    hold: Hold,
    play: String,
    last: Report,
    saved: f64,
    unsegmented: Option<Title>,
    lost: Option<Instant>,
    resumes: Vec<Instant>,
}

/// A playback started again on a tv that came back: its token, whether it
/// was paused, and when it was picked up before.
/// A play waiting for what its files say, as `Signal::Prepared` with its `id`.
struct Preparing {
    id: u64,
    file: PathBuf,
    start: f64,
    owner: ProfileId,
    again: Option<Again>,
}

struct Again {
    play: String,
    paused: bool,
    resumes: Vec<Instant>,
    said: &'static str,
}

/// What played on the TV when SemmelTV restarted for an update, kept in the
/// database for the next start to pick up.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Left {
    file: PathBuf,
    owner: ProfileId,
    position: f64,
    paused: bool,
    at: u64,
}

impl OnTv {
    fn send(&self, order: ToTv) {
        match &self.backend {
            Backend::Tv(link) => link.send(order),
            Backend::TvSet(session) => session.take(order),
        }
    }

    fn handed_off(&self) -> bool {
        matches!(self.backend, Backend::TvSet(_))
    }

    fn act(&mut self, (hold, act): (Hold, hold::Act)) {
        self.hold = hold;
        match act {
            hold::Act::Nothing => {}
            hold::Act::Pause => self.pause(true),
            hold::Act::Resume => self.pause(false),
        }
    }

    fn pause(&self, paused: bool) {
        self.send(ToTv::Pause {
            play: self.play.clone(),
            paused,
        });
    }

    fn mute(&self, muted: bool) {
        self.send(ToTv::Mute {
            play: self.play.clone(),
            muted,
        });
    }

    fn command(&self, command: Command) {
        self.send(ToTv::Command {
            play: self.play.clone(),
            command,
        });
    }

    fn stop(&self) {
        self.send(ToTv::Stop {
            play: self.play.clone(),
        });
    }

    fn say(&self, text: &str) {
        self.toast("", text, player::SAY_FOR);
    }

    fn toast(&self, heading: &str, status: &str, hold: Duration) {
        self.send(ToTv::Toast {
            heading: heading.to_string(),
            status: status.to_string(),
            hold_ms: u64::try_from(hold.as_millis()).unwrap_or(u64::MAX),
        });
    }

    fn playing(&self) -> bool {
        !self.last.state.paused
    }
}

pub struct Actor {
    core: Core,
    db: Db,
    library: Arc<Library>,
    profile: ProfileId,
    people: Vec<(ProfileId, String)>,
    progress: Arc<Progress>,
    on_tv: Option<OnTv>,
    store: Store,
    storage: Option<storage::Cache>,
    storage_stuck: bool,
    tending: bool,
    tend_again: bool,
    fetcher: Sender<Vec<PathBuf>>,
    lead: Option<PathBuf>,
    requested: Vec<PathBuf>,
    downloaded: Arc<HashSet<PathBuf>>,
    downloading: Option<PathBuf>,
    online: bool,
    problem: Option<String>,
    queued: Option<PathBuf>,
    choosing: Option<Sender<db::AudioPrefs>>,
    remove: tending::Remove,
    preparing: Option<Preparing>,
    prepared: u64,
    files: Files,
    up_next: Option<UpNext>,
    subtitles: SubtitleSettings,
    television: Television,
    downloads: Option<Downloads>,
    audio: Option<Audio>,
    waking_tv: Arc<AtomicBool>,
    screen: screen::Screen,
    notice: Option<screen::Said>,
    pace: audio::Pace,
    handoff: Option<Handoff>,
    tv: Link,
    segments: segments::Finder,
    questions: HashMap<u64, oneshot::Sender<Outcome>>,
    asked: u64,
    heard: Instant,
    library_revision: u64,
    library_sent: Option<u64>,
    home_sent: Option<messages::Home>,
    bake: Option<Shown>,
    update_card: Option<Instant>,
    leaving: Option<Instant>,
    left: Option<Left>,
}

/// How long the TV shows a bake-off's result before the film goes on or
/// ends by itself.
const RESULT_FOR: Duration = Duration::from_secs(10);

/// A bake-off the TV is challenged to, while it shows over the film.
struct Shown {
    id: String,
    stage: &'static str,
    /// Whether the film played when the challenge paused it.
    resume: bool,
    /// When the result showed, and whether the TV lost.
    result: Option<(Instant, bool)>,
}

/// Whether `request` steers what the TV plays, which nothing can while the
/// tv is away.
fn steers_the_tv(request: &Request) -> bool {
    match request {
        Request::Control(control) => !control.is_tv(),
        Request::PlayPause | Request::Seek(_) | Request::HideSkip | Request::Speed(_) | Request::Subtitles(_) => true,
        _ => false,
    }
}

/// Runs `work` on the headphones on a blocking thread, since it may wait
/// for their lock.
async fn audio_lock<T: Send + 'static>(audio: Audio, work: impl FnOnce(&Audio) -> T + Send + 'static) -> T {
    spawn_blocking(move || work(&audio))
        .await
        .expect("the headphones' work panicked")
}

fn asked(request: UiRequest) -> Request {
    match request {
        UiRequest::PlayFile(path) => Request::PlayFile(path),
        UiRequest::WatchWithOnTv { show, with } => Request::WatchWithOnTv { show, with },
        UiRequest::TvProfile(profile) => Request::TvProfile(profile),
        UiRequest::TogetherOnTv(show) => Request::TogetherOnTv(show),
    }
}

fn resume_point(progress: &Progress, file: &Path) -> f64 {
    progress.files.get(file).map_or(0.0, progress::Watched::resume_at)
}

impl Actor {
    pub async fn new(core: Core, profile: ProfileId, hardware: Hardware) -> Self {
        let Hardware {
            store,
            storage,
            handoff,
            tv,
            segments,
            files,
            remove,
        } = hardware;
        let db = core.db().clone();
        let library = core.library();
        let progress = step("the TV profile's progress", core.progress(profile)).await;
        let downloading = core.clone();
        let fetcher = fetch::spawn(store.clone(), move |path| downloading.signal(Signal::Downloading(path)));
        let audio = core.audio().cloned();
        if let Some(audio) = &audio {
            let claimed = step("whose headphones", sound::claimed(&db)).await;
            step(
                "the headphones' owners",
                audio_lock(audio.clone(), move |audio| audio.set_kept(claimed)),
            )
            .await;
        }
        let scanning = (store.clone(), library.clone());
        let downloaded = step(
            "what is downloaded",
            spawn_blocking(move || scanning.0.downloaded(scanning.1.paths())),
        )
        .await
        .unwrap_or_default();
        let mut actor = Self {
            subtitles: core.subtitles().clone(),
            television: core.television().clone(),
            downloads: core.downloads().cloned(),
            core,
            db,
            library,
            profile,
            people: Vec::new(),
            progress,
            on_tv: None,
            store,
            storage,
            storage_stuck: false,
            tending: false,
            tend_again: false,
            fetcher,
            lead: None,
            requested: Vec::new(),
            downloaded: Arc::new(downloaded),
            downloading: None,
            online: true,
            problem: None,
            queued: None,
            choosing: audio.clone().map(chooser),
            remove,
            preparing: None,
            prepared: 0,
            files,
            up_next: None,
            audio,
            waking_tv: Arc::new(AtomicBool::new(false)),
            screen: screen::Screen::Tv,
            notice: None,
            pace: audio::Pace::default(),
            handoff,
            tv,
            segments,
            questions: HashMap::new(),
            asked: 0,
            heard: Instant::now(),
            library_revision: 1,
            library_sent: None,
            home_sent: None,
            bake: None,
            update_card: None,
            leaving: None,
            left: None,
        };
        actor.apply_audio(profile);
        actor.people = step("the people", people(&actor.db, &actor.core)).await;
        actor.left = step("what an update left playing", left_for_update(&actor.db)).await;
        actor.publish_home();
        actor.queue_downloads(false);
        actor.tend();
        actor
    }

    pub async fn run(mut self, mut inbox: mpsc::UnboundedReceiver<Input>) {
        let mut playback = tokio::time::interval(PLAYBACK_EVERY);
        playback.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut watching_screen = tokio::time::interval(SCREEN_EVERY);
        watching_screen.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                input = inbox.recv() => match input {
                    Some(Input::Request(request, reply)) if self.tv_lost() && steers_the_tv(&request) => {
                        if let Some(reply) = reply {
                            let _ = reply.send(Outcome::refused(TV_RESTARTING));
                        }
                    }
                    Some(Input::Request(Request::Subtitles(change), reply)) if self.on_tv.is_some() => {
                        self.ask_tv_subtitles(change, reply);
                    }
                    Some(Input::Request(Request::Storage(change), reply)) => self.change_storage(change, reply),
                    Some(Input::Request(Request::WatchWithOnTv { show, with }, reply)) => {
                        self.watch_with_on_tv(show, with, reply);
                    }
                    Some(Input::Request(Request::Restarting, reply)) => self.restarting(reply).await,
                    Some(Input::Request(request, reply)) => {
                        let outcome = self.answer(request).await;
                        if let Some(reply) = reply {
                            let _ = reply.send(outcome);
                        }
                    }
                    Some(Input::Signal(signal)) => self.signalled(signal).await,
                    None => return,
                },
                _ = playback.tick() => {
                    crate::notify::alive();
                    self.tend_bakeoff();
                    if self.leaving.is_some_and(|since| since.elapsed() > NO_RESTART_WITHIN) {
                        self.play_on_without_restart().await;
                    }
                    self.tend_update_card();
                    self.publish_playback();
                    if self.on_tv.is_none() {
                        self.hear_sound();
                    }
                    if self.up_next.as_ref().is_some_and(|up| up.due(Instant::now())) {
                        self.play_up_next().await;
                    }
                    if self.on_tv.as_ref().and_then(|tv| tv.lost).is_some_and(|since| since.elapsed() > TV_LOST_FOR) {
                        eprintln!("tv: gone for {TV_LOST_FOR:?}, the playback is over");
                        self.finish_playback(Finish::Stopped).await;
                    }
                }
                _ = watching_screen.tick() => {
                    if self.screen != screen::Screen::Tv {
                        self.screen_step(screen::Event::Tick, self.profile).await;
                    }
                }
            }
        }
    }

    async fn signalled(&mut self, signal: Signal) {
        match signal {
            Signal::Tv(said) => {
                self.heard = Instant::now();
                self.tv_said(said).await;
            }
            Signal::TvGone { at } => self.tv_gone(at).await,
            Signal::TvSet { play, told } => self.tv_set_told(&play, told).await,
            Signal::Poll { online } => {
                let reconnected = online && !self.online;
                self.online = online;
                self.tend();
                if reconnected {
                    self.queue_downloads(true);
                }
            }
            Signal::Downloading(path) => {
                self.downloading = path;
                self.publish_home();
                self.tend();
            }
            Signal::Library(result) => self.reload(result).await,
            Signal::Download(found) => self.download_found(found),
            Signal::TvWaking => self.pause_for_tv(),
            Signal::TvShowing => self.resume_after_tv(),
            Signal::TvSilent => self.tell("The TV did not answer"),
            Signal::Audio(route) => self.move_audio(&route),
            Signal::Headphones(paired) => self.headphones_found(paired),
            Signal::Tended(tended) => self.tended(tended),
            Signal::Prepared { id, prepared } => self.prepared(id, prepared).await,
            Signal::Removed => self.tend(),
            Signal::Paired { show } => self.publish_together(&show),
            Signal::WatchWith {
                show,
                with,
                by,
                paired,
                reply,
            } => {
                let outcome = self.watched_with(&show, with, by, paired).await;
                if let Some(reply) = reply {
                    let _ = reply.send(outcome);
                }
            }
            Signal::Reloaded { profile, progress } => {
                if profile == self.profile {
                    self.progress = progress;
                    self.publish_home();
                }
            }
            Signal::People(people) => {
                self.people = people;
                self.publish_home();
            }
        }
    }

    async fn key(&mut self, command: Command) {
        if self.bake.as_ref().is_some_and(|shown| shown.stage != "over") {
            self.tv.send(ToTv::BakeKey(command));
            return;
        }
        if command == Command::Dialogue {
            self.toggle_dialogue().await;
            return;
        }
        if command == Command::Back && self.preparing.take().is_some() {
            eprintln!("tv player: Back before the file was found, so nothing plays");
            return;
        }
        if self.up_next.is_some() {
            match UpNext::press(command) {
                Press::PlayNow => {
                    eprintln!("up next: OK plays it now");
                    self.play_up_next().await;
                }
                Press::Cancel => {
                    eprintln!("up next: cancelled");
                    self.cancel_up_next();
                }
                Press::Nothing => {}
            }
            return;
        }
        if let Some(tv) = &self.on_tv {
            tv.command(command);
            return;
        }
        if command == Command::Subtitles {
            return;
        }
        if command == Command::Up {
            self.refresh_people();
        }
        self.tv.send(ToTv::Key(command));
    }

    async fn apply_dialogue(&mut self, by: ProfileId) -> Outcome {
        let on = self.core.dialogue(by).await.load(Ordering::SeqCst);
        let said = sound::dialogue_said(on);
        if let Some(tv) = self.on_tv.as_ref().filter(|tv| tv.owner == by) {
            tv.send(ToTv::Dialogue {
                on,
                said: said.to_string(),
            });
        }
        Outcome::done(said)
    }

    async fn toggle_dialogue(&mut self) {
        let by = self.on_tv.as_ref().map_or(self.profile, |tv| tv.owner);
        let flag = self.core.dialogue(by).await;
        let on = !flag.load(Ordering::SeqCst);
        flag.store(on, Ordering::SeqCst);
        self.core.saves().save(move |db| async move {
            if let Err(error) = db.save_dialogue(by, on).await {
                eprintln!("dialogue mode: {error}");
            }
        });
        let outcome = self.apply_dialogue(by).await;
        if self.on_tv.is_none() {
            self.show_message(outcome.message);
        }
    }

    fn asked_by(&self) -> Option<String> {
        self.people
            .iter()
            .find(|(id, _)| *id == self.profile)
            .map(|(_, name)| name.clone())
    }

    /// Sends the menu its home screen when it changed, and the library first
    /// when that changed since it was last sent.
    fn publish_home(&mut self) {
        if self.library_sent != Some(self.library_revision) {
            self.library_sent = Some(self.library_revision);
            self.home_sent = None;
            self.tv.send(ToTv::Library {
                revision: self.library_revision,
                library: (*self.library).clone(),
            });
        }
        let home = messages::Home {
            library_revision: self.library_revision,
            profile: self.profile,
            people: self.people.clone(),
            progress: (*self.progress).clone(),
            offline: self.store.is_enabled(),
            downloaded: (*self.downloaded).clone(),
            downloading: self.downloading.clone(),
            problem: self.problem.clone(),
        };
        if self.home_sent.as_ref() != Some(&home) {
            self.home_sent = Some(home.clone());
            self.tv.send(ToTv::Home(home));
        }
    }

    /// Reads who has a profile behind the writes before, and has each one's
    /// progress and dialogue mode in memory before the actor asks for them.
    fn refresh_people(&self) {
        let core = self.core.clone();
        self.core.saves().save(move |db| async move {
            let people = people(&db, &core).await;
            core.signal(Signal::People(people));
        });
    }

    async fn progress_of(&self, profile: ProfileId) -> Arc<Progress> {
        if profile == self.profile {
            self.progress.clone()
        } else {
            self.core.progress(profile).await
        }
    }

    async fn keep(&mut self, profile: ProfileId, progress: Progress) {
        let progress = Arc::new(progress);
        self.core.set_progress(profile, progress.clone());
        if profile == self.profile {
            self.progress = progress.clone();
        }
        let saved = progress.clone();
        self.core.saves().save(move |db| async move {
            if let Err(error) = db.save_progress(profile, &saved).await {
                eprintln!("progress: {error}");
            }
        });
        if profile == self.profile {
            self.publish_home();
            self.queue_downloads(false);
        }
    }

    fn job(&self) -> tending::Job {
        tending::Job {
            store: self.store.clone(),
            cache: self.storage.clone(),
            library: self.library.clone(),
            progress: self.progress.clone(),
            playing: self.on_tv.as_ref().map(|tv| tv.file.clone()),
            queued: self.requested.clone(),
            stuck: self.storage_stuck,
        }
    }

    fn tend(&mut self) {
        if self.tending {
            self.tend_again = true;
            return;
        }
        self.tending = true;
        self.tend_again = false;
        let job = self.job();
        let core = self.core.clone();
        tokio::spawn(async move { core.signal(Signal::Tended(tending::tend(job).await)) });
    }

    fn tended(&mut self, tended: Tended) {
        self.tending = false;
        self.downloaded = Arc::new(tended.downloaded);
        self.storage_stuck = tended.stuck;
        if let Some(report) = tended.report {
            self.core.published.storage.set(report);
        }
        self.publish_home();
        if self.tend_again {
            self.tend();
        }
    }

    async fn reload(&mut self, result: Result<Library, String>) {
        let library = match result {
            Ok(library) => library,
            Err(error) => {
                eprintln!("{error}");
                self.problem = Some(error);
                self.publish_home();
                return;
            }
        };
        self.problem = None;
        if *self.library == library {
            self.publish_home();
            return;
        }
        let added = crate::push::additions(&self.library, &library, watching::now());
        let push = self.core.push().clone();
        tokio::spawn(async move { push.added(added, watching::now()).await });
        let library = Arc::new(library);
        if self.db.set_library(&library) {
            let (core, profile) = (self.core.clone(), self.profile);
            self.core.saves().save(move |_| async move {
                core.forget_progress();
                let progress = core.progress(profile).await;
                core.signal(Signal::Reloaded { profile, progress });
            });
        }
        self.library = library.clone();
        self.library_revision += 1;
        self.core.set_library(library);
        self.publish_home();
        self.queue_downloads(false);
        self.tend();
    }

    async fn play(&mut self, path: &Path) {
        let start = resume_point(&self.progress, path);
        self.play_from(path, start, self.profile).await;
    }

    fn play_from<'a>(&'a mut self, path: &'a Path, start: f64, owner: ProfileId) -> Boxed<'a> {
        self.play_as(path, start, owner, None)
    }

    /// Plays `path` from `start` for `owner`, as a new playback or as one
    /// picked up `again`, once its files are found away from the actor.
    fn play_as<'a>(&'a mut self, path: &'a Path, start: f64, owner: ProfileId, again: Option<Again>) -> Boxed<'a> {
        Box::pin(async move {
            self.cancel_up_next();
            if self.screen != screen::Screen::Tv {
                self.screen_step(screen::Event::TvPlays, owner).await;
            }
            self.prepared += 1;
            let id = self.prepared;
            self.preparing = Some(Preparing {
                id,
                file: path.to_path_buf(),
                start,
                owner,
                again,
            });
            let probing = self.handoff.is_some() && self.television.renderer().is_some();
            let (files, file, online, core) = (self.files.clone(), path.to_path_buf(), self.online, self.core.clone());
            tokio::spawn(async move {
                let within = (prepare::FIND_WITHIN, prepare::PROBE_WITHIN);
                let prepared = prepare::prepare(files, file, online, probing, within).await;
                core.signal(Signal::Prepared { id, prepared });
            });
        })
    }

    /// Plays what `play_as` prepared, unless another play or a stop came since.
    async fn prepared(&mut self, id: u64, prepared: Prepared) {
        let Some(Preparing {
            file,
            start,
            owner,
            again,
            ..
        }) = self.preparing.take_if(|preparing| preparing.id == id)
        else {
            return;
        };
        let path = file.as_path();
        let picked_up = again.as_ref().map(|again| again.said);
        let (source, sidecars, media) = match prepared {
            Prepared::Ready(ready) => {
                let prepare::Ready {
                    source,
                    sidecars,
                    media,
                } = *ready;
                (source, sidecars, media)
            }
            Prepared::OutOfReach => {
                self.show_message(OUT_OF_REACH.to_string());
                return;
            }
            Prepared::Silent => {
                self.show_message(MOUNT_SILENT.to_string());
                return;
            }
        };
        self.subtitles.set_owner(owner);
        self.apply_audio(owner);
        let theirs = self.progress_of(owner).await;
        let tracks = match theirs.files.get(path) {
            Some(watched) if start > 0.0 => watched.tracks(),
            _ => progress::Tracks::default(),
        };
        let dialogue = self.core.dialogue(owner).await.load(Ordering::SeqCst);
        let (token, paused, resumes) = match again {
            Some(Again {
                play, paused, resumes, ..
            }) => (play, paused, resumes),
            None => (crate::auth::token(), false, Vec::new()),
        };
        let play = Play {
            play: token,
            owner,
            file: path.to_path_buf(),
            source: source.clone(),
            start,
            paused,
            tracks,
            caption: self.caption(path),
            subtitles: self.subtitles.for_play(owner, path, sidecars),
            audio_device: self.audio_device(),
            dialogue,
            next: up_next::follows(&self.library, path),
        };
        let backend = match self.tv_set_takes(media) {
            Some((handoff, renderer, media)) => {
                let feed = renderer::feed(&media, tracks.audio);
                eprintln!("tv player: {} goes to the TV's own player as {feed:?}", path.display());
                let start = renderer::Start {
                    renderer,
                    port: handoff.port,
                    path: source.clone(),
                    media,
                    feed,
                    from: start,
                    name: self.caption(path),
                };
                Backend::TvSet(self.tv_set(&play.play, start))
            }
            None => {
                self.tv.send(ToTv::Play(Box::new(play.clone())));
                Backend::Tv(self.tv.clone())
            }
        };
        let handed_off = matches!(backend, Backend::TvSet(_));
        self.on_tv = Some(OnTv {
            backend,
            shown: self.now_playing_for(path),
            file: path.to_path_buf(),
            owner,
            hold: Hold::default(),
            last: Report::before(&play),
            saved: play.start,
            unsegmented: if handed_off { None } else { self.title(path) },
            lost: None,
            resumes,
            play: play.play,
        });
        self.publish_playback();
        if let (Some(tv), Some(said)) = (&self.on_tv, picked_up) {
            tv.say(said);
        }
    }

    /// Hands `play` to the TV set's own player, which tells the core what it
    /// sees as `Signal::TvSet`. Dropping the session stops the TV set.
    fn tv_set(&self, play: &str, start: renderer::Start) -> renderer::Session {
        let telling = self.core.clone();
        let (waking, returning) = (self.television.clone(), self.television.clone());
        let play = play.to_string();
        renderer::Session::start(
            start,
            move || {
                if let Ok(tv::Woken::TurnedOn) = waking.wake() {
                    waking.wait_until_showing(TV_WAKE_WAIT);
                }
            },
            move |told| {
                if let renderer::Told::Ended { .. } = told
                    && let Err(failure) = returning.show_semmeltv()
                {
                    eprintln!("TV: could not switch back to SemmelTV: {failure:?}");
                }
                telling.signal(Signal::TvSet {
                    play: play.clone(),
                    told,
                });
            },
        )
    }

    /// What the TV set's player told about `play`, taken as a tv's report.
    async fn tv_set_told(&mut self, play: &str, told: renderer::Told) {
        let Some((tracks, file, owner)) = self
            .on_tv
            .as_ref()
            .filter(|tv| tv.play == play)
            .map(|tv| (tv.last.tracks, tv.file.clone(), tv.owner))
        else {
            return;
        };
        match told {
            renderer::Told::Started => self.started(play).await,
            renderer::Told::Failed(error) => self.failed(play, &error),
            renderer::Told::Seen(seen) => self.reported(seen.report(play, tracks)).await,
            renderer::Told::Ended { finish, seen } => {
                self.ended(seen.ended(play, &file, owner, finish, tracks)).await;
            }
        }
    }

    async fn started(&mut self, play: &str) {
        let Some(tv) = self.on_tv.as_ref().filter(|tv| tv.play == play) else {
            return;
        };
        let (file, owner, handed_off) = (tv.file.clone(), tv.owner, tv.handed_off());
        if let Some(until) = &mut self.update_card {
            *until = (*until).min(Instant::now() + UPDATE_HIDES_AFTER);
        }
        if !handed_off {
            self.wake_tv();
        }
        self.record_start(&file, owner).await;
        self.lead = Some(file);
        self.queue_downloads(true);
        self.tend_bakeoff();
    }

    fn failed(&mut self, play: &str, error: &str) {
        if self.on_tv.as_ref().is_none_or(|tv| tv.play != play) {
            return;
        }
        self.on_tv = None;
        self.publish_playback();
        self.show_message(format!("The video did not start: {error}"));
    }

    fn ask_tv_subtitles(&mut self, change: subtitles::Change, reply: Option<oneshot::Sender<Outcome>>) {
        let Some(tv) = &self.on_tv else {
            return;
        };
        if tv.handed_off() {
            if let Some(reply) = reply {
                let _ = reply.send(Outcome::refused(TV_SET_SUBTITLES));
            }
            return;
        }
        if let subtitles::Change::Add { video, .. } = &change
            && *video != tv.file
        {
            if let Some(reply) = reply {
                let _ = reply.send(Outcome::refused("That is not what plays now"));
            }
            return;
        }
        self.asked += 1;
        let id = self.asked;
        let (answer, answered) = oneshot::channel();
        self.questions.retain(|_, waiting| !waiting.is_closed());
        self.questions.insert(id, answer);
        tv.send(ToTv::Subtitles {
            id,
            play: tv.play.clone(),
            change,
        });
        tokio::spawn(async move {
            let outcome = match tokio::time::timeout(TV_ANSWER_TIME, answered).await {
                Ok(Ok(outcome)) => outcome,
                _ => Outcome::refused(TV_SILENT),
            };
            if let Some(reply) = reply {
                let _ = reply.send(outcome);
            }
        });
    }

    async fn tv_said(&mut self, said: FromTv) {
        match said {
            FromTv::Report(report) => self.reported(*report).await,
            FromTv::Finished(ended) => self.ended(ended).await,
            FromTv::Started { play } => self.started(&play).await,
            FromTv::Failed { play, error } => self.failed(&play, &error),
            FromTv::Key(command) => self.key(command).await,
            FromTv::Answered { id, message, ok } => {
                if let Some(answer) = self.questions.remove(&id) {
                    let _ = answer.send(Outcome { message, ok });
                }
            }
            FromTv::Remember { file, remembered } => self.subtitles.remember(file, remembered),
            FromTv::Ask(request) => {
                self.answer(asked(request)).await;
            }
            FromTv::Hello { playing, ended, .. } => self.tv_came(playing, ended).await,
            FromTv::Bakeoff(answer) => self.bake_answer(answer).await,
        }
    }

    /// Shows the bake-off the TV is challenged to as it goes: the film pauses
    /// when the challenge comes, the TV gets each new stage, and a while after
    /// the result the film goes on, or ends if the TV lost.
    fn tend_bakeoff(&mut self) {
        let now = Instant::now();
        let can_bake = self
            .on_tv
            .as_ref()
            .is_some_and(|tv| !tv.handed_off() && tv.lost.is_none());
        let found = {
            let mut arena = self.core.arena();
            arena.tv_can_bake = can_bake;
            if !can_bake {
                arena.seat_gone(&Seat::Tv, now);
            }
            arena
                .for_seat(&Seat::Tv)
                .map(str::to_string)
                .and_then(|id| arena.view(&id, Side::Defender, now).map(|view| (id, view)))
        };
        let Some((id, view)) = found else {
            if self.bake.take().is_some_and(|shown| shown.stage != "over") {
                self.tv.send(ToTv::Bakeoff(None));
            }
            return;
        };
        let stage = match &view {
            View::Asked { .. } => "asked",
            View::Baking { .. } => "baking",
            View::Done { .. } => "done",
        };
        let lost = matches!(
            view,
            View::Done {
                winner: Some(Side::Challenger),
                ..
            }
        );
        match &mut self.bake {
            Some(shown) if shown.id == id => {
                if shown.stage == "over" {
                    return;
                }
                if shown.stage != stage {
                    shown.stage = stage;
                    if stage == "done" {
                        shown.result = Some((now, lost));
                    }
                    self.tv.send(ToTv::Bakeoff(Some(view)));
                }
                if shown.result.is_some_and(|(at, _)| now >= at + RESULT_FOR) {
                    self.finish_bakeoff();
                }
            }
            _ if stage == "done" => {}
            _ => {
                let resume = self.on_tv.as_ref().is_some_and(OnTv::playing);
                if let Some(tv) = &self.on_tv {
                    tv.pause(true);
                }
                eprintln!("bake-off {}: the TV is challenged", id.get(..8).unwrap_or(&id));
                self.bake = Some(Shown {
                    id,
                    stage,
                    resume,
                    result: None,
                });
                self.tv.send(ToTv::Bakeoff(Some(view)));
            }
        }
    }

    /// Takes the bake-off off the TV: the film goes on where it paused, or
    /// ends when the TV lost.
    fn finish_bakeoff(&mut self) {
        let Some(shown) = self.bake.as_mut().filter(|shown| shown.stage == "done") else {
            return;
        };
        shown.stage = "over";
        self.tv.send(ToTv::Bakeoff(None));
        let (lost, resume) = (shown.result.is_some_and(|(_, lost)| lost), shown.resume);
        if let Some(tv) = &self.on_tv {
            if lost {
                tv.stop();
            } else if resume {
                tv.pause(false);
            }
        }
    }

    async fn bake_answer(&mut self, answer: Baked) {
        let Some(id) = self
            .bake
            .as_ref()
            .filter(|shown| shown.stage != "over")
            .map(|shown| shown.id.clone())
        else {
            return;
        };
        let now = Instant::now();
        match answer {
            Baked::Accept => {
                let mut seed = [0u8; 8];
                rand::fill(&mut seed);
                let _ = self.core.arena().accept(&id, u64::from_le_bytes(seed), now);
            }
            Baked::DropOff => {
                let _ = self.core.arena().drop_off(&id, now);
            }
            Baked::Presses { presses } => {
                let seconds: [Option<f64>; SEMMELS] =
                    presses.map(|press| press.filter(|ms| ms.is_finite()).map(|ms| ms / 1000.0));
                let pressed = self
                    .core
                    .arena()
                    .press(&id, Side::Defender, Pressed::Remote, seconds, now);
                if pressed.is_ok() {
                    stream::save_calibration(&self.core).await;
                }
            }
            Baked::Dismiss => {
                self.finish_bakeoff();
                return;
            }
        }
        self.tend_bakeoff();
    }

    fn tv_lost(&self) -> bool {
        self.on_tv.as_ref().is_some_and(|tv| tv.lost.is_some())
    }

    /// A tv connected. It gets the menu again, then the core records what
    /// ended while it was away and goes on with what plays: the playback
    /// the tv kept, or the one it lost.
    async fn tv_came(&mut self, playing: Option<Running>, ended: Option<Ended>) {
        self.library_sent = None;
        self.home_sent = None;
        self.publish_home();
        if let Some(ended) = ended {
            self.ended_away(ended).await;
        }
        if playing.is_none() && self.on_tv.is_none() {
            self.take_over_from_update();
        }
        if self.update_card.is_none() {
            self.tv.send(ToTv::Update(false));
        }
        match playing {
            Some(running) if self.on_tv.as_ref().is_some_and(|tv| tv.play == running.play) => {
                if let Some(tv) = &mut self.on_tv {
                    tv.lost = None;
                }
                eprintln!("tv: back, still playing");
                self.publish_playback();
            }
            Some(running) => self.adopt(running),
            None => self.pick_up().await,
        }
    }

    /// The tv went away. What it played keeps its place, and the core waits
    /// for it to come back.
    async fn tv_gone(&mut self, at: Instant) {
        let Some(tv) = self.on_tv.as_mut().filter(|tv| !tv.handed_off() && tv.lost.is_none()) else {
            eprintln!("tv: gone");
            return;
        };
        tv.lost = Some(at);
        eprintln!("tv: gone while it played {}", tv.file.display());
        self.record_place().await;
        self.publish_playback();
    }

    /// Records a playback that ended while the core was away.
    async fn ended_away(&mut self, ended: Ended) {
        if self.on_tv.is_none() {
            self.adopt(Running {
                play: ended.play.clone(),
                owner: ended.owner,
                file: ended.file.clone(),
                position: ended.place.position,
                paused: false,
            });
        }
        eprintln!(
            "tv: {} ended as {:?} while the core was away",
            ended.file.display(),
            ended.finish
        );
        self.ended(ended).await;
    }

    /// Takes over a playback a tv kept going while the core was away.
    fn adopt(&mut self, running: Running) {
        if self.on_tv.as_ref().is_some_and(OnTv::handed_off) {
            return;
        }
        eprintln!("tv: took over {} at {:.0} s", running.file.display(), running.position);
        self.subtitles.set_owner(running.owner);
        let last = Report {
            play: Some(running.play.clone()),
            state: player::State {
                position: running.position,
                paused: running.paused,
                speed: 1.0,
                started: true,
                ..player::State::default()
            },
            place: progress::Place {
                position: running.position,
                ..progress::Place::default()
            },
            ..Report::default()
        };
        self.on_tv = Some(OnTv {
            backend: Backend::Tv(self.tv.clone()),
            shown: self.now_playing_for(&running.file),
            unsegmented: self.title(&running.file),
            file: running.file,
            owner: running.owner,
            hold: Hold::default(),
            play: running.play,
            last,
            saved: running.position,
            lost: None,
            resumes: Vec::new(),
        });
        self.publish_playback();
    }

    /// The tv came back with nothing playing. What it lost plays again a
    /// little before where it stopped, unless it was lost long ago, was being
    /// stopped, something else waits to play, or it stopped the tv twice.
    async fn pick_up(&mut self) {
        let queued = self.queued.is_some();
        let said = if self.update_card.is_some() {
            PICKED_UP_AFTER_UPDATE
        } else {
            PICKED_UP
        };
        let Some(tv) = self.on_tv.as_mut().filter(|tv| !tv.handed_off()) else {
            return;
        };
        let now = Instant::now();
        let since = *tv.lost.get_or_insert(now);
        tv.resumes.retain(|at| now.duration_since(*at) < PICKED_UP_WITHIN);
        if queued || tv.hold.stopping() || now.duration_since(since) > TV_LOST_FOR {
            eprintln!("tv: back; what it played is over");
            self.finish_playback(Finish::Stopped).await;
            return;
        }
        if tv.resumes.len() >= PICKED_UP_AT_MOST {
            eprintln!("tv: back, but {} stopped it twice; not again", tv.file.display());
            self.finish_playback(Finish::Stopped).await;
            self.show_message("This video stopped the TV twice".to_string());
            return;
        }
        let mut resumes = tv.resumes.clone();
        resumes.push(now);
        let again = Again {
            play: tv.play.clone(),
            paused: tv.last.state.paused,
            resumes,
            said,
        };
        let (file, owner) = (tv.file.clone(), tv.owner);
        let from = (tv.last.place.position - PICKED_UP_BEFORE).max(0.0);
        eprintln!("tv: back; picking up {} at {from:.0} s", file.display());
        self.play_as(&file, from, owner, Some(again)).await;
    }

    /// SemmelTV is about to restart for an update. The TV shows the update
    /// card, and the answer waits until what played is kept and the card has
    /// been seen, before the restart takes the screen.
    async fn restarting(&mut self, reply: Option<oneshot::Sender<Outcome>>) {
        let kept = self.leave_for_update().await;
        let Some(reply) = reply else {
            return;
        };
        let Some(kept) = kept else {
            let _ = reply.send(Outcome::done(NOTHING_PLAYING));
            return;
        };
        tokio::spawn(async move {
            let _ = tokio::join!(
                tokio::time::sleep(UPDATE_SHOWN_FOR),
                tokio::time::timeout(UPDATE_SHOWN_FOR * 2, kept)
            );
            let _ = reply.send(Outcome::done(UPDATE_SHOWN));
        });
    }

    /// Keeps the place of what plays on the TV and leaves it for the next
    /// start, in memory and in the database, then shows the update card and
    /// stops the film. Gives what says when the database has it, or None
    /// when nothing played.
    async fn leave_for_update(&mut self) -> Option<oneshot::Receiver<()>> {
        let tv = self.on_tv.as_ref().filter(|tv| !tv.handed_off() && tv.lost.is_none())?;
        let left = Left {
            file: tv.file.clone(),
            owner: tv.owner,
            position: tv.last.place.position,
            paused: tv.last.state.paused,
            at: watching::now(),
        };
        self.record_place().await;
        let stored = serde_json::to_string(&left).expect("a place serialises");
        let (done, kept) = oneshot::channel();
        self.core.saves().save(move |db| async move {
            if let Err(error) = db.set_setting(LEFT_FOR_UPDATE, Some(&stored)).await {
                eprintln!("update: could not keep what plays: {error}");
            }
            let _ = done.send(());
        });
        eprintln!("update: {} left at {:.0} s", left.file.display(), left.position);
        self.left = Some(left);
        self.tv.send(ToTv::Update(true));
        if let Some(tv) = self.on_tv.take() {
            tv.stop();
        }
        self.leaving = Some(Instant::now());
        self.publish_playback();
        Some(kept)
    }

    /// The restart an update announced never came: the film plays on.
    async fn play_on_without_restart(&mut self) {
        self.leaving = None;
        eprintln!("update: no restart came within {NO_RESTART_WITHIN:?}; playing on");
        if self.on_tv.is_none() && self.preparing.is_none() {
            self.take_over_from_update();
            self.pick_up().await;
        } else {
            self.update_card = Some(Instant::now());
        }
    }

    /// Takes over what an update left playing, as a playback the tv lost,
    /// for the tv's `Hello` to pick up, and shows the update card until it
    /// plays.
    fn take_over_from_update(&mut self) {
        let Some(left) = self.left.take() else {
            return;
        };
        self.core.saves().save(|db| async move {
            if let Err(error) = db.set_setting(LEFT_FOR_UPDATE, None).await {
                eprintln!("update: {error}");
            }
        });
        let ago = Duration::from_secs(watching::now().saturating_sub(left.at));
        if ago > TV_LOST_FOR {
            eprintln!(
                "update: {} was left {ago:?} ago, too long to pick up",
                left.file.display()
            );
            return;
        }
        self.adopt(Running {
            play: crate::auth::token(),
            owner: left.owner,
            file: left.file,
            position: left.position,
            paused: left.paused,
        });
        let now = Instant::now();
        if let Some(tv) = &mut self.on_tv {
            tv.lost = Some(now.checked_sub(ago).unwrap_or(now));
        }
        self.update_card = Some(now + TV_LOST_FOR);
        self.tv.send(ToTv::Update(true));
        self.publish_playback();
    }

    /// Takes the update card away once the film plays again, or once nothing
    /// will.
    fn tend_update_card(&mut self) {
        let Some(until) = self.update_card else {
            return;
        };
        if Instant::now() >= until || (self.on_tv.is_none() && self.preparing.is_none()) {
            self.update_card = None;
            self.tv.send(ToTv::Update(false));
        }
    }

    async fn reported(&mut self, report: Report) {
        let Some(tv) = self
            .on_tv
            .as_mut()
            .filter(|tv| report.play.as_deref() == Some(tv.play.as_str()))
        else {
            return;
        };
        let due = report.state.started && player::save_due(tv.saved, report.place.position);
        if due {
            tv.saved = report.place.position;
        }
        let duration = report.place.duration;
        let unsegmented = tv.unsegmented.take_if(|_| duration > 0.0);
        tv.last = report;
        if let (Some(title), Backend::Tv(link)) = (unsegmented, &tv.backend) {
            let (find, link, play) = (self.segments.clone(), link.clone(), tv.play.clone());
            tokio::spawn(async move {
                match spawn_blocking(move || find(title, duration)).await {
                    Ok(Ok(found)) => link.send(ToTv::Segments {
                        play,
                        segments: found.segments,
                        credits: found.credits,
                    }),
                    Ok(Err(error)) => eprintln!("segments: {error}"),
                    Err(error) => eprintln!("segments: {error}"),
                }
            });
        }
        self.publish_playback();
        self.hear_sound();
        if due {
            self.record_place().await;
        }
    }

    async fn ended(&mut self, ended: Ended) {
        let Some(tv) = self.on_tv.as_mut().filter(|tv| tv.play == ended.play) else {
            return;
        };
        tv.last.place = ended.place;
        tv.last.tracks = ended.tracks;
        tv.last.played = ended.played;
        self.finish_playback(ended.finish).await;
    }

    fn tv_set_takes(
        &self,
        media: Option<stream::probe::Media>,
    ) -> Option<(Handoff, tv::upnp::Renderer, stream::probe::Media)> {
        let handoff = self.handoff.clone()?;
        let renderer = self.television.renderer()?;
        media
            .filter(|media| handoff.takes(media))
            .map(|media| (handoff, renderer, media))
    }

    fn audio_device(&self) -> String {
        self.audio
            .as_ref()
            .map_or_else(|| audio::TV_DEVICE.to_string(), Audio::device)
    }

    fn move_audio(&self, route: &audio::Route) {
        if let Some(tv) = &self.on_tv {
            tv.send(ToTv::AudioDevice {
                device: route.device.clone(),
                said: route.said.clone(),
            });
        }
    }

    fn queue_downloads(&mut self, again: bool) {
        if !self.store.is_enabled() || !self.online {
            return;
        }
        let paths = watching::downloads(&self.library, &self.progress, self.lead.as_deref());
        if again || paths != self.requested {
            self.requested = paths.clone();
            let _ = self.fetcher.send(paths);
        }
    }

    fn caption(&self, path: &Path) -> String {
        if let Some((show, episode)) = self.library.find_episode(path) {
            return format!("{} · {}", show.title, episode.label());
        }
        let movie = self.library.movies.iter().find(|movie| movie.path == path);
        movie.map_or_else(|| path.display().to_string(), |movie| movie.title.clone())
    }

    fn title(&self, path: &Path) -> Option<Title> {
        if let Some((show, episode)) = self.library.find_episode(path) {
            return Some(Title {
                tmdb: show.tmdb?,
                episode: Some((episode.season, episode.number)),
            });
        }
        let movie = self.library.movies.iter().find(|movie| movie.path == path)?;
        Some(Title {
            tmdb: movie.tmdb?,
            episode: None,
        })
    }

    async fn record_start(&mut self, path: &Path, owner: ProfileId) {
        let mut progress = (*self.progress_of(owner).await).clone();
        if let Some((show, episode)) = self.library.find_episode(path) {
            let position = Position {
                season: episode.season,
                episode: episode.number,
                finished: false,
            };
            progress.shows.insert(show.key.clone(), position);
        }
        progress.played(path, watching::now());
        self.keep(owner, progress).await;
        self.tell_partners(owner, path);
        self.tell_owner(owner, path);
        self.note_tv_play().await;
    }

    fn tell_owner(&self, owner: ProfileId, path: &Path) {
        let (push, library, path) = (self.core.push().clone(), self.library.clone(), path.to_path_buf());
        tokio::spawn(async move { push.playing_on_tv(owner, &library, &path, watching::now()).await });
    }

    fn tell_partners(&self, player: ProfileId, path: &Path) {
        let Some((show, episode)) = self.library.find_episode(path) else {
            return;
        };
        let (push, show, episode) = (self.core.push().clone(), show.clone(), episode.clone());
        tokio::spawn(async move { push.carried_on(player, &show, &episode, watching::now()).await });
    }

    async fn note_tv_play(&self) {
        let Some(tv) = &self.on_tv else {
            return;
        };
        let (play, owner, caption, played) = (tv.play.clone(), tv.owner, self.caption(&tv.file), tv.last.played);
        self.core.saves().save(move |db| async move {
            if let Err(error) = db
                .note_play(&play, watching::now(), owner, "TV", &caption, played)
                .await
            {
                eprintln!("usage: {error}");
            }
        });
    }

    async fn record_place(&mut self) {
        let Some(OnTv { file, owner, last, .. }) = &self.on_tv else {
            return;
        };
        let (file, owner) = (file.clone(), *owner);
        let (place, tracks, played) = (last.place, last.tracks, last.played);
        let mut progress = (*self.progress_of(owner).await).clone();
        progress.record(&file, place);
        progress.keep_tracks(&file, tracks);
        progress.played(&file, watching::now());
        watching::record(&mut progress, &self.library, &file, played, watching::now());
        self.keep(owner, progress).await;
        self.note_tv_play().await;
    }

    async fn finish_playback(&mut self, finish: Finish) {
        let ended = matches!(finish, Finish::Ended | Finish::Next);
        self.record_place().await;
        let ended_play = self.on_tv.take().map(|OnTv { file, owner, .. }| (file, owner));
        self.publish_playback();
        if let Some((file, owner)) = &ended_play {
            let (file, owner) = (file.as_path(), *owner);
            let mut progress = (*self.progress_of(owner).await).clone();
            let changed = if ended {
                watching::complete(&mut progress, &self.library, file);
                true
            } else {
                watching::stopped(&mut progress, &self.library, file)
            };
            if changed {
                self.keep(owner, progress).await;
            }
        }
        self.subtitles.set_owner(self.profile);
        if finish == Finish::Failed {
            self.show_message("This video could not be played".to_string());
        }
        self.publish_home();
        self.queue_downloads(false);
        self.tend();
        if let Some(path) = self.queued.take() {
            self.play(&path).await;
        } else if let Some((file, owner)) = ended_play.filter(|_| ended) {
            self.up_next = UpNext::after(&self.library, &file, owner, Instant::now());
            if finish == Finish::Next {
                eprintln!("up next: Play next plays it now");
                self.play_up_next().await;
            } else if let Some(up) = &self.up_next {
                eprintln!("up next: {} in {:?}", up.file.display(), *crate::up_next::COUNTDOWN);
                self.tv
                    .send(ToTv::UpNext(Some(UpNextCard::of(&up.card, Instant::now()))));
            }
        }
    }

    async fn play_up_next(&mut self) {
        let Some(UpNext { file, owner, .. }) = self.up_next.take() else {
            return;
        };
        self.tv.send(ToTv::UpNext(None));
        let start = resume_point(&*self.progress_of(owner).await, &file);
        self.play_from(&file, start, owner).await;
    }

    fn cancel_up_next(&mut self) {
        if self.up_next.take().is_some() {
            self.tv.send(ToTv::UpNext(None));
        }
    }

    async fn answer(&mut self, request: Request) -> Outcome {
        let action = match request {
            Request::Storage(change) => {
                self.change_storage(change, None);
                return Outcome::done("Removing it from pCloud");
            }
            Request::Restarting => {
                let shown = self.leave_for_update().await.is_some();
                return Outcome::done(if shown { UPDATE_SHOWN } else { NOTHING_PLAYING });
            }
            Request::Understand(text, by) => {
                let progress = self.progress_of(by).await;
                let action = intent::understand(&text, &self.library, &progress);
                if matches!(action, Action::Play(_)) {
                    self.become_profile(by).await;
                }
                action
            }
            Request::Play(pick, by) => {
                let progress = self.progress_of(by).await;
                let action = intent::choose(&pick, &self.library, &progress);
                if matches!(action, Action::Play(_)) {
                    self.become_profile(by).await;
                }
                action
            }
            Request::PlayFile(path) => Action::Play(path),
            Request::Key(command) => {
                self.key(command).await;
                return Outcome::done("");
            }
            Request::Control(control) => Action::Control(control),
            Request::PlayPause => {
                let paused = self.on_tv.as_ref().is_some_and(|tv| tv.last.state.paused);
                Action::Control(if paused { Control::Resume } else { Control::Pause })
            }
            Request::Seek(seek) => return self.seek(seek),
            Request::HideSkip => return self.hide_skip(),
            Request::Speed(speed) => return self.speed(speed),
            Request::Subtitles(change) => return self.change_subtitles(change),
            Request::Dismiss(kind, key, by) => return self.dismiss(kind, &key, by).await,
            Request::Mark(kind, key, episode, mark, by) => return self.mark(kind, &key, episode, mark, by).await,
            Request::CatchUp { show, to, by } => return self.catch_up(&show, to, by).await,
            Request::TogetherOnTv(show) => {
                self.publish_together(&show);
                return Outcome::done("");
            }
            Request::WatchWithOnTv { show, with } => {
                self.watch_with_on_tv(show, with, None);
                return Outcome::done("");
            }
            Request::Phone(report) => return self.phone(report).await,
            Request::Screen(command, by) => {
                if matches!(command, ScreenCommand::Flip { to: ScreenKind::Tv, .. }) {
                    self.become_profile(by).await;
                }
                return self.screen_command(command, by).await;
            }
            Request::TvProfile(by) => return self.show_profile(by).await,
            Request::Dialogue(by) => return self.apply_dialogue(by).await,
        };
        let message = self.describe(&action);
        let ok = match &action {
            Action::Play(_) => true,
            Action::Control(control) if control.is_tv() => true,
            Action::Control(_) => self.on_tv.is_some(),
            Action::Download { .. } => self.downloads.is_some(),
            Action::Refuse(_) => false,
        };
        self.act(action).await;
        self.publish_playback();
        Outcome { message, ok }
    }

    async fn dismiss(&mut self, kind: watching::Kind, key: &str, by: ProfileId) -> Outcome {
        let mut progress = (*self.progress_of(by).await).clone();
        if !watching::dismiss(&mut progress, &self.library, kind, key, watching::now()) {
            return Outcome::refused(format!("There is no {key} in the library"));
        }
        self.keep(by, progress).await;
        Outcome::done("Removed from Started and Forgotten")
    }

    async fn mark(
        &mut self,
        kind: watching::Kind,
        key: &str,
        episode: Option<(u32, u32)>,
        mark: watching::Mark,
        by: ProfileId,
    ) -> Outcome {
        let mut progress = (*self.progress_of(by).await).clone();
        let library = self.library.clone();
        if watching::mark(&mut progress, &library, kind, key, episode, mark).is_none() {
            return Outcome::refused(format!("There is no {key} in the library"));
        }
        let name = match (kind, episode) {
            (watching::Kind::Movie, _) => library
                .movies
                .iter()
                .find(|movie| movie.key == key)
                .map(|movie| movie.title.clone()),
            (watching::Kind::Series, None) => library
                .shows
                .iter()
                .find(|show| show.key == key)
                .map(|show| show.title.clone()),
            (watching::Kind::Series, Some((season, number))) => {
                library.shows.iter().find(|show| show.key == key).and_then(|show| {
                    Some(format!(
                        "{} · {}",
                        show.title,
                        show.episode(season, number)?.short_label()
                    ))
                })
            }
        }
        .unwrap_or_else(|| key.to_string());
        self.keep(by, progress).await;
        let state = match mark {
            watching::Mark::Watched => "watched",
            watching::Mark::Unwatched => "unwatched",
        };
        Outcome::done(format!("{name} is marked as {state}"))
    }

    async fn catch_up(&mut self, key: &str, to: ProfileId, by: ProfileId) -> Outcome {
        let library = self.library.clone();
        let Some(show) = library.shows.iter().find(|show| show.key == key) else {
            return Outcome::refused(format!("There is no {key} in the library"));
        };
        let name = self.name_of(to);
        let theirs = self.progress_of(to).await;
        let mut mine = (*self.progress_of(by).await).clone();
        if !together::catch_up(&mut mine, &theirs, show, watching::now()) {
            return Outcome::refused(format!("{name} hasn't started {} yet", show.title));
        }
        let place = match mine.resume_episode(show) {
            Some(episode) => format!("{} · {}", show.title, episode.short_label()),
            None => format!("the end of {}", show.title),
        };
        self.keep(by, mine).await;
        Outcome::done(format!("You're at {name}'s place now: {place}"))
    }

    /// Sends the tv who watches `key` together with the TV's profile, read
    /// from the database behind the writes before it.
    fn publish_together(&self, key: &str) {
        let (core, library, profile, mine, tv, key) = (
            self.core.clone(),
            self.library.clone(),
            self.profile,
            self.progress.clone(),
            self.tv.clone(),
            key.to_string(),
        );
        self.core.saves().save(move |db| async move {
            if let Some(together) = together_on_tv(&db, &core, &library, profile, &mine, &key).await {
                tv.send(ToTv::Together(together));
            }
        });
    }

    /// Pairs the TV's profile with `with` for `key`, or moves it to their
    /// place when they already are a pair, once the database says which.
    fn watch_with_on_tv(&self, show: String, with: ProfileId, reply: Option<oneshot::Sender<Outcome>>) {
        let (by, core) = (self.profile, self.core.clone());
        self.core.saves().save(move |db| async move {
            let paired = db
                .partners(by, &show)
                .await
                .is_ok_and(|partners| partners.contains(&with));
            core.signal(Signal::WatchWith {
                show,
                with,
                by,
                paired,
                reply,
            });
        });
    }

    async fn watched_with(&mut self, show: &str, with: ProfileId, by: ProfileId, paired: bool) -> Outcome {
        let outcome = if paired {
            let outcome = self.catch_up(show, with, by).await;
            self.publish_together(show);
            outcome
        } else {
            self.pair(show, with, by)
        };
        self.show_message(outcome.message.clone());
        outcome
    }

    fn pair(&self, key: &str, with: ProfileId, by: ProfileId) -> Outcome {
        let Some(show) = self.library.shows.iter().find(|show| show.key == key) else {
            return Outcome::refused(format!("There is no {key} in the library"));
        };
        let name = self.name_of(with);
        if by == with {
            return Outcome::refused(format!("You can't watch {} with {name}", show.title));
        }
        let (core, key) = (self.core.clone(), key.to_string());
        self.core.saves().save(move |db| async move {
            match db.set_together(by, with, &key, true).await {
                Ok(true) => core.signal(Signal::Paired { show: key }),
                Ok(false) => eprintln!("together: {} with {} in {key} was refused", by.0, with.0),
                Err(error) => eprintln!("together: {} with {}: {error}", by.0, with.0),
            }
        });
        Outcome::done(format!("You watch {} together with {name}", show.title))
    }

    fn name_of(&self, profile: ProfileId) -> String {
        self.people
            .iter()
            .find(|(id, _)| *id == profile)
            .map_or_else(|| "them".to_string(), |(_, name)| name.clone())
    }

    fn place_of(&self, file: &Path, position: f64) -> Option<screen::Place> {
        if let Some((show, episode)) = self.library.find_episode(file) {
            return Some(screen::Place {
                title: screen::Title::Episode {
                    show: show.key.clone(),
                    season: episode.season,
                    episode: episode.number,
                },
                name: show.title.clone(),
                caption: episode.label(),
                file: file.to_path_buf(),
                position,
            });
        }
        let movie = self.library.movies.iter().find(|movie| movie.path == file)?;
        Some(screen::Place {
            title: screen::Title::Movie {
                movie: movie.key.clone(),
            },
            name: movie.title.clone(),
            caption: movie.year.map(|year| year.to_string()).unwrap_or_default(),
            file: file.to_path_buf(),
            position,
        })
    }

    fn tv_place(&self) -> Option<screen::Place> {
        let tv = self.on_tv.as_ref().filter(|tv| !tv.hold.stopping())?;
        self.place_of(&tv.file, tv.last.place.position)
    }

    async fn screen_step(&mut self, event: screen::Event, by: ProfileId) {
        let tv = self.tv_place();
        let before = self.screen.clone();
        let reason = event.describe();
        let tick = event.is_tick();
        let effects = self.screen.step(event, tv.as_ref(), Instant::now());
        if self.screen != screen::Screen::Tv {
            self.cancel_up_next();
        }
        if before.summary() != self.screen.summary() {
            eprintln!("screen: {} -> {} ({reason})", before.summary(), self.screen.summary());
        } else if before == self.screen && !tick {
            eprintln!("screen: ignored {reason}; it stays {}", self.screen.summary());
        }
        for effect in effects {
            match effect {
                screen::Effect::PauseTv => {
                    if let Some(tv) = &mut self.on_tv {
                        let playing = tv.playing();
                        tv.act(tv.hold.hand_off(playing));
                    }
                }
                screen::Effect::ResumeTv => {
                    if let Some(tv) = &mut self.on_tv {
                        let (hold, act) = tv.hold.hand_back();
                        tv.hold = hold;
                        if act == hold::Act::Resume {
                            self.control(Control::Resume);
                        }
                    }
                }
                screen::Effect::StopTv => {
                    if let Some(tv) = &mut self.on_tv {
                        tv.hold = Hold::Stopping;
                    }
                    self.control(Control::Stop);
                }
                screen::Effect::PlayTv(place) => {
                    let start = if place.position > 1.0 {
                        place.position
                    } else {
                        resume_point(&*self.progress_of(by).await, &place.file)
                    };
                    self.play_from(&place.file, start, by).await;
                }
                screen::Effect::ShowOnTv(text) => self.show_message(text),
                screen::Effect::Tell(notice) => {
                    eprintln!("screen: told phones \"{}\"", notice.text);
                    let id = self.notice.as_ref().map_or(1, |said| said.id + 1);
                    self.notice = Some(screen::Said {
                        id,
                        text: notice.text,
                        tone: notice.tone,
                    });
                }
            }
        }
        self.core
            .bus
            .screen(screen::View::of(&self.screen, self.notice.clone()));
    }

    async fn screen_command(&mut self, command: ScreenCommand, by: ProfileId) -> Outcome {
        match command {
            ScreenCommand::Flip {
                to: ScreenKind::Tv,
                position,
                ..
            } => {
                let event = screen::Event::Flip {
                    to: screen::Target::Tv,
                    position,
                };
                self.screen_step(event, by).await;
                Outcome::done("Watching on the TV")
            }
            ScreenCommand::Flip {
                to: ScreenKind::Phone,
                device,
                ..
            } => {
                let name = device.name.clone();
                let event = screen::Event::Flip {
                    to: screen::Target::Phone(device),
                    position: None,
                };
                self.screen_step(event, by).await;
                Outcome::done(format!("Watching on {name}"))
            }
            ScreenCommand::Play {
                device,
                pick,
                takes_over,
            } => {
                if !takes_over && !self.screen.free_for(&device, self.tv_place().as_ref()) {
                    return Outcome::refused("Something else plays now");
                }
                let place = match self.screen_place(&pick, None, by).await {
                    Ok(place) => place,
                    Err(refusal) => return refusal,
                };
                let message = format!("Playing {} on {}", place.name, device.name);
                self.screen_step(screen::Event::Play { device, place }, by).await;
                Outcome::done(message)
            }
            ScreenCommand::Resume { device, pick, position } => {
                let place = match self.screen_place(&pick, Some(position), by).await {
                    Ok(place) => place,
                    Err(refusal) => return refusal,
                };
                let message = format!("Playing {} on {} again", place.name, device.name);
                let event = screen::Event::Resume {
                    device: device.clone(),
                    place,
                };
                self.screen_step(event, by).await;
                match &self.screen {
                    screen::Screen::Phone {
                        device: holder,
                        play: Some(screen::PhonePlay::Starting { .. }),
                    } if *holder == device => Outcome::done(message),
                    _ => Outcome::refused("Something else plays now"),
                }
            }
            ScreenCommand::Stop { device } => {
                self.screen_step(screen::Event::Stop { device }, by).await;
                Outcome::done("Stopped")
            }
            ScreenCommand::Failed { device, reason } => {
                self.screen_step(screen::Event::Failed { device, reason }, by).await;
                Outcome::done("Noted")
            }
        }
    }

    async fn screen_place(
        &self,
        pick: &intent::Pick,
        position: Option<f64>,
        by: ProfileId,
    ) -> Result<screen::Place, Outcome> {
        let progress = self.progress_of(by).await;
        let path = match intent::choose(pick, &self.library, &progress) {
            Action::Play(path) => path,
            other => return Err(Outcome::refused(self.describe(&other))),
        };
        let position = position
            .filter(|position| position.is_finite() && *position >= 0.0)
            .unwrap_or_else(|| resume_point(&progress, &path));
        self.place_of(&path, position)
            .ok_or_else(|| Outcome::refused("That is not in the library"))
    }

    async fn phone(&mut self, report: stream::Report) -> Outcome {
        let by = report.profile;
        if let Some(device) = report.device.clone() {
            let session = report.session.clone();
            let event = match report.moment {
                stream::Moment::Start => Some(screen::Event::Started {
                    device,
                    session,
                    position: report.position,
                }),
                stream::Moment::Playing | stream::Moment::Paused => {
                    self.place_of(&report.file, report.position)
                        .map(|place| screen::Event::Report {
                            device,
                            session,
                            place,
                            paused: report.moment == stream::Moment::Paused,
                        })
                }
                stream::Moment::Stopped | stream::Moment::Ended => Some(screen::Event::Stopped { device, session }),
            };
            match event {
                Some(event) => self.screen_step(event, by).await,
                None => eprintln!("screen: a report for {} is not in the library", report.file.display()),
            }
        }
        if self.on_tv.as_ref().is_some_and(|tv| tv.file == report.file) {
            eprintln!(
                "stream {}: {:?} at {:.0} s not recorded, the TV plays {}",
                report.session.get(..8).unwrap_or(&report.session),
                report.moment,
                report.position,
                report.file.display()
            );
            return Outcome::refused("The TV is playing this, so it keeps its own place");
        }
        let file = report.file.as_path();
        let now = watching::now();
        let mut progress = (*self.progress_of(by).await).clone();
        if report.moment == stream::Moment::Start {
            self.tell_partners(by, file);
        }
        if report.moment == stream::Moment::Start
            && let Some((show, episode)) = self.library.find_episode(file)
        {
            let position = Position {
                season: episode.season,
                episode: episode.number,
                finished: false,
            };
            progress.shows.insert(show.key.clone(), position);
        }
        let place = progress::Place {
            position: report.position,
            duration: report.duration,
            credits: report.credits,
        };
        progress.record(file, place);
        progress.played(file, now);
        watching::record(&mut progress, &self.library, file, report.played, now);
        match report.moment {
            stream::Moment::Ended => watching::complete(&mut progress, &self.library, file),
            stream::Moment::Stopped => {
                watching::stopped(&mut progress, &self.library, file);
            }
            _ => {}
        }
        self.keep(by, progress).await;
        let device = report.device.as_ref().map_or("Phone", |device| device.name.as_str());
        let play = format!("phone-{}", report.session);
        let noted = self
            .db
            .note_play(&play, now, by, device, &self.caption(file), report.played)
            .await;
        if let Err(error) = noted {
            eprintln!("usage: {error}");
        }
        Outcome::done("Kept the place")
    }

    /// Removes a title from pCloud on a task of its own, which answers when
    /// pCloud is done.
    fn change_storage(&self, change: storage::Change, reply: Option<oneshot::Sender<Outcome>>) {
        let storage::Change::Remove(title) = change;
        let (job, core, remove) = (self.job(), self.core.clone(), self.remove.clone());
        tokio::spawn(async move {
            let outcome = remove(job, title).await;
            if outcome.ok {
                core.signal(Signal::Removed);
            }
            if let Some(reply) = reply {
                let _ = reply.send(outcome);
            }
        });
    }

    fn change_subtitles(&self, change: subtitles::Change) -> Outcome {
        match change {
            subtitles::Change::Restyle(_) => Outcome::done("Saved"),
            _ => Outcome::refused(NOTHING_PLAYING),
        }
    }

    fn speed(&self, speed: f64) -> Outcome {
        let Some(tv) = &self.on_tv else {
            return Outcome::refused(NOTHING_PLAYING);
        };
        let speed = if tv.handed_off() {
            1.0
        } else {
            player::speed_within(speed)
        };
        tv.send(ToTv::Speed {
            play: tv.play.clone(),
            speed,
        });
        let said = format!("Speed {}", player::speed_label(speed));
        tv.say(&said);
        Outcome::done(said)
    }

    fn seek(&self, seek: Seek) -> Outcome {
        let Some(tv) = &self.on_tv else {
            return Outcome::refused(NOTHING_PLAYING);
        };
        let seek_to = |target| {
            tv.send(ToTv::Seek {
                play: tv.play.clone(),
                target,
            });
        };
        match seek {
            Seek::To(seconds) => seek_to(Target::To(seconds)),
            Seek::By(seconds) => seek_to(Target::By(seconds)),
            Seek::Step(Direction::Back) => tv.command(Command::Left),
            Seek::Step(Direction::Forward) => tv.command(Command::Right),
            Seek::Segment => return self.skip_segment(),
        }
        Outcome::done("Seeking")
    }

    fn skip_segment(&self) -> Outcome {
        let Some((tv, kind)) = self.on_tv.as_ref().and_then(|tv| Some((tv, tv.last.state.skip?))) else {
            return Outcome::refused("The TV shows nothing to skip right now");
        };
        tv.send(ToTv::SkipSegment { play: tv.play.clone() });
        Outcome::done(match kind {
            Kind::Intro => "Skipped the intro",
            Kind::Recap => "Skipped the recap",
            Kind::Next => "Playing the next episode",
        })
    }

    fn hide_skip(&self) -> Outcome {
        let Some(tv) = self.on_tv.as_ref().filter(|tv| tv.last.state.skip.is_some()) else {
            return Outcome::refused("The TV shows nothing to skip right now");
        };
        tv.send(ToTv::HideSkip { play: tv.play.clone() });
        Outcome::done("")
    }

    fn now_playing_for(&self, path: &Path) -> NowPlaying {
        if let Some((show, episode)) = self.library.find_episode(path) {
            return NowPlaying {
                title: show.title.clone(),
                caption: format!("{} · {}", episode.short_label(), episode.label()),
                playing: Playing::Episode {
                    show: show.key.clone(),
                    season: episode.season,
                    episode: episode.number,
                },
                loading: true,
                ..NowPlaying::default()
            };
        }
        let movie = self.library.movies.iter().find(|movie| movie.path == path);
        NowPlaying {
            title: movie.map_or_else(|| path.display().to_string(), |movie| movie.title.clone()),
            caption: movie
                .and_then(|movie| movie.year)
                .map(|year| year.to_string())
                .unwrap_or_default(),
            playing: movie.map_or(Playing::File, |movie| Playing::Movie {
                movie: movie.key.clone(),
            }),
            loading: true,
            ..NowPlaying::default()
        }
    }

    fn hear_sound(&mut self) {
        let Some(audio) = &self.audio else {
            return;
        };
        let playing = self
            .on_tv
            .as_ref()
            .map(|on_tv| on_tv.last.state)
            .filter(|state| state.started && !state.paused)
            .map(|state| (state.position, state.speed));
        let now = Instant::now();
        audio.heard(self.pace.observe(now, playing), now);
    }

    fn publish_playback(&self) {
        let stale = self.heard.elapsed() > TV_STALE;
        let now_playing = self.on_tv.as_ref().map(|tv| {
            let (state, base) = (tv.last.state, &tv.shown);
            let away = tv.lost.is_some() || (stale && !tv.handed_off());
            NowPlaying {
                position: state.position,
                duration: state.duration,
                paused: state.paused,
                muted: state.muted,
                speed: state.speed,
                subtitle: state.subtitle,
                subtitle_delay: state.subtitle_delay,
                audio: state.audio,
                loading: !state.started || away,
                jump: state.jump,
                skip: state.skip.map(Skip::from),
                ..base.clone()
            }
        });
        self.core.bus.playback(now_playing);
        let subtitles = self.on_tv.as_ref().and_then(|tv| tv.last.subtitles.clone());
        self.subtitles.publish(subtitles);
    }

    fn describe(&self, action: &Action) -> String {
        match action {
            Action::Play(path) => {
                let title = self
                    .library
                    .find_episode(path)
                    .map(|(show, episode)| {
                        format!("{} · season {}, episode {}", show.title, episode.season, episode.number)
                    })
                    .or_else(|| {
                        let movie = self.library.movies.iter().find(|movie| &movie.path == path)?;
                        Some(movie.title.clone())
                    })
                    .unwrap_or_else(|| path.display().to_string());
                format!("Playing {title}")
            }
            Action::Download { title, season, episode } => {
                let what = match (season, episode) {
                    (Some(season), Some(episode)) => format!("{title} season {season} episode {episode}"),
                    (Some(season), None) => format!("{title} season {season}"),
                    _ => title.clone(),
                };
                match self.downloads {
                    Some(_) => format!("Looking for {what}…"),
                    None => format!("Download {what} (downloading is not set up)"),
                }
            }
            Action::Refuse(reason) => reason.clone(),
            Action::Control(control) => {
                let name = match control {
                    Control::Pause => "Pause",
                    Control::Resume => "Resume",
                    Control::Stop => "Stop",
                    Control::Mute => "Mute",
                    Control::Unmute => "Unmute",
                    Control::TvOn => return "Turn the TV on".to_string(),
                    Control::TvOff => return "Turn the TV off".to_string(),
                    Control::ShowSemmelTv => return "Switch the TV to SemmelTV".to_string(),
                };
                match self.on_tv {
                    Some(_) => name.to_string(),
                    None => format!("{name}, but {}", NOTHING_PLAYING.to_lowercase()),
                }
            }
        }
    }

    async fn act(&mut self, action: Action) {
        match action {
            Action::Play(path) => match &self.on_tv {
                Some(tv) => {
                    tv.stop();
                    self.queued = Some(path);
                }
                None => self.play(&path).await,
            },
            Action::Download { title, season, episode } => self.look_for(title, season, episode),
            Action::Refuse(_) => {}
            Action::Control(control) => self.control(control),
        }
    }

    fn control(&mut self, control: Control) {
        if control == Control::Stop {
            self.preparing = None;
        }
        if let (Control::Pause | Control::Resume, Some(tv)) = (control, &mut self.on_tv) {
            tv.hold = tv.hold.viewer_chose();
        }
        match (control, &self.on_tv) {
            (Control::TvOn | Control::TvOff | Control::ShowSemmelTv, _) => {
                let television = self.television.clone();
                let core = self.core.clone();
                spawn_blocking(move || {
                    let sent = if control == Control::TvOff {
                        television.turn_off()
                    } else {
                        television.show_semmeltv()
                    };
                    if let Err(failure) = sent {
                        eprintln!("TV: {failure:?}");
                        core.signal(Signal::TvSilent);
                    }
                });
            }
            (_, None) => {}
            (Control::Pause, Some(tv)) => tv.pause(true),
            (Control::Resume, Some(tv)) => {
                tv.pause(false);
                self.wake_tv();
            }
            (Control::Stop, Some(tv)) => tv.stop(),
            (Control::Mute, Some(tv)) => tv.mute(true),
            (Control::Unmute, Some(tv)) => tv.mute(false),
        }
    }

    fn wake_tv(&self) {
        if self.waking_tv.swap(true, Ordering::SeqCst) {
            return;
        }
        let television = self.television.clone();
        let core = self.core.clone();
        let waking = self.waking_tv.clone();
        spawn_blocking(move || {
            match television.wake() {
                Ok(tv::Woken::AlreadyShowing) => {}
                Ok(tv::Woken::Switched) => eprintln!("TV: on another input at playback, switching to SemmelTV"),
                Ok(tv::Woken::TurnedOn) => {
                    eprintln!("TV: off at playback, turning it on and pausing until it shows SemmelTV");
                    core.signal(Signal::TvWaking);
                    if television.wait_until_showing(TV_WAKE_WAIT) {
                        std::thread::sleep(TV_PICTURE_SETTLE);
                    } else {
                        eprintln!("TV: still not showing SemmelTV after {TV_WAKE_WAIT:?}, playing anyway");
                    }
                    core.signal(Signal::TvShowing);
                }
                Err(failure) => eprintln!("TV: could not turn it on for playback: {failure:?}"),
            }
            waking.store(false, Ordering::SeqCst);
        });
    }

    fn pause_for_tv(&mut self) {
        if let Some(tv) = &mut self.on_tv {
            let playing = tv.playing();
            tv.act(tv.hold.tv_waking(playing));
        }
    }

    fn resume_after_tv(&mut self) {
        if let Some(tv) = &mut self.on_tv {
            tv.act(tv.hold.tv_showing());
        }
    }

    fn look_for(&mut self, title: String, season: Option<u32>, episode: Option<u32>) {
        let Some(downloads) = self.downloads.clone() else {
            return;
        };
        let core = self.core.clone();
        let said = match season {
            Some(season) => format!("{title} season {season}"),
            None => title.clone(),
        };
        let by = self.asked_by();
        spawn_blocking(move || {
            let found = match downloads.search(title, season, episode, said, by) {
                Ok(proposal) => Found::Proposal(proposal),
                Err(error) => Found::Failed(error),
            };
            core.signal(Signal::Download(found));
        });
    }

    fn download_found(&mut self, found: Found) {
        match found {
            Found::Proposal(proposal) => {
                let heading = proposal.heading().to_string();
                match proposal.best() {
                    Some(best) => {
                        self.tell_download(
                            &heading,
                            &format!("{}. Confirm on the phone's Downloads page.", best.facts),
                            OFFER_TIME,
                            true,
                        );
                    }
                    None => {
                        let status = match &proposal.answer {
                            Answer::Question(question) => {
                                format!("{} Choose on the phone's Downloads page.", question.text)
                            }
                            _ => proposal.message().to_string(),
                        };
                        self.tell_download(&heading, &status, NOTICE_TIME, false);
                    }
                }
            }
            Found::Failed(error) => {
                eprintln!("{error}");
                self.tell_download("", "The download service did not answer", NOTICE_TIME, false);
            }
        }
    }

    fn tell_download(&self, heading: &str, status: &str, hold: Duration, ok: bool) {
        self.core.bus.notice(Notice {
            heading: heading.to_string(),
            status: status.to_string(),
            ok,
        });
        if let Some(tv) = &self.on_tv {
            tv.toast(heading, status, hold);
        }
        self.tv.send(ToTv::Panel(Panel {
            heading: heading.to_string(),
            status: status.to_string(),
            hold,
        }));
    }

    fn tell(&self, text: &str) {
        match &self.on_tv {
            Some(tv) => tv.say(text),
            None => self.show_message(text.to_string()),
        }
    }

    async fn become_profile(&mut self, profile: ProfileId) {
        if profile == self.profile {
            return;
        }
        self.profile = profile;
        self.progress = self.core.progress(profile).await;
        if self.on_tv.is_none() {
            self.subtitles.set_owner(profile);
        }
        self.core.set_tv_profile(profile);
        self.core.saves().save(move |db| async move {
            if let Err(error) = db.set_setting(TV_PROFILE, Some(&profile.0.to_string())).await {
                eprintln!("tv profile: {error}");
            }
        });
        if self.on_tv.is_none() {
            self.apply_audio(profile);
        }
        self.refresh_people();
        self.queue_downloads(false);
    }

    /// The radio said which headphones are paired. Headphones nobody owns go
    /// to the first profile, and the TV profile's choice is applied again.
    fn headphones_found(&self, paired: Option<audio::Device>) {
        let Some(audio) = self.audio.clone() else {
            return;
        };
        eprintln!(
            "headphones: paired {}",
            paired.as_ref().map_or("none", |device| device.address.as_str())
        );
        self.core.saves().save(move |db| async move {
            if let Some(headphones) = paired
                && sound::claimed(&db).await.is_empty()
            {
                let prefs = db::AudioPrefs::Unpaired.pairing(headphones.address.clone());
                if let Err(error) = db.save_audio_prefs(ProfileId::FIRST, &prefs).await {
                    eprintln!("headphones: {error}");
                }
            }
            audio.set_kept(sound::claimed(&db).await);
        });
        self.apply_audio(self.profile);
    }

    fn apply_audio(&self, profile: ProfileId) {
        if self.audio.is_none() {
            return;
        }
        let Some(choosing) = self.choosing.clone() else {
            return;
        };
        self.core.saves().save(move |db| async move {
            let prefs = db.audio_prefs(profile).await.unwrap_or_else(|error| {
                eprintln!("headphones: {error}");
                db::AudioPrefs::default()
            });
            let _ = choosing.send(prefs);
        });
    }

    async fn show_profile(&mut self, profile: ProfileId) -> Outcome {
        self.become_profile(profile).await;
        let name = self.name_of(profile);
        Outcome::done(format!("The TV shows what {name} is watching"))
    }

    fn show_message(&self, message: String) {
        self.tv.send(ToTv::Message(message));
    }
}

/// Who has a profile, with each one's progress and dialogue mode loaded into
/// the core's memory on the way.
/// What an update left playing, read once at the start.
async fn left_for_update(db: &Db) -> Option<Left> {
    let stored = db
        .setting(LEFT_FOR_UPDATE)
        .await
        .inspect_err(|error| eprintln!("update: {error}"))
        .ok()??;
    serde_json::from_str(&stored)
        .inspect_err(|error| eprintln!("update: {error}"))
        .ok()
}

async fn people(db: &Db, core: &Core) -> Vec<(ProfileId, String)> {
    let profiles = db.profiles().await.unwrap_or_else(|error| {
        eprintln!("profiles: {error}");
        Vec::new()
    });
    for profile in &profiles {
        core.progress(profile.id).await;
        core.dialogue(profile.id).await;
    }
    profiles.into_iter().map(|profile| (profile.id, profile.name)).collect()
}

/// Who watches `key` together with `profile`, and where each one is.
async fn together_on_tv(
    db: &Db,
    core: &Core,
    library: &Library,
    profile: ProfileId,
    mine: &Progress,
    key: &str,
) -> Option<Together> {
    let show = library.shows.iter().find(|show| show.key == key)?;
    let partners = db.partners(profile, key).await.unwrap_or_else(|error| {
        eprintln!("together: partners of {} in {key}: {error}", profile.0);
        Vec::new()
    });
    let profiles = db.profiles().await.unwrap_or_else(|error| {
        eprintln!("together: profiles: {error}");
        Vec::new()
    });
    let here = together::Spot::of(mine, show);
    let mut people = Vec::new();
    for partner in &partners {
        let Some(found) = profiles.iter().find(|found| found.id == *partner) else {
            continue;
        };
        let spot = together::Spot::of(&*core.progress(*partner).await, show);
        people.push(Mate {
            id: *partner,
            name: found.name.clone(),
            line: spot.line(spot.same_as(here)),
            paired: true,
        });
    }
    let others = profiles
        .iter()
        .filter(|found| found.id != profile && !partners.contains(&found.id));
    people.extend(others.map(|found| Mate {
        id: found.id,
        name: found.name.clone(),
        line: "Watch together".to_string(),
        paired: false,
    }));
    Some(Together {
        show: key.to_string(),
        people,
    })
}

/// A thread that picks a profile's headphones with bluetoothctl and BlueALSA,
/// which can take seconds or hang. Of the profiles sent while it is busy only
/// the last is picked.
fn chooser(audio: Audio) -> Sender<db::AudioPrefs> {
    let (choose, chosen) = std::sync::mpsc::channel::<db::AudioPrefs>();
    let spawned = std::thread::Builder::new()
        .name("headphones".to_string())
        .spawn(move || {
            while let Ok(mut prefs) = chosen.recv() {
                while let Ok(newer) = chosen.try_recv() {
                    prefs = newer;
                }
                let headphones = prefs.headphones().and_then(|address| audio.bonded(address));
                let have = headphones.is_some();
                audio.select(headphones);
                let connected = audio
                    .view()
                    .headphones
                    .is_some_and(|headphones| headphones.state == audio::Link::Connected);
                if prefs.output() == db::Output::Headphones && have && connected {
                    audio.choose(audio::Output::Headphones);
                }
            }
        });
    if let Err(error) = spawned {
        eprintln!("headphones: {error}");
    }
    choose
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link;
    use crate::player::Player;
    use tokio::task::block_in_place;

    fn hardware() -> Hardware {
        Hardware {
            store: Store::new(PathBuf::from("/lib"), None, None),
            storage: None,
            handoff: None,
            tv: Link::pair().0,
            segments: Arc::new(|_, _| Err("no segments in tests".to_string())),
            files: Files {
                find: Arc::new(|path, _| Some((path.to_path_buf(), Vec::new()))),
                probe: Arc::new(|_| Box::pin(async { Err("no probing in tests".to_string()) })),
            },
            remove: tending::remover(),
        }
    }

    async fn core() -> Core {
        let (core, inbox) = Core::detached(db::test_library()).await;
        let actor = Actor::new(core.clone(), ProfileId::FIRST, hardware()).await;
        tokio::spawn(actor.run(inbox));
        core
    }

    async fn core_with_tv(core: Core, inbox: mpsc::UnboundedReceiver<Input>, tv: Link) -> Core {
        let hardware = Hardware { tv, ..hardware() };
        let actor = Actor::new(core.clone(), ProfileId::FIRST, hardware).await;
        tokio::spawn(actor.run(inbox));
        core
    }

    fn up(core: &Core) -> PathBuf {
        core.library().movies[0].path.clone()
    }

    /// A core whose TV plays the first movie on a fake tv end, which the test
    /// speaks for. Returns the play's token.
    async fn playing() -> (Core, String) {
        let (core, inbox) = Core::detached(db::test_library()).await;
        let (link, orders) = Link::pair();
        let core = core_with_tv(core, inbox, link).await;
        core.ask(Request::PlayFile(up(&core))).await.unwrap();
        let play = told(&orders, |order| match order {
            ToTv::Play(play) => Some(play.play),
            _ => None,
        });
        (core, play)
    }

    fn reported(play: &str, position: f64, started: bool) -> Signal {
        Signal::Tv(FromTv::Report(Box::new(Report {
            play: Some(play.to_string()),
            state: player::State {
                position,
                duration: 9000.0,
                speed: 1.0,
                started,
                ..player::State::default()
            },
            place: progress::Place {
                position,
                duration: 9000.0,
                credits: None,
            },
            played: position / 2.0,
            ..Report::default()
        })))
    }

    /// Waits until the phones are shown what the TV plays.
    async fn shown(core: &Core) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while core.now_playing().is_none() {
            assert!(Instant::now() < deadline, "nothing is shown as playing");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Waits until the actor has handled everything sent before.
    async fn settled(core: &Core) {
        core.ask(Request::HideSkip).await.unwrap();
    }

    async fn saved_at(core: &Core) -> Option<f64> {
        core.progress(ProfileId::FIRST)
            .await
            .files
            .get(&up(core))
            .and_then(progress::Watched::stopped_at)
    }

    fn position(core: &Core) -> Option<f64> {
        core.now_playing().map(|playing| playing.position)
    }

    /// Asks and checks the answer came within 100 ms.
    async fn at_once(core: &Core, request: Request) -> Outcome {
        let asked = Instant::now();
        let outcome = core.ask(request).await.expect("an answer");
        assert!(asked.elapsed() < Duration::from_millis(100), "{:?}", asked.elapsed());
        outcome
    }

    /// Asks for the subtitles to change and checks the answer says the TV did
    /// not answer, after the 2 s the core waits for it.
    async fn subtitles_unanswered(core: &Core) {
        let asked = Instant::now();
        let outcome = core.ask(Request::Subtitles(subtitles::Change::Toggle)).await;
        let waited = asked.elapsed();
        assert_eq!(outcome, Some(Outcome::refused(TV_SILENT)));
        assert!(
            waited >= Duration::from_millis(1900) && waited < Duration::from_millis(2100),
            "{waited:?}"
        );
    }

    /// Everything the API asks about playback, each answered within 100 ms
    /// (and the subtitles within 2.1 s) while the tv takes no order at all.
    async fn the_api_answers_while_the_tv_takes_nothing(core: &Core) {
        for _ in 0..link::QUEUE + 50 {
            core.send(Request::Key(Command::Left));
        }
        let waiting = {
            let core = core.clone();
            tokio::spawn(async move { subtitles_unanswered(&core).await })
        };
        assert_eq!(
            at_once(core, Request::Seek(Seek::To(90.0))).await,
            Outcome::done("Seeking")
        );
        assert_eq!(
            at_once(core, Request::Seek(Seek::Step(Direction::Forward))).await,
            Outcome::done("Seeking")
        );
        assert_eq!(at_once(core, Request::Speed(1.23)).await, Outcome::done("Speed 1.25×"));
        assert_eq!(
            at_once(core, Request::Control(Control::Pause)).await,
            Outcome::done("Pause")
        );
        assert_eq!(
            at_once(core, Request::Control(Control::Mute)).await,
            Outcome::done("Mute")
        );
        at_once(core, Request::PlayPause).await;
        assert_eq!(
            at_once(core, Request::HideSkip).await,
            Outcome::refused("The TV shows nothing to skip right now")
        );
        let read = Instant::now();
        let shown = core.now_playing().expect("the playback the API shows");
        assert!(read.elapsed() < Duration::from_millis(100));
        assert_eq!(shown.title, "Up");
        waiting.await.unwrap();
    }

    /// Runs `test` on a runtime of its own and leaves that runtime behind,
    /// so a test whose actor gets stuck fails instead of waiting on the stuck
    /// thread forever.
    fn on_a_runtime_left_behind(test: impl Future<Output = ()>) {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runtime.block_on(test)));
        runtime.shutdown_background();
        if let Err(panic) = outcome {
            std::panic::resume_unwind(panic);
        }
    }

    #[test]
    fn a_tv_end_that_never_reads_leaves_the_api_answering() {
        on_a_runtime_left_behind(async {
            let (core, inbox) = Core::detached(db::test_library()).await;
            let (link, _never_read) = Link::pair();
            let core = core_with_tv(core, inbox, link).await;
            assert!(core.ask(Request::PlayFile(up(&core))).await.unwrap().ok);
            shown(&core).await;
            the_api_answers_while_the_tv_takes_nothing(&core).await;
        });
    }

    #[test]
    fn a_mount_that_never_answers_leaves_the_api_answering_and_back_cancels_the_play() {
        on_a_runtime_left_behind(async {
            let (core, inbox) = Core::detached(db::test_library()).await;
            let (link, orders) = Link::pair();
            let hardware = Hardware {
                tv: link,
                files: Files {
                    find: Arc::new(|_, _| {
                        loop {
                            std::thread::park();
                        }
                    }),
                    probe: Arc::new(|_| Box::pin(std::future::pending())),
                },
                ..hardware()
            };
            let actor = Actor::new(core.clone(), ProfileId::FIRST, hardware).await;
            tokio::spawn(actor.run(inbox));
            assert!(at_once(&core, Request::PlayFile(up(&core))).await.ok);
            for _ in 0..20 {
                at_once(&core, Request::Seek(Seek::To(90.0))).await;
                at_once(&core, Request::Speed(1.5)).await;
                at_once(&core, Request::Control(Control::Mute)).await;
                at_once(&core, Request::HideSkip).await;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert_eq!(core.now_playing(), None);
            at_once(&core, Request::Key(Command::Back)).await;
            at_once(&core, Request::Key(Command::Back)).await;
            assert!(
                orders.try_iter().all(|order| !matches!(order, ToTv::Play(_))),
                "nothing plays while its file is not found"
            );
        });
    }

    #[test]
    fn a_start_whose_radio_never_says_what_is_paired_serves_at_once() {
        on_a_runtime_left_behind(async {
            let (core, inbox) =
                Core::detached_with_audio(db::test_library(), |bus, found| Some(Audio::silent(bus, found))).await;
            let (link, _orders) = Link::pair();
            let started = Instant::now();
            let core = tokio::time::timeout(Duration::from_secs(2), core_with_tv(core, inbox, link))
                .await
                .expect("the actor started without waiting on the radio");
            assert!(
                started.elapsed() < Duration::from_millis(500),
                "{:?}",
                started.elapsed()
            );
            assert_eq!(
                at_once(&core, Request::Control(Control::Pause)).await,
                Outcome::refused("Pause, but nothing is playing")
            );
        });
    }

    #[test]
    fn headphones_whose_radio_never_answers_leave_the_api_answering() {
        on_a_runtime_left_behind(async {
            let (core, inbox) =
                Core::detached_with_audio(db::test_library(), |bus, found| Some(Audio::silent(bus, found))).await;
            let paired = db::AudioPrefs::Unpaired.pairing("00:11:22:33:44:55".to_string());
            core.db().save_audio_prefs(ProfileId::FIRST, &paired).await.unwrap();
            let (link, orders) = Link::pair();
            let core = core_with_tv(core, inbox, link).await;
            assert!(at_once(&core, Request::PlayFile(up(&core))).await.ok);
            told(&orders, |order| match order {
                ToTv::Play(play) => Some(play.play),
                _ => None,
            });
            shown(&core).await;
            the_api_answers_while_the_tv_takes_nothing(&core).await;
        });
    }

    #[test]
    fn a_removal_from_pcloud_that_never_ends_leaves_the_api_answering() {
        on_a_runtime_left_behind(async {
            let (core, inbox) = Core::detached(db::test_library()).await;
            let hardware = Hardware {
                remove: Arc::new(|_, _| Box::pin(std::future::pending())),
                ..hardware()
            };
            let actor = Actor::new(core.clone(), ProfileId::FIRST, hardware).await;
            tokio::spawn(actor.run(inbox));
            let removing = {
                let core = core.clone();
                tokio::spawn(async move {
                    let title = storage::Title::Movie("up-2009".to_string());
                    core.ask(Request::Storage(storage::Change::Remove(title))).await
                })
            };
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(at_once(&core, Request::PlayFile(up(&core))).await.ok);
            shown(&core).await;
            for _ in 0..10 {
                at_once(&core, Request::Seek(Seek::To(90.0))).await;
                at_once(&core, Request::Control(Control::Pause)).await;
            }
            assert!(!removing.is_finished(), "the removal still waits on pCloud");
        });
    }

    #[test]
    fn a_database_that_stops_writing_leaves_the_api_answering_and_keeps_every_write() {
        on_a_runtime_left_behind(async {
            let (core, orders) = core_and_tv().await;
            settled(&core).await;
            core.saves().flushed().await;
            let release = core.db().hold().await;
            assert!(at_once(&core, Request::PlayFile(up(&core))).await.ok);
            let play = told(&orders, |order| match order {
                ToTv::Play(play) => Some(play.play),
                _ => None,
            });
            core.signal(Signal::Tv(FromTv::Started { play: play.clone() }));
            core.signal(reported(&play, 45.0, true));
            settled(&core).await;
            let mami = ProfileId(2);
            at_once(&core, Request::Key(Command::Up)).await;
            at_once(&core, Request::Key(Command::Dialogue)).await;
            at_once(&core, Request::TvProfile(mami)).await;
            at_once(&core, Request::TvProfile(ProfileId::FIRST)).await;
            core.send(Request::WatchWithOnTv {
                show: "dark".to_string(),
                with: mami,
            });
            the_api_answers_while_the_tv_takes_nothing(&core).await;
            assert_eq!(saved_at(&core).await, Some(45.0), "the place is kept in memory");
            drop(release);
            core.saves().flushed().await;
            core.forget_progress();
            assert_eq!(saved_at(&core).await, Some(45.0), "and lands in the database");
            assert!(core.db().dialogue(ProfileId::FIRST).await.unwrap());
        });
    }

    #[test]
    fn a_player_lock_held_forever_leaves_the_api_answering() {
        on_a_runtime_left_behind(async {
            let (core, inbox) = Core::detached(db::test_library()).await;
            let (started, start) = std::sync::mpsc::channel();
            let telling = core.clone();
            let tv = link::local::spawn(
                move |said| telling.signal(Signal::Tv(said)),
                move |play, callbacks| {
                    let player = Player::without_mpv(play, callbacks);
                    player.hold_forever();
                    let _ = started.send(());
                    Ok(player)
                },
                |_| {},
            );
            let core = core_with_tv(core, inbox, tv).await;
            assert!(core.ask(Request::PlayFile(up(&core))).await.unwrap().ok);
            start.recv_timeout(Duration::from_secs(2)).unwrap();
            the_api_answers_while_the_tv_takes_nothing(&core).await;
        });
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_subtitle_change_is_answered_with_what_the_tv_says() {
        let (core, inbox) = Core::detached(db::test_library()).await;
        let telling = core.clone();
        let tv = link::local::spawn(
            move |said| telling.signal(Signal::Tv(said)),
            |play, callbacks| Ok(Player::without_mpv(play, callbacks)),
            |_| {},
        );
        let core = core_with_tv(core, inbox, tv).await;
        core.ask(Request::PlayFile(up(&core))).await.unwrap();
        let asked = Instant::now();
        let answer = core
            .ask(Request::Subtitles(subtitles::Change::Delay(0.5)))
            .await
            .unwrap();
        assert!(asked.elapsed() < Duration::from_secs(1), "{:?}", asked.elapsed());
        assert_ne!(answer, Outcome::refused(TV_SILENT));
    }

    async fn tv_challenged(core: &Core, orders: &std::sync::mpsc::Receiver<ToTv>) -> String {
        core.ask(Request::PlayFile(up(core))).await.unwrap();
        let play = told(orders, |order| match order {
            ToTv::Play(play) => Some(play.play),
            _ => None,
        });
        core.signal(Signal::Tv(FromTv::Started { play: play.clone() }));
        core.signal(reported(&play, 30.0, true));
        settled(core).await;
        let challenger = crate::challenge::Challenger {
            profile: ProfileId(2),
            person: "Mami".to_string(),
            device: "iPad".to_string(),
        };
        let watcher = crate::challenge::Watcher {
            seat: Seat::Tv,
            profile: ProfileId::FIRST,
            person: "Frederik".to_string(),
            device: "the TV".to_string(),
            title: "Up".to_string(),
        };
        assert!(core.arena().tv_can_bake, "a TV that plays can be challenged");
        core.arena()
            .challenge("bake".to_string(), challenger, watcher, Instant::now())
            .unwrap();
        told(orders, |order| match order {
            ToTv::Pause { paused: true, .. } => Some(()),
            _ => None,
        });
        told(orders, |order| match order {
            ToTv::Bakeoff(Some(View::Asked { challenger, .. })) => {
                assert_eq!(challenger, "Mami");
                Some(())
            }
            _ => None,
        });
        play
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_tv_challenged_to_a_bake_off_pauses_bakes_with_the_remote_and_stops_its_film_when_it_loses() {
        let (core, inbox) = Core::detached(db::test_library()).await;
        let (link, orders) = Link::pair();
        let actor = Actor::new(core.clone(), ProfileId::FIRST, Hardware { tv: link, ..hardware() }).await;
        tokio::spawn(actor.run(inbox));
        let play = tv_challenged(&core, &orders).await;
        core.send(Request::Key(Command::Select));
        told(&orders, |order| match order {
            ToTv::BakeKey(Command::Select) => Some(()),
            ToTv::Command { .. } => panic!("the remote reached the film during the bake-off"),
            _ => None,
        });
        core.signal(Signal::Tv(FromTv::Bakeoff(Baked::Accept)));
        told(&orders, |order| match order {
            ToTv::Bakeoff(Some(View::Baking { golden_ms, .. })) => {
                assert_eq!(golden_ms.len(), 3);
                Some(())
            }
            _ => None,
        });
        core.arena()
            .press(
                "bake",
                Side::Challenger,
                Pressed::Touch,
                [Some(0.05); 3],
                Instant::now(),
            )
            .unwrap();
        core.signal(Signal::Tv(FromTv::Bakeoff(Baked::Presses { presses: [None; 3] })));
        told(&orders, |order| match order {
            ToTv::Bakeoff(Some(View::Done { winner, .. })) => {
                assert_eq!(winner, Some(Side::Challenger));
                Some(())
            }
            _ => None,
        });
        core.signal(Signal::Tv(FromTv::Bakeoff(Baked::Dismiss)));
        told(&orders, |order| match order {
            ToTv::Bakeoff(None) => Some(()),
            _ => None,
        });
        told(&orders, |order| match order {
            ToTv::Stop { play: stopped } => {
                assert_eq!(stopped, play);
                Some(())
            }
            ToTv::Pause { paused: false, .. } => panic!("the film went on after the TV lost"),
            _ => None,
        });
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_tv_whose_challenger_goes_away_plays_on_after_the_result() {
        let (core, inbox) = Core::detached(db::test_library()).await;
        let (link, orders) = Link::pair();
        let actor = Actor::new(core.clone(), ProfileId::FIRST, Hardware { tv: link, ..hardware() }).await;
        tokio::spawn(actor.run(inbox));
        tv_challenged(&core, &orders).await;
        core.arena().withdraw("bake", Instant::now());
        told(&orders, |order| match order {
            ToTv::Bakeoff(Some(View::Done { winner, .. })) => {
                assert_eq!(winner, None);
                Some(())
            }
            _ => None,
        });
        core.signal(Signal::Tv(FromTv::Bakeoff(Baked::Dismiss)));
        told(&orders, |order| match order {
            ToTv::Pause { paused: false, .. } => Some(()),
            ToTv::Stop { .. } => panic!("the film stopped though nobody won"),
            _ => None,
        });
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn segments_are_looked_up_once_per_play_and_sent_with_its_token() {
        let mut library = db::test_library();
        library.movies[0].tmdb = Some(14160);
        let (core, inbox) = Core::detached(library).await;
        let (link, orders) = Link::pair();
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let asking = asked.clone();
        let hardware = Hardware {
            tv: link,
            segments: Arc::new(move |title, duration| {
                asking.lock().unwrap().push((title, duration));
                Ok(segments::Found {
                    segments: vec![segments::Segment {
                        kind: Kind::Intro,
                        start: 60.0,
                        end: 120.0,
                    }],
                    credits: Some(8700.0),
                })
            }),
            ..hardware()
        };
        let actor = Actor::new(core.clone(), ProfileId::FIRST, hardware).await;
        tokio::spawn(actor.run(inbox));
        core.ask(Request::PlayFile(up(&core))).await.unwrap();
        let play = told(&orders, |order| match order {
            ToTv::Play(play) => Some(play.play),
            _ => None,
        });
        core.signal(Signal::Tv(FromTv::Report(Box::new(Report {
            play: Some(play.clone()),
            ..Report::default()
        }))));
        settled(&core).await;
        assert!(
            asked.lock().unwrap().is_empty(),
            "nothing is looked up before the duration is known"
        );
        core.signal(reported(&play, 1.0, true));
        core.signal(reported(&play, 2.0, true));
        let segments = |orders: &std::sync::mpsc::Receiver<ToTv>| loop {
            match orders.recv_timeout(Duration::from_secs(2)) {
                Ok(ToTv::Segments {
                    play,
                    segments,
                    credits,
                }) => break Some((play, segments, credits)),
                Ok(_) => {}
                Err(_) => break None,
            }
        };
        let (told, intro, credits) = segments(&orders).expect("the tv is told the segments");
        assert_eq!(told, play);
        assert_eq!(intro[0].kind, Kind::Intro);
        assert_eq!(credits, Some(8700.0));
        core.signal(reported(&play, 3.0, true));
        settled(&core).await;
        assert_eq!(segments(&orders), None, "the segments are sent once");
        assert_eq!(
            *asked.lock().unwrap(),
            vec![(
                Title {
                    tmdb: 14160,
                    episode: None
                },
                9000.0
            )]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_subtitle_delay_set_on_the_tv_comes_back_with_the_next_play_of_that_file() {
        let (core, inbox) = Core::detached(db::test_library()).await;
        let telling = core.clone();
        let (started, plays) = std::sync::mpsc::channel();
        let tv = link::local::spawn(
            move |said| telling.signal(Signal::Tv(said)),
            move |play, callbacks| {
                let _ = started.send((play.play.clone(), play.subtitles.clone()));
                Ok(Player::without_mpv(play, callbacks))
            },
            |_| {},
        );
        let core = core_with_tv(core, inbox, tv).await;
        core.ask(Request::PlayFile(up(&core))).await.unwrap();
        let (first, memory) = plays.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(memory.remembered, subtitles::Remembered::default());
        let answer = core
            .ask(Request::Subtitles(subtitles::Change::Delay(0.5)))
            .await
            .unwrap();
        assert_eq!(answer, Outcome::done("Subtitle delay +0.5 s"));
        settled(&core).await;
        assert_eq!(core.subtitles().remembered(&up(&core)).delay, 0.5);
        core.signal(Signal::Tv(FromTv::Finished(Ended {
            play: first,
            file: up(&core),
            owner: ProfileId::FIRST,
            finish: Finish::Stopped,
            place: progress::Place::default(),
            tracks: progress::Tracks::default(),
            played: 0.0,
        })));
        core.ask(Request::PlayFile(up(&core))).await.unwrap();
        let (_, again) = plays.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(again.remembered.delay, 0.5);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fetched_subtitles_for_another_file_are_not_offered_to_the_tv() {
        let (core, _play) = playing().await;
        let other = PathBuf::from("/lib/Movies/Cars/Cars.mkv");
        let answer = core
            .ask(Request::Subtitles(subtitles::Change::Add {
                video: other.clone(),
                sidecar: crate::sidecar::Sidecar {
                    path: other.with_extension("en.srt"),
                    language: Some("en".to_string()),
                    title: "English".to_string(),
                    forced: false,
                    hearing_impaired: false,
                },
            }))
            .await
            .unwrap();
        assert_eq!(answer, Outcome::refused("That is not what plays now"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_tracks_the_tv_reports_are_what_the_subtitles_api_shows() {
        let (core, play) = playing().await;
        let mut shown = subtitles::Playing::default();
        shown.visible = true;
        shown.track = Some(2);
        core.signal(Signal::Tv(FromTv::Report(Box::new(Report {
            play: Some(play.clone()),
            subtitles: Some(shown.clone()),
            ..Report::default()
        }))));
        settled(&core).await;
        assert_eq!(core.subtitles().playing(), Some(shown));
        core.signal(Signal::Tv(FromTv::Finished(Ended {
            play,
            file: up(&core),
            owner: ProfileId::FIRST,
            finish: Finish::Stopped,
            place: progress::Place::default(),
            tracks: progress::Tracks::default(),
            played: 0.0,
        })));
        settled(&core).await;
        assert_eq!(core.subtitles().playing(), None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_video_that_does_not_start_says_why_and_leaves_nothing_playing() {
        let (core, inbox) = Core::detached(db::test_library()).await;
        let telling = core.clone();
        let (shown, menu) = std::sync::mpsc::channel();
        let tv = link::local::spawn(
            move |said| telling.signal(Signal::Tv(said)),
            |_, _| Err(std::io::Error::other("mpv is missing")),
            move |order| {
                let _ = shown.send(order);
            },
        );
        let core = core_with_tv(core, inbox, tv).await;
        core.ask(Request::PlayFile(up(&core))).await.unwrap();
        let message = told(&menu, |order| match order {
            ToTv::Message(message) => Some(message),
            _ => None,
        });
        assert_eq!(message, "The video did not start: mpv is missing");
        settled(&core).await;
        assert_eq!(core.now_playing(), None);
        assert_eq!(saved_at(&core).await, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_report_from_the_tv_moves_what_the_phones_see_and_one_for_another_play_does_not() {
        let (core, play) = playing().await;
        core.signal(reported("an-old-play", 500.0, true));
        settled(&core).await;
        assert_ne!(position(&core), Some(500.0));
        core.signal(reported(&play, 100.0, true));
        settled(&core).await;
        assert_eq!(position(&core), Some(100.0));
        assert_eq!(core.now_playing().map(|playing| playing.loading), Some(false));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_place_is_saved_after_thirty_seconds_of_reported_movement() {
        let (core, play) = playing().await;
        core.signal(reported(&play, 100.0, false));
        core.signal(reported(&play, 10.0, true));
        core.signal(reported(&play, 29.5, true));
        settled(&core).await;
        assert_eq!(saved_at(&core).await, None, "not started, then under 30 s");
        core.signal(reported(&play, 31.0, true));
        settled(&core).await;
        assert_eq!(saved_at(&core).await, Some(31.0));
        core.signal(reported(&play, 50.0, true));
        settled(&core).await;
        assert_eq!(saved_at(&core).await, Some(31.0));
        core.signal(reported(&play, 61.0, true));
        settled(&core).await;
        assert_eq!(saved_at(&core).await, Some(61.0));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_finished_playback_saves_the_place_it_ended_at() {
        let (core, play) = playing().await;
        core.signal(reported(&play, 40.0, true));
        core.signal(Signal::Tv(FromTv::Finished(Ended {
            play,
            file: up(&core),
            owner: ProfileId::FIRST,
            finish: Finish::Stopped,
            place: progress::Place {
                position: 1234.0,
                duration: 9000.0,
                credits: None,
            },
            tracks: progress::Tracks::default(),
            played: 600.0,
        })));
        settled(&core).await;
        assert_eq!(saved_at(&core).await, Some(1234.0));
        assert_eq!(core.now_playing(), None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn what_the_tv_set_sees_is_kept_like_a_report_and_its_end_saves_the_place() {
        let (core, play) = playing().await;
        let seen = |position| renderer::Seen {
            position,
            duration: 9000.0,
            started: true,
            ..renderer::Seen::default()
        };
        let told = |play: &str, told| Signal::TvSet {
            play: play.to_string(),
            told,
        };
        core.signal(told("an-old-play", renderer::Told::Seen(seen(500.0))));
        core.signal(told(&play, renderer::Told::Seen(seen(100.0))));
        settled(&core).await;
        assert_eq!(position(&core), Some(100.0));
        core.signal(told(
            &play,
            renderer::Told::Ended {
                finish: Finish::Stopped,
                seen: seen(1234.0),
            },
        ));
        settled(&core).await;
        assert_eq!(saved_at(&core).await, Some(1234.0));
        assert_eq!(core.now_playing(), None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_download_asked_for_on_the_tv_names_the_tvs_profile() {
        let (core, _inbox) = Core::detached(db::test_library()).await;
        let mut actor = Actor::new(core, ProfileId::FIRST, hardware()).await;
        assert_eq!(actor.asked_by().as_deref(), Some("Frederik"));
        actor.profile = ProfileId(2);
        assert_eq!(actor.asked_by().as_deref(), Some("Mami"));
        actor.profile = ProfileId(99);
        assert_eq!(actor.asked_by(), None);
    }

    fn report(core: &Core, moment: stream::Moment, position: f64, profile: ProfileId) -> stream::Report {
        stream::Report {
            file: core.library().movies[0].path.clone(),
            position,
            duration: 9000.0,
            credits: None,
            played: 120.0,
            moment,
            profile,
            device: None,
            session: "a-session".to_string(),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_phones_place_is_kept_for_its_profile_and_survives_a_reload_from_the_database() {
        let core = core().await;
        let mami = ProfileId(2);
        let up = core.library().movies[0].path.clone();
        let kept = core
            .ask(Request::Phone(report(&core, stream::Moment::Paused, 600.0, mami)))
            .await;
        assert_eq!(kept, Some(Outcome::done("Kept the place")));
        assert_eq!(core.progress(mami).await.files[&up].stopped_at(), Some(600.0));
        assert!(!core.progress(ProfileId::FIRST).await.files.contains_key(&up));
        core.saves().flushed().await;
        core.forget_progress();
        assert_eq!(core.progress(mami).await.files[&up].stopped_at(), Some(600.0));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_home_the_tv_draws_follows_the_tv_profile() {
        let (core, orders) = core_and_tv().await;
        let mami = ProfileId(2);
        let library = told(&orders, |order| match order {
            ToTv::Library { revision: 1, library } => Some(library),
            _ => None,
        });
        let first = told(&orders, home);
        assert_eq!(first.profile, ProfileId::FIRST);
        core.ask(Request::Phone(report(&core, stream::Moment::Paused, 600.0, mami)))
            .await;
        assert!(watching::current(&library, &first.progress).is_empty());
        let shown = core.ask(Request::TvProfile(mami)).await.unwrap();
        assert_eq!(shown.message, "The TV shows what Mami is watching");
        let home = told(&orders, |order| {
            assert!(!matches!(order, ToTv::Library { .. }), "the library did not change");
            home(order).filter(|home| home.profile == mami)
        });
        assert_eq!(core.tv_profile(), mami);
        assert_eq!(home.library_revision, 1);
        assert_eq!(watching::current(&library, &home.progress).len(), 1);
        assert!(home.people.iter().any(|(id, name)| *id == mami && name == "Mami"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_mark_answers_by_name_and_an_unknown_title_is_refused() {
        let core = core().await;
        let mark = |key: &str| {
            Request::Mark(
                watching::Kind::Movie,
                key.to_string(),
                None,
                watching::Mark::Watched,
                ProfileId::FIRST,
            )
        };
        let marked = core.ask(mark("up")).await.unwrap();
        assert_eq!(marked, Outcome::done("Up is marked as watched"));
        let library = core.library();
        assert!(watching::movie_finished(
            &*core.progress(ProfileId::FIRST).await,
            &library.movies[0]
        ));
        assert!(!core.ask(mark("jaws")).await.unwrap().ok);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn catching_up_moves_my_place_to_my_partners_and_keeps_theirs() {
        let core = core().await;
        let mami = ProfileId(2);
        let catch_up = || Request::CatchUp {
            show: "dark".to_string(),
            to: mami,
            by: ProfileId::FIRST,
        };
        assert_eq!(
            core.ask(catch_up()).await.unwrap(),
            Outcome::refused("Mami hasn't started Dark yet")
        );
        let mut theirs = Progress::default();
        theirs.shows.insert(
            "dark".to_string(),
            Position {
                season: 1,
                episode: 2,
                finished: false,
            },
        );
        let place = progress::Place {
            position: 900.0,
            duration: 3000.0,
            credits: None,
        };
        theirs.record(&core.library().shows[0].seasons[0].episodes[0].path, place);
        core.publish_progress(mami, theirs.clone());
        assert_eq!(
            core.ask(catch_up()).await.unwrap(),
            Outcome::done("You're at Mami's place now: Dark · S1 E2")
        );
        let library = core.library();
        let mine = core.progress(ProfileId::FIRST).await;
        assert_eq!(
            together::Spot::of(&mine, &library.shows[0]),
            together::Spot::of(&theirs, &library.shows[0])
        );
        assert_eq!(*core.progress(mami).await, theirs);
        assert_eq!(
            watching::current(&library, &mine)
                .first()
                .and_then(|current| current.episode),
            Some((1, 2))
        );
    }

    /// A core whose tv end is the test: the menu's news and the player's
    /// orders arrive on the receiver.
    async fn core_and_tv() -> (Core, std::sync::mpsc::Receiver<ToTv>) {
        let (core, inbox) = Core::detached(db::test_library()).await;
        let (link, orders) = Link::pair();
        (core_with_tv(core, inbox, link).await, orders)
    }

    /// Waits for the first thing the tv is told that `pick` takes, passing
    /// over the rest.
    fn told<T>(orders: &std::sync::mpsc::Receiver<ToTv>, pick: impl Fn(ToTv) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(5);
        block_in_place(|| {
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                let order = orders.recv_timeout(left).expect("the tv was told in time");
                if let Some(picked) = pick(order) {
                    return picked;
                }
            }
        })
    }

    fn home(order: ToTv) -> Option<messages::Home> {
        match order {
            ToTv::Home(home) => Some(home),
            _ => None,
        }
    }

    fn next_together(orders: &std::sync::mpsc::Receiver<ToTv>) -> crate::bus::Together {
        told(orders, |order| match order {
            ToTv::Together(together) => Some(together),
            _ => None,
        })
    }

    fn people(together: &crate::bus::Together) -> Vec<(&str, &str, bool)> {
        together
            .people
            .iter()
            .map(|mate| (mate.name.as_str(), mate.line.as_str(), mate.paired))
            .collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_tv_lists_who_to_watch_a_series_with_and_ok_pairs_then_moves_to_their_place() {
        let (core, orders) = core_and_tv().await;
        let mami = ProfileId(2);
        core.signal(Signal::Tv(FromTv::Ask(UiRequest::TogetherOnTv("dark".to_string()))));
        let together = next_together(&orders);
        assert_eq!(together.show, "dark");
        assert_eq!(
            people(&together),
            [
                ("Mami", "Watch together", false),
                ("Miriam", "Watch together", false),
                ("Ramu", "Watch together", false)
            ]
        );
        assert_eq!(together.people[0].id, mami);
        let with_mami = || Request::WatchWithOnTv {
            show: "dark".to_string(),
            with: mami,
        };
        assert_eq!(
            core.ask(with_mami()).await.unwrap(),
            Outcome::done("You watch Dark together with Mami")
        );
        let together = next_together(&orders);
        assert_eq!(people(&together)[0], ("Mami", "Hasn't started yet", true));
        assert_eq!(core.db().partners(mami, "dark").await.unwrap(), [ProfileId::FIRST]);
        assert_eq!(people(&together).len(), 3);
        assert_eq!(
            core.ask(with_mami()).await.unwrap(),
            Outcome::refused("Mami hasn't started Dark yet")
        );
        let mut theirs = Progress::default();
        theirs.shows.insert(
            "dark".to_string(),
            Position {
                season: 1,
                episode: 2,
                finished: false,
            },
        );
        core.publish_progress(mami, theirs);
        core.signal(Signal::Tv(FromTv::Ask(UiRequest::TogetherOnTv("dark".to_string()))));
        let mut together = next_together(&orders);
        while together.people[0].line == "Hasn't started yet" {
            together = next_together(&orders);
        }
        assert_eq!(people(&together)[0], ("Mami", "Up next S1 E2", true));
        assert_eq!(
            core.ask(with_mami()).await.unwrap(),
            Outcome::done("You're at Mami's place now: Dark · S1 E2")
        );
        let mut together = next_together(&orders);
        while together.people[0].line == "Up next S1 E2" {
            together = next_together(&orders);
        }
        assert_eq!(people(&together)[0], ("Mami", "At your place", true));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_new_library_reaches_both_frontends_at_once_and_is_sent_once() {
        let (core, orders) = core_and_tv().await;
        told(&orders, home);
        let mut library = (*core.library()).clone();
        let mut diaries = library.movies[0].clone();
        diaries.id = 87;
        diaries.key = crate::library::movie_key("The Princess Diaries", Some(2001));
        diaries.title = "The Princess Diaries".to_string();
        diaries.path = PathBuf::from("/lib/Movies/The Princess Diaries (2001)/Diaries.mkv");
        library.movies.push(diaries);
        core.signal(Signal::Library(Ok(library.clone())));
        let sent = told(&orders, |order| match order {
            ToTv::Library { revision, library } => Some((revision, library)),
            _ => None,
        });
        assert_eq!(sent, (2, library.clone()));
        assert_eq!(told(&orders, home).library_revision, 2);
        assert!(core.library().movies.iter().any(|movie| movie.id == 87));
        core.signal(Signal::Library(Ok(library)));
        core.ask(Request::TvProfile(ProfileId(2))).await.unwrap();
        let home = told(&orders, |order| {
            assert!(!matches!(order, ToTv::Library { .. }), "the same library again");
            home(order).filter(|home| home.profile == ProfileId(2))
        });
        assert_eq!(home.library_revision, 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_remote_key_goes_to_the_screen_while_nothing_plays() {
        let (core, orders) = core_and_tv().await;
        core.signal(Signal::Tv(FromTv::Key(Command::Down)));
        let key = told(&orders, |order| match order {
            ToTv::Key(key) => Some(key),
            _ => None,
        });
        assert_eq!(key, Command::Down);
        let paused = core.ask(Request::Control(Control::Pause)).await.unwrap();
        assert_eq!(paused, Outcome::refused("Pause, but nothing is playing"));
    }
    /// A core whose fake tv started the first movie and reported it at
    /// `position`: the core, what its tv is told, and the play's token.
    async fn reported_at(position: f64) -> (Core, std::sync::mpsc::Receiver<ToTv>, String) {
        let (core, orders) = core_and_tv().await;
        core.ask(Request::PlayFile(up(&core))).await.unwrap();
        let play = told(&orders, |order| match order {
            ToTv::Play(play) => Some(play.play),
            _ => None,
        });
        core.signal(Signal::Tv(FromTv::Started { play: play.clone() }));
        core.signal(reported(&play, position, true));
        settled(&core).await;
        (core, orders, play)
    }

    fn back(playing: Option<Running>, ended: Option<Ended>) -> Signal {
        Signal::Tv(FromTv::Hello {
            protocol: messages::PROTOCOL,
            revision: "this-build".to_string(),
            playing,
            ended,
        })
    }

    fn gone(ago: Duration) -> Signal {
        Signal::TvGone {
            at: Instant::now() - ago,
        }
    }

    fn played_again(orders: &std::sync::mpsc::Receiver<ToTv>) -> Box<Play> {
        told(orders, |order| match order {
            ToTv::Play(play) => Some(play),
            _ => None,
        })
    }

    fn plays_told(orders: &std::sync::mpsc::Receiver<ToTv>) -> usize {
        orders.try_iter().filter(|order| matches!(order, ToTv::Play(_))).count()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_tv_lost_mid_film_keeps_its_place_refuses_controls_and_is_picked_up_three_seconds_back() {
        let (core, orders, play) = reported_at(20.0).await;
        assert_eq!(saved_at(&core).await, None);
        core.signal(gone(Duration::ZERO));
        settled(&core).await;
        assert_eq!(saved_at(&core).await, Some(20.0), "saved at once");
        assert_eq!(core.now_playing().map(|playing| playing.loading), Some(true));
        assert_eq!(
            core.ask(Request::Seek(Seek::To(90.0))).await,
            Some(Outcome::refused(TV_RESTARTING))
        );
        assert_eq!(
            core.ask(Request::Control(Control::Pause)).await,
            Some(Outcome::refused(TV_RESTARTING))
        );
        core.signal(back(None, None));
        let again = played_again(&orders);
        assert_eq!(
            (again.play.as_str(), again.start, again.paused),
            (play.as_str(), 17.0, false)
        );
        let toast = told(&orders, |order| match order {
            ToTv::Toast { status, .. } => Some(status),
            _ => None,
        });
        assert_eq!(toast, "Picked up where it stopped");
        assert_eq!(
            core.ask(Request::Seek(Seek::To(90.0))).await,
            Some(Outcome::done("Seeking"))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_tv_back_after_more_than_two_minutes_finds_the_film_stopped() {
        let (core, orders, _) = reported_at(20.0).await;
        core.signal(gone(Duration::from_secs(150)));
        core.signal(back(None, None));
        settled(&core).await;
        assert_eq!(core.now_playing(), None);
        assert_eq!(plays_told(&orders), 0);
        assert_eq!(saved_at(&core).await, Some(20.0));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_film_that_stopped_the_tv_twice_is_not_picked_up_a_third_time() {
        let (core, orders, play) = reported_at(600.0).await;
        for _ in 0..2 {
            core.signal(gone(Duration::ZERO));
            core.signal(back(None, None));
            assert_eq!(played_again(&orders).play, play);
        }
        core.signal(gone(Duration::ZERO));
        core.signal(back(None, None));
        let message = told(&orders, |order| match order {
            ToTv::Play(_) => panic!("picked up a third time"),
            ToTv::Message(message) => Some(message),
            _ => None,
        });
        assert_eq!(message, "This video stopped the TV twice");
        assert_eq!(core.now_playing(), None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_tv_back_with_the_film_still_playing_carries_on_without_starting_it_again() {
        let (core, orders, play) = reported_at(200.0).await;
        core.signal(gone(Duration::ZERO));
        core.signal(back(
            Some(Running {
                play: play.clone(),
                owner: ProfileId::FIRST,
                file: up(&core),
                position: 201.0,
                paused: false,
            }),
            None,
        ));
        settled(&core).await;
        assert_eq!(plays_told(&orders), 0);
        assert_eq!(
            core.ask(Request::Seek(Seek::To(90.0))).await,
            Some(Outcome::done("Seeking"))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn what_was_asked_to_play_while_the_tv_was_away_plays_when_it_is_back() {
        let (core, orders, play) = reported_at(200.0).await;
        core.signal(gone(Duration::ZERO));
        let dark = core.library().shows[0].seasons[0].episodes[0].path.clone();
        assert!(core.ask(Request::PlayFile(dark.clone())).await.unwrap().ok);
        core.signal(back(None, None));
        let next = played_again(&orders);
        assert_ne!(next.play, play);
        assert_eq!(next.file, dark);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_restarted_core_takes_over_the_film_the_tv_kept_playing() {
        let (core, orders) = core_and_tv().await;
        core.signal(back(
            Some(Running {
                play: "kept".to_string(),
                owner: ProfileId::FIRST,
                file: up(&core),
                position: 500.0,
                paused: false,
            }),
            None,
        ));
        settled(&core).await;
        let shown = core.now_playing().expect("the film the tv kept");
        assert_eq!(
            (shown.title.as_str(), shown.position, shown.loading),
            ("Up", 500.0, false)
        );
        assert_eq!(
            core.ask(Request::Seek(Seek::To(90.0))).await,
            Some(Outcome::done("Seeking"))
        );
        let sought = told(&orders, |order| match order {
            ToTv::Seek { play, .. } => Some(play),
            _ => None,
        });
        assert_eq!(sought, "kept");
        core.signal(reported("kept", 530.0, true));
        settled(&core).await;
        assert_eq!(position(&core), Some(530.0));
        assert_eq!(saved_at(&core).await, Some(530.0));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_restarted_core_records_the_film_that_ended_while_it_was_away() {
        let (core, _orders) = core_and_tv().await;
        core.signal(back(
            None,
            Some(Ended {
                play: "ended-away".to_string(),
                file: up(&core),
                owner: ProfileId::FIRST,
                finish: Finish::Stopped,
                place: progress::Place {
                    position: 1234.0,
                    duration: 9000.0,
                    credits: None,
                },
                tracks: progress::Tracks::default(),
                played: 600.0,
            }),
        ));
        settled(&core).await;
        assert_eq!(saved_at(&core).await, Some(1234.0));
        assert_eq!(core.now_playing(), None);
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn a_tv_silent_for_five_seconds_shows_the_phones_a_loading_film_until_it_reports() {
        let (core, _orders, play) = reported_at(200.0).await;
        assert_eq!(core.now_playing().map(|playing| playing.loading), Some(false));
        tokio::time::sleep(TV_STALE + Duration::from_millis(700)).await;
        assert_eq!(core.now_playing().map(|playing| playing.loading), Some(true));
        core.signal(reported(&play, 205.0, true));
        settled(&core).await;
        assert_eq!(core.now_playing().map(|playing| playing.loading), Some(false));
    }

    fn update_card(order: ToTv) -> Option<bool> {
        match order {
            ToTv::Update(shown) => Some(shown),
            _ => None,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_update_mid_film_shows_the_card_stops_the_film_and_leaves_its_place() {
        let (core, orders, play) = reported_at(20.0).await;
        let asked = Instant::now();
        let outcome = core.ask(Request::Restarting).await;
        assert!(asked.elapsed() >= UPDATE_SHOWN_FOR, "{:?}", asked.elapsed());
        assert_eq!(outcome, Some(Outcome::done(UPDATE_SHOWN)));
        assert!(told(&orders, update_card), "the card shows before the film stops");
        let stopped = told(&orders, |order| match order {
            ToTv::Stop { play } => Some(play),
            _ => None,
        });
        assert_eq!(stopped, play);
        assert_eq!(saved_at(&core).await, Some(20.0));
        let left: Left = serde_json::from_str(&core.db().setting(LEFT_FOR_UPDATE).await.unwrap().unwrap()).unwrap();
        assert_eq!(
            (left.file, left.owner, left.position, left.paused),
            (up(&core), ProfileId::FIRST, 20.0, false)
        );
        assert!(watching::now() - left.at < 5);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_update_while_nothing_plays_answers_at_once_and_leaves_nothing() {
        let (core, orders) = core_and_tv().await;
        let outcome = at_once(&core, Request::Restarting).await;
        assert_eq!(outcome, Outcome::done(NOTHING_PLAYING));
        assert!(!orders.try_iter().any(|order| matches!(order, ToTv::Update(_))));
        core.saves().flushed().await;
        assert_eq!(core.db().setting(LEFT_FOR_UPDATE).await.unwrap(), None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_tv_that_crashed_is_picked_up_as_before_and_shows_no_update_card() {
        let (core, orders, _) = reported_at(20.0).await;
        core.signal(gone(Duration::ZERO));
        assert_eq!(
            at_once(&core, Request::Restarting).await,
            Outcome::done(NOTHING_PLAYING)
        );
        core.signal(back(None, None));
        let toast = told(&orders, |order| match order {
            ToTv::Update(true) => panic!("an update card after a crash"),
            ToTv::Toast { status, .. } => Some(status),
            _ => None,
        });
        assert_eq!(toast, PICKED_UP);
    }

    /// A core that starts after an update left the first movie `ago` seconds
    /// before, at 600 s and paused.
    async fn started_after_update(ago: u64) -> (Core, std::sync::mpsc::Receiver<ToTv>) {
        let (core, inbox) = Core::detached(db::test_library()).await;
        let left = Left {
            file: up(&core),
            owner: ProfileId::FIRST,
            position: 600.0,
            paused: true,
            at: watching::now() - ago,
        };
        let stored = serde_json::to_string(&left).unwrap();
        core.db().set_setting(LEFT_FOR_UPDATE, Some(&stored)).await.unwrap();
        let (link, orders) = Link::pair();
        (core_with_tv(core, inbox, link).await, orders)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn after_an_update_the_tv_shows_the_card_and_picks_the_film_up_three_seconds_back() {
        let (core, orders) = started_after_update(4).await;
        core.signal(back(None, None));
        assert!(told(&orders, update_card));
        let again = played_again(&orders);
        core.saves().flushed().await;
        assert_eq!(core.db().setting(LEFT_FOR_UPDATE).await.unwrap(), None);
        assert_eq!(
            (again.file.clone(), again.start, again.paused),
            (up(&core), 597.0, true)
        );
        let toast = told(&orders, |order| match order {
            ToTv::Toast { status, .. } => Some(status),
            _ => None,
        });
        assert_eq!(toast, PICKED_UP_AFTER_UPDATE);
        core.signal(Signal::Tv(FromTv::Started {
            play: again.play.clone(),
        }));
        let hidden = Instant::now();
        assert!(!told(&orders, update_card));
        assert!(hidden.elapsed() < UPDATE_HIDES_AFTER + Duration::from_secs(1));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_update_whose_restart_never_comes_plays_the_film_on() {
        let (core, orders, _) = reported_at(20.0).await;
        core.ask(Request::Restarting).await.unwrap();
        assert!(told(&orders, update_card));
        let again = played_again(&orders);
        assert_eq!((again.file.clone(), again.start), (up(&core), 17.0));
        core.signal(Signal::Tv(FromTv::Started {
            play: again.play.clone(),
        }));
        told(&orders, |order| matches!(order, ToTv::Update(false)).then_some(()));
        core.saves().flushed().await;
        assert_eq!(core.db().setting(LEFT_FOR_UPDATE).await.unwrap(), None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_update_left_more_than_two_minutes_ago_is_not_picked_up() {
        let (core, orders) = started_after_update(150).await;
        core.signal(back(None, None));
        settled(&core).await;
        assert_eq!(core.now_playing(), None);
        assert_eq!(plays_told(&orders), 0);
        core.saves().flushed().await;
        assert_eq!(core.db().setting(LEFT_FOR_UPDATE).await.unwrap(), None);
    }
}
