use super::tending::{self, Tended};
use super::{Core, Input, Outcome, Request, ScreenCommand, ScreenKind, Seek, Signal, TV_PROFILE, sound};
use crate::audio::{self, Audio};
use crate::bus::{Home, Mate, NowPlaying, Panel, Playing, Skip, Together, Voice as VoiceEvent};
use crate::command::Command;
use crate::db::{self, Db};
use crate::downloads::remote::Downloads;
use crate::downloads::{Answer, Outcome as Started, Proposal};
use crate::fetch;
use crate::hold::{self, Hold};
use crate::intent::{self, Action, Control};
use crate::led::Led;
use crate::library::Library;
use crate::link::messages::{Ended, FromTv, Play, PlaySubtitles, Report, Target, ToTv};
use crate::link::{self, Link};
use crate::offline::{Source, Store};
use crate::player::renderer::{self, Handoff};
use crate::player::{self, Finish, Player};
use crate::profile::ProfileId;
use crate::progress::{self, Position, Progress};
use crate::screen;
use crate::seek::Direction;
use crate::segments::{Kind, Title};
use crate::storage;
use crate::stream;
use crate::subtitles::Remembered;
use crate::subtitles::{self, Subtitles};
use crate::together;
use crate::tv::{self, Television};
use crate::up_next::{self, Press, UpNext};
use crate::voice::{self, Heard, Voice};
use crate::watching;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tokio::task::{block_in_place, spawn_blocking};
use tokio::time::MissedTickBehavior;

const VOICE_RESULT_TIME: Duration = Duration::from_secs(6);
const VOICE_WAITING_TIME: Duration = Duration::from_secs(20);
const OUT_OF_REACH: &str = "Not downloaded, and pCloud can't be reached right now";
const NOTHING_PLAYING: &str = "Nothing is playing";
const PLAYBACK_EVERY: Duration = Duration::from_millis(500);
const SCREEN_EVERY: Duration = Duration::from_secs(2);
const OFFER_TIME: Duration = Duration::from_secs(60);
const TV_WAKE_WAIT: Duration = Duration::from_secs(20);
const TV_PICTURE_SETTLE: Duration = Duration::from_secs(2);
const TV_ANSWER_TIME: Duration = Duration::from_secs(2);
const TV_SILENT: &str = "The TV did not answer";

type Boxed<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

pub struct Hardware {
    pub store: Store,
    pub storage: Option<storage::Cache>,
    pub speech: Option<String>,
    pub wake_word: Option<voice::WakeWord>,
    pub microphone: Option<String>,
    pub led: Option<Led>,
    pub handoff: Option<Handoff>,
    pub tv: Link,
}

pub enum Found {
    Proposal(Proposal),
    Started(Started),
    Failed(String),
}

struct Offer {
    proposal: String,
    label: String,
    until: Instant,
}

/// What the TV plays: the link its orders go down, what it shows, the file,
/// whose progress it moves and the last report about it. One value, so the
/// TV never plays without the rest or the rest without a playback.
struct OnTv {
    link: Link,
    handed_off: bool,
    shown: NowPlaying,
    file: PathBuf,
    owner: ProfileId,
    hold: Hold,
    play: String,
    last: Report,
    saved: f64,
}

impl OnTv {
    fn act(&mut self, (hold, act): (Hold, hold::Act)) {
        self.hold = hold;
        match act {
            hold::Act::Nothing => {}
            hold::Act::Pause => self.pause(true),
            hold::Act::Resume => self.pause(false),
        }
    }

    fn pause(&self, paused: bool) {
        self.link.send(ToTv::Pause {
            play: self.play.clone(),
            paused,
        });
    }

    fn mute(&self, muted: bool) {
        self.link.send(ToTv::Mute {
            play: self.play.clone(),
            muted,
        });
    }

    fn command(&self, command: Command) {
        self.link.send(ToTv::Command {
            play: self.play.clone(),
            command,
        });
    }

    fn stop(&self) {
        self.link.send(ToTv::Stop {
            play: self.play.clone(),
        });
    }

    fn say(&self, text: &str) {
        self.toast("", text, player::SAY_FOR);
    }

