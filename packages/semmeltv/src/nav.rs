use crate::command::Command;
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
    Newest,
    Shows,
    Movies,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    Moved,
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

pub struct Nav {
    library: Library,
    rows: Vec<HomeRow>,
    newest: Vec<Item>,
    home: Cursor,
    show: Option<(usize, Cursor)>,
}

impl Nav {
    pub fn new(library: Library) -> Self {
        let newest = library.newest(NEWEST_COUNT);
        let mut rows = Vec::new();
        if !newest.is_empty() {
            rows.push(HomeRow::Newest);
        }
        if !library.shows.is_empty() {
            rows.push(HomeRow::Shows);
        }
        if !library.movies.is_empty() {
            rows.push(HomeRow::Movies);
        }
        let home = Cursor::new(rows.len());
        Self {
            library,
            rows,
            newest,
            home,
            show: None,
        }
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

    pub fn replace(&mut self, library: Library) -> Reload {
        let old_rows: Vec<_> = self.rows.iter().map(|row| (*row, self.row_keys(*row))).collect();
        let old_home = self.home.clone();
        let old_show = self
            .show
            .take()
            .map(|(index, cursor)| (self.library.shows[index].clone(), cursor));

        let fresh = Self::new(library);
        self.library = fresh.library;
        self.rows = fresh.rows;
        self.newest = fresh.newest;
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

        match command {
            Command::Back => Effect::None,
            Command::Select | Command::PlayPause => self.activate_home(command),
            _ => {
                let lengths: Vec<_> = self.rows.iter().map(|row| self.row_items(*row).len()).collect();
                moved(self.home.step(command, &lengths))
            }
        }
    }

    fn activate_home(&mut self, command: Command) -> Effect {
        match self.focused_item() {
            Some(Item::Show(index)) => {
                let show = &self.library.shows[index];
                if let Some(episode) = show.only_episode() {
                    play_episode(episode)
                } else if command == Command::Select {
                    self.show = Some((index, Cursor::new(show.seasons.len())));
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
        Library::from_manifest("/lib".as_ref(), &manifest, |_| true, |_| false)
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
    fn newly_added_shows_the_twelve_newest_titles_once_each() {
        let mut text = show_text("Alpha", &[(1, 1), (1, 2)], "2026-01-20");
        for day in 1..=15 {
            text += &movie_text(&format!("Film {day}"), &format!("2026-01-{day:02}"));
        }
        let nav = Nav::new(library(&text));
        let items = nav.row_items(HomeRow::Newest);
        assert_eq!(items.len(), NEWEST_COUNT);
        assert_eq!(nav.library().item_key(items[0]), "alpha");
        assert_eq!(nav.library().item_key(items[1]), "film-15");
        assert_eq!(nav.library().item_key(items[11]), "film-5");
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
    fn a_newly_added_show_opens_and_back_returns_to_that_row() {
        let mut nav = nav();
        nav.apply(Right);
        assert_eq!(focused_key(&nav), "beta");
        assert_eq!(nav.apply(Select), Effect::OpenShow);
        assert_eq!(nav.open_show().unwrap().0.title, "Beta");
        assert_eq!(nav.apply(Back), Effect::CloseShow);
        assert_eq!((nav.home_cursor().row, nav.home_cursor().col()), (0, 1));
    }

    #[test]
    fn a_newly_added_film_plays() {
        let mut nav = nav();
        nav.apply(Right);
        nav.apply(Right);
        assert_eq!(
            nav.apply(Select),
            Effect::Play("/lib/Movies/Delta (2001)/Delta.2001.1080p.mkv".into())
        );
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
    fn back_on_home_does_nothing() {
        assert_eq!(nav().apply(Back), Effect::None);
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
    fn a_new_title_does_not_move_focus_off_the_current_one() {
        let older = movie_text("Apple", "2026-01-01") + &movie_text("Mango", "2026-01-02");
        let mut nav = Nav::new(library(&older));
        nav.apply(Down);
        nav.apply(Right);
        assert_eq!(focused_key(&nav), "apple");

        let newer = older + &movie_text("Zebra", "2026-02-01");
        assert_eq!(nav.replace(library(&newer)), Reload::NoShow);
        assert_eq!(focused_key(&nav), "apple");
        assert_eq!(nav.home_cursor().col(), 2);
        assert_eq!(nav.home_cursor().row, 1);
    }

    #[test]
    fn the_first_title_grows_the_home_rows_around_the_focus() {
        let movies = movie_text("Apple", "2026-01-01");
        let mut nav = Nav::new(library(&movies));
        nav.apply(Down);
        assert_eq!(nav.home_rows(), [HomeRow::Newest, HomeRow::Movies]);

        let all = show_text("Dark", &[(1, 1), (1, 2)], "2026-02-01") + &movies;
        nav.replace(library(&all));
        assert_eq!(nav.home_rows()[nav.home_cursor().row], HomeRow::Movies);
        assert_eq!(focused_key(&nav), "apple");
    }

    #[test]
    fn a_title_pushed_out_of_newly_added_keeps_focus_in_its_own_row() {
        let mut text = movie_text("Oldest", "2026-01-01");
        for day in 2..=12 {
            text += &movie_text(&format!("Film {day}"), &format!("2026-01-{day:02}"));
        }
        let mut nav = Nav::new(library(&text));
        for _ in 0..11 {
            nav.apply(Right);
        }
        assert_eq!(focused_key(&nav), "oldest");

        text += &movie_text("Newest", "2026-02-01");
        nav.replace(library(&text));
        assert_eq!(focused_key(&nav), "oldest");
        assert_eq!(nav.home_rows()[nav.home_cursor().row], HomeRow::Movies);
    }

    #[test]
    fn a_title_that_went_away_leaves_focus_nearby() {
        let text = movie_text("Apple", "2026-01-01") + &movie_text("Mango", "2026-01-02");
        let mut nav = Nav::new(library(&text));
        nav.apply(Right);
        assert_eq!(focused_key(&nav), "apple");
        nav.replace(library(&movie_text("Mango", "2026-01-02")));
        assert_eq!(focused_key(&nav), "mango");
        nav.replace(Library::default());
        assert_eq!(nav.focused_item(), None);
        assert_eq!(nav.apply(Select), Effect::None);
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
}
