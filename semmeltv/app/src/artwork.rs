use crate::manifest::{Date, Manifest, MovieEntry, ReadError, Scores, ShowEntry};
use crate::mdblist;
use crate::tmdb::{Client, Found, Kind, Season};
use image::ImageReader;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, BufWriter, Cursor};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const DIR: &str = "Artwork";
pub const CARD_WIDTH: u32 = 640;
pub const CARD_HEIGHT: u32 = 360;
const REFRESH_AFTER: Duration = Duration::from_secs(150 * 24 * 60 * 60);
const RATINGS_REFRESH_DAYS: i64 = 30;
const REQUESTS_PER_RUN: usize = 900;
const JPEG_QUALITY: u8 = 85;
const BACKDROP_SIZE: &str = "w780";
const STILL_SIZE: &str = "original";

pub fn backdrop(art: &Path, kind: Kind, id: u64) -> PathBuf {
    let kind = match kind {
        Kind::Movie => "movie",
        Kind::Tv => "tv",
    };
    art.join(kind).join(id.to_string()).join("backdrop.jpg")
}

pub fn still(art: &Path, show: u64, season: u32, episode: u32) -> PathBuf {
    art.join("tv")
        .join(show.to_string())
        .join(format!("s{season:02}e{episode:02}.jpg"))
}

pub trait Source {
    fn details(&self, kind: Kind, id: u64) -> Result<Found, String>;
    fn season(&self, id: u64, season: u32) -> Result<Season, String>;
    fn image(&self, size: &str, path: &str) -> Result<Vec<u8>, String>;
}

pub trait Ratings {
    fn ratings(&self, kind: Kind, tmdb: u64) -> Result<Option<mdblist::Found>, mdblist::Error>;
}

impl Ratings for mdblist::Client {
    fn ratings(&self, kind: Kind, tmdb: u64) -> Result<Option<mdblist::Found>, mdblist::Error> {
        mdblist::Client::ratings(self, kind, tmdb)
    }
}

impl Source for Client {
    fn details(&self, kind: Kind, id: u64) -> Result<Found, String> {
        Client::details(self, kind, id).map_err(|error| error.to_string())
    }

    fn season(&self, id: u64, season: u32) -> Result<Season, String> {
        Client::season(self, id, season).map_err(|error| error.to_string())
    }

    fn image(&self, size: &str, path: &str) -> Result<Vec<u8>, String> {
        Client::image(self, size, path).map_err(|error| error.to_string())
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub images: usize,
    pub without_id: Vec<String>,
    pub rated: usize,
    pub unrated: Vec<String>,
    pub ratings_waiting: usize,
    pub requests: usize,
    pub problems: Vec<String>,
}

struct Run<'a, S> {
    art: PathBuf,
    source: &'a S,
    ratings: Option<&'a dyn Ratings>,
    ratings_stopped: bool,
    refresh: bool,
    now: SystemTime,
    today: Date,
    summary: Summary,
}

struct Rating<'a> {
    kind: Kind,
    tmdb: Option<u64>,
    imdb: &'a mut Option<String>,
    scores: &'a mut Scores,
    rated: &'a mut Option<Date>,
}

pub fn run(
    manifest_path: &Path,
    source: &impl Source,
    ratings: Option<&dyn Ratings>,
    refresh: bool,
) -> Result<Summary, String> {
    let shown = manifest_path.display();
    let mut manifest = Manifest::read(manifest_path).map_err(|error| match error {
        ReadError::Unreadable(error) => format!("{shown}: {error}"),
        ReadError::Invalid(error) => format!("{shown}: {error}"),
    })?;
    let now = SystemTime::now();
    let mut run = Run {
        art: manifest_path.parent().unwrap_or(Path::new(".")).join(DIR),
        source,
        ratings,
        ratings_stopped: false,
        refresh,
        now,
        today: Date::from_unix(now.duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs())),
        summary: Summary::default(),
    };
    let save = |manifest: &Manifest| {
        manifest
            .write(manifest_path)
            .map_err(|error| format!("{shown}: {error}"))
    };
    for index in 0..manifest.show.len() {
        let before = manifest.show[index].clone();
        let show = &mut manifest.show[index];
        run.show(show);
        let label = show.title.clone();
        run.rate(
            &label,
            Rating {
                kind: Kind::Tv,
                tmdb: show.tmdb,
                imdb: &mut show.imdb,
                scores: &mut show.ratings,
                rated: &mut show.rated,
            },
        );
        if manifest.show[index] != before {
            save(&manifest)?;
        }
    }
    for index in 0..manifest.movie.len() {
        let before = manifest.movie[index].clone();
        let movie = &mut manifest.movie[index];
        run.movie(movie);
        let label = movie_label(movie);
        run.rate(
            &label,
            Rating {
                kind: Kind::Movie,
                tmdb: movie.tmdb,
                imdb: &mut movie.imdb,
                scores: &mut movie.ratings,
                rated: &mut movie.rated,
            },
        );
        if manifest.movie[index] != before {
            save(&manifest)?;
        }
    }
    Ok(run.summary)
}

