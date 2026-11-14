pub mod import;
mod progress;
pub mod schema;

use crate::profile::ProfileId;
use crate::subtitles::{self, Remembered};
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use schema::{audio_prefs, files, settings, subtitle_settings};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations");
const TV_TOKEN: &str = "tv_token";

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone)]
pub struct Db {
    inner: Arc<Inner>,
}

struct Inner {
    connection: Mutex<SqliteConnection>,
    library: PathBuf,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Output {
    #[default]
    Tv,
    Headphones,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioPrefs {
    pub output: Output,
    pub headphones: Option<String>,
}

impl Db {
    pub fn open(file: &Path, library: &Path) -> Result<Self> {
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        let url = file.to_str().ok_or("the database path is not UTF-8")?;
        let mut connection = SqliteConnection::establish(url)?;
        connection.batch_execute("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
        Self::ready(connection, library)
    }

    #[cfg(test)]
    pub fn memory(library: &Path) -> Self {
        let connection = SqliteConnection::establish(":memory:").unwrap();
        Self::ready(connection, library).unwrap()
    }

    fn ready(mut connection: SqliteConnection, library: &Path) -> Result<Self> {
        connection.batch_execute("PRAGMA busy_timeout = 5000; PRAGMA foreign_keys = ON;")?;
        connection.run_pending_migrations(MIGRATIONS)?;
        Ok(Self {
            inner: Arc::new(Inner {
                connection: Mutex::new(connection),
                library: library.to_path_buf(),
            }),
        })
    }

    fn run<T>(&self, work: impl FnOnce(&mut SqliteConnection) -> QueryResult<T>) -> Result<T> {
        let mut connection = self.inner.connection.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(work(&mut connection)?)
    }

    fn key(&self, file: &Path) -> String {
        file.strip_prefix(&self.inner.library)
            .unwrap_or(file)
            .to_string_lossy()
            .into_owned()
    }

    fn path(&self, key: &str) -> PathBuf {
        self.inner.library.join(key)
    }

    pub fn setting(&self, name: &str) -> Result<Option<String>> {
        self.run(|connection| {
            settings::table
                .find(name)
                .select(settings::value)
                .first(connection)
                .optional()
        })
    }

    pub fn set_setting(&self, name: &str, value: Option<&str>) -> Result<()> {
        self.run(|connection| {
            match value {
                Some(value) => diesel::insert_into(settings::table)
                    .values((settings::key.eq(name), settings::value.eq(value)))
                    .on_conflict(settings::key)
                    .do_update()
                    .set(settings::value.eq(value))
                    .execute(connection)?,
                None => diesel::delete(settings::table.find(name)).execute(connection)?,
            };
            Ok(())
        })
    }

    pub fn tv_token(&self) -> Setting {
        Setting {
            db: self.clone(),
            name: TV_TOKEN,
        }
    }

    pub fn subtitle_settings(&self, profile: ProfileId) -> Result<Option<subtitles::Settings>> {
        let stored: Option<String> = self.run(|connection| {
            subtitle_settings::table
                .find(profile.0)
                .select(subtitle_settings::settings)
                .first(connection)
                .optional()
        })?;
        Ok(stored.map(|text| serde_json::from_str(&text)).transpose()?)
    }

    pub fn save_subtitle_settings(&self, profile: ProfileId, value: &subtitles::Settings) -> Result<()> {
        let text = serde_json::to_string(value)?;
        self.run(|connection| {
            diesel::insert_into(subtitle_settings::table)
                .values((
                    subtitle_settings::profile_id.eq(profile.0),
                    subtitle_settings::settings.eq(&text),
                ))
                .on_conflict(subtitle_settings::profile_id)
                .do_update()
                .set(subtitle_settings::settings.eq(&text))
                .execute(connection)
                .map(drop)
        })
    }

    pub fn subtitle_files(&self) -> Result<BTreeMap<PathBuf, Remembered>> {
        let rows: Vec<(String, f64, Option<i64>)> = self.run(|connection| {
            files::table
                .filter(files::subtitle_delay.ne(0.0).or(files::embedded_subtitles.is_not_null()))
                .select((files::path, files::subtitle_delay, files::embedded_subtitles))
                .load(connection)
        })?;
        Ok(rows
            .into_iter()
            .map(|(key, delay, embedded)| {
                let remembered = Remembered {
                    delay,
                    embedded: embedded.and_then(|count| usize::try_from(count).ok()),
                };
                (self.path(&key), remembered)
            })
            .collect())
    }

    pub fn save_subtitle_file(&self, file: &Path, remembered: Remembered) -> Result<()> {
        let key = self.key(file);
        let embedded = remembered.embedded.map(|count| count as i64);
        self.run(|connection| {
            diesel::insert_into(files::table)
                .values((
                    files::path.eq(&key),
                    files::subtitle_delay.eq(remembered.delay),
                    files::embedded_subtitles.eq(embedded),
                ))
                .on_conflict(files::path)
                .do_update()
                .set((
                    files::subtitle_delay.eq(remembered.delay),
                    files::embedded_subtitles.eq(embedded),
                ))
                .execute(connection)
                .map(drop)
        })
    }

    pub fn audio_prefs(&self, profile: ProfileId) -> Result<AudioPrefs> {
        let stored: Option<(String, Option<String>)> = self.run(|connection| {
            audio_prefs::table
                .find(profile.0)
                .select((audio_prefs::output, audio_prefs::headphones_address))
                .first(connection)
                .optional()
        })?;
        Ok(stored.map_or_else(AudioPrefs::default, |(output, headphones)| AudioPrefs {
            output: if output == "headphones" {
                Output::Headphones
            } else {
                Output::Tv
            },
            headphones,
        }))
    }

    pub fn save_audio_prefs(&self, profile: ProfileId, prefs: &AudioPrefs) -> Result<()> {
        let output = match prefs.output {
            Output::Tv => "tv",
            Output::Headphones => "headphones",
        };
        self.run(|connection| {
            diesel::insert_into(audio_prefs::table)
                .values((
                    audio_prefs::profile_id.eq(profile.0),
                    audio_prefs::output.eq(output),
                    audio_prefs::headphones_address.eq(prefs.headphones.as_deref()),
                ))
                .on_conflict(audio_prefs::profile_id)
                .do_update()
                .set((
                    audio_prefs::output.eq(output),
                    audio_prefs::headphones_address.eq(prefs.headphones.as_deref()),
                ))
                .execute(connection)
                .map(drop)
        })
    }
}

#[derive(Clone)]
pub struct Setting {
    db: Db,
    name: &'static str,
}

impl Setting {
    pub fn get(&self) -> Option<String> {
        self.db
            .setting(self.name)
            .unwrap_or_else(|error| {
                eprintln!("database: {}: {error}", self.name);
                None
            })
            .filter(|value| !value.trim().is_empty())
    }

    pub fn set(&self, value: &str) -> Result<()> {
        self.db.set_setting(self.name, Some(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Db {
        Db::memory(Path::new("/lib"))
    }

    #[test]
    fn a_new_database_has_the_four_profiles_and_no_passwords() {
        let names: Vec<(i64, String, String, Option<String>)> = db()
            .run(|connection| {
                schema::profiles::table
                    .order(schema::profiles::id)
                    .select((
                        schema::profiles::id,
                        schema::profiles::name,
                        schema::profiles::role,
                        schema::profiles::password_hash,
                    ))
                    .load(connection)
            })
            .unwrap();
        assert_eq!(
            names,
            [
                (1, "Frederik".to_string(), "admin".to_string(), None),
                (2, "Mami".to_string(), "viewer".to_string(), None),
                (3, "Miriam".to_string(), "viewer".to_string(), None),
                (4, "Ramu".to_string(), "viewer".to_string(), None),
            ]
        );
    }

    #[test]
    fn foreign_keys_hold() {
        let db = db();
        let orphan = db.save_audio_prefs(ProfileId(99), &AudioPrefs::default());
        assert!(orphan.is_err());
    }

    #[test]
    fn a_setting_is_kept_replaced_and_forgotten() {
        let db = db();
        assert_eq!(db.setting("x").unwrap(), None);
        db.set_setting("x", Some("1")).unwrap();
        db.set_setting("x", Some("2")).unwrap();
        assert_eq!(db.setting("x").unwrap().as_deref(), Some("2"));
        db.set_setting("x", None).unwrap();
        assert_eq!(db.setting("x").unwrap(), None);
    }

    #[test]
    fn the_tv_token_is_one_for_the_whole_tv() {
        let db = db();
        assert_eq!(db.tv_token().get(), None);
        db.tv_token().set("12345678").unwrap();
        assert_eq!(db.tv_token().get().as_deref(), Some("12345678"));
    }

    #[test]
    fn subtitle_settings_are_kept_per_profile() {
        let db = db();
        let larger = subtitles::Settings {
            size: 1.4,
            ..subtitles::Settings::default()
        };
        db.save_subtitle_settings(ProfileId(2), &larger).unwrap();
        assert_eq!(db.subtitle_settings(ProfileId(2)).unwrap(), Some(larger));
        assert_eq!(db.subtitle_settings(ProfileId(1)).unwrap(), None);
    }

    #[test]
    fn a_files_subtitle_delay_is_kept_by_its_path_in_the_library() {
        let db = db();
        let file = Path::new("/lib/Movies/Up/Up.mkv");
        let remembered = Remembered {
            delay: 0.5,
            embedded: Some(2),
        };
        db.save_subtitle_file(file, remembered).unwrap();
        assert_eq!(db.subtitle_files().unwrap(), BTreeMap::from([(file.to_path_buf(), remembered)]));
        let stored: String = db.run(|connection| files::table.select(files::path).first(connection)).unwrap();
        assert_eq!(stored, "Movies/Up/Up.mkv");
        db.save_subtitle_file(file, Remembered::default()).unwrap();
        assert!(db.subtitle_files().unwrap().is_empty());
    }

    #[test]
    fn audio_prefs_default_to_the_tv() {
        let db = db();
        assert_eq!(db.audio_prefs(ProfileId(3)).unwrap(), AudioPrefs::default());
        let prefs = AudioPrefs {
            output: Output::Headphones,
            headphones: Some("AA:BB:CC:DD:EE:FF".to_string()),
        };
        db.save_audio_prefs(ProfileId(3), &prefs).unwrap();
        assert_eq!(db.audio_prefs(ProfileId(3)).unwrap(), prefs);
    }
}
