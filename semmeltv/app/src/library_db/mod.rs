mod models;

use crate::db;
use crate::manifest::{self, Date, EpisodeEntry, Manifest, MovieEntry, ReadError, Scores, ShowEntry, Tenths};
use models::{Episode, Movie, Show};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;
use toasty::stmt::{Type, Value};
use toasty_driver_sqlite::Sqlite;

pub static MIGRATIONS: toasty::migration::MigrationSet = toasty::embed_migrations!("toasty/library");

fn driver(path: &Path) -> Sqlite {
    Sqlite::open(path).busy_timeout(Duration::from_secs(5))
}

pub async fn read(path: &Path) -> Result<Manifest, ReadError> {
    if !path.exists() {
        return Err(ReadError::Unreadable(io::Error::from(io::ErrorKind::NotFound)));
    }
    read_stored(path)
        .await
        .map_err(|error| ReadError::Invalid(error.to_string()))
}

async fn read_stored(path: &Path) -> toasty::Result<Manifest> {
    let mut db = toasty::Db::builder().models(models::all()).build(driver(path)).await?;
    if let Some(problem) = unknown_migrations(&mut db).await? {
        return Err(toasty::Error::from_args(format_args!("{problem}")));
    }
    if missing_migrations(&mut db).await? {
        drop(db);
        return read_upgraded(path).await;
    }
    sorted(&mut db).await
}

async fn sorted(db: &mut toasty::Db) -> toasty::Result<Manifest> {
    let mut manifest = load(db).await?;
    manifest.show.sort_by_key(|show| show.id);
    manifest.movie.sort_by_key(|movie| movie.id);
    Ok(manifest)
}

async fn read_upgraded(path: &Path) -> toasty::Result<Manifest> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let copy = std::env::temp_dir().join(format!("semmeltv-library-{}-{nanos}.db", std::process::id()));
    fs::copy(path, &copy).map_err(|error| toasty::Error::from_args(format_args!("{}: {error}", copy.display())))?;
    let read = async {
        let mut db = toasty::Db::builder().models(models::all()).build(driver(&copy)).await?;
        MIGRATIONS.apply(&db).await?;
        sorted(&mut db).await
    }
    .await;
    let _ = fs::remove_file(&copy);
    read
}

pub async fn write(path: &Path, manifest: &Manifest) -> io::Result<()> {
    let temporary = manifest::temporary(path);
    let _ = fs::remove_file(&temporary);
    if path.exists() {
        fs::copy(path, &temporary)?;
    }
    write_stored(&temporary, manifest.clone())
        .await
        .map_err(|error| io::Error::other(error.to_string()))?;
    fs::rename(temporary, path)
}

async fn write_stored(file: &Path, wanted: Manifest) -> toasty::Result<()> {
    let mut db = toasty::Db::builder().models(models::all()).build(driver(file)).await?;
    if let Some(problem) = unknown_migrations(&mut db).await? {
        return Err(toasty::Error::from_args(format_args!("{problem}")));
    }
    MIGRATIONS.apply(&db).await?;
    let mut transaction = db.transaction().await?;
    store(&mut transaction, wanted).await?;
    transaction.commit().await
}

async fn applied_migrations(db: &mut toasty::Db) -> toasty::Result<Option<HashSet<i64>>> {
    let exists = toasty::sql::query("SELECT count(*) FROM sqlite_master WHERE name = '__toasty_migrations'")
        .column_types([Type::I64])
        .exec(db)
        .await?;
    if first_number(&exists) == Some(0) {
        return Ok(None);
    }
    let applied = toasty::sql::query("SELECT id FROM __toasty_migrations")
        .column_types([Type::I64])
        .exec(db)
        .await?;
    Ok(Some(applied.iter().filter_map(number).collect()))
}

fn known_migrations() -> HashSet<i64> {
    MIGRATIONS
        .migrations()
        .iter()
        .map(|migration| migration.id() as i64)
        .collect()
}

