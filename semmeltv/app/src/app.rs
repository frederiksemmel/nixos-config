use crate::artwork::{Cache, Matches};
use crate::cec;
use crate::command::Command;
use crate::details::Details;
use crate::fetch;
use crate::intent::{self, Action, Control};
use crate::library::{self, Episode, Item, Library, Show};
use crate::loader::{self, Job, Pixels};
use crate::nav::{Cursor, Effect, HomeRow, Nav, Reload};
use crate::offline::{self, Presence, Source, Store};
use crate::player::{Finish, Player};
use crate::progress::{Position, Progress};
use crate::segments::Title;
use crate::tags::Tags;
use crate::voice::{self, Heard, Voice};
use crate::watch::{self, Change};
use crate::{AppWindow, CardData, OfflineState, ShelfData};
use slint::{Color, ComponentHandle, Image, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

const MESSAGE_TIME: Duration = Duration::from_secs(4);
const EMPTY_LIBRARY: &str = "The library is empty";
const OUT_OF_REACH: &str = "Not downloaded, and pCloud can't be reached right now";
const NOTHING_PLAYING: &str = "Nothing is playing";

#[derive(Clone, Copy)]
enum Screen {
    Home,
    Show,
}

#[derive(Clone)]
pub struct Remote {
    commands: Sender<Command>,
    ui: slint::Weak<AppWindow>,
}

impl Remote {
    pub fn send(&self, command: Command) {
        if self.commands.send(command).is_ok() {
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
    pub remote: Option<String>,
    pub store: Store,
    pub cache: Cache,
    pub whisper: Option<String>,
    pub microphone: Option<String>,
}

struct Playing {
    file: PathBuf,
    show: Option<String>,
}

#[derive(Default)]
struct Deferred {
    library: Option<Result<Library, String>>,
    art: bool,
}

struct App {
    ui: AppWindow,
    nav: Nav,
    cache: Cache,
    names: Matches,
    images: Sender<Job<Target>>,
    player: Option<Player>,
    progress: Progress,
    playing: Option<Playing>,
    message_timer: Timer,
    store: Store,
    fetcher: Sender<Vec<PathBuf>>,
    downloaded: HashSet<PathBuf>,
    downloading: Option<PathBuf>,
    online: bool,
    problem: Option<String>,
    deferred: Deferred,
    voice: Option<Voice>,
    unmute_after_listening: bool,
    queued: Option<PathBuf>,
}

pub fn run(setup: Setup) -> Result<(), slint::PlatformError> {
    let Setup {
        root,
        remote,
        store,
        cache,
        whisper,
        microphone,
    } = setup;
    let ui = AppWindow::new()?;
    ui.set_tmdb_notice(cache.has_tmdb_art());
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
    let voice = whisper.map(|whisper| {
        let handle = ui.as_weak();
        voice::spawn(whisper, microphone, move |event| {
            if heard.send(event).is_ok() {
                let _ = handle.upgrade_in_event_loop(|ui| ui.invoke_voice_heard());
            }
        })
    });

    let saved = cache.saved_manifest();
    let library = library::load_saved(&root, &saved).unwrap_or_default();
    let names = cache.load_matches();
    let progress = cache.load_progress();
    let art = cache.art_dir();
    let matches = cache.matches_path();
    let app = Rc::new(RefCell::new(App {
        ui: ui.clone_strong(),
        downloaded: store.downloaded(library.paths()),
        nav: Nav::new(library),
        cache,
        names,
        images,
        player: None,
        progress,
        playing: None,
        message_timer: Timer::default(),
        store,
        fetcher,
        downloading: None,
        online: true,
        problem: None,
        deferred: Deferred::default(),
        voice,
        unmute_after_listening: false,
        queued: None,
    }));
    app.borrow_mut().present_home();

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
    let (commands, received_commands) = mpsc::channel();
    let remote_input = Rc::downgrade(&app);
    ui.on_remote_input(move || {
        if let Some(app) = remote_input.upgrade() {
            for command in received_commands.try_iter() {
                App::dispatch(&app, command);
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
    cec::spawn(Remote {
        commands,
        ui: ui.as_weak(),
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
            saved,
            art,
            matches,
            remote,
        },
        notify_watch,
    );
    ui.run()
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
        if let Some(player) = &app.borrow().player {
            player.command(command);
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
                self.online = online;
                self.refresh_offline();
            }
            Change::Downloading(path) => {
                self.downloading = path;
                self.refresh_offline();
            }
            Change::Library(result) if self.player.is_some() => self.deferred.library = Some(result),
            Change::Art if self.player.is_some() => self.deferred.art = true,
            Change::Library(result) => self.reload(result),
            Change::Art => self.refresh_art(),
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
        match self.nav.replace(library) {
            Reload::ShowGone => self.ui.set_show_open(false),
            Reload::ShowChanged => self.present_show(),
            Reload::NoShow | Reload::ShowKept => {}
        }
        self.present_home();
    }

    fn refresh_art(&mut self) {
        self.ui.set_tmdb_notice(self.cache.has_tmdb_art());
        self.names = self.cache.load_matches();
        self.present_home();
        if self.nav.open_show().is_some() {
            self.present_show();
        }
    }

    fn refresh_offline(&mut self) {
        self.downloaded = self.store.downloaded(self.nav.library().paths());
        for (index, row) in self.nav.home_rows().iter().enumerate() {
            let Some(shelf) = self.ui.get_home_shelves().row_data(index) else {
                continue;
            };
            let statuses = self.nav.row_items(*row).into_iter().map(|item| self.item_status(item));
            update_statuses(&shelf, statuses);
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
        let watch_later_dir = self.cache.watch_later_dir(path);
        match Player::start(
            &source,
            &watch_later_dir,
            self.caption(path),
            self.title(path),
            saved,
            finished,
        ) {
            Ok(player) => {
                self.player = Some(player);
                self.record_start(path);
                self.fetch_for(path);
            }
            Err(error) => self.show_message(format!("mpv did not start: {error}")),
        }
    }

    fn fetch_for(&self, path: &Path) {
        if !self.store.is_enabled() || !self.online {
            return;
        }
        let next = self.nav.library().find_episode(path).and_then(|(show, episode)| {
            show.episode_after(episode.season, episode.number)
                .map(|next| next.path.clone())
        });
        let paths = std::iter::once(path.to_path_buf()).chain(next).collect();
        let _ = self.fetcher.send(paths);
    }

    fn caption(&self, path: &Path) -> String {
        if let Some((show, episode)) = self.nav.library().find_episode(path) {
            return format!("{} · {}", show.title, self.episode_label(show, episode));
        }
        let movie = self.nav.library().movies.iter().find(|movie| movie.path == path);
        movie.map_or_else(|| path.display().to_string(), |movie| movie.title.clone())
    }

    fn title(&self, path: &Path) -> Option<Title> {
        let (show, episode) = self.nav.library().find_episode(path)?;
        Some(Title {
            tmdb: show.tmdb?,
            season: episode.season,
            episode: episode.number,
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
            show: show.as_ref().map(|(key, _)| key.clone()),
        });
        if let Some((key, position)) = show {
            self.progress.shows.insert(key, position);
            self.save_progress();
        }
    }

    fn record_place(&mut self) {
        if let (Some(player), Some(playing)) = (&self.player, &self.playing) {
            self.progress.record(&playing.file, player.place());
            self.save_progress();
        }
    }

    fn finish_playback(&mut self, success: bool, ended: bool) {
        self.record_place();
        self.player = None;
        if let Some(playing) = self.playing.take().filter(|_| ended) {
            self.progress.finish(&playing.file);
            if let Some(position) = playing.show.and_then(|key| self.progress.shows.get_mut(&key)) {
                position.finished = true;
            }
            self.save_progress();
        }
        if !success {
            self.show_message("This video could not be played".to_string());
        }
        let deferred = std::mem::take(&mut self.deferred);
        if let Some(result) = deferred.library {
            self.reload(result);
        }
        if deferred.art {
            self.refresh_art();
        }
        self.refresh_offline();
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
            if let Some(player) = self.player.as_ref().filter(|player| !player.muted()) {
                player.set_muted(true);
                self.unmute_after_listening = true;
            }
            self.tell("Listening…");
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
            (_, Some(action)) => self.act(action),
            (Heard::NoMicrophone, _) => self.tell("No microphone is connected"),
            (Heard::Failed(error), _) => {
                eprintln!("voice: {error}");
                self.tell("Voice control failed");
            }
            _ => self.tell("I didn't catch that"),
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
            Action::Download { title, .. } => self.tell(&format!("Downloading {title} by voice is not set up yet")),
            Action::Refuse(reason) => self.tell(&reason),
            Action::Control(control) => self.control(control),
        }
    }

    fn control(&self, control: Control) {
        let player = self.player.as_ref();
        match (control, player) {
            (Control::TvOn | Control::TvOff, _) => {
                if let Err(error) = cec::switch_tv(control == Control::TvOn) {
                    eprintln!("HDMI-CEC: {error}");
                    self.tell("The TV did not answer");
                }
            }
            (_, None) => self.tell(NOTHING_PLAYING),
            (Control::Pause, Some(player)) => player.set_paused(true),
            (Control::Resume, Some(player)) => player.set_paused(false),
            (Control::Stop, Some(player)) => player.command(Command::Back),
            (Control::Mute, Some(player)) => player.set_muted(true),
            (Control::Unmute, Some(player)) => player.set_muted(false),
        }
    }

    fn tell(&self, text: &str) {
        match &self.player {
            Some(player) => player.say(text),
            None => self.show_message(text.to_string()),
        }
    }

    fn save_progress(&self) {
        if let Err(error) = self.cache.save_progress(&self.progress) {
            eprintln!("progress: {error}");
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
        if let Some((_, cursor)) = self.nav.open_show() {
            self.ui.set_show_row(cursor.row as i32);
            self.ui.set_show_cols(cols_model(cursor));
        }
        self.present_details();
    }

    fn present_details(&self) {
        let line = match self.nav.open_show() {
            Some((show, cursor)) => show
                .seasons
                .get(cursor.row)
                .and_then(|season| season.episodes.get(cursor.col()))
                .map(|episode| {
                    let status = self.file_status(&episode.path).1;
                    self.file_details(None, &episode.tags, &show.episode_key(episode), &episode.path, &status)
                }),
            None => self.nav.focused_item().map(|item| self.item_details(item)),
        };
        self.ui.set_details(line.unwrap_or_default().into());
    }

    fn item_details(&self, item: Item) -> String {
        let status = self.item_status(item).1;
        match item {
            Item::Movie(index) => {
                let movie = self.nav.movie(index);
                self.file_details(None, &movie.tags, &movie.key, &movie.path, &status)
            }
            Item::Show(index) => {
                let show = self.nav.show(index);
                let next = self.progress.resume_episode(show).or_else(|| show.first_episode());
                next.map_or(status.clone(), |episode| {
                    let label = show.only_episode().is_none().then(|| episode.short_label());
                    self.file_details(label, &episode.tags, &show.episode_key(episode), &episode.path, &status)
                })
            }
        }
    }

    fn file_details(&self, episode: Option<String>, tags: &Tags, key: &str, file: &Path, download: &str) -> String {
        let watched = self.progress.files.get(file).copied().unwrap_or_default();
        let runtime = self.names.runtimes.get(key).map(|minutes| f64::from(*minutes) * 60.0);
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
                    self.cache.card_art(show.poster.as_deref(), &show.key),
                )
            }
            Item::Movie(index) => {
                let movie = self.nav.movie(index);
                let year = movie.year.map(|year| year.to_string()).unwrap_or_default();
                (
                    card(&movie.key, movie.title.clone(), year),
                    self.cache.card_art(movie.poster.as_deref(), &movie.key),
                )
            }
        };
        set_status(&mut card, self.item_status(item));
        (card, art)
    }

    fn present_home(&mut self) {
        let carried = carried_art(&self.ui.get_home_shelves());
        let mut shelves = Vec::new();
        for (index, row) in self.nav.home_rows().iter().enumerate() {
            let mut cards = Vec::new();
            for (position, item) in self.nav.row_items(*row).into_iter().enumerate() {
                let (mut card, art) = self.home_card(item);
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
                let mut card = card(&key, self.episode_label(show, episode), String::new());
                set_status(&mut card, self.file_status(&episode.path));
                match carried.get(&card.key) {
                    Some(image) => {
                        card.art = image.clone();
                        card.has_art = true;
                    }
                    None => {
                        let art = self.cache.card_art(None, &key);
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
        self.ui.set_show_subtitle(show.subtitle().into());
        self.ui.set_show_shelves(ModelRc::new(VecModel::from(shelves)));
        self.ui.set_show_row(cursor.row as i32);
        self.ui.set_show_cols(cols_model(cursor));
        self.ui.set_show_open(true);
        self.present_details();
    }

    fn episode_label(&self, show: &Show, episode: &Episode) -> String {
        match (&episode.title, self.names.episode_names.get(&show.episode_key(episode))) {
            (None, Some(name)) => format!("{}. {name}", episode.number),
            _ => episode.label(),
        }
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
