use super::tending::{self, Tended};
use super::{Core, Input, Outcome, Request, ScreenCommand, ScreenKind, Seek, Signal, TV_PROFILE, sound};
use crate::audio::{self, Audio};
use crate::bus::{Home, NowPlaying, Panel, Playing, Voice as VoiceEvent};
use crate::card;
use crate::command::Command;
use crate::db::{self, Db};
use crate::downloads::remote::Downloads;
use crate::downloads::{Outcome as Started, Proposal};
use crate::fetch;
use crate::hold::{self, Hold};
use crate::intent::{self, Action, Control};
use crate::led::Led;
use crate::library::Library;
use crate::offline::{Source, Store};
use crate::overlay;
use crate::player::{self, Finish, Player};
use crate::profile::ProfileId;
use crate::progress::{self, Position, Progress};
use crate::screen;
use crate::seek::Direction;
use crate::segments::Title;
use crate::storage;
use crate::stream;
use crate::subtitles::{self, Subtitles};
use crate::tv::{self, Television};
use crate::voice::{self, Heard, Voice};
use crate::watching;
use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
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

type Boxed<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

pub struct Hardware {
    pub store: Store,
    pub storage: Option<storage::Cache>,
    pub speech: Option<String>,
    pub wake_word: Option<voice::WakeWord>,
    pub microphone: Option<String>,
    pub led: Option<Led>,
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

/// What the TV plays: the running player, what it shows, the file and whose
/// progress it moves. One value, so the TV never holds a player without the
/// rest or the rest without a player.
struct OnTv {
    player: Player,
    shown: NowPlaying,
    file: PathBuf,
    owner: ProfileId,
    hold: Hold,
}

impl OnTv {
    fn act(&mut self, (hold, act): (Hold, hold::Act)) {
        self.hold = hold;
        match act {
            hold::Act::Nothing => {}
            hold::Act::Pause => self.player.set_paused(true),
            hold::Act::Resume => self.player.set_paused(false),
        }
    }

    fn playing(&self) -> bool {
        !self.player.state().paused
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
    deferred: Option<Result<Library, String>>,
    voice: Option<Voice>,
    ear: voice::Ear,
    queued: Option<PathBuf>,
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
            if sound::claimed(&db, None).await.is_empty()
                && let Some(headphones) = audio.headphones()
            {
                let prefs = db::AudioPrefs::Unpaired.pairing(headphones.address.clone());
                if let Err(error) = db.save_audio_prefs(ProfileId::FIRST, &prefs).await {
                    eprintln!("headphones: {error}");
                }
            }
            audio.set_kept(sound::claimed(&db, None).await);
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
            deferred: None,
            voice,
            ear: voice::Ear::Idle,
            queued: None,
            led,
            audio,
            offer: None,
            waking_tv: Arc::new(AtomicBool::new(false)),
            screen: screen::Screen::Tv,
            notice: None,
            pace: audio::Pace::default(),
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
                    self.hear_sound();
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
            Signal::Saved => self.record_place().await,
            Signal::Finished(finish) => {
                self.finish_playback(finish != Finish::Failed, finish == Finish::Ended)
                    .await;
            }
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
            Signal::Library(result) if self.on_tv.is_some() => self.deferred = Some(result),
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
        if let Some(player) = self.player() {
            player.command(command);
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
            if self.screen != screen::Screen::Tv {
                self.screen_step(screen::Event::TvPlays, owner).await;
            }
            let source = match block_in_place(|| self.store.source(path, self.online)) {
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
            let saving = self.core.clone();
            let saved = move || saving.signal(Signal::Saved);
            let finishing = self.core.clone();
            let finished = move |finish: Finish| finishing.signal(Signal::Finished(finish));
            let keys = self.core.clone();
            let started = block_in_place(|| {
                Player::start(
                    &source,
                    start,
                    tracks,
                    self.caption(path),
                    self.billing(path, start),
                    self.title(path),
                    self.subtitles.clone(),
                    &self.audio_device(),
                    saved,
                    move |command| keys.send(Request::Key(command)),
                    finished,
                )
            });
            match started {
                Ok(player) => {
                    self.on_tv = Some(OnTv {
                        player,
                        shown: self.now_playing_for(path),
                        file: path.to_path_buf(),
                        owner,
                        hold: Hold::default(),
                    });
                    self.publish_playback();
                    self.wake_tv();
                    self.record_start(path, owner).await;
                    self.lead = Some(path.to_path_buf());
                    self.queue_downloads(true);
                }
                Err(error) => self.show_message(format!("mpv did not start: {error}")),
            }
        })
    }

