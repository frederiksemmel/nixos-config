use super::models::{Kind, VideoKind};
use crate::library::Library;
use crate::watching;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Video {
    pub kind: VideoKind,
    pub id: i64,
}

#[derive(Clone, Debug, Default)]
pub struct Index {
    videos: HashMap<PathBuf, Video>,
    paths: HashMap<Video, PathBuf>,
    shows: HashMap<String, i64>,
    show_keys: HashMap<i64, String>,
    movies: HashMap<String, i64>,
    movie_keys: HashMap<i64, String>,
}

#[derive(Clone, Debug, Default)]
pub struct Known {
    pub videos: HashSet<Video>,
    pub shows: HashSet<i64>,
    pub titles: HashSet<(Kind, i64)>,
}

pub fn kind(kind: watching::Kind) -> Kind {
    match kind {
        watching::Kind::Series => Kind::Series,
        watching::Kind::Movie => Kind::Movie,
    }
}

pub fn watching_kind(kind: Kind) -> watching::Kind {
    match kind {
        Kind::Series => watching::Kind::Series,
        Kind::Movie => watching::Kind::Movie,
    }
}

impl Index {
    pub fn is_empty(&self) -> bool {
        self.videos.is_empty() && self.shows.is_empty() && self.movies.is_empty()
    }

    pub fn add(&mut self, library: &Library) {
        for show in library.shows.iter().filter(|show| show.id != 0) {
            self.shows.insert(show.key.clone(), show.id);
            self.show_keys.insert(show.id, show.key.clone());
            for episode in show.episodes().filter(|episode| episode.id != 0) {
                self.link(
                    &episode.path,
                    Video {
                        kind: VideoKind::Episode,
                        id: episode.id,
                    },
                );
            }
        }
        for movie in library.movies.iter().filter(|movie| movie.id != 0) {
            self.movies.insert(movie.key.clone(), movie.id);
            self.movie_keys.insert(movie.id, movie.key.clone());
            self.link(
                &movie.path,
                Video {
                    kind: VideoKind::Movie,
                    id: movie.id,
                },
            );
        }
    }

    fn link(&mut self, path: &Path, video: Video) {
        if let Some(old) = self.paths.insert(video, path.to_path_buf()) {
            self.videos.remove(&old);
        }
        self.videos.insert(path.to_path_buf(), video);
    }

    pub fn video(&self, path: &Path) -> Option<Video> {
        self.videos.get(path).copied()
    }

    pub fn path(&self, video: Video) -> Option<&PathBuf> {
        self.paths.get(&video)
    }

    pub fn title(&self, kind: watching::Kind, key: &str) -> Option<i64> {
        match kind {
            watching::Kind::Series => self.shows.get(key).copied(),
            watching::Kind::Movie => self.movies.get(key).copied(),
        }
    }

    pub fn show(&self, key: &str) -> Option<i64> {
        self.shows.get(key).copied()
    }

    pub fn show_key(&self, id: i64) -> Option<&String> {
        self.show_keys.get(&id)
    }

    pub fn title_key(&self, kind: Kind, id: i64) -> Option<&String> {
        match kind {
            Kind::Series => self.show_keys.get(&id),
            Kind::Movie => self.movie_keys.get(&id),
        }
    }

    pub fn known(&self) -> Known {
        Known {
            videos: self.paths.keys().copied().collect(),
            shows: self.show_keys.keys().copied().collect(),
            titles: self
                .show_keys
                .keys()
                .map(|id| (Kind::Series, *id))
                .chain(self.movie_keys.keys().map(|id| (Kind::Movie, *id)))
                .collect(),
        }
    }
}
