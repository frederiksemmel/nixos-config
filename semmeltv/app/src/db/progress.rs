use super::schema::{dismissed, file_places, files, show_places, watching};
use super::{Db, Result};
use crate::profile::ProfileId;
use crate::progress::{Position, Progress, Tracks, Watched};
use crate::watching::{Dismissal, Entry, Kind, Watching};
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Queryable, Selectable, Insertable)]
#[diesel(table_name = files, check_for_backend(diesel::sqlite::Sqlite))]
struct FileFacts {
    path: String,
    duration: Option<f64>,
    credits: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Queryable, Selectable, Insertable)]
#[diesel(table_name = file_places, check_for_backend(diesel::sqlite::Sqlite))]
struct FilePlace {
    profile_id: i64,
    path: String,
    stopped_at: Option<f64>,
    last_played: Option<i64>,
    finished: bool,
    audio_track: Option<i64>,
    subtitle_track: Option<i64>,
    subtitles_shown: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Queryable, Selectable, Insertable)]
#[diesel(table_name = show_places, check_for_backend(diesel::sqlite::Sqlite))]
struct ShowPlace {
    profile_id: i64,
    show_key: String,
    season: i64,
    episode: i64,
    finished: bool,
}

#[derive(Clone, Debug, PartialEq, Queryable, Selectable, Insertable)]
#[diesel(table_name = watching, check_for_backend(diesel::sqlite::Sqlite))]
struct WatchingEntry {
    profile_id: i64,
    kind: String,
    key: String,
    last_played: i64,
}

#[derive(Clone, Debug, PartialEq, Queryable, Selectable, Insertable)]
#[diesel(table_name = dismissed, check_for_backend(diesel::sqlite::Sqlite))]
struct DismissedFile {
    profile_id: i64,
    path: String,
    at: i64,
}

#[derive(Debug, Default, PartialEq)]
struct Rows {
    facts: Vec<FileFacts>,
    places: Vec<FilePlace>,
    shows: Vec<ShowPlace>,
    watching: Vec<WatchingEntry>,
    dismissed: Vec<DismissedFile>,
}

fn load(connection: &mut SqliteConnection, profile: ProfileId) -> QueryResult<Rows> {
    Ok(Rows {
        facts: files::table
            .filter(files::duration.is_not_null().or(files::credits.is_not_null()))
            .select(FileFacts::as_select())
            .load(connection)?,
        places: file_places::table
            .filter(file_places::profile_id.eq(profile.0))
            .select(FilePlace::as_select())
            .load(connection)?,
        shows: show_places::table
            .filter(show_places::profile_id.eq(profile.0))
            .select(ShowPlace::as_select())
            .load(connection)?,
        watching: watching::table
            .filter(watching::profile_id.eq(profile.0))
            .order((watching::last_played.desc(), watching::kind, watching::key))
            .select(WatchingEntry::as_select())
            .load(connection)?,
        dismissed: dismissed::table
            .filter(dismissed::profile_id.eq(profile.0))
            .order((dismissed::at, dismissed::path))
            .select(DismissedFile::as_select())
            .load(connection)?,
    })
}

fn changed<K: Ord, R: PartialEq>(stored: Vec<R>, wanted: Vec<R>, key: impl Fn(&R) -> K) -> (Vec<R>, Vec<K>) {
    let mut gone: BTreeMap<K, R> = stored.into_iter().map(|row| (key(&row), row)).collect();
    let mut write = Vec::new();
    for row in wanted {
        if gone.remove(&key(&row)).is_none_or(|stored| stored != row) {
            write.push(row);
        }
    }
    (write, gone.into_keys().collect())
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Series => "series",
        Kind::Movie => "movie",
    }
}

fn kind_of(name: &str) -> Option<Kind> {
    match name {
        "series" => Some(Kind::Series),
        "movie" => Some(Kind::Movie),
        _ => None,
    }
}

impl Db {
    pub fn progress(&self, profile: ProfileId) -> Result<Progress> {
        let rows = self.run(|connection| load(connection, profile))?;
        Ok(self.assemble(rows))
    }

