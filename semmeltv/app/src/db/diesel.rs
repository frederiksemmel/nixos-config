use super::Result;
use std::fs;
use std::path::{Path, PathBuf};
use toasty::sql::statement;

const COPY: &[&str] = &[
    "DELETE FROM profiles",
    "INSERT INTO profiles (id, name, role, password_hash, color, created_at)
        SELECT id, name, role, password_hash, color, created_at FROM earlier.profiles",
    "INSERT INTO passkeys (id, profile_id, credential_id, rp_id, name, passkey, created_at, last_used_at)
        SELECT id, profile_id, credential_id, rp_id, name, passkey, created_at, last_used_at FROM earlier.passkeys",
    "INSERT INTO sessions (token_hash, profile_id, created_at, last_seen_at, expires_at, label)
        SELECT token_hash, profile_id, created_at, last_seen_at, expires_at, COALESCE(label, '') FROM earlier.sessions",
    "INSERT INTO invites (token_hash, profile_id, created_by, created_at, expires_at, used_at)
        SELECT token_hash, profile_id, created_by, created_at, expires_at, used_at FROM earlier.invites",
    "INSERT INTO files (path, duration, credits, subtitle_delay, embedded_subtitles)
        SELECT path, duration, credits, subtitle_delay, embedded_subtitles FROM earlier.files",
    "INSERT INTO file_places (profile_id, path, stopped_at, last_played, finished, audio_track, subtitle_track, subtitles_shown)
        SELECT profile_id, path, stopped_at, last_played, finished, audio_track, subtitle_track, subtitles_shown FROM earlier.file_places",
    "INSERT INTO show_places (profile_id, show_key, season, episode, finished)
        SELECT profile_id, show_key, season, episode, finished FROM earlier.show_places",
    "INSERT INTO watching (profile_id, kind, title_key, last_played)
        SELECT profile_id, kind, key, last_played FROM earlier.watching",
    "INSERT INTO dismissed (profile_id, path, at)
        SELECT profile_id, path, at FROM earlier.dismissed",
    "INSERT INTO subtitle_settings (profile_id, settings)
        SELECT profile_id, settings FROM earlier.subtitle_settings",
    "INSERT INTO audio_prefs (profile_id, output, headphones_address)
        SELECT profile_id, output, headphones_address FROM earlier.audio_prefs",
    "INSERT INTO settings (name, value)
        SELECT key, value FROM earlier.settings",
];

fn beside(file: &Path, suffix: &str) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

pub fn set_aside(file: &Path) -> Result<Option<PathBuf>> {
    if !file.exists() {
        return Ok(None);
    }
    let connection = rusqlite::Connection::open(file)?;
    let diesel: bool = connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name = '__diesel_schema_migrations')",
        [],
        |row| row.get(0),
    )?;
    if diesel {
        connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        connection.pragma_update(None, "journal_mode", "DELETE")?;
    }
    connection.close().map_err(|(_, error)| error)?;
    if !diesel {
        return Ok(None);
    }
    let earlier = file.with_extension("diesel.db");
    fs::rename(file, &earlier)?;
    for suffix in ["-wal", "-shm"] {
        let _ = fs::remove_file(beside(file, suffix));
    }
    eprintln!("database: moving the Diesel database to Toasty; the old file is {}", earlier.display());
    Ok(Some(earlier))
}

