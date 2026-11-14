mod auth;
mod diesel;
pub mod import;
mod models;
mod progress;

use crate::profile::ProfileId;
use crate::subtitles::{self, Remembered};
use models::{AudioPrefs as StoredAudioPrefs, File, Setting as StoredSetting, SubtitleSettings};
use std::collections::BTreeMap;
use std::fs;
use std::future::Future;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::Duration;
use tokio::runtime::Runtime;
use toasty_driver_sqlite::{JournalMode, Sqlite, Synchronous};

static MIGRATIONS: toasty::migration::MigrationSet = toasty::embed_migrations!("toasty");
const TV_TOKEN: &str = "tv_token";

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone)]
pub struct Db {
    inner: Arc<Inner>,
}

struct Inner {
    db: toasty::Db,
    runtime: Option<Runtime>,
    library: PathBuf,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
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

fn runtime() -> Result<Runtime> {
    Ok(tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name("semmeltv-db")
        .enable_all()
        .build()?)
}

fn wait<T: Send + 'static>(
    runtime: &Runtime,
    work: impl Future<Output = toasty::Result<T>> + Send + 'static,
) -> Result<T> {
    let (done, result) = mpsc::sync_channel(1);
    runtime.spawn(async move {
        let _ = done.send(work.await);
    });
    Ok(result.recv().map_err(|_| "the database task stopped")??)
}

