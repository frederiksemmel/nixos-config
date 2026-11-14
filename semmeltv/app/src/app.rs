use crate::api::{self, Outcome, Pending, Request, Seek, Shared};
use crate::artwork;
use crate::audio::{self, Audio};
use crate::bus::{Bus, NowPlaying, Voice as VoiceEvent};
use crate::cache::Cache;
use crate::cec;
use crate::command::Command;
use crate::details::{self, Details, Ratings};
use crate::display;
use crate::downloads::remote::Downloads;
use crate::downloads::{Outcome as Started, Proposal};
use crate::fetch;
use crate::heatmap;
use crate::intent::{self, Action, Control};
use crate::led::Led;
use crate::library::{self, Item, Library};
use crate::loader::{self, Job, Pixels};
use crate::nav::{Cursor, Effect, HomeRow, Nav, Reload, Watch};
use crate::offline::{self, Presence, Source, Store};
use crate::player::{self, Finish, Player};
use crate::progress::{self, Position, Progress};
use crate::seek::Direction;
use crate::segments::Title;
use crate::screen;
use crate::storage::{self, Keep};
use crate::stream;
use crate::subtitles::{self, Subtitles};
use crate::tags::Tags;
use crate::tmdb::Kind;
use crate::tv::{self, Television};
use crate::voice::{self, Heard, Voice};
use crate::watch::{self, Change};
use crate::watching::{self, Current};
use crate::{AppWindow, CardData, OfflineState, ShelfData};
use slint::{Color, ComponentHandle, Image, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

const MESSAGE_TIME: Duration = Duration::from_secs(4);
const VOICE_RESULT_TIME: Duration = Duration::from_secs(6);
const VOICE_WAITING_TIME: Duration = Duration::from_secs(20);
const EMPTY_LIBRARY: &str = "The library is empty";
const OUT_OF_REACH: &str = "Not downloaded, and pCloud can't be reached right now";
const NOTHING_PLAYING: &str = "Nothing is playing";
const PLAYBACK_EVERY: Duration = Duration::from_millis(500);
const SCREEN_EVERY: Duration = Duration::from_secs(2);
const OFFER_TIME: Duration = Duration::from_secs(60);
const TV_WAKE_WAIT: Duration = Duration::from_secs(20);
const TV_PICTURE_SETTLE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
enum Screen {
    Home,
    Show,
}

enum Input {
    Command(Command),
    Request(Pending),
    Stop,
    Download(Found),
    TvWaking,
    TvShowing,
    Audio(audio::Route),
}

enum Found {
    Proposal(Proposal),
    Started(Started),
    Failed(String),
}

struct Offer {
    proposal: String,
    label: String,
    until: Instant,
}

#[derive(Clone)]
pub struct Remote {
    inputs: Sender<Input>,
    ui: slint::Weak<AppWindow>,
}

impl Remote {
    pub fn send(&self, command: Command) {
        self.deliver(Input::Command(command));
    }

    pub fn stop(&self) {
        self.deliver(Input::Stop);
    }

    fn move_audio(&self, route: audio::Route) {
        self.deliver(Input::Audio(route));
    }

    fn request(&self, pending: Pending) {
        self.deliver(Input::Request(pending));
    }

    fn deliver(&self, input: Input) {
        if self.inputs.send(input).is_ok() {
            let _ = self.ui.upgrade_in_event_loop(|ui| ui.invoke_remote_input());
        }
    }
}

struct Target {
    screen: Screen,
    shelf: usize,
    card: usize,
    key: SharedString,
}

pub struct Setup {
    pub root: PathBuf,
    pub meta: PathBuf,
    pub remote: Option<String>,
    pub store: Store,
    pub storage: Option<storage::Cache>,
    pub cache: Cache,
    pub speech: Option<String>,
    pub wake_word: Option<voice::WakeWord>,
    pub microphone: Option<String>,
    pub led: Option<Led>,
    pub api: Option<api::Config>,
    pub tv: tv::Settings,
    pub downloads: Option<Downloads>,
    pub headphones: bool,
}

struct Playing {
    file: PathBuf,
}

struct App {
    ui: AppWindow,
    nav: Nav,
    cache: Cache,
    art: PathBuf,
    images: Sender<Job<Target>>,
    player: Option<Player>,
    progress: Progress,
    playing: Option<Playing>,
    message_timer: Timer,
    voice_timer: Timer,
    store: Store,
    storage: Option<storage::Cache>,
    storage_stuck: bool,
    fetcher: Sender<Vec<PathBuf>>,
    lead: Option<PathBuf>,
    requested: Vec<PathBuf>,
    downloaded: HashSet<PathBuf>,
    downloading: Option<PathBuf>,
    online: bool,
    problem: Option<String>,
    deferred: Option<Result<Library, String>>,
    voice: Option<Voice>,
    voice_active: bool,
    voice_cancelled: bool,
    unmute_after_listening: bool,
    keys: Remote,
    queued: Option<PathBuf>,
    led: Option<Led>,
    bus: Bus,
    shared: Arc<Shared>,
    now_playing: Option<NowPlaying>,
    subtitles: Subtitles,
    television: Television,
    downloads: Option<Downloads>,
    audio: Option<Audio>,
    offer: Option<Offer>,
    waking_tv: Arc<AtomicBool>,
    paused_for_tv: Cell<bool>,
    paused_for_handoff: bool,
    screen: screen::Screen,
    notice: Option<screen::Said>,
    stopping_tv: bool,
}

pub fn run(setup: Setup) -> Result<(), slint::PlatformError> {
    let Setup {
        root,
        meta,
        remote,
        store,
        storage,
        cache,
        speech,
        wake_word,
        microphone,
        led,
        api,
        tv,
        downloads,
        headphones,
    } = setup;
    let ui = AppWindow::new()?;
    splash_after_first_frame(&ui);
    let handle = ui.as_weak();
    let images = loader::spawn(move |target, pixels| {
        let handle = handle.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = handle.upgrade() {
                show_image(&ui, target, pixels);
            }
        });
    });

    let (changes, received) = mpsc::channel();
    let notify = {
        let handle = ui.as_weak();
        move |change: Change| {
            if changes.send(change).is_ok() {
                let _ = handle.upgrade_in_event_loop(|ui| ui.invoke_changed());
            }
        }
    };
    let notify_watch = notify.clone();
    let fetcher = fetch::spawn(store.clone(), move |path| notify(Change::Downloading(path)));

    let (heard, received_heard) = mpsc::channel();
    let voice = speech.map(|speech| {
        let handle = ui.as_weak();
        voice::spawn(speech, wake_word, voice::Source::Microphone(microphone), move |event| {
            if heard.send(event).is_ok() {
                let _ = handle.upgrade_in_event_loop(|ui| ui.invoke_voice_heard());
            }
        })
    });

    let (inputs, received_inputs) = mpsc::channel();
    let keys = Remote {
        inputs,
        ui: ui.as_weak(),
    };
    let saved = cache.saved_manifest();
    let library = library::load_saved(&root, &saved).unwrap_or_default();
    let progress = cache.load_progress();
    let bus = Bus::default();
    let subtitles = Subtitles::load(cache.subtitles_path());
    let television = Television::new(tv);
    let audio = headphones.then(|| {
        let keys = keys.clone();
        Audio::spawn(bus.clone(), Box::new(move |route| keys.move_audio(route)))
    });
    let shared = Arc::new(Shared::new(subtitles.clone()));
    shared.set_library(&library);
    shared.set_progress(&progress);
    let art = meta.join(artwork::DIR);
    let app = Rc::new(RefCell::new(App {
        ui: ui.clone_strong(),
        downloaded: store.downloaded(library.paths()),
        nav: Nav::new(library),
        cache,
        art: art.clone(),
        images,
        player: None,
        progress,
        playing: None,
        message_timer: Timer::default(),
        voice_timer: Timer::default(),
        store,
        storage,
        storage_stuck: false,
        fetcher,
        lead: None,
        requested: Vec::new(),
        downloading: None,
        online: true,
        problem: None,
        deferred: None,
        voice,
        voice_active: false,
        voice_cancelled: false,
        unmute_after_listening: false,
        keys: keys.clone(),
        queued: None,
        led,
        bus: bus.clone(),
        shared: shared.clone(),
        now_playing: None,
        subtitles,
        television: television.clone(),
        downloads,
        audio: audio.clone(),
        offer: None,
        waking_tv: Arc::new(AtomicBool::new(false)),
        paused_for_tv: Cell::new(false),
        paused_for_handoff: false,
        screen: screen::Screen::Tv,
        notice: None,
        stopping_tv: false,
    }));
    app.borrow_mut().present_home();
    app.borrow_mut().queue_downloads(false);
    app.borrow_mut().tend_storage();

    let input = Rc::downgrade(&app);
    ui.on_key(move |text| {
        let Some(command) = Command::from_key(&text) else {
            return false;
        };
        if let Some(app) = input.upgrade() {
            App::dispatch(&app, command);
        }
        true
    });
    let remote_input = Rc::downgrade(&app);
    ui.on_remote_input(move || {
        if let Some(app) = remote_input.upgrade() {
            for input in received_inputs.try_iter() {
                match input {
                    Input::Command(command) => App::dispatch(&app, command),
                    Input::Request(Pending { request, reply }) => {
                        let outcome = app.borrow_mut().answer(request);
                        let _ = reply.send(outcome);
                    }
                    Input::Stop => app.borrow().control(Control::Stop),
                    Input::Download(found) => app.borrow_mut().download_found(found),
                    Input::TvWaking => app.borrow().pause_for_tv(),
                    Input::TvShowing => app.borrow().resume_after_tv(),
                    Input::Audio(route) => app.borrow().move_audio(&route),
                }
            }
        }
    });
    let voice_input = Rc::downgrade(&app);
    ui.on_voice_heard(move || {
        if let Some(app) = voice_input.upgrade() {
            for event in received_heard.try_iter() {
                app.borrow_mut().hear(event);
            }
        }
    });
    if let Some(config) = api {
        let keys = keys.clone();
        let ask: api::Ask = Arc::new(move |pending| keys.request(pending));
        api::spawn(config, shared, meta.clone(), bus, ask, television, audio);
    }
    cec::spawn(keys);

    let playback_timer = Timer::default();
    let ticking = Rc::downgrade(&app);
    playback_timer.start(TimerMode::Repeated, PLAYBACK_EVERY, move || {
        if let Some(app) = ticking.upgrade()
            && let Ok(app) = app.try_borrow()
        {
            app.publish_playback();
        }
    });

    let screen_timer = Timer::default();
    let watching_screen = Rc::downgrade(&app);
    screen_timer.start(TimerMode::Repeated, SCREEN_EVERY, move || {
        if let Some(app) = watching_screen.upgrade()
            && let Ok(mut app) = app.try_borrow_mut()
            && app.screen != screen::Screen::Tv
        {
            app.screen_step(screen::Event::Tick);
        }
    });

    let finished = Rc::downgrade(&app);
    ui.on_playback_finished(move |success, ended| {
        if let Some(app) = finished.upgrade() {
            app.borrow_mut().finish_playback(success, ended);
        }
    });

    let saved_position = Rc::downgrade(&app);
    ui.on_position_saved(move || {
        if let Some(app) = saved_position.upgrade() {
            app.borrow_mut().record_place();
        }
    });

    let resized = Rc::downgrade(&app);
    ui.on_resized(move || {
        if resized.upgrade().is_some_and(|app| app.borrow().player.is_none()) {
            thread::spawn(display::show_ui);
        }
    });
    thread::spawn(display::show_ui);

    let changed = Rc::downgrade(&app);
    ui.on_changed(move || {
        if let Some(app) = changed.upgrade() {
            for change in received.try_iter() {
                app.borrow_mut().apply(change);
            }
        }
    });
    watch::spawn(
        watch::Paths {
            root,
            meta,
            saved,
            remote,
        },
        notify_watch,
    );
    ui.run()
}

