use crate::artwork;
use crate::bus::{Mate, Panel, Together};
use crate::command::Command;
use crate::details::{self, Details, Ratings};
use crate::display;
use crate::heatmap;
use crate::library::{Item, Library};
use crate::link::messages::{self, FromTv, ToTv, UiRequest};
use crate::loader::{self, Art, Job, Pixels};
use crate::nav::{Cursor, Effect, HomeRow, Nav, Reload, Watch};
use crate::offline::{self, Presence};
use crate::profile::ProfileId;
use crate::progress::Progress;
use crate::tags::Tags;
use crate::tmdb::Kind;
use crate::up_next;
use crate::watching::{self, Current};
use crate::{AppWindow, CardData, OfflineState, ShelfData, UpNextData};
use slint::{Color, ComponentHandle, Image, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

const MESSAGE_TIME: Duration = Duration::from_secs(4);
const UP_NEXT_TICK: Duration = Duration::from_millis(50);
const BEAT_EVERY: Duration = Duration::from_secs(1);
const EMPTY_LIBRARY: &str = "The library is empty";
const TOGETHER_TITLE: &str = "Watch together";

#[derive(Clone, Copy)]
enum Page {
    Home,
    Show,
    UpNext,
}

struct Target {
    page: Page,
    shelf: usize,
    card: usize,
    key: SharedString,
}

/// What the home screen shows: the library with the TV profile's progress,
/// the people to switch to and how the downloads stand.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Home {
    pub library: Arc<Library>,
    pub profile: ProfileId,
    pub people: Vec<(ProfileId, String)>,
    pub progress: Arc<Progress>,
    pub offline: bool,
    pub downloaded: Arc<HashSet<PathBuf>>,
    pub downloading: Option<PathBuf>,
    pub problem: Option<String>,
}

/// The menu's end of the link: what the tv end passes on for the menu, and
/// the window to wake when something arrives.
pub struct Inbox {
    received: Receiver<ToTv>,
    window: Arc<OnceLock<slint::Weak<AppWindow>>>,
}

/// Where the tv end puts what the menu draws, and the inbox the menu reads
/// it from. What arrives before the window opens waits for it.
pub fn inbox() -> (impl Fn(ToTv) + Send + 'static, Inbox) {
    let (sent, received) = mpsc::channel();
    let window: Arc<OnceLock<slint::Weak<AppWindow>>> = Arc::default();
    let waking = window.clone();
    let deliver = move |order| {
        if sent.send(order).is_ok()
            && let Some(window) = waking.get()
        {
            let _ = window.upgrade_in_event_loop(|ui| ui.invoke_changed());
        }
    };
    (deliver, Inbox { received, window })
}

type Tell = Rc<dyn Fn(FromTv)>;

struct Screen {
    ui: AppWindow,
    tell: Tell,
    nav: Nav,
    home: Arc<Home>,
    library: Arc<Library>,
    together: Option<Together>,
    art: PathBuf,
    images: Sender<Job<Target>>,
    message_timer: Timer,
    panel_timer: Timer,
    up_next_timer: Timer,
}

