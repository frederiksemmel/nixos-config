use super::{Db, Result};
use crate::profile::ProfileId;
use crate::progress::{Progress, Tracks};
use crate::subtitles::{Remembered, Settings};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const DONE: &str = "imported_cache";
const PROGRESS: &str = "progress.json";
const SUBTITLES: &str = "subtitles.json";
const SUBTITLE_FILES: &str = "subtitle-files.json";
const TV_TOKEN: &str = "tv-token";
const WATCH_LATER: &str = "watch-later";
const MOVED: &str = "before-database";

pub fn from_cache(db: &Db, cache: &Path, profile: ProfileId, now: u64) -> Result<()> {
    if db.setting(DONE)?.is_some() {
        return Ok(());
    }
    let mut imported = Vec::new();
    let mut progress: Progress = match read(&cache.join(PROGRESS))? {
        Some(bytes) => {
            imported.push(PROGRESS);
            serde_json::from_slice(&bytes)?
        }
        None => Progress::default(),
    };
    let saved = watch_later(&cache.join(WATCH_LATER))?;
    if !saved.is_empty() {
        imported.push(WATCH_LATER);
    }
    for (file, (start, tracks)) in saved {
        let Some(watched) = progress.files.get_mut(&file) else {
            continue;
        };
        if watched.finished {
            continue;
        }
        watched.tracks = tracks;
        if watched.stopped_at.is_none() {
            watched.stopped_at = start;
        }
    }
    db.save_progress(profile, &progress)?;
    if let Some(bytes) = read(&cache.join(SUBTITLES))? {
        let settings: Settings = serde_json::from_slice(&bytes)?;
        db.save_subtitle_settings(profile, &settings)?;
        imported.push(SUBTITLES);
    }
    if let Some(bytes) = read(&cache.join(SUBTITLE_FILES))? {
        let files: BTreeMap<PathBuf, Remembered> = serde_json::from_slice(&bytes)?;
        for (file, remembered) in files {
            db.save_subtitle_file(&file, remembered)?;
        }
        imported.push(SUBTITLE_FILES);
    }
    if let Some(bytes) = read(&cache.join(TV_TOKEN))? {
        let token = String::from_utf8_lossy(&bytes).trim().to_string();
        if !token.is_empty() {
            db.tv_token().set(&token)?;
        }
        imported.push(TV_TOKEN);
    }
    db.set_setting(DONE, Some(&now.to_string()))?;
    let moved = cache.join(MOVED);
    for name in &imported {
        fs::create_dir_all(&moved)?;
        fs::rename(cache.join(name), moved.join(name))?;
    }
    if !imported.is_empty() {
        eprintln!(
            "database: imported {} from {}: {} shows, {} files, {} in Currently watching; the old files are in {}",
            imported.join(", "),
            cache.display(),
            progress.shows.len(),
            progress.files.len(),
            progress.watching.entries().len(),
            moved.display(),
        );
    }
    Ok(())
}

fn read(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn watch_later(root: &Path) -> io::Result<BTreeMap<PathBuf, (Option<f64>, Tracks)>> {
    let mut found = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
                continue;
            }
            let Ok(relative) = dir.strip_prefix(root) else {
                continue;
            };
            let file = Path::new("/").join(relative);
            found.insert(file, saved(&fs::read_to_string(entry.path())?));
        }
    }
    Ok(found)
}