fn splash_after_first_frame(ui: &AppWindow) {
    let handle = ui.as_weak();
    let notified = ui.window().set_rendering_notifier(move |state, _| {
        if let slint::RenderingState::AfterRendering = state
            && let Some(ui) = handle.upgrade()
            && !ui.get_painted()
        {
            ui.set_painted(true);
        }
    });
    if notified.is_err() {
        ui.set_painted(true);
    }
}

fn show_image(ui: &AppWindow, target: Target, pixels: Pixels) {
    let shelves = match target.screen {
        Screen::Home => ui.get_home_shelves(),
        Screen::Show => ui.get_show_shelves(),
    };
    let Some(shelf) = shelves.row_data(target.shelf) else {
        return;
    };
    if let Some(mut card) = shelf.cards.row_data(target.card)
        && card.key == target.key
    {
        card.art = Image::from_rgb8(pixels);
        card.has_art = true;
        shelf.cards.set_row_data(target.card, card);
    }
}

fn tints(title: &str) -> (Color, Color) {
    let mut hasher = DefaultHasher::new();
    title.hash(&mut hasher);
    let hue = (hasher.finish() % 360) as f32;
    (
        Color::from_hsva(hue, 0.5, 0.42, 1.0),
        Color::from_hsva((hue + 40.0) % 360.0, 0.6, 0.2, 1.0),
    )
}

fn card(key: &str, title: String, detail: String) -> CardData {
    let (tint_a, tint_b) = tints(&title);
    CardData {
        key: key.into(),
        title: title.into(),
        detail: detail.into(),
        tint_a,
        tint_b,
        ..CardData::default()
    }
}

fn cols_model(cursor: &Cursor) -> ModelRc<i32> {
    ModelRc::new(VecModel::from(
        cursor.cols.iter().map(|col| *col as i32).collect::<Vec<_>>(),
    ))
}

fn row_title(row: HomeRow) -> &'static str {
    match row {
        HomeRow::Watching => "Currently Watching",
        HomeRow::Newest => "Newly Added",
        HomeRow::Shows => "TV Shows",
        HomeRow::Movies => "Movies",
    }
}

fn carried_art(shelves: &ModelRc<ShelfData>) -> HashMap<SharedString, Image> {
    shelves
        .iter()
        .flat_map(|shelf| shelf.cards.iter().collect::<Vec<_>>())
        .filter(|card| card.has_art)
        .map(|card| (card.key, card.art))
        .collect()
}

