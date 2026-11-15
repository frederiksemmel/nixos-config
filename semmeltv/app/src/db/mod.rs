mod auth;
mod index;
mod models;
mod progress;

use crate::library::Library;
use crate::profile::ProfileId;
use crate::subtitles::{self, Remembered};
use index::{Index, Video};
use models::{AudioPrefs as StoredAudioPrefs, Setting as StoredSetting, SubtitleSettings, VideoFact};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use toasty_driver_sqlite::{JournalMode, Sqlite, Synchronous};

static MIGRATIONS: toasty::migration::MigrationSet = toasty::embed_migrations!("toasty/tv");

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone)]
pub struct Db {
    db: toasty::Db,
    index: Arc<RwLock<Index>>,
    blind: Arc<Mutex<HashSet<ProfileId>>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Output {
    #[default]
    Tv,
    Headphones,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum AudioPrefs {
    #[default]
    Unpaired,
    Paired {
        address: String,
        output: Output,
    },
}

impl AudioPrefs {
    pub fn headphones(&self) -> Option<&str> {
        match self {
            Self::Unpaired => None,
            Self::Paired { address, .. } => Some(address),
        }
    }

    pub fn output(&self) -> Output {
        match self {
            Self::Unpaired => Output::Tv,
            Self::Paired { output, .. } => *output,
        }
    }

    pub fn choosing(self, output: Output) -> Self {
        match self {
            Self::Unpaired => Self::Unpaired,
            Self::Paired { address, .. } => Self::Paired { address, output },
        }
    }

    pub fn pairing(self, address: String) -> Self {
        let output = self.output();
        Self::Paired { address, output }
    }
}

fn set_aside_unknown(file: &Path) -> Result<()> {
    if !file.exists() || fs::metadata(file)?.len() == 0 {
        return Ok(());
    }
    let known: HashSet<i64> = MIGRATIONS.migrations().iter().map(|migration| migration.id() as i64).collect();
    let applied: Vec<i64> = {
        let connection = rusqlite::Connection::open(file)?;
        let tracked: bool = connection.query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name = '__toasty_migrations')",
            [],
            |row| row.get(0),
        )?;
        if !tracked {
            return Ok(());
        }
        let mut statement = connection.prepare("SELECT id FROM __toasty_migrations")?;
        statement.query_map([], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?
    };
    if applied.iter().all(|id| known.contains(id)) {
        return Ok(());
    }
    for suffix in ["", "-wal", "-shm"] {
        let mut from = file.as_os_str().to_owned();
        from.push(suffix);
        let mut to = file.as_os_str().to_owned();
        to.push(".replaced");
        to.push(suffix);
        if Path::new(&from).exists() {
            fs::rename(&from, &to)?;
        }
    }
    eprintln!(
        "database: {} has a schema this build does not know; it was moved to {}.replaced and a new one started",
        file.display(),
        file.display()
    );
    Ok(())
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
    pub async fn open(file: &Path) -> Result<Self> {
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        set_aside_unknown(file)?;
        fs::OpenOptions::new().create(true).append(true).mode(0o600).open(file)?;
        let driver = Sqlite::open(file)
            .journal_mode(JournalMode::Wal)
            .synchronous(Synchronous::Normal)
            .busy_timeout(Duration::from_secs(5));
        let db = Self::ready(driver).await?;
        private(file)?;
        Ok(db)
    }

    #[cfg(test)]
    pub async fn memory() -> Self {
        Self::ready(Sqlite::in_memory()).await.unwrap()
    }

    async fn ready(driver: Sqlite) -> Result<Self> {
        let db = toasty::Db::builder().models(models::all()).build(driver).await?;
        MIGRATIONS.apply(&db).await?;
        Ok(Self {
            db,
            index: Arc::default(),
            blind: Arc::default(),
        })
    }

    pub fn set_library(&self, library: &Library) -> bool {
        let mut index = self.index.write().unwrap_or_else(|poisoned| poisoned.into_inner());
        let was_empty = index.is_empty();
        index.add(library);
        was_empty && !index.is_empty()
    }

    fn video(&self, file: &Path) -> Option<Video> {
        self.index
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .video(file)
    }

    pub async fn setting(&self, name: &str) -> Result<Option<String>> {
        let mut db = self.db.clone();
        let found = StoredSetting::filter_by_name(name.to_string()).first().exec(&mut db).await?;
        Ok(found.map(|setting| setting.value))
    }

    pub async fn set_setting(&self, name: &str, value: Option<&str>) -> Result<()> {
        let mut db = self.db.clone();
        match value {
            Some(value) => {
                StoredSetting::upsert_by_name(name.to_string())
                    .value(value.to_string())
                    .exec(&mut db)
                    .await?;
            }
            None => StoredSetting::filter_by_name(name.to_string()).delete().exec(&mut db).await?,
        }
        Ok(())
    }

    pub async fn subtitle_settings(&self, profile: ProfileId) -> Result<Option<subtitles::Settings>> {
        let mut db = self.db.clone();
        let stored = SubtitleSettings::filter_by_profile_id(profile.0)
            .first()
            .exec(&mut db)
            .await?;
        Ok(stored.map(|stored| stored.settings.0))
    }

    pub async fn save_subtitle_settings(&self, profile: ProfileId, value: &subtitles::Settings) -> Result<()> {
        let mut db = self.db.clone();
        SubtitleSettings::upsert_by_profile_id(profile.0)
            .settings(value.clone())
            .exec(&mut db)
            .await?;
        Ok(())
    }

    pub async fn subtitle_files(&self) -> Result<BTreeMap<PathBuf, Remembered>> {
        let mut db = self.db.clone();
        let fields = VideoFact::fields();
        let rows = VideoFact::filter(fields.subtitle_delay().ne(0.0).or(fields.embedded_subtitles().is_some()))
            .exec(&mut db)
            .await?;
        let index = self.index.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(rows
            .into_iter()
            .filter_map(|fact| {
                let path = index.path(Video {
                    kind: fact.kind,
                    id: fact.video_id,
                })?;
                let remembered = Remembered {
                    delay: fact.subtitle_delay,
                    embedded: fact.embedded_subtitles.and_then(|count| usize::try_from(count).ok()),
                };
                Some((path.clone(), remembered))
            })
            .collect())
    }

    pub async fn save_subtitle_file(&self, file: &Path, remembered: Remembered) -> Result<()> {
        let Some(Video { kind, id }) = self.video(file) else {
            return Err(format!("{} is not in the library", file.display()).into());
        };
        let embedded = remembered.embedded.map(|count| count as i64);
        let mut db = self.db.clone();
        VideoFact::upsert_by_kind_and_video_id(kind, id)
            .subtitle_delay(remembered.delay)
            .embedded_subtitles(embedded)
            .exec(&mut db)
            .await?;
        Ok(())
    }

    pub async fn audio_prefs(&self, profile: ProfileId) -> Result<AudioPrefs> {
        let mut db = self.db.clone();
        let stored = StoredAudioPrefs::filter_by_profile_id(profile.0).first().exec(&mut db).await?;
        Ok(stored.map_or_else(AudioPrefs::default, |stored| match stored.headphones {
            models::Headphones::Unpaired => AudioPrefs::Unpaired,
            models::Headphones::Paired { address, listening } => AudioPrefs::Paired {
                address,
                output: if listening { Output::Headphones } else { Output::Tv },
            },
        }))
    }

    pub async fn save_audio_prefs(&self, profile: ProfileId, prefs: &AudioPrefs) -> Result<()> {
        let headphones = match prefs {
            AudioPrefs::Unpaired => models::Headphones::Unpaired,
            AudioPrefs::Paired { address, output } => models::Headphones::Paired {
                address: address.clone(),
                listening: *output == Output::Headphones,
            },
        };
        let mut db = self.db.clone();
        if models::Profile::filter_by_id(profile.0).first().exec(&mut db).await?.is_none() {
            return Err(toasty::Error::record_not_found(format!("profile {}", profile.0)).into());
        }
        StoredAudioPrefs::upsert_by_profile_id(profile.0)
            .headphones(headphones)
            .exec(&mut db)
            .await?;
        Ok(())
    }
}

pub async fn toasty_cli(file: &Path, arguments: Vec<String>) -> Result<()> {
    toasty_cli_for(file, models::all(), "toasty/tv", arguments).await
}

pub(crate) async fn toasty_cli_for(
    file: &Path,
    models: toasty::ModelSet,
    migrations: &str,
    arguments: Vec<String>,
) -> Result<()> {
    let config = toasty_cli::Config::new().migration(toasty_cli::MigrationConfig::new().path(migrations));
    let db = toasty::Db::builder().models(models).build(Sqlite::open(file)).await?;
    toasty_cli::ToastyCli::with_config(db, config)
        .parse_from(std::iter::once("toasty".to_string()).chain(arguments))
        .await
        .map_err(|error| toasty::Error::from_args(format_args!("{error:#}")))?;
    Ok(())
}

#[cfg(test)]
pub fn test_library() -> Library {
    progress::tests::library()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> Db {
        let db = Db::memory().await;
        db.set_library(&progress::tests::library());
        db
    }

    #[tokio::test]
    async fn a_new_database_has_the_four_profiles_and_no_passwords() {
        let mut raw = db().await.db.clone();
        let profiles = models::Profile::all()
            .order_by(models::Profile::fields().id().asc())
            .exec(&mut raw)
            .await
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

    #[tokio::test]
    async fn deleting_a_profile_takes_its_state_along_and_leaves_the_others() {
        use crate::auth::store::Invite;
        use crate::progress::{Place, Progress};
        let db = db().await;
        for profile in [ProfileId(3), ProfileId(4)] {
            let mut progress = Progress::default();
            let file = Path::new("/lib/Movies/Up/Up.mkv");
            progress.record(file, Place { position: 60.0, duration: 5760.0, credits: None });
            progress.watching.played(crate::watching::Kind::Movie, "up", 1);
            progress.shows.insert("dark".to_string(), crate::progress::Position { season: 1, episode: 1, finished: false });
            db.save_progress(profile, &progress).await.unwrap();
            db.save_subtitle_settings(profile, &subtitles::Settings::default()).await.unwrap();
            let prefs = AudioPrefs::Paired { address: format!("AA:{}", profile.0), output: Output::Headphones };
            db.save_audio_prefs(profile, &prefs).await.unwrap();
        }
        let invite = Invite {
            token_hash: vec![9],
            profile: ProfileId(2),
            created_by: Some(ProfileId(4)),
            created_at: 1,
            expires_at: 2,
            used_at: None,
        };
        db.create_invite(&invite).await.unwrap();

        assert!(db.delete_profile(ProfileId(4)).await.unwrap());

        let gone = db.progress(ProfileId(4)).await.unwrap();
        assert!(gone.shows.is_empty() && gone.watching.entries().is_empty());
        assert_eq!(gone.files[Path::new("/lib/Movies/Up/Up.mkv")].stopped_at(), None);
        assert_eq!(gone.files[Path::new("/lib/Movies/Up/Up.mkv")].duration, Some(5760.0));
        assert_eq!(db.subtitle_settings(ProfileId(4)).await.unwrap(), None);
        assert_eq!(db.audio_prefs(ProfileId(4)).await.unwrap(), AudioPrefs::default());
        assert_eq!(db.invite(&[9]).await.unwrap().unwrap().created_by, None);
        let kept = db.progress(ProfileId(3)).await.unwrap();
        assert_eq!(kept.files[Path::new("/lib/Movies/Up/Up.mkv")].stopped_at(), Some(60.0));
        assert_eq!(db.audio_prefs(ProfileId(3)).await.unwrap().output(), Output::Headphones);
        assert!(!db.delete_profile(ProfileId(4)).await.unwrap());
    }

    #[tokio::test]
    async fn prefs_for_a_profile_that_does_not_exist_are_refused() {
        let db = db().await;
        let orphan = db.save_audio_prefs(ProfileId(99), &AudioPrefs::default()).await;
        assert!(orphan.is_err());
    }

    #[tokio::test]
    async fn a_setting_is_kept_replaced_and_forgotten() {
        let db = db().await;
        assert_eq!(db.setting("x").await.unwrap(), None);
        db.set_setting("x", Some("1")).await.unwrap();
        db.set_setting("x", Some("2")).await.unwrap();
        assert_eq!(db.setting("x").await.unwrap().as_deref(), Some("2"));
        db.set_setting("x", None).await.unwrap();
        assert_eq!(db.setting("x").await.unwrap(), None);
    }

    #[tokio::test]
    async fn the_file_is_the_apps_alone_and_keeps_what_was_saved() {
        let dir = std::env::temp_dir().join(format!("semmeltv-db-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let file = dir.join("state/semmeltv.db");
        let db = Db::open(&file).await.unwrap();
        db.set_setting("x", Some("1")).await.unwrap();
        drop(db);
        let mode = fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let again = Db::open(&file).await.unwrap();
        assert_eq!(again.setting("x").await.unwrap().as_deref(), Some("1"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn subtitle_settings_are_kept_per_profile() {
        let db = db().await;
        let larger = subtitles::Settings {
            size: 1.4,
            ..subtitles::Settings::default()
        };
        db.save_subtitle_settings(ProfileId(2), &larger).await.unwrap();
        assert_eq!(db.subtitle_settings(ProfileId(2)).await.unwrap(), Some(larger));
        assert_eq!(db.subtitle_settings(ProfileId(1)).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_files_subtitle_delay_is_kept_by_its_path_in_the_library() {
        let db = db().await;
        let file = Path::new("/lib/Movies/Up/Up.mkv");
        let remembered = Remembered {
            delay: 0.5,
            embedded: Some(2),
        };
        db.save_subtitle_file(file, remembered).await.unwrap();
        assert_eq!(db.subtitle_files().await.unwrap(), BTreeMap::from([(file.to_path_buf(), remembered)]));
        let mut raw = db.db.clone();
        let stored = VideoFact::all().exec(&mut raw).await.unwrap();
        assert_eq!((stored[0].kind, stored[0].video_id), (models::VideoKind::Movie, 9));
        db.save_subtitle_file(file, Remembered::default()).await.unwrap();
        assert!(db.subtitle_files().await.unwrap().is_empty());
        assert!(db.save_subtitle_file(Path::new("/lib/elsewhere.mkv"), remembered).await.is_err());
    }

    #[tokio::test]
    async fn a_database_from_an_older_schema_is_set_aside_and_started_fresh() {
        let dir = std::env::temp_dir().join(format!("semmeltv-replaced-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("semmeltv.db");
        let old = rusqlite::Connection::open(&file).unwrap();
        old.execute_batch(
            "CREATE TABLE __toasty_migrations (id INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at TEXT NOT NULL);
             INSERT INTO __toasty_migrations VALUES (42, '0000_initial.sql', '2026-11-14');
             CREATE TABLE profiles (id INTEGER PRIMARY KEY, name TEXT);",
        )
        .unwrap();
        drop(old);
        let db = Db::open(&file).await.unwrap();
        assert!(dir.join("semmeltv.db.replaced").exists());
        db.set_setting("x", Some("1")).await.unwrap();
        drop(db);
        let again = Db::open(&file).await.unwrap();
        assert_eq!(again.setting("x").await.unwrap().as_deref(), Some("1"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn audio_prefs_default_to_the_tv() {
        let db = db().await;
        assert_eq!(db.audio_prefs(ProfileId(3)).await.unwrap(), AudioPrefs::default());
        let prefs = AudioPrefs::Paired {
            address: "AA:BB:CC:DD:EE:FF".to_string(),
            output: Output::Headphones,
        };
        db.save_audio_prefs(ProfileId(3), &prefs).await.unwrap();
        assert_eq!(db.audio_prefs(ProfileId(3)).await.unwrap(), prefs);
        let on_tv = prefs.choosing(Output::Tv);
        db.save_audio_prefs(ProfileId(3), &on_tv).await.unwrap();
        assert_eq!(db.audio_prefs(ProfileId(3)).await.unwrap(), on_tv);
        db.save_audio_prefs(ProfileId(3), &AudioPrefs::Unpaired).await.unwrap();
        assert_eq!(db.audio_prefs(ProfileId(3)).await.unwrap(), AudioPrefs::Unpaired);
    }

    #[test]
    fn without_headphones_the_sound_stays_on_the_tv_and_new_headphones_keep_the_choice() {
        let unpaired = AudioPrefs::Unpaired.choosing(Output::Headphones);
        assert_eq!((unpaired.headphones(), unpaired.output()), (None, Output::Tv));
        let listening = unpaired.pairing("AA".to_string()).choosing(Output::Headphones);
        assert_eq!((listening.headphones(), listening.output()), (Some("AA"), Output::Headphones));
        let replaced = listening.pairing("BB".to_string());
        assert_eq!((replaced.headphones(), replaced.output()), (Some("BB"), Output::Headphones));
    }

    #[tokio::test]
    async fn the_table_refuses_an_address_without_headphones_and_headphones_without_one() {
        let dir = std::env::temp_dir().join(format!("semmeltv-audio-prefs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let file = dir.join("semmeltv.db");
        Db::open(&file).await.unwrap();
        let connection = rusqlite::Connection::open(&file).unwrap();
        let insert = |headphones: &str, address: Option<&str>, listening: Option<bool>| {
            connection.execute(
                "INSERT OR REPLACE INTO audio_prefs (profile_id, headphones, headphones_address, headphones_listening) VALUES (3, ?1, ?2, ?3)",
                rusqlite::params![headphones, address, listening],
            )
        };
        assert!(insert("paired", Some("AA"), Some(true)).is_ok());
        assert!(insert("unpaired", None, None).is_ok());
        assert!(insert("paired", None, Some(true)).is_err());
        assert!(insert("unpaired", Some("AA"), Some(false)).is_err());
        drop(connection);
        fs::remove_dir_all(dir).unwrap();
    }
}