async fn unknown_migrations(db: &mut toasty::Db) -> toasty::Result<Option<String>> {
    let Some(applied) = applied_migrations(db).await? else {
        return Ok(None);
    };
    let unknown = applied.difference(&known_migrations()).count();
    Ok((unknown > 0)
        .then(|| format!("the library was written by a newer SemmelTV ({unknown} migrations this one does not know)")))
}

async fn missing_migrations(db: &mut toasty::Db) -> toasty::Result<bool> {
    Ok(applied_migrations(db)
        .await?
        .is_some_and(|applied| !known_migrations().is_subset(&applied)))
}

fn number(row: &Value) -> Option<i64> {
    match row {
        Value::Record(record) => match record.fields.first()? {
            Value::I64(number) => Some(*number),
            _ => None,
        },
        _ => None,
    }
}

fn first_number(rows: &[Value]) -> Option<i64> {
    rows.first().and_then(number)
}

fn scores(stored: models::Scores) -> Scores {
    let tenths = |value: Option<i64>| value.and_then(|value| u8::try_from(value).ok()).map(Tenths);
    let percent = |value: Option<i64>| value.and_then(|value| u8::try_from(value).ok());
    Scores {
        imdb: tenths(stored.imdb),
        rt: percent(stored.rt),
        popcorn: percent(stored.popcorn),
        metacritic: percent(stored.metacritic),
        letterboxd: tenths(stored.letterboxd),
        ebert: tenths(stored.ebert),
    }
}

fn stored_scores(scores: &Scores) -> models::Scores {
    models::Scores {
        imdb: scores.imdb.map(|tenths| tenths.0.into()),
        rt: scores.rt.map(Into::into),
        popcorn: scores.popcorn.map(Into::into),
        metacritic: scores.metacritic.map(Into::into),
        letterboxd: scores.letterboxd.map(|tenths| tenths.0.into()),
        ebert: scores.ebert.map(|tenths| tenths.0.into()),
    }
}

fn date(text: &str) -> Date {
    Date::parse(text).unwrap_or_default()
}

fn small<T: TryFrom<i64>>(value: Option<i64>) -> Option<T> {
    value.and_then(|value| T::try_from(value).ok())
}

fn show_entry(show: Show, episodes: Vec<EpisodeEntry>) -> ShowEntry {
    ShowEntry {
        id: Some(show.id),
        title: show.title,
        year: small(show.year),
        last_aired: small(show.last_aired),
        running: show.running,
        tmdb: small(show.tmdb),
        imdb: show.imdb,
        ratings: scores(show.ratings),
        rated: show.rated.as_deref().and_then(Date::parse),
        review: show.review,
        episodes,
        about: show.about.0,
    }
}

fn episode_entry(episode: Episode) -> EpisodeEntry {
    EpisodeEntry {
        id: Some(episode.id),
        season: u32::try_from(episode.season).unwrap_or_default(),
        episode: u32::try_from(episode.number).unwrap_or_default(),
        title: episode.title,
        path: episode.path,
        added: date(&episode.added),
        runtime: small(episode.runtime),
        ratings: scores(episode.ratings),
        video: episode.video,
        resolution: episode.resolution,
        audio: episode.audio,
        review: episode.review,
        downloaded_by: episode.downloaded_by,
    }
}

fn movie_entry(movie: Movie) -> MovieEntry {
    MovieEntry {
        id: Some(movie.id),
        title: movie.title,
        year: small(movie.year),
        tmdb: small(movie.tmdb),
        imdb: movie.imdb,
        ratings: scores(movie.ratings),
        rated: movie.rated.as_deref().and_then(Date::parse),
        path: movie.path,
        added: date(&movie.added),
        runtime: small(movie.runtime),
        video: movie.video,
        resolution: movie.resolution,
        audio: movie.audio,
        review: movie.review,
        downloaded_by: movie.downloaded_by,
        about: movie.about.0,
    }
}

fn wide<T: Into<i64>>(value: Option<T>) -> Option<i64> {
    value.map(Into::into)
}