fn offline_state(presence: Presence) -> OfflineState {
    match presence {
        Presence::Unknown => OfflineState::Unknown,
        Presence::Remote => OfflineState::Remote,
        Presence::Downloading => OfflineState::Downloading,
        Presence::Partial => OfflineState::Partial,
        Presence::Downloaded => OfflineState::Downloaded,
    }
}

fn set_status(card: &mut CardData, status: (Presence, String)) {
    card.offline = offline_state(status.0);
    card.offline_text = status.1.into();
}

impl App {
    fn dispatch(app: &Rc<RefCell<Self>>, command: Command) {
        if command == Command::Listen {
            app.borrow().listen();
            return;
        }
        if command == Command::Back && app.borrow().voice_active {
            app.borrow_mut().cancel_voice();
            return;
        }
        if let Some(player) = &app.borrow().player {
            player.command(command);
            return;
        }
        if command == Command::Subtitles {
            return;
        }
        let effect = app.borrow_mut().nav.apply(command);
        match effect {
            Effect::None => {}
            Effect::Moved => app.borrow().present_focus(),
            Effect::OpenShow => app.borrow_mut().present_show(),
            Effect::CloseShow => {
                let this = app.borrow();
                this.ui.set_show_open(false);
                this.present_focus();
            }
            Effect::Play(path) => app.borrow_mut().play(&path),
        }
    }

    fn apply(&mut self, change: Change) {
        match change {
            Change::Poll { online } => {
                let reconnected = online && !self.online;
                self.online = online;
                self.refresh_offline();
                self.tend_storage();
                if reconnected {
                    self.queue_downloads(true);
                }
            }
            Change::Downloading(path) => {
                self.downloading = path;
                self.refresh_offline();
                self.tend_storage();
            }
            Change::Library(result) if self.player.is_some() => self.deferred = Some(result),
            Change::Library(result) => self.reload(result),
        }
    }

    fn reload(&mut self, result: Result<Library, String>) {
        let library = match result {
            Ok(library) => library,
            Err(error) => {
                eprintln!("{error}");
                self.problem = Some(error);
                self.present_home();
                return;
            }
        };
        self.problem = None;
        if *self.nav.library() == library {
            self.present_home();
            return;
        }
        self.downloaded = self.store.downloaded(library.paths());
        self.shared.set_library(&library);
        match self.nav.replace(library) {
            Reload::ShowGone => self.ui.set_show_open(false),
            Reload::ShowChanged => self.present_show(),
            Reload::NoShow | Reload::ShowKept => {}
        }
        self.present_home();
        self.queue_downloads(false);
    }

    fn refresh_offline(&mut self) {
        self.downloaded = self.store.downloaded(self.nav.library().paths());
        let current = watching::current(self.nav.library(), &self.progress);
        for (index, row) in self.nav.home_rows().iter().enumerate() {
            let Some(shelf) = self.ui.get_home_shelves().row_data(index) else {
                continue;
            };
            let statuses: Vec<_> = match row {
                HomeRow::Watching => current.iter().map(|c| self.file_status(&c.file)).collect(),
                _ => self
                    .nav
                    .row_items(*row)
                    .into_iter()
                    .map(|item| self.item_status(item))
                    .collect(),
            };
            update_statuses(&shelf, statuses.into_iter());
        }
        if let Some((show, _)) = self.nav.open_show() {
            for (index, season) in show.seasons.iter().enumerate() {
                if let Some(shelf) = self.ui.get_show_shelves().row_data(index) {
                    update_statuses(&shelf, season.episodes.iter().map(|e| self.file_status(&e.path)));
                }
            }
        }
        self.present_details();
    }

    fn file_status(&self, path: &Path) -> (Presence, String) {
        if !self.store.is_enabled() {
            return (Presence::Unknown, String::new());
        }
        let downloading = self.downloading.as_deref() == Some(path);
        offline::single(self.downloaded.contains(path), downloading)
    }

    fn item_status(&self, item: Item) -> (Presence, String) {
        match item {
            Item::Show(index) if self.store.is_enabled() => {
                let show = self.nav.show(index);
                match show.only_episode() {
                    Some(episode) => self.file_status(&episode.path),
                    None => {
                        let done = show.episodes().filter(|e| self.downloaded.contains(&e.path)).count();
                        let busy = show.episodes().any(|e| self.downloading.as_ref() == Some(&e.path));
                        offline::group(done, show.episode_count(), busy)
                    }
                }
            }
            Item::Show(_) => (Presence::Unknown, String::new()),
            Item::Movie(index) => self.file_status(&self.nav.movie(index).path),
        }
    }

    fn play(&mut self, path: &Path) {
        self.stopping_tv = false;
        if self.screen != screen::Screen::Tv {
            self.screen_step(screen::Event::TvPlays);
        }
        let source = match self.store.source(path, self.online) {
            Source::Stream => path.to_path_buf(),
            Source::Cached(file) => file,
            Source::Unavailable => {
                self.show_message(OUT_OF_REACH.to_string());
                return;
            }
        };
        let handle = self.ui.as_weak();
        let saved = move || {
            let _ = handle.upgrade_in_event_loop(|ui| ui.invoke_position_saved());
        };
        let handle = self.ui.as_weak();
        let finished = move |finish: Finish| {
            let _ = handle.upgrade_in_event_loop(move |ui| {
                ui.invoke_playback_finished(finish != Finish::Failed, finish == Finish::Ended)
            });
        };
        let keys = self.keys.clone();
        let watch_later_dir = self.cache.watch_later_dir(path);
        match Player::start(
            &source,
            &watch_later_dir,
            self.caption(path),
            self.title(path),
            self.subtitles.clone(),
            &self.audio_device(),
            saved,
            move |command| keys.send(command),
            finished,
        ) {
            Ok(player) => {
                self.player = Some(player);
                self.now_playing = Some(self.now_playing_for(path));
                self.publish_playback();
                self.paused_for_tv.set(false);
                self.wake_tv();
                self.record_start(path);
                self.lead = Some(path.to_path_buf());
                self.queue_downloads(true);
            }
            Err(error) => self.show_message(format!("mpv did not start: {error}")),
        }
    }

    fn audio_device(&self) -> String {
        self.audio
            .as_ref()
            .map_or_else(|| audio::TV_DEVICE.to_string(), Audio::device)
    }

    fn move_audio(&self, route: &audio::Route) {
        if let Some(player) = &self.player {
            player.set_audio_device(&route.device);
            player.say(&route.said);
        }
    }

    fn queue_downloads(&mut self, again: bool) {
        if !self.store.is_enabled() || !self.online {
            return;
        }
        let paths = watching::downloads(self.nav.library(), &self.progress, self.lead.as_deref());
        if again || paths != self.requested {
            self.requested = paths.clone();
            let _ = self.fetcher.send(paths);
        }
    }

    fn caption(&self, path: &Path) -> String {
        if let Some((show, episode)) = self.nav.library().find_episode(path) {
            return format!("{} · {}", show.title, episode.label());
        }
        let movie = self.nav.library().movies.iter().find(|movie| movie.path == path);
        movie.map_or_else(|| path.display().to_string(), |movie| movie.title.clone())
    }

