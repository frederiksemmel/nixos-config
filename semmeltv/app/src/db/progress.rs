use super::models::{self, Dismissal as StoredDismissal, File, FilePlace, ShowPlace, Watching as StoredWatching};
use super::{Db, Result};
use crate::profile::ProfileId;
use crate::progress::{Position, Progress, Tracks, Watched};
use crate::watching::{Dismissal, Entry, Kind, Watching};
use std::collections::BTreeMap;
use toasty::Executor;
use toasty_core::driver::operation::TransactionMode;

#[derive(Clone, Debug, PartialEq)]
struct Facts {
    path: String,
    duration: Option<f64>,
    credits: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
struct Spot {
    path: String,
    stopped_at: Option<f64>,
    last_played: Option<i64>,
    finished: bool,
    audio_track: Option<i64>,
    subtitle_track: Option<i64>,
    subtitles_shown: Option<bool>,
}

#[derive(Clone, Debug, PartialEq)]
struct Show {
    key: String,
    season: i64,
    episode: i64,
    finished: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct Started {
    kind: models::Kind,
    key: String,
    last_played: i64,
}

#[derive(Clone, Debug, PartialEq)]
struct Dismissed {
    path: String,
    at: i64,
}

#[derive(Debug, Default, PartialEq)]
struct Rows {
    facts: Vec<Facts>,
    spots: Vec<Spot>,
    shows: Vec<Show>,
    watching: Vec<Started>,
    dismissed: Vec<Dismissed>,
}

async fn load(executor: &mut dyn Executor, profile: i64) -> toasty::Result<Rows> {
    let file = File::fields();
    let facts = File::filter(file.duration().is_some().or(file.credits().is_some()))
        .exec(executor)
        .await?;
    let spots = FilePlace::filter(FilePlace::fields().profile_id().eq(profile))
        .exec(executor)
        .await?;
    let shows = ShowPlace::filter(ShowPlace::fields().profile_id().eq(profile))
        .exec(executor)
        .await?;
    let watching = StoredWatching::filter(StoredWatching::fields().profile_id().eq(profile))
        .order_by((
            StoredWatching::fields().last_played().desc(),
            StoredWatching::fields().title_key().asc(),
        ))
        .exec(executor)
        .await?;
    let dismissed = StoredDismissal::filter(StoredDismissal::fields().profile_id().eq(profile))
        .order_by((StoredDismissal::fields().at().asc(), StoredDismissal::fields().path().asc()))
        .exec(executor)
        .await?;
    Ok(Rows {
        facts: facts
            .into_iter()
            .map(|file| Facts {
                path: file.path,
                duration: file.duration,
                credits: file.credits,
            })
            .collect(),
        spots: spots
            .into_iter()
            .map(|place| Spot {
                path: place.path,
                stopped_at: place.stopped_at,
                last_played: place.last_played,
                finished: place.finished,
                audio_track: place.audio_track,
                subtitle_track: place.subtitle_track,
                subtitles_shown: place.subtitles_shown,
            })
            .collect(),
        shows: shows
            .into_iter()
            .map(|show| Show {
                key: show.show_key,
                season: show.season,
                episode: show.episode,
                finished: show.finished,
            })
            .collect(),
        watching: watching
            .into_iter()
            .map(|entry| Started {
                kind: entry.kind,
                key: entry.title_key,
                last_played: entry.last_played,
            })
            .collect(),
        dismissed: dismissed
            .into_iter()
            .map(|dismissal| Dismissed {
                path: dismissal.path,
                at: dismissal.at,
            })
            .collect(),
    })
}

async fn store(executor: &mut dyn Executor, profile: i64, wanted: Rows) -> toasty::Result<()> {
    let stored = load(executor, profile).await?;
    let (facts, _) = changed(stored.facts, wanted.facts, |row| row.path.clone());
    for row in facts {
        File::upsert_by_path(row.path)
            .duration(row.duration)
            .credits(row.credits)
            .exec(executor)
            .await?;
    }
    let (spots, gone) = changed(stored.spots, wanted.spots, |row| row.path.clone());
    for row in spots {
        FilePlace::upsert_by_profile_id_and_path(profile, row.path)
            .stopped_at(row.stopped_at)
            .last_played(row.last_played)
            .finished(row.finished)
            .audio_track(row.audio_track)
            .subtitle_track(row.subtitle_track)
            .subtitles_shown(row.subtitles_shown)
            .exec(executor)
            .await?;
    }
    if !gone.is_empty() {
        let fields = FilePlace::fields();
        FilePlace::filter(fields.profile_id().eq(profile).and(fields.path().in_list(gone)))
            .delete()
            .exec(executor)
            .await?;
    }
    let (shows, gone) = changed(stored.shows, wanted.shows, |row| row.key.clone());
    for row in shows {
        ShowPlace::upsert_by_profile_id_and_show_key(profile, row.key)
            .season(row.season)
            .episode(row.episode)
            .finished(row.finished)
            .exec(executor)
            .await?;
    }
    if !gone.is_empty() {
        let fields = ShowPlace::fields();
        ShowPlace::filter(fields.profile_id().eq(profile).and(fields.show_key().in_list(gone)))
            .delete()
            .exec(executor)
            .await?;
    }
    let (entries, gone) = changed(stored.watching, wanted.watching, |row| {
        (kind_name(row.kind), row.key.clone())
    });
    for row in entries {
        StoredWatching::upsert_by_profile_id_and_kind_and_title_key(profile, row.kind, row.key)
            .last_played(row.last_played)
            .exec(executor)
            .await?;
    }
    for (kind, key) in gone {
        let fields = StoredWatching::fields();
        StoredWatching::filter(
            fields
                .profile_id()
                .eq(profile)
                .and(fields.kind().eq(kind_of(kind)))
                .and(fields.title_key().eq(key)),
        )
        .delete()
        .exec(executor)
        .await?;
    }
    let (dismissals, gone) = changed(stored.dismissed, wanted.dismissed, |row| row.path.clone());
    for row in dismissals {
        StoredDismissal::upsert_by_profile_id_and_path(profile, row.path)
            .at(row.at)
            .exec(executor)
            .await?;
    }
    if !gone.is_empty() {
        let fields = StoredDismissal::fields();
        StoredDismissal::filter(fields.profile_id().eq(profile).and(fields.path().in_list(gone)))
            .delete()
            .exec(executor)
            .await?;
    }
    Ok(())
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

fn kind_name(kind: models::Kind) -> &'static str {
    match kind {
        models::Kind::Series => "series",
        models::Kind::Movie => "movie",
    }
}

fn kind_of(name: &str) -> models::Kind {
    if name == "series" {
        models::Kind::Series
    } else {
        models::Kind::Movie
    }
}

impl Db {
    pub fn progress(&self, profile: ProfileId) -> Result<Progress> {
        let rows = self.wait(move |mut db| async move { load(&mut db, profile.0).await })?;
        Ok(self.assemble(rows))
    }