    pub fn save_progress(&self, profile: ProfileId, progress: &Progress) -> Result<()> {
        let wanted = self.rows(profile, progress);
        self.run(|connection| {
            connection.immediate_transaction(|connection| {
                let stored = load(connection, profile)?;
                let (facts, _) = changed(stored.facts, wanted.facts, |row| row.path.clone());
                for row in &facts {
                    diesel::insert_into(files::table)
                        .values(row)
                        .on_conflict(files::path)
                        .do_update()
                        .set((files::duration.eq(row.duration), files::credits.eq(row.credits)))
                        .execute(connection)?;
                }
                let (places, gone) = changed(stored.places, wanted.places, |row| row.path.clone());
                for row in &places {
                    diesel::replace_into(file_places::table).values(row).execute(connection)?;
                }
                diesel::delete(
                    file_places::table
                        .filter(file_places::profile_id.eq(profile.0))
                        .filter(file_places::path.eq_any(gone)),
                )
                .execute(connection)?;
                let (shows, gone) = changed(stored.shows, wanted.shows, |row| row.show_key.clone());
                for row in &shows {
                    diesel::replace_into(show_places::table).values(row).execute(connection)?;
                }
                diesel::delete(
                    show_places::table
                        .filter(show_places::profile_id.eq(profile.0))
                        .filter(show_places::show_key.eq_any(gone)),
                )
                .execute(connection)?;
                let (entries, gone) = changed(stored.watching, wanted.watching, |row| {
                    (row.kind.clone(), row.key.clone())
                });
                for row in &entries {
                    diesel::replace_into(watching::table).values(row).execute(connection)?;
                }
                for (kind, key) in gone {
                    diesel::delete(
                        watching::table
                            .filter(watching::profile_id.eq(profile.0))
                            .filter(watching::kind.eq(kind))
                            .filter(watching::key.eq(key)),
                    )
                    .execute(connection)?;
                }
                let (dismissals, gone) = changed(stored.dismissed, wanted.dismissed, |row| row.path.clone());
                for row in &dismissals {
                    diesel::replace_into(dismissed::table).values(row).execute(connection)?;
                }
                diesel::delete(
                    dismissed::table
                        .filter(dismissed::profile_id.eq(profile.0))
                        .filter(dismissed::path.eq_any(gone)),
                )
                .execute(connection)?;
                Ok(())
            })
        })
    }

    fn rows(&self, profile: ProfileId, progress: &Progress) -> Rows {
        let mut rows = Rows::default();
        for (file, watched) in &progress.files {
            let path = self.key(file);
            if watched.duration.is_some() || watched.credits.is_some() {
                rows.facts.push(FileFacts {
                    path: path.clone(),
                    duration: watched.duration,
                    credits: watched.credits,
                });
            }
            let place = FilePlace {
                profile_id: profile.0,
                path,
                stopped_at: watched.stopped_at,
                last_played: watched.last_played.map(|at| at as i64),
                finished: watched.finished,
                audio_track: watched.tracks.audio,
                subtitle_track: watched.tracks.subtitle,
                subtitles_shown: watched.tracks.subtitles_shown,
            };
            let empty = FilePlace {
                path: place.path.clone(),
                ..FilePlace::empty(profile)
            };
            if place != empty {
                rows.places.push(place);
            }
        }
        rows.shows = progress
            .shows
            .iter()
            .map(|(key, position)| ShowPlace {
                profile_id: profile.0,
                show_key: key.clone(),
                season: position.season.into(),
                episode: position.episode.into(),
                finished: position.finished,
            })
            .collect();
        rows.watching = progress
            .watching
            .entries()
            .iter()
            .map(|entry| WatchingEntry {
                profile_id: profile.0,
                kind: kind_name(entry.kind).to_string(),
                key: entry.key.clone(),
                last_played: entry.last_played as i64,
            })
            .collect();
        let mut dismissed: BTreeMap<String, i64> = BTreeMap::new();
        for dismissal in &progress.dismissed {
            dismissed.insert(self.key(&dismissal.file), dismissal.at as i64);
        }
        rows.dismissed = dismissed
            .into_iter()
            .map(|(path, at)| DismissedFile {
                profile_id: profile.0,
                path,
                at,
            })
            .collect();
        rows
    }

    fn assemble(&self, rows: Rows) -> Progress {
        let mut progress = Progress::default();
        for facts in rows.facts {
            let watched = progress.files.entry(self.path(&facts.path)).or_default();
            watched.duration = facts.duration;
            watched.credits = facts.credits;
        }
        for place in rows.places {
            let watched: &mut Watched = progress.files.entry(self.path(&place.path)).or_default();
            watched.stopped_at = place.stopped_at;
            watched.last_played = place.last_played.and_then(|at| u64::try_from(at).ok());
            watched.finished = place.finished;
            watched.tracks = Tracks {
                audio: place.audio_track,
                subtitle: place.subtitle_track,
                subtitles_shown: place.subtitles_shown,
            };
        }
        for show in rows.shows {
            let (Ok(season), Ok(episode)) = (u32::try_from(show.season), u32::try_from(show.episode)) else {
                continue;
            };
            let position = Position {
                season,
                episode,
                finished: show.finished,
            };
            progress.shows.insert(show.show_key, position);
        }
        progress.watching = Watching::from_entries(
            rows.watching
                .into_iter()
                .filter_map(|entry| {
                    Some(Entry {
                        kind: kind_of(&entry.kind)?,
                        key: entry.key,
                        last_played: u64::try_from(entry.last_played).ok()?,
                    })
                })
                .collect(),
        );
        progress.dismissed = rows
            .dismissed
            .into_iter()
            .map(|dismissed| Dismissal {
                file: self.path(&dismissed.path),
                at: u64::try_from(dismissed.at).unwrap_or_default(),
            })
            .collect();
        progress
    }
}