    fn title(&self, path: &Path) -> Option<Title> {
        let library = self.nav.library();
        if let Some((show, episode)) = library.find_episode(path) {
            return Some(Title {
                tmdb: show.tmdb?,
                episode: Some((episode.season, episode.number)),
            });
        }
        let movie = library.movies.iter().find(|movie| movie.path == path)?;
        Some(Title {
            tmdb: movie.tmdb?,
            episode: None,
        })
    }

    fn record_start(&mut self, path: &Path) {
        let found = self.nav.library().find_episode(path);
        let show = found.map(|(show, episode)| {
            let position = Position {
                season: episode.season,
                episode: episode.number,
                finished: false,
            };
            (show.key.clone(), position)
        });
        self.playing = Some(Playing {
            file: path.to_path_buf(),
        });
        if let Some((key, position)) = show {
            self.progress.shows.insert(key, position);
        }
        self.progress.played(path, watching::now());
        self.save_progress();
    }

    fn record_place(&mut self) {
        if let (Some(player), Some(playing)) = (&self.player, &self.playing) {
            self.progress.record(&playing.file, player.place());
            self.progress.played(&playing.file, watching::now());
            let library = self.nav.library();
            let joined = watching::record(
                &mut self.progress,
                library,
                &playing.file,
                player.played(),
                watching::now(),
            );
            self.save_progress();
            if joined {
                self.queue_downloads(false);
            }
        }
    }

    fn finish_playback(&mut self, success: bool, ended: bool) {
        self.stopping_tv = false;
        self.record_place();
        self.player = None;
        self.now_playing = None;
        self.publish_playback();
        if let Some(playing) = self.playing.take() {
            let library = self.nav.library();
            if ended {
                watching::complete(&mut self.progress, library, &playing.file);
                self.save_progress();
            } else if watching::stopped(&mut self.progress, library, &playing.file) {
                self.forget_place(&playing.file);
                self.save_progress();
            }
        }
        if !success {
            self.show_message("This video could not be played".to_string());
        }
        if let Some(result) = self.deferred.take() {
            self.reload(result);
        }
        self.present_home();
        self.queue_downloads(false);
        if let Some(path) = self.queued.take() {
            self.play(&path);
        }
    }

    fn listen(&self) {
        match &self.voice {
            Some(voice) => voice.listen(),
            None => self.tell("Voice control is not set up"),
        }
    }

    fn hear(&mut self, heard: Heard) {
        if heard == Heard::Listening {
            self.voice_cancelled = false;
        } else if self.voice_cancelled {
            if heard != Heard::Transcribing {
                self.voice_cancelled = false;
            }
            return;
        }
        self.voice_active = matches!(heard, Heard::Listening | Heard::Transcribing);
        if let Some(led) = &self.led {
            led.set(self.voice_active);
        }
        if heard == Heard::Transcribing {
            self.bus.voice(VoiceEvent::Transcribing);
            self.show_voice("", "Working out what you said…", None);
            return;
        }
        if heard == Heard::Listening {
            self.bus.voice(VoiceEvent::Listening);
            if let Some(player) = self.player.as_ref().filter(|player| !player.muted()) {
                player.set_muted(true);
                self.unmute_after_listening = true;
            }
            self.show_voice("", "Listening…", None);
            return;
        }
        if let Heard::Transcript(text) = &heard
            && self.answer_offer(text)
        {
            return;
        }
        let action = match &heard {
            Heard::Transcript(text) if !text.is_empty() => {
                eprintln!("heard: {text}");
                Some(intent::understand(text, self.nav.library(), &self.progress))
            }
            _ => None,
        };
        if std::mem::take(&mut self.unmute_after_listening)
            && action != Some(Action::Control(Control::Mute))
            && let Some(player) = &self.player
        {
            player.set_muted(false);
        }
        match (heard, action) {
            (Heard::Transcript(text), Some(action)) => {
                let understood = !matches!(action, Action::Refuse(_));
                let outcome = self.describe(&action);
                self.bus.voice(VoiceEvent::Heard {
                    text: text.clone(),
                    action: outcome.clone(),
                    understood,
                });
                self.show_voice(&format!("“{text}”"), &outcome, Some(VOICE_RESULT_TIME));
                self.act(action);
            }
            (Heard::NoMicrophone, _) => {
                self.bus.voice(VoiceEvent::NoMicrophone);
                self.show_voice("", "No microphone is connected", Some(VOICE_RESULT_TIME));
            }
            (Heard::Failed(error), _) => {
                eprintln!("voice: {error}");
                self.bus.voice(VoiceEvent::Failed { message: error });
                self.show_voice("", "Voice control failed", Some(VOICE_RESULT_TIME));
            }
            (Heard::Transcript(_), None) => {
                self.bus.voice(VoiceEvent::Nothing);
                self.show_voice("", "I didn't catch that", Some(VOICE_RESULT_TIME));
            }
            _ => {
                self.bus.voice(VoiceEvent::Nothing);
                self.show_voice("", "I didn't hear anything", Some(VOICE_RESULT_TIME));
            }
        }
    }

    fn cancel_voice(&mut self) {
        if let Some(voice) = &self.voice {
            voice.cancel();
        }
        self.voice_active = false;
        self.voice_cancelled = true;
        if let Some(led) = &self.led {
            led.set(false);
        }
        if std::mem::take(&mut self.unmute_after_listening)
            && let Some(player) = &self.player
        {
            player.set_muted(false);
        }
        self.bus.voice(VoiceEvent::Cancelled);
        self.voice_timer.stop();
        self.ui.set_voice_transcript("".into());
        self.ui.set_voice_status("".into());
        if let Some(player) = &self.player {
            player.hide_toast();
        }
    }

    fn answer(&mut self, request: Request) -> Outcome {
        let action = match request {
            Request::Understand(text) => intent::understand(&text, self.nav.library(), &self.progress),
            Request::Play(pick) => intent::choose(&pick, self.nav.library(), &self.progress),
            Request::Control(control) => Action::Control(control),
            Request::PlayPause => {
                let paused = self.player.as_ref().is_some_and(|player| player.state().paused);
                Action::Control(if paused { Control::Resume } else { Control::Pause })
            }
            Request::Seek(seek) => return self.seek(seek),
            Request::Speed(speed) => return self.speed(speed),
            Request::Subtitles(change) => return self.change_subtitles(change),
            Request::Storage(change) => return self.change_storage(change),
            Request::Dismiss(kind, key) => return self.dismiss(kind, &key),
            Request::Mark(kind, key, episode, mark) => return self.mark(kind, &key, episode, mark),
            Request::Phone(report) => return self.phone(report),
            Request::Screen(command) => return self.screen_command(command),
            Request::Listen => {
                return match &self.voice {
                    Some(voice) => {
                        voice.listen();
                        Outcome {
                            message: "Listening".to_string(),
                            ok: true,
                        }
                    }
                    None => Outcome {
                        message: "Voice control is not set up".to_string(),
                        ok: false,
                    },
                };
            }
        };
        let message = self.describe(&action);
        let ok = match &action {
            Action::Play(_) => true,
            Action::Control(control) if control.is_tv() => true,
            Action::Control(_) => self.player.is_some(),
            Action::Download { .. } => self.downloads.is_some(),
            Action::Refuse(_) => false,
        };
        self.act(action);
        self.publish_playback();
        Outcome { message, ok }
    }

