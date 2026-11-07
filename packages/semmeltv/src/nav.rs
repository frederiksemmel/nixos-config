use crate::command::Command;
use crate::library::{Episode, Library, Movie, Show};
use std::path::PathBuf;

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

pub struct Nav {
    library: Library,
    rows: Vec<HomeRow>,
    home: Cursor,
    show: Option<(usize, Cursor)>,
}

impl Nav {
    pub fn new(library: Library) -> Self {
        let mut rows = Vec::new();
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

    pub fn shows(&self) -> &[Show] {
        &self.library.shows
    }

    pub fn movies(&self) -> &[Movie] {
        &self.library.movies
    }

    pub fn open_show(&self) -> Option<(&Show, &Cursor)> {
        self.show
            .as_ref()
            .map(|(index, cursor)| (&self.library.shows[*index], cursor))
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
                let lengths: Vec<_> = self.rows.iter().map(|row| self.row_len(*row)).collect();
                moved(self.home.step(command, &lengths))
            }
        }
    }

    fn row_len(&self, row: HomeRow) -> usize {
        match row {
            HomeRow::Shows => self.library.shows.len(),
            HomeRow::Movies => self.library.movies.len(),
        }
    }

    fn activate_home(&mut self, command: Command) -> Effect {
        let col = self.home.col();
        match self.rows.get(self.home.row) {
            Some(HomeRow::Shows) => {
                let show = &self.library.shows[col];
                if let Some(episode) = show.only_episode() {
                    play_episode(episode)
                } else if command == Command::Select {
                    self.show = Some((col, Cursor::new(show.seasons.len())));
                    Effect::OpenShow
                } else {
                    Effect::None
                }
            }
            Some(HomeRow::Movies) => Effect::Play(self.library.movies[col].path.clone()),
            None => Effect::None,
        }
    }
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
    use crate::library::{self, FileEntry};
    use Command::*;

    fn nav() -> Nav {
        let paths = [
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
        let entries = paths
            .iter()
            .map(|p| FileEntry {
                path: p.into(),
                size: 1,
                modified: 0,
            })
            .collect();
        Nav::new(library::build("/lib".as_ref(), entries))
    }

    #[test]
    fn moving_stops_at_the_edges() {
        let mut nav = nav();
        assert_eq!(nav.apply(Left), Effect::None);
        assert_eq!(nav.apply(Up), Effect::None);
        for _ in 0..2 {
            assert_eq!(nav.apply(Right), Effect::Moved);
        }
        assert_eq!(nav.apply(Right), Effect::None);
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
        nav.apply(Up);
        assert_eq!(nav.home_cursor().col(), 2);
        nav.apply(Down);
        assert_eq!(nav.home_cursor().col(), 1);
    }

    #[test]
    fn back_returns_to_the_card_that_opened_the_show() {
        let mut nav = nav();
        nav.apply(Right);
        assert_eq!(nav.apply(Select), Effect::OpenShow);
        assert_eq!(nav.open_show().unwrap().0.title, "Beta");
        nav.apply(Right);
        assert_eq!(nav.apply(Back), Effect::CloseShow);
        assert_eq!(nav.open_show(), None);
        assert_eq!((nav.home_cursor().row, nav.home_cursor().col()), (0, 1));
    }

    #[test]
    fn select_plays_movies_and_episodes() {
        let mut nav = nav();
        nav.apply(Down);
        nav.apply(Right);
        assert_eq!(
            nav.apply(Select),
            Effect::Play("/lib/Movies/Epsilon (2002)/Epsilon.2002.1080p.mkv".into())
        );
        nav.apply(Up);
        nav.apply(Left);
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
        let mut nav = Nav::new(library::build("/lib".as_ref(), Vec::new()));
        for command in [Up, Down, Left, Right, Select, Back, PlayPause] {
            assert_eq!(nav.apply(command), Effect::None);
        }
    }
}