impl<S: Source> Run<'_, S> {
    fn wanted(&self, path: &Path) -> bool {
        match fs::metadata(path).and_then(|metadata| metadata.modified()) {
            Ok(modified) => self.refresh && self.now.duration_since(modified).is_ok_and(|age| age >= REFRESH_AFTER),
            Err(_) => true,
        }
    }

    fn fetch(&mut self, target: &Path, url_path: Option<&str>, size: &str, label: &str) {
        let Some(url_path) = url_path else {
            let _ = fs::remove_file(target);
            return;
        };
        let saved = self
            .source
            .image(size, url_path)
            .and_then(|bytes| write_card(&bytes, target).map_err(|error| error.to_string()));
        match saved {
            Ok(()) => self.summary.images += 1,
            Err(error) => {
                let _ = fs::remove_file(target);
                self.summary.problems.push(format!("{label}: image failed: {error}"));
            }
        }
    }

    fn show(&mut self, show: &mut ShowEntry) {
        println!("{}", show.title);
        let Some(id) = show.tmdb else {
            self.summary.without_id.push(show.title.clone());
            return;
        };
        let seasons: BTreeSet<u32> = show
            .episodes
            .iter()
            .filter(|episode| episode.season > 0)
            .filter(|episode| {
                episode.title.is_none()
                    || episode.runtime.is_none()
                    || self.wanted(&still(&self.art, id, episode.season, episode.episode))
            })
            .map(|episode| episode.season)
            .collect();
        let backdrop = backdrop(&self.art, Kind::Tv, id);
        if self.wanted(&backdrop) {
            match self.source.details(Kind::Tv, id) {
                Ok(found) => self.fetch(&backdrop, found.backdrop.as_deref(), BACKDROP_SIZE, &show.title),
                Err(error) => self
                    .summary
                    .problems
                    .push(format!("{}: TMDB {id}: {error}", show.title)),
            }
        }
        for number in seasons {
            let season = match self.source.season(id, number) {
                Ok(season) => season,
                Err(error) => {
                    let problem = format!("{} season {number}: TMDB {id}: {error}", show.title);
                    self.summary.problems.push(problem);
                    continue;
                }
            };
            for episode in show.episodes.iter_mut().filter(|episode| episode.season == number) {
                let Some(found) = season.episode(episode.episode) else {
                    continue;
                };
                if episode.title.is_none() {
                    episode.title = found.name.clone().filter(|name| !name.trim().is_empty());
                }
                if episode.runtime.is_none() {
                    episode.runtime = found.runtime.filter(|minutes| *minutes > 0);
                }
                let target = still(&self.art, id, episode.season, episode.episode);
                if self.wanted(&target) {
                    let label = format!("{} S{}E{}", show.title, episode.season, episode.episode);
                    self.fetch(&target, found.still_path.as_deref(), STILL_SIZE, &label);
                }
            }
        }
    }

    fn rate(&mut self, label: &str, rating: Rating) {
        let (Some(ratings), Some(tmdb)) = (self.ratings, rating.tmdb) else {
            return;
        };
        let due = match *rating.rated {
            None => true,
            Some(rated) => self.refresh && self.today.days() - rated.days() >= RATINGS_REFRESH_DAYS,
        };
        if !due {
            return;
        }
        if self.ratings_stopped || self.summary.requests >= REQUESTS_PER_RUN {
            self.summary.ratings_waiting += 1;
            return;
        }
        self.summary.requests += 1;
        match ratings.ratings(rating.kind, tmdb) {
            Ok(Some(found)) => {
                if found.imdb.is_some() {
                    *rating.imdb = found.imdb;
                }
                *rating.scores = found.scores;
                *rating.rated = Some(self.today);
                if found.scores.is_empty() {
                    self.summary.unrated.push(format!("{label}: MDBList has no ratings"));
                } else {
                    self.summary.rated += 1;
                }
            }
            Ok(None) => {
                *rating.scores = Scores::default();
                *rating.rated = Some(self.today);
                self.summary
                    .unrated
                    .push(format!("{label}: MDBList does not know TMDB {tmdb}"));
            }
            Err(mdblist::Error::Stop(reason)) => {
                self.ratings_stopped = true;
                self.summary.ratings_waiting += 1;
                self.summary.problems.push(format!("MDBList: {reason}"));
            }
            Err(mdblist::Error::Failed(reason)) => {
                self.summary
                    .problems
                    .push(format!("{label}: MDBList TMDB {tmdb}: {reason}"));
            }
        }
    }

    fn movie(&mut self, movie: &mut MovieEntry) {
        let label = movie_label(movie);
        println!("{label}");
        let Some(id) = movie.tmdb else {
            self.summary.without_id.push(label);
            return;
        };
        let target = backdrop(&self.art, Kind::Movie, id);
        let wanted = self.wanted(&target);
        if !wanted && movie.runtime.is_some() {
            return;
        }
        match self.source.details(Kind::Movie, id) {
            Ok(found) => {
                if movie.runtime.is_none() {
                    movie.runtime = found.runtime;
                }
                if wanted {
                    self.fetch(&target, found.backdrop.as_deref(), BACKDROP_SIZE, &label);
                }
            }
            Err(error) => self.summary.problems.push(format!("{label}: TMDB {id}: {error}")),
        }
    }
}