fn saved(text: &str) -> (Option<f64>, Tracks) {
    let mut start = None;
    let mut tracks = Tracks::default();
    for line in text.lines() {
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        match name {
            "start" => start = value.parse().ok(),
            "aid" => tracks.audio = value.parse().ok(),
            "sid" => tracks.subtitle = value.parse().ok(),
            "sub-visibility" => tracks.subtitles_shown = Some(value == "yes"),
            _ => {}
        }
    }
    (start, tracks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::watching::Kind;

    const PROGRESS_JSON: &str = r#"{
      "shows": { "dark": { "season": 1, "episode": 2, "finished": false } },
      "files": {
        "/lib/Series/Dark/S01/Dark.S01E02.mkv": { "stopped_at": 600.5, "duration": 2700.0, "last_played": 1800000100, "finished": false },
        "/lib/Movies/Up/Up.mkv": { "stopped_at": null, "duration": 5760.0, "last_played": 1800000000, "finished": true, "credits": 5600.0 }
      },
      "watching": [
        { "kind": "series", "key": "dark", "last_played": 1800000100 },
        { "kind": "movie", "key": "up", "last_played": 1800000000 }
      ],
      "dismissed": [ { "file": "/lib/Movies/Up/Up.mkv", "at": 1800000050 } ]
    }"#;

    fn cache() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("semmeltv-import-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_old_files_move_into_the_database_once() {
        let cache = cache();
        fs::write(cache.join(PROGRESS), PROGRESS_JSON).unwrap();
        fs::write(cache.join(SUBTITLES), serde_json::to_vec(&Settings { size: 0.95, ..Settings::default() }).unwrap()).unwrap();
        fs::write(cache.join(SUBTITLE_FILES), r#"{ "/lib/Movies/Up/Up.mkv": { "delay": 0.5, "embedded": 2 } }"#).unwrap();
        fs::write(cache.join(TV_TOKEN), "12345678").unwrap();
        let entry = cache.join("watch-later/lib/Series/Dark/S01/Dark.S01E02.mkv");
        fs::create_dir_all(&entry).unwrap();
        fs::write(entry.join("B6AA78104D2DEF066AE6086929D07CA2"), "start=600.000000\naid=2\nsid=3\nsub-visibility=yes\n").unwrap();
        let db = Db::memory(Path::new("/lib"));

        from_cache(&db, &cache, ProfileId::FIRST, 1_800_000_200).unwrap();

        let progress = db.progress(ProfileId::FIRST).unwrap();
        let dark = progress.files[Path::new("/lib/Series/Dark/S01/Dark.S01E02.mkv")];
        assert_eq!(dark.stopped_at, Some(600.5));
        assert_eq!(
            dark.tracks,
            Tracks {
                audio: Some(2),
                subtitle: Some(3),
                subtitles_shown: Some(true),
            }
        );
        let up = progress.files[Path::new("/lib/Movies/Up/Up.mkv")];
        assert!(up.finished);
        assert_eq!(up.credits, Some(5600.0));
        assert_eq!(progress.shows["dark"].episode, 2);
        assert_eq!(progress.watching.last_played(Kind::Series, "dark"), Some(1_800_000_100));
        assert_eq!(progress.watching.entries()[0].key, "dark");
        assert_eq!(progress.dismissed.len(), 1);
        assert_eq!(db.subtitle_settings(ProfileId::FIRST).unwrap().unwrap().size, 0.95);
        assert_eq!(db.subtitle_files().unwrap()[Path::new("/lib/Movies/Up/Up.mkv")].delay, 0.5);
        assert_eq!(db.tv_token().get().as_deref(), Some("12345678"));
        assert!(!cache.join(PROGRESS).exists());
        assert!(!cache.join(WATCH_LATER).exists());
        assert!(cache.join(MOVED).join(PROGRESS).exists());

        fs::write(cache.join(PROGRESS), "{ not json").unwrap();
        from_cache(&db, &cache, ProfileId::FIRST, 1_800_000_300).unwrap();
        assert_eq!(db.progress(ProfileId::FIRST).unwrap(), progress);
        fs::remove_dir_all(cache).unwrap();
    }

    #[test]
    fn a_cache_with_nothing_in_it_imports_nothing() {
        let cache = cache();
        let db = Db::memory(Path::new("/lib"));
        from_cache(&db, &cache, ProfileId::FIRST, 1).unwrap();
        assert_eq!(db.progress(ProfileId::FIRST).unwrap(), Progress::default());
        assert!(!cache.join(MOVED).exists());
        fs::remove_dir_all(cache).unwrap();
    }
}