    fn cached_titles(&self, cache: &storage::Cache) -> Result<Vec<storage::Cached>, String> {
        let manifest = library::read_manifest(&cache.manifest)?;
        let library = Library::from_manifest(&cache.library, &manifest, |_| true);
        let activity = storage::Activity {
            playing: self.playing.as_ref().map(|playing| playing.file.as_path()),
            queued: &self.requested,
        };
        Ok(storage::titles(
            &cache.library,
            &library,
            &self.progress,
            cache.scan(),
            &activity,
        ))
    }

    fn tend_storage(&mut self) {
        let Some(cache) = self.storage.clone() else {
            return;
        };
        let disk = match cache.disk() {
            Ok(disk) => disk,
            Err(error) => return eprintln!("storage: {}: {error}", cache.data.display()),
        };
        let titles = match self.cached_titles(&cache) {
            Ok(titles) => titles,
            Err(error) => return eprintln!("storage: not tending the offline cache: {error}"),
        };
        let used: u64 = titles.iter().map(|title| title.bytes).sum();
        let excess = storage::excess(used, disk.free, cache.marks);
        let chosen = storage::plan(&titles, used, disk.free, cache.marks);
        let freeing: u64 = chosen.iter().map(|&index| titles[index].bytes).sum();
        let now = watching::now();
        let why = format!(
            "the offline cache holds {} of {} with {} free on the disk",
            storage::size(used),
            storage::size(cache.marks.high),
            storage::size(disk.free)
        );
        for &index in &chosen {
            let title = &titles[index];
            match cache.remove(&title.files) {
                Ok(()) => eprintln!(
                    "storage: removed {}, {}: {why}",
                    title.describe(now),
                    storage::size(title.bytes)
                ),
                Err(error) => eprintln!("storage: could not remove {}: {error}", title.describe(now)),
            }
        }
        let stuck = freeing < excess;
        if stuck && !self.storage_stuck {
            eprintln!("storage: {why}, and every other title is kept");
        }
        self.storage_stuck = stuck;
        if chosen.is_empty() {
            self.shared.storage.set(storage::Report::new(disk, cache.marks, titles));
        } else {
            self.refresh_offline();
            self.publish_storage();
        }
    }

    fn publish_storage(&self) {
        let Some(cache) = &self.storage else {
            return;
        };
        match (cache.disk(), self.cached_titles(cache)) {
            (Ok(disk), Ok(titles)) => self.shared.storage.set(storage::Report::new(disk, cache.marks, titles)),
            (Err(error), _) => eprintln!("storage: {}: {error}", cache.data.display()),
            (_, Err(error)) => eprintln!("storage: {error}"),
        }
    }

    fn dismiss(&mut self, kind: watching::Kind, key: &str) -> Outcome {
        if !watching::dismiss(&mut self.progress, self.nav.library(), kind, key, watching::now()) {
            return Outcome {
                message: format!("There is no {key} in the library"),
                ok: false,
            };
        }
        self.save_progress();
        Outcome {
            message: "Removed from Started and Forgotten".to_string(),
            ok: true,
        }
    }

    fn mark(&mut self, kind: watching::Kind, key: &str, episode: Option<(u32, u32)>, mark: watching::Mark) -> Outcome {
        let library = self.nav.library();
        let Some(files) = watching::mark(&mut self.progress, library, kind, key, episode, mark) else {
            return Outcome {
                message: format!("There is no {key} in the library"),
                ok: false,
            };
        };
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
        for file in &files {
            self.forget_place(file);
        }
        self.save_progress();
        self.present_home();
        self.queue_downloads(false);
        let state = match mark {
            watching::Mark::Watched => "watched",
            watching::Mark::Unwatched => "unwatched",
        };
        Outcome {
            message: format!("{name} is marked as {state}"),
            ok: true,
        }
    }