fn movie_label(movie: &MovieEntry) -> String {
    match movie.year {
        Some(year) => format!("{} ({year})", movie.title),
        None => movie.title.clone(),
    }
}

fn write_card(bytes: &[u8], target: &Path) -> io::Result<()> {
    let image = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()?
        .decode()
        .map_err(io::Error::other)?
        .resize_to_fill(CARD_WIDTH, CARD_HEIGHT, FilterType::Triangle)
        .into_rgb8();
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = target.with_extension("tmp");
    let writer = BufWriter::new(fs::File::create(&temporary)?);
    JpegEncoder::new_with_quality(writer, JPEG_QUALITY)
        .encode_image(&image)
        .map_err(io::Error::other)?;
    fs::rename(temporary, target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Tenths;
    use image::{ImageFormat, RgbImage};
    use std::cell::RefCell;
    use std::env;
    use std::fs::File;

    const MANIFEST: &str = r#"[[show]]
title = "Dark"
tmdb = 70523
episodes = [
  { season = 0, episode = 1, path = "Series/Dark/Special.mkv", added = 2024-05-01 },
  { season = 1, episode = 1, path = "Series/Dark/S01E01.mkv", added = 2024-05-01 },
  { season = 1, episode = 2, title = "Lies", path = "Series/Dark/S01E02.mkv", added = 2024-05-01 },
]

[[show]]
title = "Home Videos"
episodes = [
  { season = 1, episode = 1, path = "Series/Home/S01E01.mkv", added = 2024-05-01 },
]

[[movie]]
title = "Up"
year = 2009
tmdb = 14160
path = "Movies/Up.mkv"
added = 2024-05-01

[[movie]]
title = "Wind"
path = "Movies/Wind.avi"
added = 2024-05-01
"#;

    struct Fake {
        calls: RefCell<Vec<String>>,
        png: Vec<u8>,
        images_fail: bool,
    }

    impl Fake {
        fn new() -> Self {
            let mut png = Vec::new();
            RgbImage::new(64, 36)
                .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
                .unwrap();
            Self {
                calls: RefCell::new(Vec::new()),
                png,
                images_fail: false,
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.take()
        }
    }

    impl Source for Fake {
        fn details(&self, kind: Kind, id: u64) -> Result<Found, String> {
            self.calls.borrow_mut().push(format!("details {kind:?} {id}"));
            Ok(Found {
                id,
                backdrop: Some(format!("/{id}.jpg")),
                runtime: (kind == Kind::Movie).then_some(96),
                ..Found::default()
            })
        }

        fn season(&self, id: u64, season: u32) -> Result<Season, String> {
            self.calls.borrow_mut().push(format!("season {id} {season}"));
            let json = r#"{"episodes":[
                {"episode_number":1,"name":"Secrets","still_path":"/s1.jpg","runtime":51},
                {"episode_number":2,"name":"Wrong","still_path":null,"runtime":44}]}"#;
            Ok(serde_json::from_str(json).unwrap())
        }

        fn image(&self, size: &str, path: &str) -> Result<Vec<u8>, String> {
            self.calls.borrow_mut().push(format!("image {size} {path}"));
            if self.images_fail {
                Err("offline".to_string())
            } else {
                Ok(self.png.clone())
            }
        }
    }

    struct FakeMdblist {
        asked: RefCell<Vec<String>>,
        limit: usize,
    }

    impl FakeMdblist {
        fn new(limit: usize) -> Self {
            Self {
                asked: RefCell::new(Vec::new()),
                limit,
            }
        }
    }

    impl Ratings for FakeMdblist {
        fn ratings(&self, kind: Kind, tmdb: u64) -> Result<Option<mdblist::Found>, mdblist::Error> {
            let mut asked = self.asked.borrow_mut();
            if asked.len() >= self.limit {
                return Err(mdblist::Error::Stop("Daily API limit exceeded!".to_string()));
            }
            asked.push(format!("{kind:?} {tmdb}"));
            if kind == Kind::Tv {
                return Ok(None);
            }
            Ok(Some(mdblist::Found {
                imdb: Some(format!("tt{tmdb}")),
                scores: Scores {
                    imdb: Some(Tenths(83)),
                    rt: Some(98),
                    popcorn: Some(90),
                    metacritic: None,
                },
            }))
        }
    }

    const FILMS: &str = r#"[[movie]]
title = "Up"
year = 2009
tmdb = 14160
path = "Movies/Up.mkv"
added = 2024-05-01

[[movie]]
title = "Heat"
year = 1995
tmdb = 949
path = "Movies/Heat.mkv"
added = 2024-05-01
"#;

    fn films(name: &str) -> PathBuf {
        let dir = workdir(name);
        fs::write(dir.join("library.toml"), FILMS).unwrap();
        dir
    }

    #[test]
    fn ratings_come_from_mdblist_by_tmdb_id() {
        let dir = workdir("ratings");
        let manifest = dir.join("library.toml");
        let mdblist = FakeMdblist::new(10);
        let fake = Fake::new();
        let summary = run(&manifest, &fake, Some(&mdblist), false).unwrap();
        assert_eq!(summary.rated, 1);
        assert_eq!(summary.requests, 2);
        assert_eq!(summary.unrated, ["Dark: MDBList does not know TMDB 70523"]);
        assert_eq!(mdblist.asked.take(), ["Tv 70523", "Movie 14160"]);

        let written = Manifest::read(&manifest).unwrap();
        let up = &written.movie[0];
        assert_eq!(up.imdb.as_deref(), Some("tt14160"));
        assert_eq!(
            (
                up.ratings.imdb,
                up.ratings.rt,
                up.ratings.popcorn,
                up.ratings.metacritic
            ),
            (Some(Tenths(83)), Some(98), Some(90), None)
        );
        assert!(up.rated.is_some());
        assert_eq!(written.show[0].ratings, Scores::default());
        assert!(written.show[0].rated.is_some());
        assert_eq!(written.movie[1].rated, None);

        let again = run(&manifest, &fake, Some(&mdblist), false).unwrap();
        assert_eq!(again.requests, 0);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_daily_limit_keeps_what_was_fetched_and_leaves_the_rest() {
        let dir = films("ratings-limit");
        let manifest = dir.join("library.toml");
        let summary = run(&manifest, &Fake::new(), Some(&FakeMdblist::new(1)), false).unwrap();
        assert_eq!((summary.rated, summary.ratings_waiting), (1, 1));
        assert_eq!(summary.problems, ["MDBList: Daily API limit exceeded!"]);
        let written = Manifest::read(&manifest).unwrap();
        assert!(written.movie[0].rated.is_some());
        assert_eq!((written.movie[1].imdb.as_deref(), written.movie[1].rated), (None, None));

        let next = run(&manifest, &Fake::new(), Some(&FakeMdblist::new(5)), false).unwrap();
        assert_eq!((next.rated, next.requests), (1, 1));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn refreshing_asks_again_only_for_ratings_a_month_old() {
        let dir = films("ratings-refresh");
        let manifest = dir.join("library.toml");
        run(&manifest, &Fake::new(), Some(&FakeMdblist::new(5)), false).unwrap();
        let mut written = Manifest::read(&manifest).unwrap();
        written.movie[0].rated = Date::parse("2020-01-01");
        written.write(&manifest).unwrap();

        let mdblist = FakeMdblist::new(5);
        assert_eq!(run(&manifest, &Fake::new(), Some(&mdblist), false).unwrap().requests, 0);
        assert_eq!(run(&manifest, &Fake::new(), Some(&mdblist), true).unwrap().requests, 1);
        assert_eq!(mdblist.asked.take(), ["Movie 14160"]);
        fs::remove_dir_all(dir).unwrap();
    }

    fn workdir(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("semmeltv-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("library.toml"), MANIFEST).unwrap();
        dir
    }

    fn age(path: &Path, days: u64) {
        let then = SystemTime::now() - Duration::from_secs(days * 24 * 60 * 60);
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(then)
            .unwrap();
    }

    #[test]
    fn art_lives_under_the_kind_and_tmdb_id() {
        let art = Path::new("/work/Artwork");
        assert_eq!(
            backdrop(art, Kind::Movie, 14160),
            Path::new("/work/Artwork/movie/14160/backdrop.jpg")
        );
        assert_eq!(
            backdrop(art, Kind::Tv, 70523),
            Path::new("/work/Artwork/tv/70523/backdrop.jpg")
        );
        assert_eq!(still(art, 70523, 1, 2), Path::new("/work/Artwork/tv/70523/s01e02.jpg"));
    }

    #[test]
    fn a_run_saves_art_beside_the_manifest_and_names_and_runtimes_into_it() {
        let dir = workdir("art-run");
        let manifest = dir.join("library.toml");
        let art = dir.join(DIR);
        let fake = Fake::new();

        let summary = run(&manifest, &fake, None, false).unwrap();
        assert_eq!(summary.images, 3);
        assert_eq!(summary.without_id, ["Home Videos", "Wind"]);
        assert!(summary.problems.is_empty());
        for path in [
            backdrop(&art, Kind::Tv, 70523),
            still(&art, 70523, 1, 1),
            backdrop(&art, Kind::Movie, 14160),
        ] {
            let image = image::open(&path).unwrap();
            assert_eq!((image.width(), image.height()), (CARD_WIDTH, CARD_HEIGHT));
        }
        assert!(!still(&art, 70523, 1, 2).exists());

        let written = Manifest::read(&manifest).unwrap();
        let dark = &written.show[0].episodes;
        assert_eq!(dark[0].title, None);
        assert_eq!((dark[1].title.as_deref(), dark[1].runtime), (Some("Secrets"), Some(51)));
        assert_eq!((dark[2].title.as_deref(), dark[2].runtime), (Some("Lies"), Some(44)));
        assert_eq!(written.movie[0].runtime, Some(96));
        assert_eq!(written.movie[1].runtime, None);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_second_run_only_asks_for_what_is_still_missing() {
        let dir = workdir("art-again");
        let manifest = dir.join("library.toml");
        let fake = Fake::new();
        run(&manifest, &fake, None, false).unwrap();
        fake.calls();

        let summary = run(&manifest, &fake, None, false).unwrap();
        assert_eq!(summary.images, 0);
        assert_eq!(fake.calls(), ["season 70523 1"]);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn refreshing_replaces_only_images_older_than_five_months() {
        let dir = workdir("art-refresh");
        let manifest = dir.join("library.toml");
        let art = dir.join(DIR);
        let fake = Fake::new();
        run(&manifest, &fake, None, false).unwrap();
        age(&backdrop(&art, Kind::Movie, 14160), 160);
        age(&backdrop(&art, Kind::Tv, 70523), 100);
        fake.calls();

        assert_eq!(run(&manifest, &fake, None, false).unwrap().images, 0);
        fake.calls();
        assert_eq!(run(&manifest, &fake, None, true).unwrap().images, 1);
        assert!(fake.calls().contains(&"image w780 /14160.jpg".to_string()));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_old_image_that_cannot_be_refreshed_is_deleted() {
        let dir = workdir("art-expire");
        let manifest = dir.join("library.toml");
        let target = backdrop(&dir.join(DIR), Kind::Movie, 14160);
        run(&manifest, &Fake::new(), None, false).unwrap();
        age(&target, 200);

        let failing = Fake {
            images_fail: true,
            ..Fake::new()
        };
        let summary = run(&manifest, &failing, None, true).unwrap();
        assert!(!target.exists());
        assert_eq!(summary.problems, ["Up (2009): image failed: offline"]);
        fs::remove_dir_all(dir).unwrap();
    }
}