    fn toast(&self, heading: &str, status: &str, hold: Duration) {
        self.link.send(ToTv::Toast {
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
    voice: Option<Voice>,
    ear: voice::Ear,
    queued: Option<PathBuf>,
    up_next: Option<UpNext>,
    led: Option<Led>,
    subtitles: Subtitles,
    television: Television,
    downloads: Option<Downloads>,
    audio: Option<Audio>,
    offer: Option<Offer>,
    waking_tv: Arc<AtomicBool>,
    screen: screen::Screen,
    notice: Option<screen::Said>,
    pace: audio::Pace,
    handoff: Option<Handoff>,
    tv: Link,
    questions: HashMap<u64, oneshot::Sender<Outcome>>,
    asked: u64,
}

/// The in-process tv end for mpv: a thread that owns the player and tells
/// the core what happens through its inbox.
pub fn local_tv(core: &Core) -> Link {
    let (telling, subtitles) = (core.clone(), core.subtitles().clone());
    link::local::spawn(
        move |said| telling.signal(Signal::Tv(said)),
        move |play, callbacks| Player::start(play, subtitles.clone(), callbacks),
    )
}

fn resume_point(progress: &Progress, file: &Path) -> f64 {
    progress.files.get(file).map_or(0.0, progress::Watched::resume_at)
}

impl Actor {
    pub async fn new(core: Core, profile: ProfileId, hardware: Hardware) -> Self {
        let Hardware {
            store,
            storage,
            speech,
            wake_word,
            microphone,
            led,
            handoff,
            tv,
        } = hardware;
        let db = core.db().clone();
        let library = core.library();
        let progress = core.progress(profile).await;
        let downloading = core.clone();
        let fetcher = fetch::spawn(store.clone(), move |path| downloading.signal(Signal::Downloading(path)));
        let voice = speech.map(|speech| {
            let hearing = core.clone();
            voice::spawn(speech, wake_word, voice::Source::Microphone(microphone), move |event| {
                hearing.signal(Signal::Heard(event));
            })
        });
        let audio = core.audio().cloned();
        if let Some(audio) = &audio {
            if sound::claimed(&db).await.is_empty()
                && let Some(headphones) = audio.headphones()
            {
                let prefs = db::AudioPrefs::Unpaired.pairing(headphones.address.clone());
                if let Err(error) = db.save_audio_prefs(ProfileId::FIRST, &prefs).await {
                    eprintln!("headphones: {error}");
                }
            }
            audio.set_kept(sound::claimed(&db).await);
        }
        let scanning = (store.clone(), library.clone());
        let downloaded = spawn_blocking(move || scanning.0.downloaded(scanning.1.paths()))
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
            voice,
            ear: voice::Ear::Idle,
            queued: None,
            up_next: None,
            led,
            audio,
            offer: None,
            waking_tv: Arc::new(AtomicBool::new(false)),
            screen: screen::Screen::Tv,
            notice: None,
            pace: audio::Pace::default(),
            handoff,
            tv,
            questions: HashMap::new(),
            asked: 0,
        };
        actor.apply_audio(profile).await;
        actor.refresh_people().await;
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
                    Some(Input::Request(Request::Subtitles(change), reply)) if self.on_tv.is_some() => {
                        self.ask_tv_subtitles(change, reply);
                    }
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
                    self.publish_playback();
                    if self.on_tv.is_none() {
                        self.hear_sound();
                    }
                    if self.up_next.as_ref().is_some_and(|up| up.due(Instant::now())) {
                        self.play_up_next().await;
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
            Signal::Heard(heard) => self.hear(heard).await,
            Signal::Tv(said) => self.tv_said(said).await,
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
            Signal::Tended(tended) => self.tended(tended),
        }
    }

    async fn key(&mut self, command: Command) {
        if command == Command::Listen {
            self.listen();
            return;
        }
        if command == Command::Back && self.ear.active() {
            self.cancel_voice();
            return;
        }
        if command == Command::Dialogue {
            self.toggle_dialogue().await;
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
            self.refresh_people().await;
        }
        self.core.bus.key(command);
    }

    async fn apply_dialogue(&mut self, by: ProfileId) -> Outcome {
        let on = self.core.dialogue(by).await.load(Ordering::SeqCst);
        let said = sound::dialogue_said(on);
        if let Some(tv) = self.on_tv.as_ref().filter(|tv| tv.owner == by) {
            tv.link.send(ToTv::Dialogue {
                on,
                said: said.to_string(),
            });
        }
        Outcome::done(said)
    }

    async fn toggle_dialogue(&mut self) {
        let by = self.on_tv.as_ref().map_or(self.profile, |tv| tv.owner);
        let on = !self.core.dialogue(by).await.load(Ordering::SeqCst);
        if let Err(error) = self.core.save_dialogue(by, on).await {
            eprintln!("dialogue mode: {error}");
            self.show_message("Dialogue mode could not be saved".to_string());
            return;
        }
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

    fn publish_home(&self) {
        self.core.bus.home(Home {
            library: self.library.clone(),
            profile: self.profile,
            people: self.people.clone(),
            progress: self.progress.clone(),
            offline: self.store.is_enabled(),
            downloaded: self.downloaded.clone(),
            downloading: self.downloading.clone(),
            problem: self.problem.clone(),
        });
    }

    async fn refresh_people(&mut self) {
        let profiles = self.db.profiles().await.unwrap_or_else(|error| {
            eprintln!("profiles: {error}");
            Vec::new()
        });
        self.people = profiles.into_iter().map(|profile| (profile.id, profile.name)).collect();
        self.publish_home();
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
        if let Err(error) = self.db.save_progress(profile, &progress).await {
            eprintln!("progress: {error}");
        }
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
            self.core.forget_progress();
            self.progress = self.core.progress(self.profile).await;
        }
        self.library = library.clone();
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
        Box::pin(async move {
            self.cancel_up_next();
            if self.screen != screen::Screen::Tv {
                self.screen_step(screen::Event::TvPlays, owner).await;
            }
            let here = self.core.playable(path);
            let source = match block_in_place(|| self.store.source(path, self.online)) {
                _ if here != path => here,
                Source::Stream => path.to_path_buf(),
                Source::Cached(file) => file,
                Source::Unavailable => {
                    self.show_message(OUT_OF_REACH.to_string());
                    return;
                }
            };
            self.subtitles.set_owner(owner);
            self.apply_audio(owner).await;
            let theirs = self.progress_of(owner).await;
            let tracks = match theirs.files.get(path) {
                Some(watched) if start > 0.0 => watched.tracks(),
                _ => progress::Tracks::default(),
            };
            let dialogue = self.core.dialogue(owner).await.load(Ordering::SeqCst);
            let play = Play {
                play: crate::auth::token(),
                owner,
                file: path.to_path_buf(),
                source: source.clone(),
                start,
                paused: false,
                tracks,
                caption: self.caption(path),
                title: self.title(path),
                subtitles: PlaySubtitles {
                    settings: self.subtitles.settings_of(owner),
                    remembered: Remembered {
                        delay: self.subtitles.file_delay(&source),
                        embedded: self.subtitles.embedded(&source),
                    },
                    sidecars: Vec::new(),
                },
                audio_device: self.audio_device(),
                dialogue,
                next: up_next::follows(&self.library, path),
            };
            let (link, handed_off) = match self.tv_set_takes(&source).await {
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
                    (self.tv_set(start, tracks), true)
                }
                None => (self.tv.clone(), false),
            };
            link.send(ToTv::Play(Box::new(play.clone())));
            self.on_tv = Some(OnTv {
                link,
                handed_off,
                shown: self.now_playing_for(path),
                file: path.to_path_buf(),
                owner,
                hold: Hold::default(),
                last: Report::before(&play),
                saved: play.start,
                play: play.play,
            });
            self.publish_playback();
        })
    }

    /// A tv end of its own for one hand-off to the TV set's player. It ends,
    /// and stops the TV set, once the core lets go of its link.
    fn tv_set(&self, start: renderer::Start, tracks: progress::Tracks) -> Link {
        let telling = self.core.clone();
        let television = self.television.clone();
        let returning = self.television.clone();
        let mut once = Some((start, television));
        link::local::spawn(
            move |said| telling.signal(Signal::Tv(said)),
            move |play, callbacks| {
                let (start, television) = once
                    .take()
                    .ok_or_else(|| std::io::Error::other("this hand-off already played"))?;
                let prepare = move || {
                    if let Ok(tv::Woken::TurnedOn) = television.wake() {
                        television.wait_until_showing(TV_WAKE_WAIT);
                    }
                };
                let returning = returning.clone();
                let player::Callbacks { report, key, exit } = callbacks;
                let exit = Box::new(move |ended: Ended| {
                    if let Err(failure) = returning.show_semmeltv() {
                        eprintln!("TV: could not switch back to SemmelTV: {failure:?}");
                    }
                    exit(ended);
                });
                Player::hand_off(
                    &play.play,
                    start,
                    tracks,
                    prepare,
                    player::Callbacks { report, key, exit },
                )
            },
        )
    }

    async fn started(&mut self, play: &str) {
        let Some(tv) = self.on_tv.as_ref().filter(|tv| tv.play == play) else {
            return;
        };
        let (file, owner, handed_off) = (tv.file.clone(), tv.owner, tv.handed_off);
        if !handed_off {
            self.wake_tv();
        }
        self.record_start(&file, owner).await;
        self.lead = Some(file);
        self.queue_downloads(true);
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
        self.asked += 1;
        let id = self.asked;
        let (answer, answered) = oneshot::channel();
        self.questions.retain(|_, waiting| !waiting.is_closed());
        self.questions.insert(id, answer);
        tv.link.send(ToTv::Subtitles {
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
            FromTv::Hello { .. } | FromTv::Ask(_) | FromTv::Remember { .. } => {}
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
        tv.last = report;
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

    async fn tv_set_takes(&self, source: &Path) -> Option<(Handoff, tv::upnp::Renderer, stream::probe::Media)> {
        let handoff = self.handoff.clone()?;
        let renderer = self.television.renderer()?;
        match stream::probe::probe(source).await {
            Ok(media) if handoff.takes(&media) => Some((handoff, renderer, media)),
            Ok(_) => None,
            Err(error) => {
                eprintln!("tv player: {}: {error}", source.display());
                None
            }
        }
    }

    fn audio_device(&self) -> String {
        self.audio
            .as_ref()
            .map_or_else(|| audio::TV_DEVICE.to_string(), Audio::device)
    }

    fn move_audio(&self, route: &audio::Route) {
        if let Some(tv) = &self.on_tv {
            tv.link.send(ToTv::AudioDevice {
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
        let caption = self.caption(&tv.file);
        let noted = self
            .db
            .note_play(&tv.play, watching::now(), tv.owner, "TV", &caption, tv.last.played)
            .await;
        if let Err(error) = noted {
            eprintln!("usage: {error}");
        }
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
                self.core.bus.up_next(Some(up.card.clone()));
            }
        }
    }

    async fn play_up_next(&mut self) {
        let Some(UpNext { file, owner, .. }) = self.up_next.take() else {
            return;
        };
        self.core.bus.up_next(None);
        let start = resume_point(&*self.progress_of(owner).await, &file);
        self.play_from(&file, start, owner).await;
    }

    fn cancel_up_next(&mut self) {
        if self.up_next.take().is_some() {
            self.core.bus.up_next(None);
        }
    }

    fn listen(&self) {
        match &self.voice {
            Some(voice) => voice.listen(),
            None => self.tell("Voice control is not set up"),
        }
    }

    async fn hear(&mut self, heard: Heard) {
        let muting = heard == Heard::Listening && self.on_tv.as_ref().is_some_and(|tv| !tv.last.state.muted);
        let before = self.ear;
        let (ear, pass) = before.hear(&heard, muting);
        self.ear = ear;
        if pass == voice::Pass::Drop {
            return;
        }
        if let Some(led) = &self.led {
            led.set(ear.active());
        }
        if heard == Heard::Transcribing {
            self.core.bus.voice(VoiceEvent::Transcribing);
            self.show_voice("", "Working out what you said…", None);
            return;
        }
        if heard == Heard::Listening {
            self.core.bus.voice(VoiceEvent::Listening);
            if let Some(tv) = self.on_tv.as_ref().filter(|_| muting) {
                tv.mute(true);
            }
            self.show_voice("", "Listening…", None);
            return;
        }
        let unmute = before.unmute();
        if let Heard::Transcript(text) = &heard
            && self.answer_offer(text, unmute)
        {
            return;
        }
        let action = match &heard {
            Heard::Transcript(text) if !text.is_empty() => {
                eprintln!("heard: {text}");
                Some(intent::understand(text, &self.library, &self.progress))
            }
            _ => None,
        };
        if unmute
            && action != Some(Action::Control(Control::Mute))
            && let Some(tv) = &self.on_tv
        {
            tv.mute(false);
        }
        match (heard, action) {
            (Heard::Transcript(text), Some(action)) => {
                let understood = !matches!(action, Action::Refuse(_));
                let outcome = self.describe(&action);
                self.core.bus.voice(VoiceEvent::Heard {
                    text: text.clone(),
                    action: outcome.clone(),
                    understood,
                });
                self.show_voice(&format!("“{text}”"), &outcome, Some(VOICE_RESULT_TIME));
                self.act(action).await;
            }
            (Heard::NoMicrophone, _) => {
                self.core.bus.voice(VoiceEvent::NoMicrophone);
                self.show_voice("", "No microphone is connected", Some(VOICE_RESULT_TIME));
            }
            (Heard::Failed(error), _) => {
                eprintln!("voice: {error}");
                self.core.bus.voice(VoiceEvent::Failed { message: error });
                self.show_voice("", "Voice control failed", Some(VOICE_RESULT_TIME));
            }
            (Heard::Transcript(_), None) => {
                self.core.bus.voice(VoiceEvent::Nothing);
                self.show_voice("", "I didn't catch that", Some(VOICE_RESULT_TIME));
            }
            _ => {
                self.core.bus.voice(VoiceEvent::Nothing);
                self.show_voice("", "I didn't hear anything", Some(VOICE_RESULT_TIME));
            }
        }
    }

    fn cancel_voice(&mut self) {
        if let Some(voice) = &self.voice {
            voice.cancel();
        }
        let unmute = std::mem::replace(&mut self.ear, voice::Ear::Cancelled).unmute();
        if let Some(led) = &self.led {
            led.set(false);
        }
        if unmute && let Some(tv) = &self.on_tv {
            tv.mute(false);
        }
        self.core.bus.voice(VoiceEvent::Cancelled);
        self.core.bus.panel(None);
        if let Some(tv) = &self.on_tv {
            tv.link.send(ToTv::HideToast);
        }
    }

    async fn answer(&mut self, request: Request) -> Outcome {
        let action = match request {
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
            Request::Storage(change) => return self.change_storage(change).await,
            Request::Dismiss(kind, key, by) => return self.dismiss(kind, &key, by).await,
            Request::Mark(kind, key, episode, mark, by) => return self.mark(kind, &key, episode, mark, by).await,
            Request::CatchUp { show, to, by } => return self.catch_up(&show, to, by).await,
            Request::TogetherOnTv(show) => {
                self.publish_together(&show).await;
                return Outcome::done("");
            }
            Request::WatchWithOnTv { show, with } => return self.watch_with_on_tv(&show, with).await,
            Request::Phone(report) => return self.phone(report).await,
            Request::Screen(command, by) => {
                if matches!(command, ScreenCommand::Flip { to: ScreenKind::Tv, .. }) {
                    self.become_profile(by).await;
                }
                return self.screen_command(command, by).await;
            }
            Request::TvProfile(by) => return self.show_profile(by).await,
            Request::Dialogue(by) => return self.apply_dialogue(by).await,
            Request::Listen => {
                return match &self.voice {
                    Some(voice) => {
                        voice.listen();
                        Outcome::done("Listening")
                    }
                    None => Outcome::refused("Voice control is not set up"),
                };
            }
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
        let name = self.core.profile_name(to).await.unwrap_or_else(|| "them".to_string());
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

    async fn together_on_tv(&self, key: &str) -> Option<Together> {
        let library = self.library.clone();
        let show = library.shows.iter().find(|show| show.key == key)?;
        let partners = self.db.partners(self.profile, key).await.unwrap_or_else(|error| {
            eprintln!("together: partners of {} in {key}: {error}", self.profile.0);
            Vec::new()
        });
        let profiles = self.db.profiles().await.unwrap_or_else(|error| {
            eprintln!("together: profiles: {error}");
            Vec::new()
        });
        let here = together::Spot::of(&self.progress, show);
        let mut people = Vec::new();
        for partner in &partners {
            let Some(found) = profiles.iter().find(|found| found.id == *partner) else {
                continue;
            };
            let spot = together::Spot::of(&*self.progress_of(*partner).await, show);
            people.push(Mate {
                id: *partner,
                name: found.name.clone(),
                line: spot.line(spot.same_as(here)),
                paired: true,
            });
        }
        let others = profiles
            .iter()
            .filter(|found| found.id != self.profile && !partners.contains(&found.id));
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

    async fn publish_together(&self, key: &str) {
        if let Some(together) = self.together_on_tv(key).await {
            self.core.bus.together(together);
        }
    }

    async fn watch_with_on_tv(&mut self, key: &str, with: ProfileId) -> Outcome {
        let by = self.profile;
        let paired = self
            .db
            .partners(by, key)
            .await
            .is_ok_and(|partners| partners.contains(&with));
        let outcome = if paired {
            self.catch_up(key, with, by).await
        } else {
            self.pair(key, with, by).await
        };
        self.show_message(outcome.message.clone());
        self.publish_together(key).await;
        outcome
    }

    async fn pair(&self, key: &str, with: ProfileId, by: ProfileId) -> Outcome {
        let Some(show) = self.library.shows.iter().find(|show| show.key == key) else {
            return Outcome::refused(format!("There is no {key} in the library"));
        };
        let name = self.core.profile_name(with).await.unwrap_or_else(|| "them".to_string());
        match self.db.set_together(by, with, key, true).await {
            Ok(true) => Outcome::done(format!("You watch {} together with {name}", show.title)),
            Ok(false) => Outcome::refused(format!("You can't watch {} with {name}", show.title)),
            Err(error) => {
                eprintln!("together: {} with {}: {error}", by.0, with.0);
                Outcome::refused("Watching together could not be saved")
            }
        }
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

    async fn change_storage(&mut self, change: storage::Change) -> Outcome {
        let storage::Change::Remove(title) = change;
        let outcome = tending::remove(self.job(), title).await;
        if outcome.ok {
            self.tend();
        }
        outcome
    }

    fn change_subtitles(&self, change: subtitles::Change) -> Outcome {
        match change {
            subtitles::Change::Restyle => Outcome::done("Saved"),
            _ => Outcome::refused(NOTHING_PLAYING),
        }
    }

    fn speed(&self, speed: f64) -> Outcome {
        let Some(tv) = &self.on_tv else {
            return Outcome::refused(NOTHING_PLAYING);
        };
        let speed = if tv.handed_off {
            1.0
        } else {
            player::speed_within(speed)
        };
        tv.link.send(ToTv::Speed {
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
            tv.link.send(ToTv::Seek {
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
        tv.link.send(ToTv::SkipSegment { play: tv.play.clone() });
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
        tv.link.send(ToTv::HideSkip { play: tv.play.clone() });
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
        let now_playing = self.on_tv.as_ref().map(|OnTv { last, shown: base, .. }| {
            let state = last.state;
            NowPlaying {
                position: state.position,
                duration: state.duration,
                paused: state.paused,
                muted: state.muted,
                speed: state.speed,
                subtitle: state.subtitle,
                subtitle_delay: state.subtitle_delay,
                audio: state.audio,
                loading: !state.started,
                jump: state.jump,
                skip: state.skip.map(Skip::from),
                ..base.clone()
            }
        });
        self.core.bus.playback(now_playing);
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
        self.offer = None;
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
                            &format!("{}. Say “yes” to download it, or confirm on the phone.", best.facts),
                            OFFER_TIME,
                            true,
                        );
                        self.offer = Some(Offer {
                            proposal: proposal.id,
                            label: heading,
                            until: Instant::now() + OFFER_TIME,
                        });
                    }
                    None => {
                        let status = match &proposal.answer {
                            Answer::Question(question) => {
                                format!("{} Choose on the phone's Downloads page.", question.text)
                            }
                            _ => proposal.message().to_string(),
                        };
                        self.tell_download(&heading, &status, VOICE_RESULT_TIME, false);
                    }
                }
            }
            Found::Started(outcome) => self.tell_download("", &outcome.message, VOICE_RESULT_TIME, outcome.ok),
            Found::Failed(error) => {
                eprintln!("{error}");
                self.tell_download("", "The download service did not answer", VOICE_RESULT_TIME, false);
            }
        }
    }

    fn tell_download(&self, heading: &str, status: &str, hold: Duration, understood: bool) {
        self.core.bus.voice(VoiceEvent::Heard {
            text: heading.trim_matches(['“', '”']).to_string(),
            action: status.to_string(),
            understood,
        });
        self.show_voice(heading, status, Some(hold));
    }

    fn answer_offer(&mut self, text: &str, unmute: bool) -> bool {
        let Some(offer) = self.offer.take().filter(|offer| offer.until > Instant::now()) else {
            return false;
        };
        let Some(yes) = intent::reply(text) else {
            self.offer = Some(offer);
            return false;
        };
        let Some(downloads) = self.downloads.clone() else {
            return false;
        };
        if unmute && let Some(tv) = &self.on_tv {
            tv.mute(false);
        }
        eprintln!("heard: {text}");
        if !yes {
            let label = offer.label.clone();
            spawn_blocking(move || {
                let _ = downloads.dismiss(&offer.proposal);
            });
            self.tell_download(
                &format!("“{text}”"),
                &format!("Not downloading {label}"),
                VOICE_RESULT_TIME,
                true,
            );
            return true;
        }
        let status = format!("Starting {}…", offer.label);
        self.core.bus.voice(VoiceEvent::Heard {
            text: text.to_string(),
            action: status.clone(),
            understood: true,
        });
        self.show_voice(&format!("“{text}”"), &status, None);
        let core = self.core.clone();
        spawn_blocking(move || {
            let found = match downloads.confirm(&offer.proposal) {
                Ok(outcome) => Found::Started(outcome),
                Err(error) => Found::Failed(error),
            };
            core.signal(Signal::Download(found));
        });
        true
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
        if let Err(error) = self.db.set_setting(TV_PROFILE, Some(&profile.0.to_string())).await {
            eprintln!("tv profile: {error}");
        }
        if self.on_tv.is_none() {
            self.apply_audio(profile).await;
        }
        self.refresh_people().await;
        self.queue_downloads(false);
    }

    async fn apply_audio(&self, profile: ProfileId) {
        let Some(audio) = &self.audio else {
            return;
        };
        let prefs = self.db.audio_prefs(profile).await.unwrap_or_else(|error| {
            eprintln!("headphones: {error}");
            db::AudioPrefs::default()
        });
        block_in_place(|| {
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
        });
    }

    async fn show_profile(&mut self, profile: ProfileId) -> Outcome {
        self.become_profile(profile).await;
        let name = self
            .core
            .profile_name(profile)
            .await
            .unwrap_or_else(|| format!("profile {}", profile.0));
        Outcome::done(format!("The TV shows what {name} is watching"))
    }

    fn show_voice(&self, heard: &str, status: &str, hold: Option<Duration>) {
        if let Some(tv) = &self.on_tv {
            tv.toast(heard, status, hold.unwrap_or(VOICE_WAITING_TIME));
        }
        self.core.bus.panel(Some(Panel {
            heading: heard.to_string(),
            status: status.to_string(),
            hold,
        }));
    }

    fn show_message(&self, message: String) {
        self.core.bus.message(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Event;

    fn hardware() -> Hardware {
        Hardware {
            store: Store::new(PathBuf::from("/lib"), None, None),
            storage: None,
            speech: None,
            wake_word: None,
            microphone: None,
            led: None,
            handoff: None,
            tv: Link::pair().0,
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
        let play = match orders.recv_timeout(Duration::from_secs(2)) {
            Ok(ToTv::Play(play)) => play.play,
            other => panic!("the tv was told {other:?}"),
        };
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

    /// Waits until the actor has handled everything sent before.
    async fn settled(core: &Core) {
        core.ask(Request::Listen).await.unwrap();
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
            the_api_answers_while_the_tv_takes_nothing(&core).await;
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
                    let player = Player::without_mpv(&play.play, callbacks);
                    player.hold_forever();
                    let _ = started.send(());
                    Ok(player)
                },
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
            |play, callbacks| Ok(Player::without_mpv(&play.play, callbacks)),
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

    #[tokio::test(flavor = "multi_thread")]
    async fn a_video_that_does_not_start_says_why_and_leaves_nothing_playing() {
        let (core, inbox) = Core::detached(db::test_library()).await;
        let telling = core.clone();
        let tv = link::local::spawn(
            move |said| telling.signal(Signal::Tv(said)),
            |_, _| Err(std::io::Error::other("mpv is missing")),
        );
        let core = core_with_tv(core, inbox, tv).await;
        let mut events = core.subscribe();
        core.ask(Request::PlayFile(up(&core))).await.unwrap();
        let message = loop {
            if let Event::Message(message) = events.recv().await.unwrap() {
                break message;
            }
        };
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
    async fn a_download_asked_for_by_voice_names_the_tvs_profile() {
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
        core.forget_progress();
        assert_eq!(core.progress(mami).await.files[&up].stopped_at(), Some(600.0));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_home_the_tv_draws_follows_the_tv_profile() {
        let core = core().await;
        let mami = ProfileId(2);
        core.ask(Request::Phone(report(&core, stream::Moment::Paused, 600.0, mami)))
            .await;
        assert_eq!(core.home().profile, ProfileId::FIRST);
        assert!(watching::current(&core.home().library, &core.home().progress).is_empty());
        let mut events = core.subscribe();
        let shown = core.ask(Request::TvProfile(mami)).await.unwrap();
        assert_eq!(shown.message, "The TV shows what Mami is watching");
        let home = loop {
            if let Event::Home(home) = events.recv().await.unwrap() {
                break home;
            }
        };
        assert_eq!((home.profile, core.tv_profile()), (mami, mami));
        assert_eq!(watching::current(&home.library, &home.progress).len(), 1);
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

    async fn next_together(events: &mut tokio::sync::broadcast::Receiver<Event>) -> crate::bus::Together {
        loop {
            if let Event::Together(together) = events.recv().await.unwrap() {
                return together;
            }
        }
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
        let core = core().await;
        let mami = ProfileId(2);
        let mut events = core.subscribe();
        core.send(Request::TogetherOnTv("dark".to_string()));
        let together = next_together(&mut events).await;
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
        assert_eq!(core.db().partners(mami, "dark").await.unwrap(), [ProfileId::FIRST]);
        let together = next_together(&mut events).await;
        assert_eq!(people(&together)[0], ("Mami", "Hasn't started yet", true));
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
        core.send(Request::TogetherOnTv("dark".to_string()));
        let mut together = next_together(&mut events).await;
        while together.people[0].line == "Hasn't started yet" {
            together = next_together(&mut events).await;
        }
        assert_eq!(people(&together)[0], ("Mami", "Up next S1 E2", true));
        assert_eq!(
            core.ask(with_mami()).await.unwrap(),
            Outcome::done("You're at Mami's place now: Dark · S1 E2")
        );
        let together = next_together(&mut events).await;
        assert_eq!(people(&together)[0], ("Mami", "At your place", true));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_new_library_reaches_both_frontends_at_once() {
        let core = core().await;
        let mut events = core.subscribe();
        let mut library = (*core.library()).clone();
        let mut diaries = library.movies[0].clone();
        diaries.id = 87;
        diaries.key = crate::library::movie_key("The Princess Diaries", Some(2001));
        diaries.title = "The Princess Diaries".to_string();
        diaries.path = PathBuf::from("/lib/Movies/The Princess Diaries (2001)/Diaries.mkv");
        library.movies.push(diaries);
        core.signal(Signal::Library(Ok(library)));
        let home = loop {
            if let Event::Home(home) = events.recv().await.unwrap() {
                break home;
            }
        };
        assert!(
            home.library
                .movies
                .iter()
                .any(|movie| movie.title == "The Princess Diaries")
        );
        assert!(core.library().movies.iter().any(|movie| movie.id == 87));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_remote_key_goes_to_the_screen_while_nothing_plays() {
        let core = core().await;
        let mut events = core.subscribe();
        core.send(Request::Key(Command::Down));
        let key = loop {
            if let Event::Key(key) = events.recv().await.unwrap() {
                break key;
            }
        };
        assert_eq!(key, Command::Down);
        let paused = core.ask(Request::Control(Control::Pause)).await.unwrap();
        assert_eq!(paused, Outcome::refused("Pause, but nothing is playing"));
    }
}