    fn audio_device(&self) -> String {
        self.audio
            .as_ref()
            .map_or_else(|| audio::TV_DEVICE.to_string(), Audio::device)
    }

    fn move_audio(&self, route: &audio::Route) {
        if let Some(player) = self.player() {
            player.set_audio_device(&route.device);
            player.say(&route.said);
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

    fn billing(&self, path: &Path, start: f64) -> card::Billing {
        let (title, mut lines) = match self.library.find_episode(path) {
            Some((show, episode)) => (show.title.clone(), vec![episode.label()]),
            None => (self.caption(path), Vec::new()),
        };
        if start > 1.0 {
            lines.push(format!("Picks up at {}", overlay::clock(start)));
        }
        card::Billing { title, lines }
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
    }

    async fn record_place(&mut self) {
        let Some(OnTv {
            player, file, owner, ..
        }) = &self.on_tv
        else {
            return;
        };
        let (file, owner) = (file.clone(), *owner);
        let (place, tracks, played) = (player.place(), player.tracks(), player.played());
        let mut progress = (*self.progress_of(owner).await).clone();
        progress.record(&file, place);
        progress.keep_tracks(&file, tracks);
        progress.played(&file, watching::now());
        watching::record(&mut progress, &self.library, &file, played, watching::now());
        self.keep(owner, progress).await;
    }

    async fn finish_playback(&mut self, success: bool, ended: bool) {
        self.record_place().await;
        let ended_play = self.on_tv.take().map(|OnTv { file, owner, .. }| (file, owner));
        self.publish_playback();
        if let Some((file, owner)) = ended_play {
            let mut progress = (*self.progress_of(owner).await).clone();
            let changed = if ended {
                watching::complete(&mut progress, &self.library, &file);
                true
            } else {
                watching::stopped(&mut progress, &self.library, &file)
            };
            if changed {
                self.keep(owner, progress).await;
            }
        }
        self.subtitles.set_owner(self.profile);
        if !success {
            self.show_message("This video could not be played".to_string());
        }
        if let Some(result) = self.deferred.take() {
            self.reload(result).await;
        }
        self.publish_home();
        self.queue_downloads(false);
        self.tend();
        if let Some(path) = self.queued.take() {
            self.play(&path).await;
        }
    }

    fn listen(&self) {
        match &self.voice {
            Some(voice) => voice.listen(),
            None => self.tell("Voice control is not set up"),
        }
    }

    async fn hear(&mut self, heard: Heard) {
        let muting = heard == Heard::Listening && self.player().is_some_and(|player| !player.muted());
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
            if let Some(player) = self.player().filter(|_| muting) {
                player.set_muted(true);
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
            && let Some(player) = self.player()
        {
            player.set_muted(false);
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
        if unmute && let Some(player) = self.player() {
            player.set_muted(false);
        }
        self.core.bus.voice(VoiceEvent::Cancelled);
        self.core.bus.panel(None);
        if let Some(player) = self.player() {
            player.hide_toast();
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
                let paused = self.player().is_some_and(|player| player.state().paused);
                Action::Control(if paused { Control::Resume } else { Control::Pause })
            }
            Request::Seek(seek) => return self.seek(seek),
            Request::Speed(speed) => return self.speed(speed),
            Request::Subtitles(change) => return self.change_subtitles(change),
            Request::Storage(change) => return self.change_storage(change).await,
            Request::Dismiss(kind, key, by) => return self.dismiss(kind, &key, by).await,
            Request::Mark(kind, key, episode, mark, by) => return self.mark(kind, &key, episode, mark, by).await,
            Request::Phone(report) => return self.phone(report).await,
            Request::Screen(command, by) => {
                if matches!(command, ScreenCommand::Flip { to: ScreenKind::Tv, .. }) {
                    self.become_profile(by).await;
                }
                return self.screen_command(command, by).await;
            }
            Request::TvProfile(by) => return self.show_profile(by).await,
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
        self.place_of(&tv.file, tv.player.place().position)
    }

    async fn screen_step(&mut self, event: screen::Event, by: ProfileId) {
        let tv = self.tv_place();
        let before = self.screen.clone();
        let reason = event.describe();
        let tick = event.is_tick();
        let effects = self.screen.step(event, tv.as_ref(), Instant::now());
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
            ScreenCommand::Play { device, pick } => {
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
        match (self.player(), change) {
            (Some(player), change) => {
                let answer = player.subtitles(change);
                Outcome {
                    message: answer.message,
                    ok: answer.ok,
                }
            }
            (None, subtitles::Change::Restyle) => Outcome::done("Saved"),
            (None, _) => Outcome::refused(NOTHING_PLAYING),
        }
    }

    fn speed(&self, speed: f64) -> Outcome {
        let Some(player) = self.player() else {
            return Outcome::refused(NOTHING_PLAYING);
        };
        let speed = player.set_speed(speed);
        player.say(&format!("Speed {}", player::speed_label(speed)));
        self.publish_playback();
        Outcome::done(format!("Speed {}", player::speed_label(speed)))
    }

    fn seek(&self, seek: Seek) -> Outcome {
        let Some(player) = self.player() else {
            return Outcome::refused(NOTHING_PLAYING);
        };
        match seek {
            Seek::To(seconds) => player.seek_to(seconds),
            Seek::By(seconds) => player.seek_by(seconds),
            Seek::Step(Direction::Back) => player.command(Command::Left),
            Seek::Step(Direction::Forward) => player.command(Command::Right),
        }
        self.publish_playback();
        Outcome::done("Seeking")
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

    fn player(&self) -> Option<&Player> {
        self.on_tv.as_ref().map(|tv| &tv.player)
    }

    fn hear_sound(&mut self) {
        let Some(audio) = &self.audio else {
            return;
        };
        let playing = self
            .on_tv
            .as_ref()
            .map(|on_tv| on_tv.player.state())
            .filter(|state| state.started && !state.paused)
            .map(|state| (state.position, state.speed));
        let now = Instant::now();
        audio.heard(self.pace.observe(now, playing), now);
    }

    fn publish_playback(&self) {
        let now_playing = self.on_tv.as_ref().map(
            |OnTv {
                 player, shown: base, ..
             }| {
                let state = player.state();
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
                    ..base.clone()
                }
            },
        );
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
                Some(OnTv { player, .. }) => {
                    player.command(Command::Back);
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
        match (control, self.player()) {
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
            (Control::Pause, Some(player)) => player.set_paused(true),
            (Control::Resume, Some(player)) => {
                self.wake_tv();
                player.set_paused(false);
            }
            (Control::Stop, Some(player)) => player.command(Command::Back),
            (Control::Mute, Some(player)) => player.set_muted(true),
            (Control::Unmute, Some(player)) => player.set_muted(false),
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
        spawn_blocking(move || {
            let found = match downloads.search(title, season, episode, said) {
                Ok(proposal) => Found::Proposal(proposal),
                Err(error) => Found::Failed(error),
            };
            core.signal(Signal::Download(found));
        });
    }

    fn download_found(&mut self, found: Found) {
        match found {
            Found::Proposal(proposal) => {
                let heading = proposal.label.clone().unwrap_or_else(|| proposal.asked.clone());
                match proposal.choices.first().filter(|_| proposal.ok) {
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
                        let status = match proposal.question {
                            Some(_) => format!("{} Choose on the phone's Downloads page.", proposal.message),
                            None => proposal.message,
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
        if unmute && let Some(player) = self.player() {
            player.set_muted(false);
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
        match self.player() {
            Some(player) => player.say(text),
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
        if let Some(player) = self.player() {
            player.toast(heard, status, hold.unwrap_or(VOICE_WAITING_TIME));
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

    async fn core() -> Core {
        let (core, inbox) = Core::detached(db::test_library()).await;
        let hardware = Hardware {
            store: Store::new(PathBuf::from("/lib"), None, None),
            storage: None,
            speech: None,
            wake_word: None,
            microphone: None,
            led: None,
        };
        let actor = Actor::new(core.clone(), ProfileId::FIRST, hardware).await;
        tokio::spawn(actor.run(inbox));
        core
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
