use crate::command::Command;
use crate::heatmap;
use crate::library::{Episode, Item, Library, Movie, Show};
use std::path::PathBuf;

pub const NEWEST_COUNT: usize = 12;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    pub row: usize,
    pub cols: Vec<usize>,
}

impl Cursor {
    fn new(rows: usize) -> Self {
        Self {
            row: 0,
            cols: vec![0; rows],
        }
    }

    pub fn col(&self) -> usize {
        self.cols.get(self.row).copied().unwrap_or(0)
    }

    fn step(&mut self, command: Command, lengths: &[usize]) -> bool {
        let Some(&length) = lengths.get(self.row) else {
            return false;
        };
        let col = self.col();
        match command {
            Command::Up if self.row > 0 => self.row -= 1,
            Command::Down if self.row + 1 < lengths.len() => self.row += 1,
            Command::Left if col > 0 => self.cols[self.row] -= 1,
            Command::Right if col + 1 < length => self.cols[self.row] += 1,
            _ => return false,
        }
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HomeRow {
    Watching,
    Newest,
    Shows,
    Movies,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Focus {
    #[default]
    Home,
    Profiles(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    Moved,
    Profile(usize),
    OpenShow,
    CloseShow,
    Play(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reload {
    NoShow,
    ShowKept,
    ShowChanged,
    ShowGone,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Watch {
    pub item: Item,
    pub file: PathBuf,
}

pub struct Nav {
    library: Library,
    rows: Vec<HomeRow>,
    newest: Vec<Item>,
    watching: Vec<Watch>,
    home: Cursor,
    show: Option<(usize, Cursor)>,
    heat: Option<usize>,
    focus: Focus,
    profiles: usize,
    current: usize,
}

impl Nav {
    pub fn new(library: Library) -> Self {
        let newest = library.newest(NEWEST_COUNT);
        let rows = home_rows(&library, &newest, &[]);
        let home = Cursor::new(rows.len());
        Self {
            library,
            rows,
            newest,
            watching: Vec::new(),
            home,
            show: None,
            heat: None,
            focus: Focus::Home,
            profiles: 0,
            current: 0,
        }
    }

    pub fn set_profiles(&mut self, count: usize, current: usize) {
        self.profiles = count;
        self.current = current.min(count.saturating_sub(1));
        if let Focus::Profiles(index) = self.focus
            && index >= count
        {
            self.focus = Focus::Profiles(self.current);
        }
    }

    pub fn profile_focus(&self) -> Option<usize> {
        match self.focus {
            Focus::Profiles(index) => Some(index),
            Focus::Home => None,
        }
    }

    pub fn set_watching(&mut self, watching: Vec<Watch>) {
        if watching == self.watching {
            return;
        }
        let old_rows: Vec<_> = self.rows.iter().map(|row| (*row, self.row_keys(*row))).collect();
        let old_home = self.home.clone();
        let appears = self.watching.is_empty() && !watching.is_empty();
        self.watching = watching;
        self.rows = home_rows(&self.library, &self.newest, &self.watching);
        self.home = if appears && old_home.row == 0 && old_home.col() == 0 {
            Cursor::new(self.rows.len())
        } else {
            self.follow_home(&old_rows, &old_home)
        };
    }

    pub fn home_rows(&self) -> &[HomeRow] {
        &self.rows
    }

    pub fn home_cursor(&self) -> &Cursor {
        &self.home
    }

    pub fn library(&self) -> &Library {
        &self.library
    }

    pub fn row_items(&self, row: HomeRow) -> Vec<Item> {
        match row {
            HomeRow::Watching => self.watching.iter().map(|watch| watch.item).collect(),
            HomeRow::Newest => self.newest.clone(),
            HomeRow::Shows => (0..self.library.shows.len()).map(Item::Show).collect(),
            HomeRow::Movies => (0..self.library.movies.len()).map(Item::Movie).collect(),
        }
    }

    fn row_keys(&self, row: HomeRow) -> Vec<String> {
        let items = self.row_items(row);
        items
            .iter()
            .map(|item| self.library.item_key(*item).to_string())
            .collect()
    }

    pub fn focused_item(&self) -> Option<Item> {
        let row = *self.rows.get(self.home.row)?;
        self.row_items(row).get(self.home.col()).copied()
    }

    pub fn open_show(&self) -> Option<(&Show, &Cursor)> {
        self.show
            .as_ref()
            .map(|(index, cursor)| (&self.library.shows[*index], cursor))
    }

    pub fn heat(&self) -> Option<usize> {
        self.heat
    }

    pub fn focused_episode(&self) -> Option<&Episode> {
        let (show, cursor) = self.open_show()?;
        match self.heat {
            Some(position) => {
                let (season, number) = *heatmap::order(show).get(position)?;
                show.episode(season, number)
            }
            None => show.seasons.get(cursor.row)?.episodes.get(cursor.col()),
        }
    }

    pub fn replace(&mut self, library: Library) -> Reload {
        let old_rows: Vec<_> = self.rows.iter().map(|row| (*row, self.row_keys(*row))).collect();
        let old_home = self.home.clone();
        let old_heat = self
            .focused_episode()
            .filter(|_| self.heat.is_some())
            .map(|e| (e.season, e.number));
        self.heat = None;
        let old_show = self
            .show
            .take()
            .map(|(index, cursor)| (self.library.shows[index].clone(), cursor));

        let fresh = Self::new(library);
        let watching = self
            .watching
            .iter()
            .filter_map(|watch| {
                let key = self.library.item_key(watch.item);
                let item = fresh.library.find_item(watch.item, key)?;
                Some(Watch {
                    item,
                    file: watch.file.clone(),
                })
            })
            .collect();
        self.library = fresh.library;
        self.newest = fresh.newest;
        self.watching = watching;
        self.rows = home_rows(&self.library, &self.newest, &self.watching);
        self.home = self.follow_home(&old_rows, &old_home);

        let Some((old, cursor)) = old_show else {
            return Reload::NoShow;
        };
        let same_show = |show: &Show| show.key == old.key || (show.tmdb.is_some() && show.tmdb == old.tmdb);
        let Some(index) = self.library.shows.iter().position(same_show) else {
            return Reload::ShowGone;
        };
        let show = &self.library.shows[index];
        let changed = *show != old;
        if let Some(place) = old_heat {
            let order = heatmap::order(show);
            self.heat = order
                .iter()
                .position(|p| *p == place)
                .or((!order.is_empty()).then_some(0));
        }
        self.show = Some((index, follow_show(&cursor, &old, show)));
        if changed { Reload::ShowChanged } else { Reload::ShowKept }
    }

    fn follow_home(&self, old_rows: &[(HomeRow, Vec<String>)], old: &Cursor) -> Cursor {
        let mut home = Cursor::new(self.rows.len());
        let rows: Vec<_> = self.rows.iter().map(|row| (*row, self.row_keys(*row))).collect();
        for (index, (row, keys)) in rows.iter().enumerate() {
            let Some(old_index) = old_rows.iter().position(|(old_row, _)| old_row == row) else {
                continue;
            };
            let col = old.cols[old_index];
            let same_title = old_rows[old_index]
                .1
                .get(col)
                .and_then(|key| keys.iter().position(|k| k == key));
            home.cols[index] = same_title.unwrap_or(col.min(keys.len().saturating_sub(1)));
        }
        home.row = old.row.min(rows.len().saturating_sub(1));
        let Some((focused_row, focused_keys)) = old_rows.get(old.row) else {
            return home;
        };
        let focused_key = focused_keys.get(old.cols[old.row]);
        if let Some(index) = rows.iter().position(|(row, _)| row == focused_row) {
            home.row = index;
            if focused_key.is_none_or(|key| rows[index].1.contains(key)) {
                return home;
            }
        }
        let elsewhere = rows.iter().enumerate().find_map(|(index, (_, keys))| {
            let col = keys.iter().position(|key| Some(key) == focused_key)?;
            Some((index, col))
        });
        if let Some((index, col)) = elsewhere {
            home.row = index;
            home.cols[index] = col;
        }
        home
    }

    pub fn apply(&mut self, command: Command) -> Effect {
        if let Some((index, cursor)) = &mut self.show {
            let show = &self.library.shows[*index];
            if let Some(position) = self.heat {
                let order = heatmap::order(show);
                let (next, effect) = match command {
                    Command::Back => {
                        self.show = None;
                        (None, Effect::CloseShow)
                    }
                    Command::Select | Command::PlayPause => {
                        let episode = order
                            .get(position)
                            .and_then(|&(season, number)| show.episode(season, number));
                        (Some(position), episode.map_or(Effect::None, play_episode))
                    }
                    Command::Up => match heatmap::vertical(&order, position, true) {
                        Some(above) => (Some(above), Effect::Moved),
                        None => (None, Effect::Moved),
                    },
                    Command::Down => match heatmap::vertical(&order, position, false) {
                        Some(below) => (Some(below), Effect::Moved),
                        None => (Some(position), Effect::None),
                    },
                    Command::Left if position > 0 => (Some(position - 1), Effect::Moved),
                    Command::Right if position + 1 < order.len() => (Some(position + 1), Effect::Moved),
                    _ => (Some(position), Effect::None),
                };
                self.heat = next;
                return effect;
            }
            let last_season = cursor.row + 1 == show.seasons.len();
            if command == Command::Down && last_season {
                let order = heatmap::order(show);
                if !order.is_empty() {
                    let focused = show
                        .seasons
                        .get(cursor.row)
                        .and_then(|season| season.episodes.get(cursor.col()));
                    self.heat =
                        Some(focused.map_or(0, |episode| heatmap::entry(&order, episode.season, episode.number)));
                    return Effect::Moved;
                }
            }
            return match command {
                Command::Back => {
                    self.show = None;
                    Effect::CloseShow
                }
                Command::Select | Command::PlayPause => show
                    .seasons
                    .get(cursor.row)
                    .and_then(|season| season.episodes.get(cursor.col()))
                    .map_or(Effect::None, play_episode),
                _ => {
                    let lengths: Vec<_> = show.seasons.iter().map(|s| s.episodes.len()).collect();
                    moved(cursor.step(command, &lengths))
                }
            };
        }

        if let Focus::Profiles(index) = self.focus {
            return match command {
                Command::Up if index > 0 => {
                    self.focus = Focus::Profiles(index - 1);
                    Effect::Moved
                }
                Command::Down if index + 1 < self.profiles => {
                    self.focus = Focus::Profiles(index + 1);
                    Effect::Moved
                }
                Command::Select => {
                    self.focus = Focus::Home;
                    Effect::Profile(index)
                }
                Command::Left | Command::Back => {
                    self.focus = Focus::Home;
                    Effect::Moved
                }
                _ => Effect::None,
            };
        }

        match command {
            Command::Back => Effect::None,
            Command::Up if self.home.row == 0 && self.profiles > 1 => {
                self.focus = Focus::Profiles(self.current);
                Effect::Moved
            }
            Command::Select | Command::PlayPause => self.activate_home(command),
            _ => {
                let lengths: Vec<_> = self.rows.iter().map(|row| self.row_items(*row).len()).collect();
                moved(self.home.step(command, &lengths))
            }
        }
    }

    fn activate_home(&mut self, command: Command) -> Effect {
        if self.rows.get(self.home.row) == Some(&HomeRow::Watching) {
            return self
                .watching
                .get(self.home.col())
                .map_or(Effect::None, |watch| Effect::Play(watch.file.clone()));
        }
        match self.focused_item() {
            Some(Item::Show(index)) => {
                let show = &self.library.shows[index];
                if let Some(episode) = show.only_episode() {
                    play_episode(episode)
                } else if command == Command::Select {
                    self.show = Some((index, Cursor::new(show.seasons.len())));
                    self.heat = None;
                    Effect::OpenShow
                } else {
                    Effect::None
                }
            }
            Some(Item::Movie(index)) => Effect::Play(self.movie(index).path.clone()),
            None => Effect::None,
        }
    }

    pub fn movie(&self, index: usize) -> &Movie {
        &self.library.movies[index]
    }

    pub fn show(&self, index: usize) -> &Show {
        &self.library.shows[index]
    }
}

fn home_rows(library: &Library, newest: &[Item], watching: &[Watch]) -> Vec<HomeRow> {
    let rows = [
        (HomeRow::Watching, !watching.is_empty()),
        (HomeRow::Newest, !newest.is_empty()),
        (HomeRow::Shows, !library.shows.is_empty()),
        (HomeRow::Movies, !library.movies.is_empty()),
    ];
    rows.into_iter()
        .filter(|(_, shown)| *shown)
        .map(|(row, _)| row)
        .collect()
}

fn follow_show(cursor: &Cursor, old: &Show, new: &Show) -> Cursor {
    let mut next = Cursor::new(new.seasons.len());
    for (index, season) in new.seasons.iter().enumerate() {
        let Some(old_index) = old.seasons.iter().position(|s| s.number == season.number) else {
            continue;
        };
        let col = cursor.cols[old_index];
        let number = old.seasons[old_index].episodes[col].number;
        let same = season.episodes.iter().position(|episode| episode.number == number);
        next.cols[index] = same.unwrap_or(col.min(season.episodes.len() - 1));
    }
    let focused_number = old.seasons.get(cursor.row).map(|season| season.number);
    next.row = new
        .seasons
        .iter()
        .position(|season| Some(season.number) == focused_number)
        .unwrap_or(cursor.row.min(new.seasons.len().saturating_sub(1)));
    next
}

fn moved(changed: bool) -> Effect {
    if changed { Effect::Moved } else { Effect::None }
}

fn play_episode(episode: &Episode) -> Effect {
    Effect::Play(episode.path.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::library_from_paths;
    use crate::manifest::Manifest;
    use Command::*;

    const PATHS: [&str; 9] = [
        "Series/Alpha/Alpha S01/Alpha.S01E01.mkv",
        "Series/Alpha/Alpha S01/Alpha.S01E02.mkv",
        "Series/Alpha/Alpha S01/Alpha.S01E03.mkv",
        "Series/Alpha/Alpha S02/Alpha.S02E01.mkv",
        "Series/Beta/Beta.S01E01.mkv",
        "Series/Beta/Beta.S01E02.mkv",
        "Series/Gamma Special 2021/Gamma.Special.2021.1080p.mp4",
        "Movies/Delta (2001)/Delta.2001.1080p.mkv",
        "Movies/Epsilon (2002)/Epsilon.2002.1080p.mkv",
    ];

    fn nav() -> Nav {
        Nav::new(library_from_paths(&PATHS))
    }

    fn library(text: &str) -> Library {
        let manifest = Manifest::parse(text).unwrap();
        Library::from_manifest("/lib".as_ref(), &manifest, |_| true)
    }

    fn movie_text(title: &str, added: &str) -> String {
        format!("[[movie]]\ntitle = \"{title}\"\npath = \"Movies/{title}.mkv\"\nadded = {added}\n")
    }

    fn show_text(title: &str, episodes: &[(u32, u32)], added: &str) -> String {
        let lines: Vec<_> = episodes
            .iter()
            .map(|(season, episode)| {
                format!(
                    "{{ season = {season}, episode = {episode}, path = \"Series/{title}/{season}x{episode}.mkv\", added = {added} }},"
                )
            })
            .collect();
        format!("[[show]]\ntitle = \"{title}\"\nepisodes = [\n{}\n]\n", lines.join("\n"))
    }

    fn focused_key(nav: &Nav) -> String {
        nav.library().item_key(nav.focused_item().unwrap()).to_string()
    }

    #[test]
    fn home_has_newly_added_above_shows_and_movies() {
        assert_eq!(nav().home_rows(), [HomeRow::Newest, HomeRow::Shows, HomeRow::Movies]);
    }

    #[test]
    fn moving_stops_at_the_edges() {
        let mut nav = nav();
        assert_eq!(nav.apply(Left), Effect::None);
        assert_eq!(nav.apply(Up), Effect::None);
        for _ in 0..4 {
            assert_eq!(nav.apply(Right), Effect::Moved);
        }
        assert_eq!(nav.apply(Right), Effect::None);
        assert_eq!(nav.apply(Down), Effect::Moved);
        assert_eq!(nav.apply(Down), Effect::Moved);
        assert_eq!(nav.apply(Down), Effect::None);
    }

    #[test]
    fn each_row_remembers_its_column() {
        let mut nav = nav();
        nav.apply(Right);
        nav.apply(Right);
        nav.apply(Down);
        assert_eq!(nav.home_cursor().col(), 0);
        nav.apply(Right);
        nav.apply(Down);
        nav.apply(Right);
        nav.apply(Up);
        assert_eq!(nav.home_cursor().col(), 1);
        nav.apply(Up);
        assert_eq!(nav.home_cursor().col(), 2);
        nav.apply(Down);
        nav.apply(Down);
        assert_eq!(nav.home_cursor().col(), 1);
    }

    #[test]
    fn back_returns_to_the_card_that_opened_the_show() {
        let mut nav = nav();
        nav.apply(Down);
        nav.apply(Right);
        assert_eq!(nav.apply(Select), Effect::OpenShow);
        assert_eq!(nav.open_show().unwrap().0.title, "Beta");
        nav.apply(Right);
        assert_eq!(nav.apply(Back), Effect::CloseShow);
        assert_eq!(nav.open_show(), None);
        assert_eq!((nav.home_cursor().row, nav.home_cursor().col()), (1, 1));
    }

    #[test]
    fn select_plays_movies_and_episodes() {
        let mut nav = nav();
        nav.apply(Down);
        nav.apply(Down);
        nav.apply(Right);
        assert_eq!(
            nav.apply(Select),
            Effect::Play("/lib/Movies/Epsilon (2002)/Epsilon.2002.1080p.mkv".into())
        );
        nav.apply(Up);
        nav.apply(Select);
        nav.apply(Down);
        assert_eq!(
            nav.apply(Select),
            Effect::Play("/lib/Series/Alpha/Alpha S02/Alpha.S02E01.mkv".into())
        );
    }

    #[test]
    fn a_show_with_one_episode_plays_directly() {
        let mut nav = nav();
        nav.apply(Down);
        nav.apply(Right);
        nav.apply(Right);
        assert_eq!(
            nav.apply(Select),
            Effect::Play("/lib/Series/Gamma Special 2021/Gamma.Special.2021.1080p.mp4".into())
        );
        assert_eq!(nav.open_show(), None);
    }

    #[test]
    fn an_empty_library_ignores_input() {
        let mut nav = Nav::new(Library::default());
        assert!(nav.home_rows().is_empty());
        for command in [Up, Down, Left, Right, Select, Back, PlayPause] {
            assert_eq!(nav.apply(command), Effect::None);
        }
    }

    #[test]
    fn a_reload_keeps_home_focus_on_its_title() {
        let apple = movie_text("Apple", "2026-01-01");
        let apple_mango = apple.clone() + &movie_text("Mango", "2026-01-02");
        let mut films = movie_text("Oldest", "2026-01-01");
        for day in 2..=12 {
            films += &movie_text(&format!("Film {day}"), &format!("2026-01-{day:02}"));
        }
        let rows = [
            (
                "a new title",
                apple_mango.clone(),
                vec![Down, Right],
                ("apple", HomeRow::Movies),
                apple_mango.clone() + &movie_text("Zebra", "2026-02-01"),
                Some(("apple", HomeRow::Movies, 2)),
            ),
            (
                "the first show",
                apple.clone(),
                vec![Down],
                ("apple", HomeRow::Movies),
                show_text("Dark", &[(1, 1), (1, 2)], "2026-02-01") + &apple,
                Some(("apple", HomeRow::Movies, 0)),
            ),
            (
                "a title pushed out of Newly Added",
                films.clone(),
                vec![Right; 11],
                ("oldest", HomeRow::Newest),
                films + &movie_text("Newest", "2026-02-01"),
                Some(("oldest", HomeRow::Movies, 12)),
            ),
            (
                "the focused title went away",
                apple_mango.clone(),
                vec![Right],
                ("apple", HomeRow::Newest),
                movie_text("Mango", "2026-01-02"),
                Some(("mango", HomeRow::Newest, 0)),
            ),
            (
                "every title went away",
                apple_mango,
                vec![Right],
                ("apple", HomeRow::Newest),
                String::new(),
                None,
            ),
        ];
        for (name, before, moves, focused, after, kept) in rows {
            let mut nav = Nav::new(library(&before));
            for command in moves {
                nav.apply(command);
            }
            let row = |nav: &Nav| nav.home_rows()[nav.home_cursor().row];
            assert_eq!((focused_key(&nav).as_str(), row(&nav)), focused, "{name}");
            assert_eq!(nav.replace(library(&after)), Reload::NoShow, "{name}");
            match kept {
                Some(kept) => {
                    let now = (focused_key(&nav), row(&nav), nav.home_cursor().col());
                    assert_eq!((now.0.as_str(), now.1, now.2), kept, "{name}");
                }
                None => {
                    assert_eq!(nav.focused_item(), None, "{name}");
                    assert_eq!(nav.apply(Select), Effect::None, "{name}");
                }
            }
        }
    }

    #[test]
    fn an_open_show_stays_open_and_keeps_its_episode() {
        let text = show_text("Dark", &[(1, 1), (1, 2), (2, 1)], "2026-01-01");
        let mut nav = Nav::new(library(&text));
        nav.apply(Select);
        nav.apply(Right);
        assert_eq!(nav.replace(library(&text)), Reload::ShowKept);

        let more = show_text("Dark", &[(1, 0), (1, 1), (1, 2), (2, 1), (3, 1)], "2026-01-01");
        assert_eq!(nav.replace(library(&more)), Reload::ShowChanged);
        let (show, cursor) = nav.open_show().unwrap();
        assert_eq!(show.seasons.len(), 3);
        assert_eq!(cursor.cols, [2, 0, 0]);
        assert_eq!(nav.apply(Select), Effect::Play("/lib/Series/Dark/1x2.mkv".into()));
    }

    #[test]
    fn an_open_show_that_disappears_returns_home() {
        let text = show_text("Dark", &[(1, 1), (1, 2)], "2026-01-01") + &movie_text("Up", "2026-01-01");
        let mut nav = Nav::new(library(&text));
        nav.apply(Select);
        assert_eq!(nav.replace(library(&movie_text("Up", "2026-01-01"))), Reload::ShowGone);
        assert_eq!(nav.open_show(), None);
        assert_eq!(focused_key(&nav), "up");
    }

    fn rated_dark() -> Nav {
        let text = r#"[[show]]
title = "Dark"
episodes = [
  { season = 1, episode = 1, path = "Series/Dark/1x1.mkv", added = 2026-01-01, ratings = { imdb = 8.0 } },
  { season = 1, episode = 2, path = "Series/Dark/1x2.mkv", added = 2026-01-01, ratings = { imdb = 9.1 } },
  { season = 2, episode = 1, path = "Series/Dark/2x1.mkv", added = 2026-01-01, ratings = { imdb = 8.5 } },
  { season = 2, episode = 2, path = "Series/Dark/2x2.mkv", added = 2026-01-01 },
]
"#;
        let mut nav = Nav::new(library(text));
        nav.apply(Select);
        nav
    }

    #[test]
    fn the_heatmap_walks_episodes_with_left_right_and_seasons_with_up_down() {
        let mut nav = rated_dark();
        assert_eq!(nav.apply(Down), Effect::Moved);
        assert_eq!(nav.heat(), None);
        assert_eq!(nav.apply(Down), Effect::Moved);
        assert_eq!(nav.focused_episode().map(|e| (e.season, e.number)), Some((2, 1)));
        assert_eq!(nav.apply(Down), Effect::None);
        assert_eq!(nav.apply(Right), Effect::None);
        assert_eq!(nav.apply(Up), Effect::Moved);
        assert_eq!(nav.focused_episode().map(|e| (e.season, e.number)), Some((1, 1)));
        assert_eq!(nav.apply(Left), Effect::None);
        assert_eq!(nav.apply(Right), Effect::Moved);
        assert_eq!(nav.focused_episode().map(|e| (e.season, e.number)), Some((1, 2)));
        assert_eq!(nav.apply(Select), Effect::Play("/lib/Series/Dark/1x2.mkv".into()));
        assert_eq!(nav.apply(Right), Effect::Moved);
        assert_eq!(nav.focused_episode().map(|e| (e.season, e.number)), Some((2, 1)));
        nav.apply(Up);
        assert_eq!(nav.apply(Up), Effect::Moved);
        assert_eq!(nav.heat(), None);
        assert_eq!(nav.open_show().unwrap().1.row, 1);
    }

    #[test]
    fn back_from_the_heatmap_closes_the_show_and_reopening_starts_on_the_seasons() {
        let mut nav = rated_dark();
        nav.apply(Down);
        nav.apply(Down);
        assert_eq!(nav.apply(Back), Effect::CloseShow);
        assert_eq!(nav.heat(), None);
        nav.apply(Select);
        assert_eq!(nav.heat(), None);
        assert_eq!(nav.focused_episode().map(|e| (e.season, e.number)), Some((1, 1)));
    }

    #[test]
    fn a_show_without_ratings_has_no_heatmap_to_move_into() {
        let text = show_text("Dark", &[(1, 1), (1, 2)], "2026-01-01");
        let mut nav = Nav::new(library(&text));
        nav.apply(Select);
        assert_eq!(nav.apply(Down), Effect::None);
        assert_eq!(nav.heat(), None);
    }

    #[test]
    fn a_reload_keeps_the_heatmap_on_the_same_episode() {
        let mut nav = rated_dark();
        nav.apply(Down);
        nav.apply(Down);
        nav.apply(Left);
        let text = r#"[[show]]
title = "Dark"
episodes = [
  { season = 1, episode = 1, path = "Series/Dark/1x1.mkv", added = 2026-01-01, ratings = { imdb = 8.0 } },
  { season = 1, episode = 2, path = "Series/Dark/1x2.mkv", added = 2026-01-01, ratings = { imdb = 9.1 } },
  { season = 2, episode = 1, path = "Series/Dark/2x1.mkv", added = 2026-01-01, ratings = { imdb = 8.5 } },
  { season = 2, episode = 2, path = "Series/Dark/2x2.mkv", added = 2026-01-01, ratings = { imdb = 9.5 } },
]
"#;
        assert_eq!(nav.replace(library(text)), Reload::ShowChanged);
        assert_eq!(nav.heat(), Some(1));
        assert_eq!(nav.focused_episode().map(|e| (e.season, e.number)), Some((1, 2)));
    }

    fn watching_dark() -> (Nav, PathBuf) {
        let text = show_text("Dark", &[(1, 1), (1, 2)], "2026-01-01") + &movie_text("Up", "2026-01-01");
        let mut nav = Nav::new(library(&text));
        let file = nav.show(0).episode(1, 2).unwrap().path.clone();
        nav.set_watching(vec![Watch {
            item: Item::Show(0),
            file: file.clone(),
        }]);
        (nav, file)
    }

    #[test]
    fn currently_watching_comes_first_and_resumes_on_select() {
        let (mut nav, file) = watching_dark();
        assert_eq!(nav.home_rows()[0], HomeRow::Watching);
        assert_eq!(focused_key(&nav), "dark");
        assert_eq!(nav.apply(Select), Effect::Play(file));
    }

    #[test]
    fn focus_stays_on_its_title_when_watching_appears() {
        let text = show_text("Dark", &[(1, 1), (1, 2)], "2026-01-01") + &movie_text("Up", "2026-01-01");
        let mut nav = Nav::new(library(&text));
        nav.apply(Down);
        let row = nav.home_rows()[nav.home_cursor().row];
        nav.set_watching(vec![Watch {
            item: Item::Movie(0),
            file: PathBuf::from("/lib/Movies/Up.mkv"),
        }]);
        assert_eq!(nav.home_rows()[nav.home_cursor().row], row);
    }

    #[test]
    fn a_reload_keeps_the_watched_titles_by_key() {
        let (mut nav, _) = watching_dark();
        let text = movie_text("Alien", "2026-01-01")
            + &show_text("Dark", &[(1, 1), (1, 2)], "2026-01-01")
            + &show_text("Andor", &[(1, 1), (1, 2)], "2026-01-01");
        nav.replace(library(&text));
        let items = nav.row_items(HomeRow::Watching);
        assert_eq!(items.len(), 1);
        assert_eq!(nav.library().item_key(items[0]), "dark");
        assert_eq!(focused_key(&nav), "dark");
    }

    #[test]
    fn up_from_the_top_row_picks_a_profile_and_ok_switches() {
        let mut nav = nav();
        nav.set_profiles(4, 1);
        assert_eq!(nav.apply(Up), Effect::Moved);
        assert_eq!(nav.profile_focus(), Some(1), "the picker starts on the TV's profile");
        assert_eq!(nav.apply(Up), Effect::Moved);
        assert_eq!(nav.apply(Up), Effect::None, "the list stops at the first profile");
        assert_eq!(nav.apply(Down), Effect::Moved);
        assert_eq!(nav.apply(Down), Effect::Moved);
        assert_eq!(nav.apply(Down), Effect::Moved);
        assert_eq!(nav.apply(Down), Effect::None, "and at the last");
        assert_eq!(nav.apply(Right), Effect::None);
        assert_eq!(nav.apply(Select), Effect::Profile(3));
        assert_eq!(nav.profile_focus(), None, "choosing goes back to the rows");
    }

    #[test]
    fn left_or_back_leave_the_picker_without_switching() {
        for leave in [Left, Back] {
            let mut nav = nav();
            nav.set_profiles(3, 0);
            nav.apply(Up);
            nav.apply(Down);
            assert_eq!(nav.apply(leave), Effect::Moved);
            assert_eq!(nav.profile_focus(), None);
            assert_eq!(nav.home_cursor().row, 0);
        }
    }

    #[test]
    fn with_one_profile_there_is_nothing_to_pick() {
        let mut nav = nav();
        nav.set_profiles(1, 0);
        assert_eq!(nav.apply(Up), Effect::None);
        assert_eq!(nav.profile_focus(), None);
    }
}