    pub fn save_progress(&self, profile: ProfileId, progress: &Progress) -> Result<()> {
        let wanted = self.rows(progress);
        self.wait(move |mut db| async move {
            let mut transaction = db
                .transaction_builder()
                .mode(TransactionMode::Immediate)
                .begin()
                .await?;
            store(&mut transaction, profile.0, wanted).await?;
            transaction.commit().await
        })
    }

    fn rows(&self, progress: &Progress) -> Rows {
        let mut rows = Rows::default();
        for (file, watched) in &progress.files {
            let path = self.key(file);
            if watched.duration.is_some() || watched.credits.is_some() {
                rows.facts.push(Facts {
                    path: path.clone(),
                    duration: watched.duration,
                    credits: watched.credits,
                });
            }
            let spot = Spot {
                path,
                stopped_at: watched.stopped_at,
                last_played: watched.last_played.map(|at| at as i64),
                finished: watched.finished,
                audio_track: watched.tracks.audio,
                subtitle_track: watched.tracks.subtitle,
                subtitles_shown: watched.tracks.subtitles_shown,
            };
            let untouched = spot.stopped_at.is_none()
                && spot.last_played.is_none()
                && !spot.finished
                && watched.tracks == Tracks::default();
            if !untouched {
                rows.spots.push(spot);
            }
        }
        rows.shows = progress
            .shows
            .iter()
            .map(|(key, position)| Show {
                key: key.clone(),
                season: position.season.into(),
                episode: position.episode.into(),
                finished: position.finished,
            })
            .collect();
        rows.watching = progress
            .watching
            .entries()
            .iter()
            .map(|entry| Started {
                kind: match entry.kind {
                    Kind::Series => models::Kind::Series,
                    Kind::Movie => models::Kind::Movie,
                },
                key: entry.key.clone(),
                last_played: entry.last_played as i64,
            })
            .collect();
        let mut dismissed: BTreeMap<String, i64> = BTreeMap::new();
        for dismissal in &progress.dismissed {
            dismissed.insert(self.key(&dismissal.file), dismissal.at as i64);
        }
        rows.dismissed = dismissed.into_iter().map(|(path, at)| Dismissed { path, at }).collect();
        rows
    }

    fn assemble(&self, rows: Rows) -> Progress {
        let mut progress = Progress::default();
        for facts in rows.facts {
            let watched = progress.files.entry(self.path(&facts.path)).or_default();
            watched.duration = facts.duration;
            watched.credits = facts.credits;
        }
        for spot in rows.spots {
            let watched: &mut Watched = progress.files.entry(self.path(&spot.path)).or_default();
            watched.stopped_at = spot.stopped_at;
            watched.last_played = spot.last_played.and_then(|at| u64::try_from(at).ok());
            watched.finished = spot.finished;
            watched.tracks = Tracks {
                audio: spot.audio_track,
                subtitle: spot.subtitle_track,
                subtitles_shown: spot.subtitles_shown,
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
            progress.shows.insert(show.key, position);
        }
        progress.watching = Watching::from_entries(
            rows.watching
                .into_iter()
                .filter_map(|entry| {
                    Some(Entry {
                        kind: match entry.kind {
                            models::Kind::Series => Kind::Series,
                            models::Kind::Movie => Kind::Movie,
                        },
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
            .wait(|mut db| async move {
                let places = FilePlace::all()
                    .order_by(FilePlace::fields().path().asc())
                    .exec(&mut db)
                    .await?;
                Ok(places.into_iter().map(|place| place.path).collect())
            })
            .unwrap();
        assert_eq!(stored, ["Movies/Up/Up.mkv", "Series/Dark/S01/Dark.S01E02.mkv"]);
    }
}
