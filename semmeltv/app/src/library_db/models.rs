use crate::manifest::About;
use toasty::{Deferred, Json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, toasty::Embed)]
pub struct Scores {
    pub imdb: Option<i64>,
    pub rt: Option<i64>,
    pub popcorn: Option<i64>,
    pub metacritic: Option<i64>,
    pub letterboxd: Option<i64>,
    pub ebert: Option<i64>,
}

#[derive(Debug, toasty::Model)]
pub struct Show {
    #[key]
    #[auto]
    pub id: i64,
    pub title: String,
    pub year: Option<i64>,
    pub last_aired: Option<i64>,
    pub running: bool,
    pub tmdb: Option<i64>,
    pub imdb: Option<String>,
    pub ratings: Scores,
    pub rated: Option<String>,
    pub review: bool,
    #[column(type = text)]
    pub about: Json<About>,
    #[has_many]
    pub episodes: Deferred<Vec<Episode>>,
}

#[derive(Debug, toasty::Model)]
pub struct Episode {
    #[key]
    #[auto]
    pub id: i64,
    #[index]
    pub show_id: i64,
    #[belongs_to(key = show_id, references = id)]
    pub show: Deferred<Show>,
    pub season: i64,
    pub number: i64,
    pub title: Option<String>,
    #[unique]
    pub path: String,
    pub added: String,
    pub runtime: Option<i64>,
    pub ratings: Scores,
    pub video: Option<String>,
    pub resolution: Option<String>,
    pub audio: Option<String>,
    pub review: bool,
}

#[derive(Debug, toasty::Model)]
pub struct Movie {
    #[key]
    #[auto]
    pub id: i64,
    pub title: String,
    pub year: Option<i64>,
    pub tmdb: Option<i64>,
    pub imdb: Option<String>,
    pub ratings: Scores,
    pub rated: Option<String>,
    #[unique]
    pub path: String,
    pub added: String,
    pub runtime: Option<i64>,
    pub video: Option<String>,
    pub resolution: Option<String>,
    pub audio: Option<String>,
    pub review: bool,
    #[column(type = text)]
    pub about: Json<About>,
}

pub fn all() -> toasty::ModelSet {
    toasty::models!(Show, Episode, Movie)
}