fn tmdb(value: Option<u64>) -> Option<i64> {
    value.and_then(|value| i64::try_from(value).ok())
}

fn same_show(stored: &ShowEntry, wanted: &ShowEntry) -> bool {
    ShowEntry {
        episodes: Vec::new(),
        ..stored.clone()
    } == ShowEntry {
        episodes: Vec::new(),
        id: stored.id,
        ..wanted.clone()
    }
}

async fn store(executor: &mut dyn toasty::Executor, wanted: Manifest) -> toasty::Result<()> {
    let stored = load(executor).await?;
    let mut shows: BTreeMap<i64, ShowEntry> = stored
        .show
        .iter()
        .cloned()
        .map(|show| (show.id.unwrap_or_default(), show))
        .collect();
    let mut episodes: BTreeMap<i64, (i64, EpisodeEntry)> = stored
        .show
        .iter()
        .flat_map(|show| {
            show.episodes
                .iter()
                .map(move |episode| (show.id.unwrap_or_default(), episode.clone()))
        })
        .map(|(show, episode)| (episode.id.unwrap_or_default(), (show, episode)))
        .collect();
    let mut movies: BTreeMap<i64, MovieEntry> = stored
        .movie
        .iter()
        .cloned()
        .map(|movie| (movie.id.unwrap_or_default(), movie))
        .collect();
    let show_by_title: HashMap<String, i64> = shows.iter().map(|(id, show)| (show.title.clone(), *id)).collect();
    let episode_by_path: HashMap<String, i64> = episodes
        .iter()
        .map(|(id, (_, episode))| (episode.path.clone(), *id))
        .collect();
    let movie_by_path: HashMap<String, i64> = movies.iter().map(|(id, movie)| (movie.path.clone(), *id)).collect();

    for show in &wanted.show {
        let known = show.id.filter(|id| shows.contains_key(id)).or_else(|| {
            show_by_title
                .get(&show.title)
                .copied()
                .filter(|id| shows.contains_key(id))
        });
        let show_id = match known {
            Some(id) => {
                let before = shows.remove(&id).expect("known show");
                if !same_show(&before, show) {
                    let mut row = Show::get_by_id(executor, &id).await?;
                    toasty::update!(row {
                        title: show.title.clone(),
                        year: wide(show.year),
                        last_aired: wide(show.last_aired),
                        running: show.running,
                        tmdb: tmdb(show.tmdb),
                        imdb: show.imdb.clone(),
                        ratings: stored_scores(&show.ratings),
                        rated: show.rated.map(|date| date.to_string()),
                        review: show.review,
                        about: show.about.clone(),
                    })
                    .exec(executor)
                    .await?;
                }
                id
            }
            None => {
                let row = toasty::create!(Show {
                    title: show.title.clone(),
                    year: wide(show.year),
                    last_aired: wide(show.last_aired),
                    running: show.running,
                    tmdb: tmdb(show.tmdb),
                    imdb: show.imdb.clone(),
                    ratings: stored_scores(&show.ratings),
                    rated: show.rated.map(|date| date.to_string()),
                    review: show.review,
                    about: show.about.clone(),
                })
                .exec(executor)
                .await?;
                row.id
            }
        };
        for episode in &show.episodes {
            let known = episode.id.filter(|id| episodes.contains_key(id)).or_else(|| {
                episode_by_path
                    .get(&episode.path)
                    .copied()
                    .filter(|id| episodes.contains_key(id))
            });
            match known {
                Some(id) => {
                    let (before_show, before) = episodes.remove(&id).expect("known episode");
                    let wanted_episode = EpisodeEntry {
                        id: Some(id),
                        ..episode.clone()
                    };
                    if before_show != show_id || before != wanted_episode {
                        let mut row = Episode::get_by_id(executor, &id).await?;
                        toasty::update!(row {
                            show_id: show_id,
                            season: i64::from(episode.season),
                            number: i64::from(episode.episode),
                            title: episode.title.clone(),
                            path: episode.path.clone(),
                            added: episode.added.to_string(),
                            runtime: wide(episode.runtime),
                            ratings: stored_scores(&episode.ratings),
                            video: episode.video.clone(),
                            resolution: episode.resolution.clone(),
                            audio: episode.audio.clone(),
                            review: episode.review,
                            downloaded_by: episode.downloaded_by.clone(),
                        })
                        .exec(executor)
                        .await?;
                    }
                }
                None => {
                    toasty::create!(Episode {
                        show_id: show_id,
                        season: i64::from(episode.season),
                        number: i64::from(episode.episode),
                        title: episode.title.clone(),
                        path: episode.path.clone(),
                        added: episode.added.to_string(),
                        runtime: wide(episode.runtime),
                        ratings: stored_scores(&episode.ratings),
                        video: episode.video.clone(),
                        resolution: episode.resolution.clone(),
                        audio: episode.audio.clone(),
                        review: episode.review,
                        downloaded_by: episode.downloaded_by.clone(),
                    })
                    .exec(executor)
                    .await?;
                }
            }
        }
    }
    for movie in &wanted.movie {
        let known = movie.id.filter(|id| movies.contains_key(id)).or_else(|| {
            movie_by_path
                .get(&movie.path)
                .copied()
                .filter(|id| movies.contains_key(id))
        });
        match known {
            Some(id) => {
                let before = movies.remove(&id).expect("known movie");
                let wanted_movie = MovieEntry {
                    id: Some(id),
                    ..movie.clone()
                };
                if before != wanted_movie {
                    let mut row = Movie::get_by_id(executor, &id).await?;
                    toasty::update!(row {
                        title: movie.title.clone(),
                        year: wide(movie.year),
                        tmdb: tmdb(movie.tmdb),
                        imdb: movie.imdb.clone(),
                        ratings: stored_scores(&movie.ratings),
                        rated: movie.rated.map(|date| date.to_string()),
                        path: movie.path.clone(),
                        added: movie.added.to_string(),
                        runtime: wide(movie.runtime),
                        video: movie.video.clone(),
                        resolution: movie.resolution.clone(),
                        audio: movie.audio.clone(),
                        review: movie.review,
                        downloaded_by: movie.downloaded_by.clone(),
                        about: movie.about.clone(),
                    })
                    .exec(executor)
                    .await?;
                }
            }
            None => {
                toasty::create!(Movie {
                    title: movie.title.clone(),
                    year: wide(movie.year),
                    tmdb: tmdb(movie.tmdb),
                    imdb: movie.imdb.clone(),
                    ratings: stored_scores(&movie.ratings),
                    rated: movie.rated.map(|date| date.to_string()),
                    path: movie.path.clone(),
                    added: movie.added.to_string(),
                    runtime: wide(movie.runtime),
                    video: movie.video.clone(),
                    resolution: movie.resolution.clone(),
                    audio: movie.audio.clone(),
                    review: movie.review,
                    downloaded_by: movie.downloaded_by.clone(),
                    about: movie.about.clone(),
                })
                .exec(executor)
                .await?;
            }
        }
    }
    let gone_episodes: Vec<i64> = episodes.into_keys().collect();
    if !gone_episodes.is_empty() {
        Episode::filter(Episode::fields().id().in_list(gone_episodes))
            .delete()
            .exec(executor)
            .await?;
    }
    for id in shows.into_keys() {
        if let Some(row) = Show::filter_by_id(id).first().exec(executor).await? {
            row.delete().exec(executor).await?;
        }
    }
    let gone_movies: Vec<i64> = movies.into_keys().collect();
    if !gone_movies.is_empty() {
        Movie::filter(Movie::fields().id().in_list(gone_movies))
            .delete()
            .exec(executor)
            .await?;
    }
    Ok(())
}