    fn place_of(&self, file: &Path, position: f64) -> Option<screen::Place> {
        let library = self.nav.library();
        if let Some((show, episode)) = library.find_episode(file) {
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
        let movie = library.movies.iter().find(|movie| movie.path == file)?;
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
        if self.stopping_tv {
            return None;
        }
        let (player, playing) = (self.player.as_ref()?, self.playing.as_ref()?);
        self.place_of(&playing.file, player.place().position)
    }

    fn resume_point(&self, file: &Path) -> f64 {
        match self.progress.files.get(file) {
            Some(watched) if !watched.finished && !watched.in_credits() => watched.stopped_at.unwrap_or(0.0),
            _ => 0.0,
        }
    }

    fn screen_step(&mut self, event: screen::Event) {
        let tv = self.tv_place();
        let effects = self.screen.step(event, tv.as_ref(), Instant::now());
        for effect in effects {
            match effect {
                screen::Effect::PauseTv => {
                    self.paused_for_handoff = self.player.as_ref().is_some_and(|player| !player.state().paused);
                    self.control(Control::Pause);
                }
                screen::Effect::ResumeTv => {
                    if std::mem::take(&mut self.paused_for_handoff) {
                        self.control(Control::Resume);
                    }
                }
                screen::Effect::StopTv => {
                    self.stopping_tv = self.player.is_some();
                    self.control(Control::Stop);
                }
                screen::Effect::PlayTv(place) => {
                    if place.position > 1.0
                        && let Err(error) =
                            stream::resume::write(&self.cache.watch_later_dir(&place.file), &place.file, place.position)
                    {
                        eprintln!("screen: could not keep where {} stopped: {error}", place.file.display());
                    }
                    self.play(&place.file);
                }
                screen::Effect::ShowOnTv(text) => self.show_message(text),
                screen::Effect::Tell(notice) => {
                    let id = self.notice.as_ref().map_or(1, |said| said.id + 1);
                    self.notice = Some(screen::Said {
                        id,
                        text: notice.text,
                        tone: notice.tone,
                    });
                }
            }
        }
        self.bus.screen(screen::View::of(&self.screen, self.notice.clone()));
    }

    fn screen_command(&mut self, command: api::ScreenCommand) -> Outcome {
        let done = |message: String| Outcome { message, ok: true };
        match command {
            api::ScreenCommand::Flip {
                to: api::ScreenKind::Tv,
                position,
                ..
            } => {
                self.screen_step(screen::Event::Flip {
                    to: screen::Target::Tv,
                    position,
                });
                done("Watching on the TV".to_string())
            }
            api::ScreenCommand::Flip {
                to: api::ScreenKind::Phone,
                device,
                ..
            } => {
                let name = device.name.clone();
                self.screen_step(screen::Event::Flip {
                    to: screen::Target::Phone(device),
                    position: None,
                });
                done(format!("Watching on {name}"))
            }
            api::ScreenCommand::Play {
                device,
                movie,
                show,
                season,
                episode,
            } => {
                let pick = match (movie, show) {
                    (Some(key), None) => intent::Pick::Movie(key),
                    (None, Some(key)) => intent::Pick::Show { key, season, episode },
                    _ => {
                        return Outcome {
                            message: "Give either movie or show".to_string(),
                            ok: false,
                        };
                    }
                };
                let path = match intent::choose(&pick, self.nav.library(), &self.progress) {
                    Action::Play(path) => path,
                    other => {
                        return Outcome {
                            message: self.describe(&other),
                            ok: false,
                        };
                    }
                };
                let Some(place) = self.place_of(&path, self.resume_point(&path)) else {
                    return Outcome {
                        message: "That is not in the library".to_string(),
                        ok: false,
                    };
                };
                let message = format!("Playing {} on {}", place.name, device.name);
                self.screen_step(screen::Event::Play { device, place });
                done(message)
            }
            api::ScreenCommand::Failed { device, reason } => {
                self.screen_step(screen::Event::Failed { device, reason });
                done("Noted".to_string())
            }
        }
    }

    fn phone(&mut self, report: stream::Report) -> Outcome {
        if let Some(device) = report.device.clone() {
            let session = report.session.clone();
            let event = match report.moment {
                stream::Moment::Start => screen::Event::Started {
                    device,
                    session,
                    position: report.position,
                },
                stream::Moment::Playing | stream::Moment::Paused => screen::Event::Report {
                    device,
                    session,
                    position: report.position,
                    paused: report.moment == stream::Moment::Paused,
                },
                stream::Moment::Stopped | stream::Moment::Ended => screen::Event::Stopped { device, session },
            };
            self.screen_step(event);
        }
        if self.playing.as_ref().is_some_and(|playing| playing.file == report.file) {
            return Outcome {
                message: "The TV is playing this, so it keeps its own place".to_string(),
                ok: false,
            };
        }
        let file = report.file.as_path();
        let now = watching::now();
        let library = self.nav.library();
        if report.moment == stream::Moment::Start
            && let Some((show, episode)) = library.find_episode(file)
        {
            let position = Position {
                season: episode.season,
                episode: episode.number,
                finished: false,
            };
            self.progress.shows.insert(show.key.clone(), position);
        }
        let place = progress::Place {
            position: report.position,
            duration: report.duration,
            credits: report.credits,
        };
        self.progress.record(file, place);
        self.progress.played(file, now);
        let joined = watching::record(&mut self.progress, library, file, report.played, now);
        let remember = match report.moment {
            stream::Moment::Ended => {
                watching::complete(&mut self.progress, library, file);
                false
            }
            stream::Moment::Stopped => !watching::stopped(&mut self.progress, library, file),
            _ => true,
        };
        if remember && report.position > 0.0 {
            if let Err(error) = stream::resume::write(&self.cache.watch_later_dir(file), file, report.position) {
                eprintln!("progress: could not keep where {} stopped: {error}", file.display());
            }
        } else if !remember {
            self.forget_place(file);
        }
        self.save_progress();
        if joined || report.moment != stream::Moment::Playing && report.moment != stream::Moment::Paused {
            self.present_home();
            self.queue_downloads(false);
        }
        Outcome {
            message: "Kept the place".to_string(),
            ok: true,
        }
    }

    fn forget_place(&self, file: &Path) {
        match std::fs::remove_dir_all(self.cache.watch_later_dir(file)) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                eprintln!("progress: could not forget where {} stopped: {error}", file.display());
            }
            _ => {}
        }
    }

    fn change_storage(&mut self, change: storage::Change) -> Outcome {
        let refuse = |message: String| Outcome { message, ok: false };
        let storage::Change::Remove(title) = change;
        let Some(cache) = self.storage.clone() else {
            return refuse("This TV keeps no downloads".to_string());
        };
        let titles = match self.cached_titles(&cache) {
            Ok(titles) => titles,
            Err(error) => return refuse(error),
        };
        let Some(cached) = titles.iter().find(|cached| cached.title == title) else {
            return refuse("That title is not downloaded".to_string());
        };
        match cached.keep {
            Some(Keep::Playing) => return refuse(format!("{} is playing", cached.name)),
            Some(Keep::UpNext) => {
                return refuse(format!(
                    "{} is up next, so it would download again straight away",
                    cached.name
                ));
            }
            _ => {}
        }
        if let Err(error) = cache.remove(&cached.files) {
            eprintln!(
                "storage: could not remove {}: {error}",
                cached.describe(watching::now())
            );
            return refuse(format!("Could not remove {}: {error}", cached.name));
        }
        eprintln!(
            "storage: removed {}, {}: asked to from the web app",
            cached.describe(watching::now()),
            storage::size(cached.bytes)
        );
        let message = format!("Removed {}, {} freed", cached.name, storage::size(cached.bytes));
        self.refresh_offline();
        self.publish_storage();
        Outcome { message, ok: true }
    }

    fn change_subtitles(&self, change: subtitles::Change) -> Outcome {
        match (&self.player, change) {
            (Some(player), change) => {
                let answer = player.subtitles(change);
                Outcome {
                    message: answer.message,
                    ok: answer.ok,
                }
            }
            (None, subtitles::Change::Restyle) => Outcome {
                message: "Saved".to_string(),
                ok: true,
            },
            (None, _) => Outcome {
                message: NOTHING_PLAYING.to_string(),
                ok: false,
            },
        }
    }

    fn speed(&self, speed: f64) -> Outcome {
        let Some(player) = &self.player else {
            return Outcome {
                message: NOTHING_PLAYING.to_string(),
                ok: false,
            };
        };
        let speed = player.set_speed(speed);
        player.say(&format!("Speed {}", player::speed_label(speed)));
        self.publish_playback();
        Outcome {
            message: format!("Speed {}", player::speed_label(speed)),
            ok: true,
        }
    }

    fn seek(&self, seek: Seek) -> Outcome {
        let Some(player) = &self.player else {
            return Outcome {
                message: NOTHING_PLAYING.to_string(),
                ok: false,
            };
        };
        match seek {
            Seek::To(seconds) => player.seek_to(seconds),
            Seek::By(seconds) => player.seek_by(seconds),
            Seek::Step(Direction::Back) => player.command(Command::Left),
            Seek::Step(Direction::Forward) => player.command(Command::Right),
        }
        self.publish_playback();
        Outcome {
            message: "Seeking".to_string(),
            ok: true,
        }
    }

    fn now_playing_for(&self, path: &Path) -> NowPlaying {
        let library = self.nav.library();
        if let Some((show, episode)) = library.find_episode(path) {
            return NowPlaying {
                title: show.title.clone(),
                caption: format!("{} · {}", episode.short_label(), episode.label()),
                show: Some(show.key.clone()),
                season: Some(episode.season),
                episode: Some(episode.number),
                loading: true,
                ..NowPlaying::default()
            };
        }
        let movie = library.movies.iter().find(|movie| movie.path == path);
        NowPlaying {
            title: movie.map_or_else(|| path.display().to_string(), |movie| movie.title.clone()),
            caption: movie
                .and_then(|movie| movie.year)
                .map(|year| year.to_string())
                .unwrap_or_default(),
            movie: movie.map(|movie| movie.key.clone()),
            loading: true,
            ..NowPlaying::default()
        }
    }

    fn publish_playback(&self) {
        let now_playing = self
            .player
            .as_ref()
            .zip(self.now_playing.as_ref())
            .map(|(player, base)| {
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
            });
        self.bus.playback(now_playing);
    }

    fn describe(&self, action: &Action) -> String {
        match action {
            Action::Play(path) => {
                let library = self.nav.library();
                let title = library
                    .find_episode(path)
                    .map(|(show, episode)| {
                        format!("{} · season {}, episode {}", show.title, episode.season, episode.number)
                    })
                    .or_else(|| {
                        let movie = library.movies.iter().find(|movie| &movie.path == path)?;
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
                match self.player {
                    Some(_) => name.to_string(),
                    None => format!("{name}, but {}", NOTHING_PLAYING.to_lowercase()),
                }
            }
        }
    }

    fn act(&mut self, action: Action) {
        match action {
            Action::Play(path) => match &self.player {
                Some(player) => {
                    self.queued = Some(path);
                    player.command(Command::Back);
                }
                None => self.play(&path),
            },
            Action::Download { title, season, episode } => self.look_for(title, season, episode),
            Action::Refuse(_) => {}
            Action::Control(control) => self.control(control),
        }
    }

    fn control(&self, control: Control) {
        let player = self.player.as_ref();
        match (control, player) {
            (Control::TvOn | Control::TvOff | Control::ShowSemmelTv, _) => {
                let sent = if control == Control::TvOff {
                    self.television.turn_off()
                } else {
                    self.television.show_semmeltv()
                };
                if let Err(failure) = sent {
                    eprintln!("TV: {failure:?}");
                    self.tell("The TV did not answer");
                }
            }
            (_, None) => {}
            (Control::Pause, Some(player)) => {
                self.paused_for_tv.set(false);
                player.set_paused(true);
            }
            (Control::Resume, Some(player)) => {
                self.paused_for_tv.set(false);
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
        let remote = self.keys.clone();
        let waking = self.waking_tv.clone();
        thread::spawn(move || {
            match television.wake() {
                Ok(tv::Woken::AlreadyShowing) => {}
                Ok(tv::Woken::Switched) => eprintln!("TV: on another input at playback, switching to SemmelTV"),
                Ok(tv::Woken::TurnedOn) => {
                    eprintln!("TV: off at playback, turning it on and pausing until it shows SemmelTV");
                    remote.deliver(Input::TvWaking);
                    if television.wait_until_showing(TV_WAKE_WAIT) {
                        thread::sleep(TV_PICTURE_SETTLE);
                    } else {
                        eprintln!("TV: still not showing SemmelTV after {TV_WAKE_WAIT:?}, playing anyway");
                    }
                    remote.deliver(Input::TvShowing);
                }
                Err(failure) => eprintln!("TV: could not turn it on for playback: {failure:?}"),
            }
            waking.store(false, Ordering::SeqCst);
        });
    }

    fn pause_for_tv(&self) {
        if let Some(player) = &self.player
            && !player.state().paused
        {
            player.set_paused(true);
            self.paused_for_tv.set(true);
        }
    }

    fn resume_after_tv(&self) {
        if self.paused_for_tv.replace(false)
            && let Some(player) = &self.player
        {
            player.set_paused(false);
        }
    }

    fn look_for(&mut self, title: String, season: Option<u32>, episode: Option<u32>) {
        let Some(downloads) = self.downloads.clone() else {
            return;
        };
        self.offer = None;
        let keys = self.keys.clone();
        let said = match season {
            Some(season) => format!("{title} season {season}"),
            None => title.clone(),
        };
        thread::spawn(move || {
            let found = match downloads.search(title, season, episode, said) {
                Ok(proposal) => Found::Proposal(proposal),
                Err(error) => Found::Failed(error),
            };
            keys.deliver(Input::Download(found));
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
        self.bus.voice(VoiceEvent::Heard {
            text: heading.trim_matches(['“', '”']).to_string(),
            action: status.to_string(),
            understood,
        });
        self.show_voice(heading, status, Some(hold));
    }

    fn answer_offer(&mut self, text: &str) -> bool {
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
        if std::mem::take(&mut self.unmute_after_listening)
            && let Some(player) = &self.player
        {
            player.set_muted(false);
        }
        eprintln!("heard: {text}");
        if !yes {
            let _ = downloads.dismiss(&offer.proposal);
            self.tell_download(
                &format!("“{text}”"),
                &format!("Not downloading {}", offer.label),
                VOICE_RESULT_TIME,
                true,
            );
            return true;
        }
        let status = format!("Starting {}…", offer.label);
        self.bus.voice(VoiceEvent::Heard {
            text: text.to_string(),
            action: status.clone(),
            understood: true,
        });
        self.show_voice(&format!("“{text}”"), &status, None);
        let keys = self.keys.clone();
        thread::spawn(move || {
            let found = match downloads.confirm(&offer.proposal) {
                Ok(outcome) => Found::Started(outcome),
                Err(error) => Found::Failed(error),
            };
            keys.deliver(Input::Download(found));
        });
        true
    }

    fn tell(&self, text: &str) {
        match &self.player {
            Some(player) => player.say(text),
            None => self.show_message(text.to_string()),
        }
    }

    fn save_progress(&self) {
        self.shared.set_progress(&self.progress);
        if let Err(error) = self.cache.save_progress(&self.progress) {
            eprintln!("progress: {error}");
        }
    }

    fn show_voice(&self, heard: &str, status: &str, hold: Option<Duration>) {
        if let Some(player) = &self.player {
            player.toast(heard, status, hold.unwrap_or(VOICE_WAITING_TIME));
        }
        self.ui.set_voice_transcript(heard.into());
        self.ui.set_voice_status(status.into());
        self.ui.set_voice_busy(hold.is_none());
        match hold {
            Some(time) => {
                let handle = self.ui.as_weak();
                self.voice_timer.start(TimerMode::SingleShot, time, move || {
                    if let Some(ui) = handle.upgrade() {
                        ui.set_voice_transcript("".into());
                        ui.set_voice_status("".into());
                    }
                });
            }
            None => self.voice_timer.stop(),
        }
    }

    fn show_message(&self, message: String) {
        self.ui.set_message(message.into());
        let handle = self.ui.as_weak();
        self.message_timer.start(TimerMode::SingleShot, MESSAGE_TIME, move || {
            if let Some(ui) = handle.upgrade() {
                ui.set_message("".into());
            }
        });
    }

    fn present_focus(&self) {
        let home = self.nav.home_cursor();
        self.ui.set_home_row(home.row as i32);
        self.ui.set_home_cols(cols_model(home));
        if let Some((show, cursor)) = self.nav.open_show() {
            self.ui.set_show_row(cursor.row as i32);
            self.ui.set_show_cols(cols_model(cursor));
            self.ui.set_show_heat(heatmap::focus(show, self.nav.heat()));
        }
        self.present_details();
    }

    fn present_details(&self) {
        let line = match self.nav.open_show() {
            Some(_) => self.nav.focused_episode().map(|episode| {
                let status = self.file_status(&episode.path).1;
                self.file_details(None, &episode.tags, episode.runtime, &episode.path, &status)
            }),
            None => self.nav.focused_item().map(|item| self.item_details(item)),
        };
        self.ui.set_details(line.unwrap_or_default().into());
        let years = match (self.nav.open_show(), self.nav.focused_item()) {
            (None, Some(Item::Movie(index))) => self.nav.movie(index).year.map(|year| year.to_string()),
            (None, Some(Item::Show(index))) => self.nav.show(index).years(),
            _ => None,
        };
        self.ui.set_years(years.unwrap_or_default().into());
        let scores = match self.nav.open_show() {
            Some(_) => self.nav.focused_episode().map(|episode| &episode.ratings),
            None => self.nav.focused_item().map(|item| match item {
                Item::Movie(index) => &self.nav.movie(index).ratings,
                Item::Show(index) => &self.nav.show(index).ratings,
            }),
        };
        let ratings = scores.map(Ratings::from).unwrap_or_default();
        self.ui.set_ratings(ratings.into());
    }

    fn item_details(&self, item: Item) -> String {
        let status = self.item_status(item).1;
        match item {
            Item::Movie(index) => {
                let movie = self.nav.movie(index);
                self.file_details(None, &movie.tags, movie.runtime, &movie.path, &status)
            }
            Item::Show(index) => {
                let show = self.nav.show(index);
                let next = self.progress.resume_episode(show).or_else(|| show.first_episode());
                next.map_or(status.clone(), |episode| {
                    let label = show.only_episode().is_none().then(|| episode.short_label());
                    self.file_details(label, &episode.tags, episode.runtime, &episode.path, &status)
                })
            }
        }
    }

    fn file_details(
        &self,
        episode: Option<String>,
        tags: &Tags,
        runtime: Option<u32>,
        file: &Path,
        download: &str,
    ) -> String {
        let watched = self.progress.files.get(file).copied().unwrap_or_default();
        let runtime = runtime.map(|minutes| f64::from(minutes) * 60.0);
        Details {
            episode,
            tags: Some(tags),
            duration: watched.duration.or(runtime),
            stopped_at: watched.stopped_at,
            download,
        }
        .line()
    }

    fn request(&self, screen: Screen, shelf: usize, card: usize, key: SharedString, art: Option<PathBuf>) {
        if let Some(path) = art {
            let target = Target {
                screen,
                shelf,
                card,
                key,
            };
            let _ = self.images.send(Job { target, path });
        }
    }

    fn home_card(&self, item: Item) -> (CardData, Option<PathBuf>) {
        let (mut card, art) = match item {
            Item::Show(index) => {
                let show = self.nav.show(index);
                (
                    card(&show.key, show.title.clone(), show.subtitle()),
                    show.tmdb.map(|id| artwork::backdrop(&self.art, Kind::Tv, id)),
                )
            }
            Item::Movie(index) => {
                let movie = self.nav.movie(index);
                let year = movie.year.map(|year| year.to_string()).unwrap_or_default();
                (
                    card(&movie.key, movie.title.clone(), year),
                    movie.tmdb.map(|id| artwork::backdrop(&self.art, Kind::Movie, id)),
                )
            }
        };
        set_status(&mut card, self.item_status(item));
        (card, art)
    }

    fn watching_card(&self, current: &Current) -> (CardData, Option<PathBuf>) {
        let (mut card, art) = self.home_card(current.item);
        let episode = match (current.item, current.episode) {
            (Item::Show(index), Some((season, number))) => self.nav.show(index).episode(season, number),
            _ => None,
        };
        let parts = [
            episode.map(|episode| episode.short_label()),
            current.remaining().map(details::time_left),
        ];
        let detail: Vec<_> = parts.into_iter().flatten().collect();
        if !detail.is_empty() {
            card.detail = detail.join(" · ").into();
        }
        card.progress = current.fraction();
        set_status(&mut card, self.file_status(&current.file));
        (card, art)
    }

    fn present_home(&mut self) {
        let current = watching::current(self.nav.library(), &self.progress);
        let watches = current.iter().map(|c| Watch {
            item: c.item,
            file: c.file.clone(),
        });
        self.nav.set_watching(watches.collect());
        let carried = carried_art(&self.ui.get_home_shelves());
        let mut shelves = Vec::new();
        for (index, row) in self.nav.home_rows().iter().enumerate() {
            let mut cards = Vec::new();
            for (position, item) in self.nav.row_items(*row).into_iter().enumerate() {
                let (mut card, art) = match row {
                    HomeRow::Watching => self.watching_card(&current[position]),
                    _ => self.home_card(item),
                };
                match carried.get(&card.key) {
                    Some(image) => {
                        card.art = image.clone();
                        card.has_art = true;
                    }
                    None => self.request(Screen::Home, index, position, card.key.clone(), art),
                }
                cards.push(card);
            }
            shelves.push(ShelfData {
                title: row_title(*row).into(),
                cards: ModelRc::new(VecModel::from(cards)),
            });
        }
        let note = match &self.problem {
            Some(problem) => problem.as_str(),
            None if self.nav.library().is_empty() => EMPTY_LIBRARY,
            None => "",
        };
        self.ui.set_empty_note(note.into());
        self.ui.set_tmdb_notice(self.nav.library().uses_tmdb());
        self.ui.set_home_shelves(ModelRc::new(VecModel::from(shelves)));
        self.present_focus();
    }

    fn present_show(&mut self) {
        let Some((show, cursor)) = self.nav.open_show() else {
            return;
        };
        let carried = carried_art(&self.ui.get_show_shelves());
        let mut shelves = Vec::new();
        for (shelf, season) in show.seasons.iter().enumerate() {
            let mut cards = Vec::new();
            for (position, episode) in season.episodes.iter().enumerate() {
                let key = show.episode_key(episode);
                let mut card = card(&key, episode.label(), String::new());
                card.ratings = Ratings::from(&episode.ratings).into();
                set_status(&mut card, self.file_status(&episode.path));
                match carried.get(&card.key) {
                    Some(image) => {
                        card.art = image.clone();
                        card.has_art = true;
                    }
                    None => {
                        let art = show
                            .tmdb
                            .map(|id| artwork::still(&self.art, id, episode.season, episode.number));
                        self.request(Screen::Show, shelf, position, card.key.clone(), art);
                    }
                }
                cards.push(card);
            }
            shelves.push(ShelfData {
                title: season.label().into(),
                cards: ModelRc::new(VecModel::from(cards)),
            });
        }
        self.ui.set_show_title(show.title.as_str().into());
        let subtitle = show.years().into_iter().chain([show.subtitle()]).collect::<Vec<_>>();
        self.ui.set_show_subtitle(subtitle.join(" · ").into());
        self.ui.set_show_shelves(ModelRc::new(VecModel::from(shelves)));
        self.ui.set_show_heatmap(heatmap::data(show));
        self.ui.set_show_heat(heatmap::focus(show, self.nav.heat()));
        self.ui.set_show_row(cursor.row as i32);
        self.ui.set_show_cols(cols_model(cursor));
        self.ui.set_show_open(true);
        self.present_details();
    }
}

fn update_statuses(shelf: &ShelfData, statuses: impl Iterator<Item = (Presence, String)>) {
    for (index, status) in statuses.enumerate() {
        if let Some(mut card) = shelf.cards.row_data(index)
            && (card.offline != offline_state(status.0) || card.offline_text != status.1.as_str())
        {
            set_status(&mut card, status);
            shelf.cards.set_row_data(index, card);
        }
    }
}