/// Runs the menu until its window closes.
pub fn run(inbox: Inbox, tell: impl Fn(FromTv) + 'static, meta: &Path) -> Result<(), slint::PlatformError> {
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
    let tell: Tell = Rc::new(tell);
    let home = Arc::new(Home::default());
    let screen = Rc::new(RefCell::new(Screen {
        ui: ui.clone_strong(),
        nav: Nav::new((*home.library).clone()),
        together: None,
        tell: tell.clone(),
        library: home.library.clone(),
        home,
        art: meta.join(artwork::DIR),
        images,
        message_timer: Timer::default(),
        panel_timer: Timer::default(),
        up_next_timer: Timer::default(),
    }));
    screen.borrow_mut().present_profile();
    screen.borrow_mut().present_home();

    ui.on_key(move |text| {
        let Some(command) = Command::from_key(&text) else {
            return false;
        };
        tell(FromTv::Key(command));
        true
    });

    let changed = Rc::downgrade(&screen);
    let Inbox { received, window } = inbox;
    ui.on_changed(move || {
        if let Some(screen) = changed.upgrade() {
            for order in received.try_iter() {
                screen.borrow_mut().follow(order);
            }
        }
    });
    let _ = window.set(ui.as_weak());
    ui.invoke_changed();

    let beating = Timer::default();
    beating.start(TimerMode::Repeated, BEAT_EVERY, || crate::notify::beat("ui"));

    ui.on_resized(display::refresh);
    display::refresh();
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
    let shelves = match target.page {
        Page::Home => ui.get_home_shelves(),
        Page::Show => ui.get_show_shelves(),
        Page::UpNext => {
            let mut card = ui.get_up_next();
            if card.key == target.key {
                card.art = Image::from_rgb8(pixels);
                card.has_art = true;
                ui.set_up_next(card);
            }
            return;
        }
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

impl Screen {
    fn follow(&mut self, order: ToTv) {
        match order {
            ToTv::Library { library, .. } => self.library = Arc::new(library),
            ToTv::Home(home) => self.show_home(Arc::new(self.home_of(home))),
            ToTv::Key(command) => self.navigate(command),
            ToTv::Message(text) => self.show_message(text),
            ToTv::Panel(panel) => self.show_panel(panel),
            ToTv::UpNext(card) => self.show_up_next(card.map(|card| card.card(Instant::now()))),
            ToTv::Together(together) => self.show_together(together),
            ToTv::Update(shown) => self.ui.set_updating(shown),
            _ => {}
        }
    }

    fn home_of(&self, home: messages::Home) -> Home {
        Home {
            library: self.library.clone(),
            profile: home.profile,
            people: home.people,
            progress: Arc::new(home.progress),
            offline: home.offline,
            downloaded: Arc::new(home.downloaded),
            downloading: home.downloading,
            problem: home.problem,
        }
    }

    fn ask(&self, request: UiRequest) {
        (self.tell)(FromTv::Ask(request));
    }

    fn navigate(&mut self, command: Command) {
        match self.nav.apply(command) {
            Effect::None => {}
            Effect::Moved => self.present_focus(),
            Effect::OpenShow => {
                self.together = None;
                self.present_show();
                self.ask_together();
            }
            Effect::CloseShow => {
                self.together = None;
                self.ui.set_show_open(false);
                self.present_focus();
            }
            Effect::Together(index) => {
                if let Some(together) = &self.together
                    && let Some(mate) = together.people.get(index)
                {
                    self.ask(UiRequest::WatchWithOnTv {
                        show: together.show.clone(),
                        with: mate.id,
                    });
                }
            }
            Effect::Play(path) => self.ask(UiRequest::PlayFile(path)),
            Effect::Profile(index) => {
                if let Some((profile, _)) = self.home.people.get(index) {
                    self.ask(UiRequest::TvProfile(*profile));
                }
                self.present_focus();
            }
        }
    }

    fn show_home(&mut self, home: Arc<Home>) {
        let before = std::mem::replace(&mut self.home, home);
        let home = self.home.clone();
        if before.library != home.library {
            match self.nav.replace((*home.library).clone()) {
                Reload::ShowGone => {
                    self.together = None;
                    self.ui.set_show_open(false);
                }
                Reload::ShowChanged => self.present_show(),
                Reload::NoShow | Reload::ShowKept => {}
            }
        }
        self.present_profile();
        if before.profile != home.profile && self.nav.open_show().is_some() {
            self.together = None;
            self.nav.set_together(0);
            self.present_show();
            self.ask_together();
        }
        let only_downloads = before.library == home.library
            && before.profile == home.profile
            && before.progress == home.progress
            && before.problem == home.problem
            && before.offline == home.offline;
        if only_downloads {
            self.refresh_offline();
        } else {
            self.present_home();
        }
    }

    fn refresh_offline(&self) {
        let current = watching::current(&self.home.library, &self.home.progress);
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
                if let Some(shelf) = self.ui.get_show_shelves().row_data(index + self.together_rows()) {
                    update_statuses(&shelf, season.episodes.iter().map(|e| self.file_status(&e.path)));
                }
            }
        }
        self.present_details();
    }

    fn file_status(&self, path: &Path) -> (Presence, String) {
        if !self.home.offline {
            return (Presence::Unknown, String::new());
        }
        let downloading = self.home.downloading.as_deref() == Some(path);
        offline::single(self.home.downloaded.contains(path), downloading)
    }

    fn item_status(&self, item: Item) -> (Presence, String) {
        match item {
            Item::Show(index) if self.home.offline => {
                let show = self.nav.show(index);
                match show.only_episode() {
                    Some(episode) => self.file_status(&episode.path),
                    None => {
                        let done = show
                            .episodes()
                            .filter(|e| self.home.downloaded.contains(&e.path))
                            .count();
                        let busy = show.episodes().any(|e| self.home.downloading.as_ref() == Some(&e.path));
                        offline::group(done, show.episode_count(), busy)
                    }
                }
            }
            Item::Show(_) => (Presence::Unknown, String::new()),
            Item::Movie(index) => self.file_status(&self.nav.movie(index).path),
        }
    }

    fn present_profile(&mut self) {
        let people = &self.home.people;
        let current = people
            .iter()
            .position(|(profile, _)| *profile == self.home.profile)
            .unwrap_or(0);
        self.nav.set_profiles(people.len(), current);
        let names: Vec<SharedString> = people.iter().map(|(_, name)| name.as_str().into()).collect();
        let name = names.get(current).cloned().unwrap_or_default();
        self.ui.set_profile_name(name);
        self.ui.set_profile_names(Rc::new(VecModel::from(names)).into());
        self.ui.set_profile_current(current as i32);
    }

    fn show_panel(&self, panel: Panel) {
        self.ui.set_panel_heading(panel.heading.into());
        self.ui.set_panel_status(panel.status.into());
        let handle = self.ui.as_weak();
        self.panel_timer.start(TimerMode::SingleShot, panel.hold, move || {
            if let Some(ui) = handle.upgrade() {
                ui.set_panel_heading("".into());
                ui.set_panel_status("".into());
            }
        });
    }

    fn show_up_next(&self, card: Option<up_next::Card>) {
        let Some(card) = card else {
            self.up_next_timer.stop();
            self.ui.set_up_next_open(false);
            return;
        };
        let key: SharedString = format!("up-next-{}-{}-{}", card.show, card.season, card.number).into();
        let (tint_a, tint_b) = tints(&card.show);
        let now = Instant::now();
        self.ui.set_up_next(UpNextData {
            key: key.clone(),
            show: card.show.as_str().into(),
            episode: card.episode.as_str().into(),
            seconds: card.seconds_left(now) as i32,
            share: card.share_left(now),
            tint_a,
            tint_b,
            ..UpNextData::default()
        });
        self.ui.set_up_next_open(true);
        let art = card.tmdb.map(|id| Art {
            path: artwork::still(&self.art, id, card.season, card.number),
            fallback: Some(artwork::backdrop(&self.art, Kind::Tv, id)),
        });
        self.request(Page::UpNext, 0, 0, key, art);
        let handle = self.ui.as_weak();
        self.up_next_timer.start(TimerMode::Repeated, UP_NEXT_TICK, move || {
            if let Some(ui) = handle.upgrade() {
                let now = Instant::now();
                let mut shown = ui.get_up_next();
                shown.seconds = card.seconds_left(now) as i32;
                shown.share = card.share_left(now);
                ui.set_up_next(shown);
            }
        });
    }

    fn ask_together(&self) {
        if let Some((show, _)) = self.nav.open_show() {
            self.ask(UiRequest::TogetherOnTv(show.key.clone()));
        }
    }

    fn show_together(&mut self, together: Together) {
        if self.nav.open_show().is_none_or(|(show, _)| show.key != together.show) {
            return;
        }
        self.nav.set_together(together.people.len());
        self.together = Some(together);
        self.present_show();
    }

    fn mates(&self) -> &[Mate] {
        self.together
            .as_ref()
            .map_or(&[], |together| together.people.as_slice())
    }

    fn together_rows(&self) -> usize {
        usize::from(!self.mates().is_empty())
    }

    fn show_focus(&self, cursor: &Cursor) -> (i32, ModelRc<i32>) {
        let rows = self.together_rows();
        let row = match self.nav.together_focus() {
            Some(_) => 0,
            None => cursor.row + rows,
        };
        let together = self.nav.together_focus().unwrap_or(0);
        let cols: Vec<_> = std::iter::repeat_n(together, rows)
            .chain(cursor.cols.iter().copied())
            .map(|col| col as i32)
            .collect();
        (row as i32, ModelRc::new(VecModel::from(cols)))
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
        self.ui
            .set_profile_focus(self.nav.profile_focus().map_or(-1, |index| index as i32));
        if let Some((show, cursor)) = self.nav.open_show() {
            let (row, cols) = self.show_focus(cursor);
            self.ui.set_show_row(row);
            self.ui.set_show_cols(cols);
            self.ui.set_show_heat(heatmap::focus(show, self.nav.heat()));
        }
        self.present_details();
    }

    fn present_details(&self) {
        let mate = self.nav.together_focus().and_then(|index| self.mates().get(index));
        if let (Some((show, _)), Some(mate)) = (self.nav.open_show(), mate) {
            let line = if mate.paired {
                format!("OK takes you to {}'s place in {}", mate.name, show.title)
            } else {
                format!("OK: watch {} together with {}", show.title, mate.name)
            };
            self.ui.set_details(line.into());
            self.ui.set_years("".into());
            self.ui.set_ratings(Ratings::default().into());
            return;
        }
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
                let next = self.home.progress.resume_episode(show).or_else(|| show.first_episode());
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
        let watched = self.home.progress.files.get(file).copied().unwrap_or_default();
        let runtime = runtime.map(|minutes| f64::from(minutes) * 60.0);
        Details {
            episode,
            tags: Some(tags),
            duration: watched.duration.or(runtime),
            stopped_at: watched.stopped_at(),
            download,
        }
        .line()
    }

    fn request(&self, page: Page, shelf: usize, card: usize, key: SharedString, art: Option<Art>) {
        if let Some(art) = art {
            let target = Target { page, shelf, card, key };
            let _ = self.images.send(Job { target, art });
        }
    }

    fn title_art(&self, kind: Kind, tmdb: Option<u64>) -> Option<Art> {
        tmdb.map(|id| Art {
            path: artwork::backdrop(&self.art, kind, id),
            fallback: Some(artwork::poster(&self.art, kind, id)),
        })
    }

    fn home_card(&self, item: Item) -> (CardData, Option<Art>) {
        let (mut card, art) = match item {
            Item::Show(index) => {
                let show = self.nav.show(index);
                (
                    card(&show.key, show.title.clone(), show.subtitle()),
                    self.title_art(Kind::Tv, show.tmdb),
                )
            }
            Item::Movie(index) => {
                let movie = self.nav.movie(index);
                let year = movie.year.map(|year| year.to_string()).unwrap_or_default();
                (
                    card(&movie.key, movie.title.clone(), year),
                    self.title_art(Kind::Movie, movie.tmdb),
                )
            }
        };
        set_status(&mut card, self.item_status(item));
        (card, art)
    }

    fn watching_card(&self, current: &Current) -> (CardData, Option<Art>) {
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
        let current = watching::current(self.nav.library(), &self.home.progress);
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
                    None => self.request(Page::Home, index, position, card.key.clone(), art),
                }
                cards.push(card);
            }
            shelves.push(ShelfData {
                title: row_title(*row).into(),
                cards: ModelRc::new(VecModel::from(cards)),
            });
        }
        let note = match &self.home.problem {
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
        let mates = self.mates();
        if !mates.is_empty() {
            let cards: Vec<_> = mates
                .iter()
                .map(|mate| card(&format!("together-{}", mate.id.0), mate.name.clone(), mate.line.clone()))
                .collect();
            shelves.push(ShelfData {
                title: TOGETHER_TITLE.into(),
                cards: ModelRc::new(VecModel::from(cards)),
            });
        }
        let rows = shelves.len();
        for (season_index, season) in show.seasons.iter().enumerate() {
            let shelf = season_index + rows;
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
                        let art = show.tmdb.map(|id| Art {
                            path: artwork::still(&self.art, id, episode.season, episode.number),
                            fallback: None,
                        });
                        self.request(Page::Show, shelf, position, card.key.clone(), art);
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
        let (row, cols) = self.show_focus(cursor);
        self.ui.set_show_row(row);
        self.ui.set_show_cols(cols);
        self.ui.set_show_open(true);
        self.present_details();
    }
}