async fn load(executor: &mut dyn toasty::Executor) -> toasty::Result<Manifest> {
    let shows = Show::all().exec(executor).await?;
    let episodes = Episode::all().exec(executor).await?;
    let movies = Movie::all().exec(executor).await?;
    let mut by_show: HashMap<i64, Vec<EpisodeEntry>> = HashMap::new();
    for episode in episodes {
        by_show.entry(episode.show_id).or_default().push(episode_entry(episode));
    }
    Ok(Manifest {
        show: shows
            .into_iter()
            .map(|show| {
                let mut episodes = by_show.remove(&show.id).unwrap_or_default();
                episodes.sort_by_key(|episode| (episode.season, episode.episode, episode.id));
                show_entry(show, episodes)
            })
            .collect(),
        movie: movies.into_iter().map(movie_entry).collect(),
    })
}

pub async fn toasty_cli(file: &Path, arguments: Vec<String>) -> db::Result<()> {
    db::toasty_cli_for(file, models::all(), "toasty/library", arguments).await
}

pub fn from_toml(toml: &Path, file: &Path) -> Result<Manifest, String> {
    let text = fs::read_to_string(toml).map_err(|error| format!("{}: {error}", toml.display()))?;
    let manifest = Manifest::parse(&text).map_err(|error| format!("{}: {error}", toml.display()))?;
    manifest
        .write(file)
        .map_err(|error| format!("{}: {error}", file.display()))?;
    Manifest::read(file).map_err(|_| format!("{} could not be read back", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const TOML: &str = r#"
[[show]]
title = "Dark"
year = 2017
tmdb = 70523
imdb = "tt5753856"
ratings = { imdb = 8.7, rt = 94 }
rated = 2026-11-08
episodes = [
  { season = 1, episode = 1, title = "Secrets", path = "Series/Dark/S01E01.mkv", added = 2024-05-01, runtime = 51, video = "HEVC" },
  { season = 1, episode = 2, path = "Series/Dark/S01E02.mkv", added = 2024-05-01 },
]

[show.about]
plot = "A missing boy."
genres = ["Drama", "Mystery"]
first_aired = 2017-12-01
cast = [
  { name = "Louis Hofmann", character = "Jonas", person = 1 },
]

[[movie]]
title = "Up"
year = 2009
path = "Movies/Up/Up.mkv"
added = 2024-05-01
review = true
"#;

    fn file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("semmeltv-library-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join(manifest::FILE_NAME)
    }

    fn without_ids(mut manifest: Manifest) -> Manifest {
        for show in &mut manifest.show {
            show.id = None;
            for episode in &mut show.episodes {
                episode.id = None;
            }
        }
        for movie in &mut manifest.movie {
            movie.id = None;
        }
        manifest
    }

    #[test]
    fn a_library_survives_a_write_and_a_read_with_ids_given() {
        let path = file("round");
        let manifest = Manifest::parse(TOML).unwrap();
        manifest.write(&path).unwrap();
        let read = Manifest::read(&path).unwrap();
        assert_eq!(without_ids(read.clone()), manifest);
        assert!(read.show[0].id.is_some() && read.show[0].episodes[1].id.is_some() && read.movie[0].id.is_some());
        assert_eq!(read.show[0].about.cast[0].name, "Louis Hofmann");
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn ids_stay_when_titles_change_files_move_and_entries_come_and_go() {
        let path = file("ids");
        Manifest::parse(TOML).unwrap().write(&path).unwrap();
        let first = Manifest::read(&path).unwrap();
        let (dark, secrets, up) = (first.show[0].id, first.show[0].episodes[0].id, first.movie[0].id);

        let mut changed = first.clone();
        changed.show[0].title = "Dark (2017)".to_string();
        changed.show[0].episodes[0].path = "Series/Dark/Season 1/S01E01.mkv".to_string();
        changed.show[0].episodes.remove(1);
        changed.movie[0].ratings.rt = Some(98);
        let mut added = changed.movie[0].clone();
        added.id = None;
        added.title = "Heat".to_string();
        added.path = "Movies/Heat.mkv".to_string();
        changed.movie.push(added);
        changed.write(&path).unwrap();

        let second = Manifest::read(&path).unwrap();
        assert_eq!(second.show[0].id, dark);
        assert_eq!(second.show[0].title, "Dark (2017)");
        assert_eq!(second.show[0].episodes.len(), 1);
        assert_eq!(second.show[0].episodes[0].id, secrets);
        assert_eq!(second.show[0].episodes[0].path, "Series/Dark/Season 1/S01E01.mkv");
        let up_again = second.movie.iter().find(|movie| movie.title == "Up").unwrap();
        assert_eq!((up_again.id, up_again.ratings.rt), (up, Some(98)));
        assert!(
            second
                .movie
                .iter()
                .any(|movie| movie.title == "Heat" && movie.id.is_some() && movie.id != up)
        );
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_parsed_copy_written_again_keeps_the_ids_by_path_and_title() {
        let path = file("again");
        Manifest::parse(TOML).unwrap().write(&path).unwrap();
        let first = Manifest::read(&path).unwrap();
        Manifest::parse(TOML).unwrap().write(&path).unwrap();
        assert_eq!(Manifest::read(&path).unwrap(), first);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn who_downloaded_an_episode_or_a_film_survives_a_write_and_a_read() {
        let path = file("downloaded-by");
        let mut manifest = Manifest::parse(TOML).unwrap();
        manifest.show[0].episodes[1].downloaded_by = Some("Mami".to_string());
        manifest.movie[0].downloaded_by = Some("Frederik".to_string());
        manifest.write(&path).unwrap();
        let read = Manifest::read(&path).unwrap();
        assert_eq!(read.show[0].episodes[0].downloaded_by, None);
        assert_eq!(read.show[0].episodes[1].downloaded_by.as_deref(), Some("Mami"));
        assert_eq!(read.movie[0].downloaded_by.as_deref(), Some("Frederik"));
        assert_eq!(without_ids(Manifest::parse(&read.render()).unwrap()), manifest);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_library_from_before_downloaded_by_is_read_untouched_and_upgraded_by_the_next_write() {
        static FIRST: toasty::migration::MigrationSet =
            toasty::migration::MigrationSet::new(&MIGRATIONS_BEFORE_DOWNLOADED_BY);
        static MIGRATIONS_BEFORE_DOWNLOADED_BY: [toasty::migration::MigrationFile; 1] =
            [toasty::migration::MigrationFile::new(
                5746962358852197533,
                "0000_initial.sql",
                include_str!("../../toasty/library/migrations/0000_initial.sql"),
            )];
        let path = file("before-downloaded-by");
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let db = toasty::Db::builder()
                    .models(models::all())
                    .build(driver(&path))
                    .await
                    .unwrap();
                FIRST.apply(&db).await.unwrap();
            });
        let old = rusqlite::Connection::open(&path).unwrap();
        old.execute(
            "INSERT INTO movies (title, path, added, review, about) VALUES ('Up', 'Movies/Up/Up.mkv', '2024-05-01', 0, '{}')",
            [],
        )
        .unwrap();
        drop(old);
        let before = fs::read(&path).unwrap();

        let read = Manifest::read(&path).unwrap();
        assert_eq!(
            (read.movie[0].title.as_str(), &read.movie[0].downloaded_by),
            ("Up", &None)
        );
        assert_eq!(fs::read(&path).unwrap(), before, "reading leaves the file as it was");

        let mut changed = read.clone();
        changed.movie[0].downloaded_by = Some("Frederik".to_string());
        changed.write(&path).unwrap();
        let again = Manifest::read(&path).unwrap();
        assert_eq!(
            (again.movie[0].id, again.movie[0].downloaded_by.as_deref()),
            (read.movie[0].id, Some("Frederik"))
        );
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_missing_library_is_unreadable_and_garbage_is_invalid() {
        let path = file("missing");
        assert!(matches!(Manifest::read(&path), Err(ReadError::Unreadable(_))));
        fs::write(&path, b"not a database").unwrap();
        assert!(matches!(Manifest::read(&path), Err(ReadError::Invalid(_))));
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