fn private(file: &Path) -> Result<()> {
    for suffix in ["", "-wal", "-shm"] {
        let mut name = file.as_os_str().to_owned();
        name.push(suffix);
        let path = PathBuf::from(name);
        if path.exists() {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
    }
    Ok(())
}

impl Db {
    pub fn open(file: &Path, library: &Path) -> Result<Self> {
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        let earlier = diesel::set_aside(file)?;
        let runtime = runtime()?;
        fs::OpenOptions::new().create(true).append(true).mode(0o600).open(file)?;
        let driver = Sqlite::open(file)
            .journal_mode(JournalMode::Wal)
            .synchronous(Synchronous::Normal)
            .busy_timeout(Duration::from_secs(5));
        let db = Self::ready(runtime, driver, library)?;
        private(file)?;
        if let Some(earlier) = earlier {
            db.wait(move |db| diesel::copy(db, earlier))?;
        }
        Ok(db)
    }

    #[cfg(test)]
    pub fn memory(library: &Path) -> Self {
        Self::ready(runtime().unwrap(), Sqlite::in_memory(), library).unwrap()
    }

    fn ready(runtime: Runtime, driver: Sqlite, library: &Path) -> Result<Self> {
        let db = wait(&runtime, async move {
            let db = toasty::Db::builder().models(models::all()).build(driver).await?;
            MIGRATIONS.apply(&db).await?;
            Ok(db)
        })?;
        Ok(Self {
            inner: Arc::new(Inner {
                db,
                runtime: Some(runtime),
                library: library.to_path_buf(),
            }),
        })
    }

    fn wait<T, F>(&self, work: impl FnOnce(toasty::Db) -> F) -> Result<T>
    where
        T: Send + 'static,
        F: Future<Output = toasty::Result<T>> + Send + 'static,
    {
        let runtime = self.inner.runtime.as_ref().ok_or("the database is closed")?;
        wait(runtime, work(self.inner.db.clone()))
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
        let name = name.to_string();
        self.wait(|mut db| async move {
            let found = StoredSetting::filter_by_name(name).first().exec(&mut db).await?;
            Ok(found.map(|setting| setting.value))
        })
    }

    pub fn set_setting(&self, name: &str, value: Option<&str>) -> Result<()> {
        let (name, value) = (name.to_string(), value.map(str::to_string));
        self.wait(|mut db| async move {
            match value {
                Some(value) => {
                    StoredSetting::upsert_by_name(name).value(value).exec(&mut db).await?;
                }
                None => StoredSetting::filter_by_name(name).delete().exec(&mut db).await?,
            }
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
        self.wait(move |mut db| async move {
            let stored = SubtitleSettings::filter_by_profile_id(profile.0)
                .first()
                .exec(&mut db)
                .await?;
            Ok(stored.map(|stored| stored.settings.0))
        })
    }

    pub fn save_subtitle_settings(&self, profile: ProfileId, value: &subtitles::Settings) -> Result<()> {
        let value = value.clone();
        self.wait(move |mut db| async move {
            SubtitleSettings::upsert_by_profile_id(profile.0)
                .settings(value)
                .exec(&mut db)
                .await?;
            Ok(())
        })
    }

    pub fn subtitle_files(&self) -> Result<BTreeMap<PathBuf, Remembered>> {
        let rows = self.wait(|mut db| async move {
            let fields = File::fields();
            File::filter(fields.subtitle_delay().ne(0.0).or(fields.embedded_subtitles().is_some()))
                .exec(&mut db)
                .await
        })?;
        Ok(rows
            .into_iter()
            .map(|file| {
                let remembered = Remembered {
                    delay: file.subtitle_delay,
                    embedded: file.embedded_subtitles.and_then(|count| usize::try_from(count).ok()),
                };
                (self.path(&file.path), remembered)
            })
            .collect())
    }

    pub fn save_subtitle_file(&self, file: &Path, remembered: Remembered) -> Result<()> {
        let key = self.key(file);
        let embedded = remembered.embedded.map(|count| count as i64);
        self.wait(move |mut db| async move {
            File::upsert_by_path(key)
                .subtitle_delay(remembered.delay)
                .embedded_subtitles(embedded)
                .exec(&mut db)
                .await?;
            Ok(())
        })
    }

    pub fn audio_prefs(&self, profile: ProfileId) -> Result<AudioPrefs> {
        let stored = self.wait(move |mut db| async move {
            StoredAudioPrefs::filter_by_profile_id(profile.0).first().exec(&mut db).await
        })?;
        Ok(stored.map_or_else(AudioPrefs::default, |stored| AudioPrefs {
            output: match stored.output {
                models::Output::Tv => Output::Tv,
                models::Output::Headphones => Output::Headphones,
            },
            headphones: stored.headphones_address,
        }))
    }

    pub fn save_audio_prefs(&self, profile: ProfileId, prefs: &AudioPrefs) -> Result<()> {
        let output = match prefs.output {
            Output::Tv => models::Output::Tv,
            Output::Headphones => models::Output::Headphones,
        };
        let headphones = prefs.headphones.clone();
        self.wait(move |mut db| async move {
            if models::Profile::filter_by_id(profile.0).first().exec(&mut db).await?.is_none() {
                return Err(toasty::Error::record_not_found(format!("profile {}", profile.0)));
            }
            StoredAudioPrefs::upsert_by_profile_id(profile.0)
                .output(output)
                .headphones_address(headphones)
                .exec(&mut db)
                .await?;
            Ok(())
        })
    }
}

pub fn toasty_cli(file: &Path, arguments: Vec<String>) -> Result<()> {
    let runtime = runtime()?;
    let file = file.to_path_buf();
    wait(&runtime, async move {
        let db = toasty::Db::builder().models(models::all()).build(Sqlite::open(file)).await?;
        let config = toasty_cli::Config::load().map_err(|error| toasty::Error::from_args(format_args!("{error:#}")))?;
        toasty_cli::ToastyCli::with_config(db, config)
            .parse_from(std::iter::once("toasty".to_string()).chain(arguments))
            .await
            .map_err(|error| toasty::Error::from_args(format_args!("{error:#}")))
    })
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
        let profiles = db()
            .wait(|mut db| async move {
                models::Profile::all()
                    .order_by(models::Profile::fields().id().asc())
                    .exec(&mut db)
                    .await
            })
            .unwrap();
        let seen: Vec<_> = profiles
            .iter()
            .map(|profile| (profile.id, profile.name.as_str(), profile.role, profile.password_hash.is_none()))
            .collect();
        assert_eq!(
            seen,
            [
                (1, "Frederik", models::Role::Admin, true),
                (2, "Mami", models::Role::Viewer, true),
                (3, "Miriam", models::Role::Viewer, true),
                (4, "Ramu", models::Role::Viewer, true),
            ]
        );
    }

    #[test]
    fn deleting_a_profile_takes_its_state_along_and_leaves_the_others() {
        use crate::auth::store::{AuthStore, Invite};
        use crate::progress::{Place, Progress};
        let db = db();
        let watched = |db: &Db, profile| {
            let mut progress = Progress::default();
            let file = Path::new("/lib/Movies/Up/Up.mkv");
            progress.record(file, Place { position: 60.0, duration: 5760.0, credits: None });
            progress.watching.played(crate::watching::Kind::Movie, "up", 1);
            progress.shows.insert("dark".to_string(), crate::progress::Position { season: 1, episode: 1, finished: false });
            db.save_progress(profile, &progress).unwrap();
            db.save_subtitle_settings(profile, &subtitles::Settings::default()).unwrap();
            db.save_audio_prefs(profile, &AudioPrefs { output: Output::Headphones, headphones: None }).unwrap();
        };
        watched(&db, ProfileId(3));
        watched(&db, ProfileId(4));
        let invite = Invite {
            token_hash: vec![9],
            profile: ProfileId(2),
            created_by: Some(ProfileId(4)),
            created_at: 1,
            expires_at: 2,
            used_at: None,
        };
        db.create_invite(&invite).unwrap();

        assert!(db.delete_profile(ProfileId(4)).unwrap());

        let gone = db.progress(ProfileId(4)).unwrap();
        assert!(gone.shows.is_empty() && gone.watching.entries().is_empty());
        assert_eq!(gone.files[Path::new("/lib/Movies/Up/Up.mkv")].stopped_at, None);
        assert_eq!(gone.files[Path::new("/lib/Movies/Up/Up.mkv")].duration, Some(5760.0));
        assert_eq!(db.subtitle_settings(ProfileId(4)).unwrap(), None);
        assert_eq!(db.audio_prefs(ProfileId(4)).unwrap(), AudioPrefs::default());
        assert_eq!(db.invite(&[9]).unwrap().unwrap().created_by, None);
        let kept = db.progress(ProfileId(3)).unwrap();
        assert_eq!(kept.files[Path::new("/lib/Movies/Up/Up.mkv")].stopped_at, Some(60.0));
        assert_eq!(db.audio_prefs(ProfileId(3)).unwrap().output, Output::Headphones);
        assert!(!db.delete_profile(ProfileId(4)).unwrap());
    }

    #[test]
    fn prefs_for_a_profile_that_does_not_exist_are_refused() {
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
    fn the_file_is_the_apps_alone_and_keeps_what_was_saved() {
        let dir = std::env::temp_dir().join(format!("semmeltv-db-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let file = dir.join("state/semmeltv.db");
        let db = Db::open(&file, Path::new("/lib")).unwrap();
        db.set_setting("x", Some("1")).unwrap();
        drop(db);
        let mode = fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let again = Db::open(&file, Path::new("/lib")).unwrap();
        assert_eq!(again.setting("x").unwrap().as_deref(), Some("1"));
        fs::remove_dir_all(dir).unwrap();
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
        let stored = db
            .wait(|mut db| async move { File::all().exec(&mut db).await })
            .unwrap();
        assert_eq!(stored[0].path, "Movies/Up/Up.mkv");
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
