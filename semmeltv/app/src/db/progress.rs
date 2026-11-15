use super::index::{self, Index, Known, Video};
use super::models::{self, Dismissal as StoredDismissal, SeriesPlace, Started as StoredStarted, VideoFact, VideoPlace};
use super::{Db, Result};
use crate::profile::ProfileId;
use crate::progress::{Position, Progress, Tracks, Viewing, Watched};
use crate::watching::{Dismissal, Entry, Watching};
use std::collections::BTreeMap;
use toasty::Executor;
use toasty_core::driver::operation::TransactionMode;

#[derive(Clone, Debug, PartialEq)]
struct Facts {
    video: Video,
    duration: Option<f64>,
    credits: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
struct Spot {
    video: Video,
    viewing: Viewing,
    last_played: Option<i64>,
}

fn to_stored(viewing: Viewing) -> models::Viewing {
    match viewing {
        Viewing::NotStarted => models::Viewing::NotStarted,
        Viewing::InProgress { at, tracks } => models::Viewing::InProgress {
            at,
            audio_track: tracks.audio,
            subtitle_track: tracks.subtitle,
            subtitles_shown: tracks.subtitles_shown,
        },
        Viewing::Finished => models::Viewing::Finished,
    }
}

fn from_stored(stored: models::Viewing) -> Viewing {
    match stored {
        models::Viewing::NotStarted => Viewing::NotStarted,
        models::Viewing::InProgress {
            at,
            audio_track,
            subtitle_track,
            subtitles_shown,
        } => Viewing::InProgress {
            at,
            tracks: Tracks {
                audio: audio_track,
                subtitle: subtitle_track,
                subtitles_shown,
            },
        },
        models::Viewing::Finished => Viewing::Finished,
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Show {
    show_id: i64,
    season: i64,
    episode: i64,
    finished: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct Started {
    kind: models::Kind,
    title_id: i64,
    last_played: i64,
}

#[derive(Clone, Debug, PartialEq)]
struct Dismissed {
    video: Video,
    at: i64,
}

#[derive(Debug, Default, PartialEq)]
struct Rows {
    facts: Vec<Facts>,
    spots: Vec<Spot>,
    shows: Vec<Show>,
    started: Vec<Started>,
    dismissed: Vec<Dismissed>,
}

async fn load(executor: &mut dyn Executor, profile: i64) -> toasty::Result<Rows> {
    let fact = VideoFact::fields();
    let facts = VideoFact::filter(fact.duration().is_some().or(fact.credits().is_some()))
        .exec(executor)
        .await?;
    let spots = VideoPlace::filter(VideoPlace::fields().profile_id().eq(profile))
        .exec(executor)
        .await?;
    let shows = SeriesPlace::filter(SeriesPlace::fields().profile_id().eq(profile))
        .exec(executor)
        .await?;
    let started = StoredStarted::filter(StoredStarted::fields().profile_id().eq(profile))
        .order_by((
            StoredStarted::fields().last_played().desc(),
            StoredStarted::fields().title_id().asc(),
        ))
        .exec(executor)
        .await?;
    let dismissed = StoredDismissal::filter(StoredDismissal::fields().profile_id().eq(profile))
        .order_by((StoredDismissal::fields().at().asc(), StoredDismissal::fields().video_id().asc()))
        .exec(executor)
        .await?;
    Ok(Rows {
        facts: facts
            .into_iter()
            .map(|fact| Facts {
                video: Video {
                    kind: fact.kind,
                    id: fact.video_id,
                },
                duration: fact.duration,
                credits: fact.credits,
            })
            .collect(),
        spots: spots
            .into_iter()
            .map(|place| Spot {
                video: Video {
                    kind: place.kind,
                    id: place.video_id,
                },
                viewing: from_stored(place.viewing),
                last_played: place.last_played,
            })
            .collect(),
        shows: shows
            .into_iter()
            .map(|show| Show {
                show_id: show.show_id,
                season: show.season,
                episode: show.episode,
                finished: show.finished,
            })
            .collect(),
        started: started
            .into_iter()
            .map(|entry| Started {
                kind: entry.kind,
                title_id: entry.title_id,
                last_played: entry.last_played,
            })
            .collect(),
        dismissed: dismissed
            .into_iter()
            .map(|dismissal| Dismissed {
                video: Video {
                    kind: dismissal.kind,
                    id: dismissal.video_id,
                },
                at: dismissal.at,
            })
            .collect(),
    })
}

async fn store(executor: &mut dyn Executor, profile: i64, wanted: Rows, known: Known) -> toasty::Result<()> {
    let stored = load(executor, profile).await?;
    let (facts, _) = changed(stored.facts, wanted.facts, |row| row.video);
    for row in facts {
        VideoFact::upsert_by_kind_and_video_id(row.video.kind, row.video.id)
            .duration(row.duration)
            .credits(row.credits)
            .exec(executor)
            .await?;
    }
    let (spots, gone) = changed(stored.spots, wanted.spots, |row| row.video);
    for row in spots {
        VideoPlace::upsert_by_profile_id_and_kind_and_video_id(profile, row.video.kind, row.video.id)
            .viewing(to_stored(row.viewing))
            .last_played(row.last_played)
            .exec(executor)
            .await?;
    }
    for video in gone.into_iter().filter(|video| known.videos.contains(video)) {
        let fields = VideoPlace::fields();
        VideoPlace::filter(
            fields
                .profile_id()
                .eq(profile)
                .and(fields.kind().eq(video.kind))
                .and(fields.video_id().eq(video.id)),
        )
        .delete()
        .exec(executor)
        .await?;
    }
    let (shows, gone) = changed(stored.shows, wanted.shows, |row| row.show_id);
    for row in shows {
        SeriesPlace::upsert_by_profile_id_and_show_id(profile, row.show_id)
            .season(row.season)
            .episode(row.episode)
            .finished(row.finished)
            .exec(executor)
            .await?;
    }
    let gone: Vec<i64> = gone.into_iter().filter(|id| known.shows.contains(id)).collect();
    if !gone.is_empty() {
        let fields = SeriesPlace::fields();
        SeriesPlace::filter(fields.profile_id().eq(profile).and(fields.show_id().in_list(gone)))
            .delete()
            .exec(executor)
            .await?;
    }
    let (started, gone) = changed(stored.started, wanted.started, |row| (row.kind, row.title_id));
    for row in started {
        StoredStarted::upsert_by_profile_id_and_kind_and_title_id(profile, row.kind, row.title_id)
            .last_played(row.last_played)
            .exec(executor)
            .await?;
    }
    for (kind, id) in gone.into_iter().filter(|title| known.titles.contains(title)) {
        let fields = StoredStarted::fields();
        StoredStarted::filter(
            fields
                .profile_id()
                .eq(profile)
                .and(fields.kind().eq(kind))
                .and(fields.title_id().eq(id)),
        )
        .delete()
        .exec(executor)
        .await?;
    }
    let (dismissals, gone) = changed(stored.dismissed, wanted.dismissed, |row| row.video);
    for row in dismissals {
        StoredDismissal::upsert_by_profile_id_and_kind_and_video_id(profile, row.video.kind, row.video.id)
            .at(row.at)
            .exec(executor)
            .await?;
    }
    for video in gone.into_iter().filter(|video| known.videos.contains(video)) {
        let fields = StoredDismissal::fields();
        StoredDismissal::filter(
            fields
                .profile_id()
                .eq(profile)
                .and(fields.kind().eq(video.kind))
                .and(fields.video_id().eq(video.id)),
        )
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

impl Db {
    pub async fn progress(&self, profile: ProfileId) -> Result<Progress> {
        let mut db = self.db.clone();
        let rows = load(&mut db, profile.0).await?;
        let index = self.index.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut blind = self.blind.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if index.is_empty() {
            blind.insert(profile);
        } else {
            blind.remove(&profile);
        }
        Ok(assemble(&index, rows))
    }

    pub async fn save_progress(&self, profile: ProfileId, progress: &Progress) -> Result<()> {
        let (wanted, known) = {
            let index = self.index.read().unwrap_or_else(|poisoned| poisoned.into_inner());
            let blind = self.blind.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let known = if blind.contains(&profile) { Known::default() } else { index.known() };
            (rows(&index, progress), known)
        };
        let mut db = self.db.clone();
        let mut transaction = db
            .transaction_builder()
            .mode(TransactionMode::Immediate)
            .begin()
            .await?;
        store(&mut transaction, profile.0, wanted, known).await?;
        transaction.commit().await?;
        Ok(())
    }
}

fn rows(index: &Index, progress: &Progress) -> Rows {
    let mut rows = Rows::default();
    for (file, watched) in &progress.files {
        let Some(video) = index.video(file) else {
            continue;
        };
        if watched.duration.is_some() || watched.credits.is_some() {
            rows.facts.push(Facts {
                video,
                duration: watched.duration,
                credits: watched.credits,
            });
        }
        if watched.viewing != Viewing::NotStarted || watched.last_played.is_some() {
            rows.spots.push(Spot {
                video,
                viewing: watched.viewing,
                last_played: watched.last_played.map(|at| at as i64),
            });
        }
    }
    rows.shows = progress
        .shows
        .iter()
        .filter_map(|(key, position)| {
            Some(Show {
                show_id: index.show(key)?,
                season: position.season.into(),
                episode: position.episode.into(),
                finished: position.finished,
            })
        })
        .collect();
    rows.started = progress
        .watching
        .entries()
        .iter()
        .filter_map(|entry| {
            Some(Started {
                kind: index::kind(entry.kind),
                title_id: index.title(entry.kind, &entry.key)?,
                last_played: entry.last_played as i64,
            })
        })
        .collect();
    let mut dismissed: BTreeMap<Video, i64> = BTreeMap::new();
    for dismissal in &progress.dismissed {
        if let Some(video) = index.video(&dismissal.file) {
            dismissed.insert(video, dismissal.at as i64);
        }
    }
    rows.dismissed = dismissed.into_iter().map(|(video, at)| Dismissed { video, at }).collect();
    rows
}

fn assemble(index: &Index, rows: Rows) -> Progress {
    let mut progress = Progress::default();
    for facts in rows.facts {
        let Some(path) = index.path(facts.video) else {
            continue;
        };
        let watched = progress.files.entry(path.clone()).or_default();
        watched.duration = facts.duration;
        watched.credits = facts.credits;
    }
    for spot in rows.spots {
        let Some(path) = index.path(spot.video) else {
            continue;
        };
        let watched: &mut Watched = progress.files.entry(path.clone()).or_default();
        watched.viewing = spot.viewing;
        watched.last_played = spot.last_played.and_then(|at| u64::try_from(at).ok());
    }
    for show in rows.shows {
        let (Some(key), Ok(season), Ok(episode)) = (
            index.show_key(show.show_id),
            u32::try_from(show.season),
            u32::try_from(show.episode),
        ) else {
            continue;
        };
        let position = Position {
            season,
            episode,
            finished: show.finished,
        };
        progress.shows.insert(key.clone(), position);
    }
    progress.watching = Watching::from_entries(
        rows.started
            .into_iter()
            .filter_map(|entry| {
                Some(Entry {
                    kind: index::watching_kind(entry.kind),
                    key: index.title_key(entry.kind, entry.title_id)?.clone(),
                    last_played: u64::try_from(entry.last_played).ok()?,
                })
            })
            .collect(),
    );
    progress.dismissed = rows
        .dismissed
        .into_iter()
        .filter_map(|dismissed| {
            Some(Dismissal {
                file: index.path(dismissed.video)?.clone(),
                at: u64::try_from(dismissed.at).unwrap_or_default(),
            })
        })
        .collect();
    progress
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::library::{Episode, Library, Movie, Season, Show as LibraryShow};
    use crate::progress::Place;
    use crate::watching::Kind;
    use std::path::{Path, PathBuf};

    pub const DARK: &str = "/lib/Series/Dark/S01/Dark.S01E02.mkv";
    pub const UP: &str = "/lib/Movies/Up/Up.mkv";

    pub fn library() -> Library {
        Library {
            shows: vec![LibraryShow {
                id: 7,
                key: "dark".to_string(),
                title: "Dark".to_string(),
                seasons: vec![Season {
                    number: 1,
                    episodes: vec![Episode {
                        id: 70,
                        season: 1,
                        number: 2,
                        path: PathBuf::from(DARK),
                        ..Episode::default()
                    }],
                }],
                ..LibraryShow::default()
            }],
            movies: vec![Movie {
                id: 9,
                key: "up".to_string(),
                title: "Up".to_string(),
                path: PathBuf::from(UP),
                ..Movie::default()
            }],
        }
    }

    async fn db() -> Db {
        let db = Db::memory().await;
        db.set_library(&library());
        db
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

    #[tokio::test]
    async fn progress_survives_a_save_and_a_load() {
        let db = db().await;
        let progress = sample();
        db.save_progress(ProfileId::FIRST, &progress).await.unwrap();
        assert_eq!(db.progress(ProfileId::FIRST).await.unwrap(), progress);
    }

    #[tokio::test]
    async fn a_save_keeps_only_what_is_left() {
        let db = db().await;
        let mut progress = sample();
        db.save_progress(ProfileId::FIRST, &progress).await.unwrap();
        progress.shows.clear();
        progress.watching.leave(Kind::Movie, "up");
        progress.dismissed.clear();
        progress.finish(Path::new(DARK));
        db.save_progress(ProfileId::FIRST, &progress).await.unwrap();
        let loaded = db.progress(ProfileId::FIRST).await.unwrap();
        assert_eq!(loaded, progress);
        assert_eq!(loaded.watching.entries().len(), 1);
    }

    #[tokio::test]
    async fn places_are_kept_per_profile_and_what_a_file_is_like_is_shared() {
        let db = db().await;
        db.save_progress(ProfileId::FIRST, &sample()).await.unwrap();
        let other = db.progress(ProfileId(2)).await.unwrap();
        assert!(other.shows.is_empty());
        assert!(other.watching.entries().is_empty());
        assert!(other.dismissed.is_empty());
        let dark = other.files[Path::new(DARK)];
        assert_eq!(dark.stopped_at(), None);
        assert_eq!(dark.tracks(), Tracks::default());
        assert_eq!(dark.duration, Some(2700.0));
        assert_eq!(dark.credits, Some(2600.0));
        assert!(!other.files[Path::new(UP)].finished());
    }

    #[tokio::test]
    async fn a_renamed_file_and_show_keep_their_progress() {
        let db = db().await;
        db.save_progress(ProfileId::FIRST, &sample()).await.unwrap();
        let mut renamed = library();
        renamed.shows[0].key = "dark-2017".to_string();
        let moved = PathBuf::from("/lib/Series/Dark (2017)/Season 1/Dark.S01E02.mkv");
        renamed.shows[0].seasons[0].episodes[0].path = moved.clone();
        db.set_library(&renamed);
        let progress = db.progress(ProfileId::FIRST).await.unwrap();
        assert_eq!(progress.files[&moved].stopped_at(), Some(600.0));
        assert_eq!(progress.shows["dark-2017"].episode, 2);
        assert_eq!(progress.watching.last_played(Kind::Series, "dark-2017"), Some(1_800_000_100));
        assert!(!progress.files.contains_key(Path::new(DARK)));
    }

    #[tokio::test]
    async fn progress_read_before_the_library_is_known_never_deletes_anything() {
        let file = std::env::temp_dir().join(format!("semmeltv-blind-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&file);
        let db = Db::open(&file).await.unwrap();
        db.set_library(&library());
        db.save_progress(ProfileId::FIRST, &sample()).await.unwrap();
        drop(db);
        let blind = Db::open(&file).await.unwrap();
        let empty = blind.progress(ProfileId::FIRST).await.unwrap();
        assert_eq!(empty, Progress::default());
        blind.set_library(&library());
        blind.save_progress(ProfileId::FIRST, &empty).await.unwrap();
        assert_eq!(blind.progress(ProfileId::FIRST).await.unwrap(), sample());
        let _ = std::fs::remove_file(&file);
    }

    #[tokio::test]
    async fn a_title_played_again_after_finishing_is_stored_in_progress_with_its_tracks() {
        let db = db().await;
        let mut progress = sample();
        progress.finish(Path::new(DARK));
        db.save_progress(ProfileId::FIRST, &progress).await.unwrap();
        assert!(db.progress(ProfileId::FIRST).await.unwrap().files[Path::new(DARK)].finished());
        progress.record(
            Path::new(DARK),
            Place {
                position: 1234.0,
                duration: 2700.0,
                credits: None,
            },
        );
        let tracks = Tracks {
            audio: Some(2),
            subtitle: None,
            subtitles_shown: Some(false),
        };
        progress.keep_tracks(Path::new(DARK), tracks);
        db.save_progress(ProfileId::FIRST, &progress).await.unwrap();
        let again = db.progress(ProfileId::FIRST).await.unwrap().files[Path::new(DARK)];
        assert_eq!(again.viewing, Viewing::InProgress { at: 1234.0, tracks });
        assert_eq!(again.resume_at(), 1234.0);
    }

    #[tokio::test]
    async fn files_the_library_does_not_know_are_left_out() {
        let db = db().await;
        let mut progress = sample();
        progress.record(
            Path::new("/lib/Movies/Gone.mkv"),
            Place {
                position: 10.0,
                duration: 100.0,
                credits: None,
            },
        );
        db.save_progress(ProfileId::FIRST, &progress).await.unwrap();
        assert_eq!(db.progress(ProfileId::FIRST).await.unwrap(), sample());
    }
}