impl FilePlace {
    fn empty(profile: ProfileId) -> Self {
        Self {
            profile_id: profile.0,
            path: String::new(),
            stopped_at: None,
            last_played: None,
            finished: false,
            audio_track: None,
            subtitle_track: None,
            subtitles_shown: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::Place;
    use std::path::{Path, PathBuf};

    const DARK: &str = "/lib/Series/Dark/S01/Dark.S01E02.mkv";
    const UP: &str = "/lib/Movies/Up/Up.mkv";

    fn db() -> Db {
        Db::memory(Path::new("/lib"))
    }

    fn sample() -> Progress {
        let mut progress = Progress::default();
        progress.shows.insert(
            "dark".to_string(),
            Position {
                season: 1,
                episode: 2,
                finished: false,
            },
        );
        progress.record(
            Path::new(DARK),
            Place {
                position: 600.0,
                duration: 2700.0,
                credits: Some(2600.0),
            },
        );
        progress.played(Path::new(DARK), 1_800_000_100);
        progress.keep_tracks(
            Path::new(DARK),
            Tracks {
                audio: Some(1),
                subtitle: Some(3),
                subtitles_shown: Some(true),
            },
        );
        progress.record(
            Path::new(UP),
            Place {
                position: 0.0,
                duration: 5760.0,
                credits: None,
            },
        );
        progress.finish(Path::new(UP));
        progress.watching.played(Kind::Movie, "up", 1_800_000_000);
        progress.watching.played(Kind::Series, "dark", 1_800_000_100);
        progress.dismissed.push(Dismissal {
            file: PathBuf::from(UP),
            at: 1_800_000_050,
        });
        progress
    }

    #[test]
    fn progress_survives_a_save_and_a_load() {
        let db = db();
        let progress = sample();
        db.save_progress(ProfileId::FIRST, &progress).unwrap();
        assert_eq!(db.progress(ProfileId::FIRST).unwrap(), progress);
    }

    #[test]
    fn a_save_keeps_only_what_is_left() {
        let db = db();
        let mut progress = sample();
        db.save_progress(ProfileId::FIRST, &progress).unwrap();
        progress.shows.clear();
        progress.watching.leave(Kind::Movie, "up");
        progress.dismissed.clear();
        progress.finish(Path::new(DARK));
        db.save_progress(ProfileId::FIRST, &progress).unwrap();
        let loaded = db.progress(ProfileId::FIRST).unwrap();
        assert_eq!(loaded, progress);
        assert_eq!(loaded.watching.entries().len(), 1);
    }

    #[test]
    fn places_are_kept_per_profile_and_what_a_file_is_like_is_shared() {
        let db = db();
        db.save_progress(ProfileId::FIRST, &sample()).unwrap();
        let other = db.progress(ProfileId(2)).unwrap();
        assert!(other.shows.is_empty());
        assert!(other.watching.entries().is_empty());
        assert!(other.dismissed.is_empty());
        let dark = other.files[Path::new(DARK)];
        assert_eq!(dark.stopped_at, None);
        assert_eq!(dark.last_played, None);
        assert_eq!(dark.tracks, Tracks::default());
        assert_eq!(dark.duration, Some(2700.0));
        assert_eq!(dark.credits, Some(2600.0));
        assert!(!other.files[Path::new(UP)].finished);
    }

    #[test]
    fn another_profiles_save_leaves_the_first_profiles_places_alone() {
        let db = db();
        let first = sample();
        db.save_progress(ProfileId::FIRST, &first).unwrap();
        let mut other = db.progress(ProfileId(2)).unwrap();
        other.record(
            Path::new(UP),
            Place {
                position: 100.0,
                duration: 5760.0,
                credits: None,
            },
        );
        db.save_progress(ProfileId(2), &other).unwrap();
        assert_eq!(db.progress(ProfileId::FIRST).unwrap(), first);
        assert_eq!(db.progress(ProfileId(2)).unwrap(), other);
    }

    #[test]
    fn paths_are_stored_as_the_library_names_them() {
        let db = db();
        db.save_progress(ProfileId::FIRST, &sample()).unwrap();
        let stored: Vec<String> = db
            .run(|connection| file_places::table.select(file_places::path).order(file_places::path).load(connection))
            .unwrap();
        assert_eq!(stored, ["Movies/Up/Up.mkv", "Series/Dark/S01/Dark.S01E02.mkv"]);
    }
}