pub async fn copy(db: toasty::Db, earlier: PathBuf) -> toasty::Result<()> {
    let mut connection = db.connection().await?;
    statement("ATTACH DATABASE ?1 AS earlier")
        .bind(earlier.to_string_lossy().into_owned())
        .exec(&mut connection)
        .await?;
    let mut transaction = connection.transaction().await?;
    for sql in COPY {
        statement(*sql).exec(&mut transaction).await?;
    }
    transaction.commit().await?;
    statement("DETACH DATABASE earlier").exec(&mut connection).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::Db;
    use crate::auth::store::AuthStore;
    use crate::profile::ProfileId;
    use crate::watching::Kind;
    use std::fs;
    use std::path::Path;

    const SCHEMA: &str = include_str!("diesel-schema.sql");

    #[test]
    fn a_diesel_database_moves_into_toasty_with_everything_in_it() {
        let dir = std::env::temp_dir().join(format!("semmeltv-diesel-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("semmeltv.db");
        let old = rusqlite::Connection::open(&file).unwrap();
        old.pragma_update(None, "journal_mode", "WAL").unwrap();
        old.execute_batch(SCHEMA).unwrap();
        old.execute_batch(
            "CREATE TABLE __diesel_schema_migrations (version TEXT PRIMARY KEY, run_on TIMESTAMP);
             INSERT INTO __diesel_schema_migrations VALUES ('0001', '2026-11-14');
             UPDATE profiles SET password_hash = 'argon2-hash', color = '#FF0000' WHERE id = 2;
             INSERT INTO sessions VALUES (x'0102', 2, 10, 20, 30, NULL);
             INSERT INTO passkeys VALUES (7, 1, x'0A0B', 'tv.example.org', 'iPhone', '{}', 5, NULL);
             INSERT INTO invites VALUES (x'0C', 3, 1, 1, 2, NULL);
             INSERT INTO files VALUES ('Movies/Up/Up.mkv', 5760.0, 5600.0, 0.5, 2);
             INSERT INTO file_places VALUES (1, 'Movies/Up/Up.mkv', 600.0, 1800000000, 0, 2, 3, 1);
             INSERT INTO show_places VALUES (1, 'dark', 1, 2, 0);
             INSERT INTO watching VALUES (1, 'movie', 'up', 1800000000);
             INSERT INTO dismissed VALUES (1, 'Movies/Old.mkv', 1700000000);
             INSERT INTO subtitle_settings VALUES (1, '{\"shown\":true,\"languages\":[\"de\"],\"size\":1.2,\"position\":100.0,\"color\":\"#FFFFFF\",\"outline\":1.65,\"outline_color\":\"#000000\",\"shadow\":0.0,\"background\":0.0,\"restyle\":false}');
             INSERT INTO audio_prefs VALUES (2, 'headphones', 'AA:BB');
             INSERT INTO settings VALUES ('tv_token', '12345678');",
        )
        .unwrap();
        drop(old);

        let db = Db::open(&file, Path::new("/lib")).unwrap();

        assert!(dir.join("semmeltv.diesel.db").exists());
        let mami = db.profile(ProfileId(2)).unwrap().unwrap();
        assert_eq!(mami.password_hash.as_deref(), Some("argon2-hash"));
        assert_eq!(mami.color.as_deref(), Some("#FF0000"));
        assert_eq!(db.profiles().unwrap().len(), 4);
        assert_eq!(db.session(&[1, 2]).unwrap().unwrap().label, "");
        assert_eq!(db.passkey(&[10, 11]).unwrap().unwrap().name, "iPhone");
        assert_eq!(db.invite(&[12]).unwrap().unwrap().created_by, Some(ProfileId::FIRST));
        let progress = db.progress(ProfileId::FIRST).unwrap();
        let up = progress.files[Path::new("/lib/Movies/Up/Up.mkv")];
        assert_eq!((up.stopped_at, up.duration, up.tracks.subtitles_shown), (Some(600.0), Some(5760.0), Some(true)));
        assert_eq!(progress.shows["dark"].episode, 2);
        assert_eq!(progress.watching.last_played(Kind::Movie, "up"), Some(1_800_000_000));
        assert_eq!(progress.dismissed.len(), 1);
        assert_eq!(db.subtitle_settings(ProfileId::FIRST).unwrap().unwrap().size, 1.2);
        assert_eq!(db.subtitle_files().unwrap()[Path::new("/lib/Movies/Up/Up.mkv")].delay, 0.5);
        assert_eq!(db.audio_prefs(ProfileId(2)).unwrap().headphones.as_deref(), Some("AA:BB"));
        assert_eq!(db.tv_token().get().as_deref(), Some("12345678"));
        drop(db);

        let again = Db::open(&file, Path::new("/lib")).unwrap();
        assert_eq!(again.profiles().unwrap().len(), 4);
        assert_eq!(again.progress(ProfileId::FIRST).unwrap(), progress);
        fs::remove_dir_all(dir).unwrap();
    }
}
