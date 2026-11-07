use crate::artwork::{Cache, Matches};
use crate::cec;
use crate::command::Command;
use crate::library::{Library, Show};
use crate::loader::{self, Job, Pixels};
use crate::nav::{Cursor, Effect, HomeRow, Nav};
use crate::player::Player;
use crate::{AppWindow, CardData, ShelfData};
use slint::{Color, ComponentHandle, Image, Model, ModelRc, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

const MESSAGE_TIME: Duration = Duration::from_secs(4);

static SHOW_GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
enum Screen {
    Home,
    Show(u64),
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
}

struct App {
    ui: AppWindow,
    nav: Nav,
    cache: Cache,
    names: Matches,
    images: Sender<Job<Target>>,
    player: Option<Player>,
    message_timer: Timer,
}

pub fn run(library: Library, cache: Cache) -> Result<(), slint::PlatformError> {
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
    let names = cache.load_matches();
    let app = Rc::new(RefCell::new(App {
        ui: ui.clone_strong(),
        nav: Nav::new(library),
        cache,
        names,
        images,
        player: None,
        message_timer: Timer::default(),
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
    let (commands, received) = mpsc::channel();
    let remote_input = Rc::downgrade(&app);
    ui.on_remote_input(move || {
        if let Some(app) = remote_input.upgrade() {
            for command in received.try_iter() {
                App::dispatch(&app, command);
            }
        }
    });
    cec::spawn(Remote {
        commands,
        ui: ui.as_weak(),
    });

    let finished = Rc::downgrade(&app);
    ui.on_playback_finished(move |success| {
        if let Some(app) = finished.upgrade() {
            app.borrow_mut().finish_playback(success);
        }
    });
    ui.run()
}

fn show_image(ui: &AppWindow, target: Target, pixels: Pixels) {
    let shelves = match target.screen {
        Screen::Home => ui.get_home_shelves(),
        Screen::Show(generation) if generation == SHOW_GENERATION.load(Ordering::Relaxed) => ui.get_show_shelves(),
        Screen::Show(_) => return,
    };
    let Some(shelf) = shelves.row_data(target.shelf) else {
        return;
    };
    if let Some(mut card) = shelf.cards.row_data(target.card) {
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

fn card(title: String, detail: String) -> CardData {
    let (tint_a, tint_b) = tints(&title);
    CardData {
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

impl App {
    fn dispatch(app: &Rc<RefCell<Self>>, command: Command) {
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

    fn play(&mut self, path: &Path) {
        let handle = self.ui.as_weak();
        let finished = move |success| {
            let _ = handle.upgrade_in_event_loop(move |ui| ui.invoke_playback_finished(success));
        };
        match Player::start(path, finished) {
            Ok(player) => self.player = Some(player),
            Err(error) => self.show_message(format!("mpv did not start: {error}")),
        }
    }

    fn finish_playback(&mut self, success: bool) {
        self.player = None;
        if !success {
            self.show_message("This video could not be played".to_string());
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
    }

    fn request(&self, screen: Screen, shelf: usize, card: usize, art: Option<PathBuf>) {
        if let Some(path) = art {
            let _ = self.images.send(Job {
                target: Target { screen, shelf, card },
                path,
            });
        }
    }

    fn present_home(&mut self) {
        let mut shelves = Vec::new();
        for (index, row) in self.nav.home_rows().iter().enumerate() {
            let (title, cards): (&str, Vec<CardData>) = match row {
                HomeRow::Shows => (
                    "TV Shows",
                    self.nav
                        .shows()
                        .iter()
                        .map(|show| card(show.title.clone(), show.subtitle()))
                        .collect(),
                ),
                HomeRow::Movies => (
                    "Movies",
                    self.nav
                        .movies()
                        .iter()
                        .map(|movie| {
                            card(
                                movie.title.clone(),
                                movie.year.map(|y| y.to_string()).unwrap_or_default(),
                            )
                        })
                        .collect(),
                ),
            };
            shelves.push(ShelfData {
                title: title.into(),
                cards: ModelRc::new(VecModel::from(cards)),
            });
            let arts: Vec<_> = match row {
                HomeRow::Shows => self
                    .nav
                    .shows()
                    .iter()
                    .map(|s| self.cache.card_art(s.poster.as_deref(), &s.key))
                    .collect(),
                HomeRow::Movies => self
                    .nav
                    .movies()
                    .iter()
                    .map(|m| self.cache.card_art(m.poster.as_deref(), &m.key))
                    .collect(),
            };
            for (position, art) in arts.into_iter().enumerate() {
                self.request(Screen::Home, index, position, art);
            }
        }
        if shelves.is_empty() {
            self.ui.set_message("No videos found in the library".into());
        }
        self.ui.set_home_shelves(ModelRc::new(VecModel::from(shelves)));
        self.present_focus();
    }

    fn present_show(&mut self) {
        let Some((show, cursor)) = self.nav.open_show() else {
            return;
        };
        let generation = SHOW_GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
        let shelves: Vec<_> = show
            .seasons
            .iter()
            .map(|season| {
                let cards: Vec<_> = season
                    .episodes
                    .iter()
                    .map(|episode| card(self.episode_label(show, episode), String::new()))
                    .collect();
                ShelfData {
                    title: season.label().into(),
                    cards: ModelRc::new(VecModel::from(cards)),
                }
            })
            .collect();
        self.ui.set_show_title(show.title.as_str().into());
        self.ui.set_show_subtitle(show.subtitle().into());
        self.ui.set_show_shelves(ModelRc::new(VecModel::from(shelves)));
        self.ui.set_show_row(cursor.row as i32);
        self.ui.set_show_cols(cols_model(cursor));
        self.ui.set_show_open(true);
        for (shelf, season) in show.seasons.iter().enumerate() {
            for (position, episode) in season.episodes.iter().enumerate() {
                let art = self.cache.card_art(None, &show.episode_key(episode));
                self.request(Screen::Show(generation), shelf, position, art);
            }
        }
    }

    fn episode_label(&self, show: &Show, episode: &crate::library::Episode) -> String {
        match (&episode.title, self.names.episode_names.get(&show.episode_key(episode))) {
            (None, Some(name)) => format!("{}. {name}", episode.number),
            _ => episode.label(),
        }
    }
}
