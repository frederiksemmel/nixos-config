use super::{Core, Signal};
use crate::library::{self, Library};
use crate::library_db;
use crate::manifest::{self, Manifest, ReadError};
use crate::ready::Ready;
use std::io::ErrorKind;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tokio::task::spawn_blocking;
use tokio::time::MissedTickBehavior;

const POLL: Duration = Duration::from_secs(30);
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

pub struct Paths {
    pub root: PathBuf,
    pub meta: PathBuf,
    pub saved: PathBuf,
    pub remote: Option<String>,
    pub ready: Option<PathBuf>,
}

type Stamp = Option<(SystemTime, u64)>;

async fn stamp(path: &Path) -> Stamp {
    let metadata = tokio::fs::metadata(path).await.ok()?;
    Some((metadata.modified().ok()?, metadata.len()))
}

fn reachable(remote: &str) -> bool {
    remote
        .to_socket_addrs()
        .is_ok_and(|mut addresses| addresses.any(|address| TcpStream::connect_timeout(&address, PROBE_TIMEOUT).is_ok()))
}

async fn manifest(meta: &Path) -> Result<Manifest, String> {
    let path = meta.join(manifest::FILE_NAME);
    library_db::read(&path).await.map_err(|error| match error {
        ReadError::Unreadable(error) if error.kind() == ErrorKind::NotFound => {
            format!("No {} in {} yet", manifest::FILE_NAME, meta.display())
        }
        ReadError::Unreadable(error) => format!("{}: {error}", path.display()),
        ReadError::Invalid(error) => format!("{}: {error}", path.display()),
    })
}

pub async fn listed(root: &Path, file: &Path) -> Option<Library> {
    let manifest = library_db::read(file).await.ok()?;
    Some(Library::from_manifest(root, &manifest, |_| true))
}

pub async fn everything(root: &Path, meta: &Path) -> Result<Library, String> {
    let manifest = manifest(meta).await?;
    Ok(Library::from_manifest(root, &manifest, |_| true))
}

fn ready_here(file: Option<&Path>) -> Ready {
    file.map(Ready::read).unwrap_or_default()
}

pub async fn read(root: &Path, meta: &Path, saved: &Path, ready: Option<&Path>) -> Result<Library, String> {
    match manifest(meta).await {
        Ok(manifest) => {
            let (root, meta, saved) = (root.to_path_buf(), meta.to_path_buf(), saved.to_path_buf());
            let ready = ready.map(Path::to_path_buf);
            let present = spawn_blocking(move || {
                let ready = ready_here(ready.as_deref());
                let library = Library::from_manifest(&root, &manifest, |path| ready.present(&root, path));
                library::save_copy(&meta, &saved);
                library
            });
            present.await.map_err(|error| error.to_string())
        }
        Err(error) => listed(root, saved).await.ok_or(error),
    }
}

async fn absent(root: &Path, meta: &Path) -> Vec<PathBuf> {
    let Ok(listed) = everything(root, meta).await else {
        return Vec::new();
    };
    spawn_blocking(move || listed.paths().filter(|path| !path.is_file()).cloned().collect())
        .await
        .unwrap_or_default()
}

async fn appeared(absent: &[PathBuf]) -> bool {
    if absent.is_empty() {
        return false;
    }
    let absent = absent.to_vec();
    spawn_blocking(move || absent.iter().any(|path| path.is_file()))
        .await
        .unwrap_or(false)
}

pub fn watch(paths: Paths, core: Core) {
    tokio::spawn(async move {
        let manifest_file = paths.meta.join(manifest::FILE_NAME);
        let mut manifest_seen = None;
        let mut ready_seen = None;
        let mut missing = Vec::new();
        let mut every = tokio::time::interval(POLL);
        every.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            every.tick().await;
            let online = match paths.remote.clone() {
                Some(remote) => spawn_blocking(move || reachable(&remote)).await.unwrap_or(false),
                None => true,
            };
            core.signal(Signal::Poll { online });
            if online {
                let current = stamp(&manifest_file).await;
                let ready = match &paths.ready {
                    Some(file) => stamp(file).await,
                    None => None,
                };
                if manifest_seen != Some(current) || ready_seen != Some(ready) || appeared(&missing).await {
                    manifest_seen = Some(current);
                    ready_seen = Some(ready);
                    let library = read(&paths.root, &paths.meta, &paths.saved, paths.ready.as_deref()).await;
                    missing = absent(&paths.root, &paths.meta).await;
                    core.signal(Signal::Library(library));
                }
            } else {
                manifest_seen = None;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn a_broken_manifest_falls_back_to_the_saved_copy() {
        let root = std::env::temp_dir().join(format!("semmeltv-watch-{}", std::process::id()));
        fs::create_dir_all(root.join("Movies")).unwrap();
        fs::write(root.join("Movies/Up.mkv"), b"x").unwrap();
        let saved = root.join("saved.db");
        let good = "[[movie]]\ntitle = \"Up\"\npath = \"Movies/Up.mkv\"\nadded = 2024-01-01\n";
        library_db::write(&root.join(manifest::FILE_NAME), &Manifest::parse(good).unwrap())
            .await
            .unwrap();
        assert_eq!(read(&root, &root, &saved, None).await.unwrap().movies.len(), 1);
        assert_eq!(
            fs::read(&saved).unwrap(),
            fs::read(root.join(manifest::FILE_NAME)).unwrap()
        );

        fs::write(root.join(manifest::FILE_NAME), "not a database").unwrap();
        assert_eq!(read(&root, &root, &saved, None).await.unwrap().movies.len(), 1);
        fs::remove_file(root.join(manifest::FILE_NAME)).unwrap();
        fs::remove_file(root.join("Movies/Up.mkv")).unwrap();
        assert_eq!(read(&root, &root, &saved, None).await.unwrap().movies.len(), 1);

        fs::remove_file(&saved).unwrap();
        assert!(
            read(&root, &root, &saved, None)
                .await
                .unwrap_err()
                .contains("library.db")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn a_title_whose_file_arrives_after_its_entry_shows_up_once_the_file_is_there() {
        let root = std::env::temp_dir().join(format!("semmeltv-arrives-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("Movies")).unwrap();
        fs::write(root.join("Movies/Up.mkv"), b"x").unwrap();
        let saved = root.join("saved.db");
        let both = "[[movie]]\ntitle = \"Up\"\npath = \"Movies/Up.mkv\"\nadded = 2024-01-01\n\n\
                    [[movie]]\ntitle = \"The Princess Diaries\"\nyear = 2001\npath = \"Movies/Diaries.mkv\"\nadded = 2026-11-15\n";
        library_db::write(&root.join(manifest::FILE_NAME), &Manifest::parse(both).unwrap())
            .await
            .unwrap();

        assert_eq!(read(&root, &root, &saved, None).await.unwrap().movies.len(), 1);
        let missing = absent(&root, &root).await;
        assert_eq!(missing, [root.join("Movies/Diaries.mkv")]);
        assert!(!appeared(&missing).await);

        fs::write(root.join("Movies/Diaries.mkv"), b"x").unwrap();
        assert!(appeared(&missing).await);
        assert_eq!(read(&root, &root, &saved, None).await.unwrap().movies.len(), 2);
        assert!(absent(&root, &root).await.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn a_title_still_uploading_shows_up_from_its_downloaded_copy() {
        let root = std::env::temp_dir().join(format!("semmeltv-ready-here-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("downloads")).unwrap();
        let saved = root.join("saved.db");
        let listed =
            "[[movie]]\ntitle = \"Up\"\nyear = 2009\npath = \"Movies/Up (2009)/Up (2009).mkv\"\nadded = 2026-11-17\n";
        library_db::write(&root.join(manifest::FILE_NAME), &Manifest::parse(listed).unwrap())
            .await
            .unwrap();
        let ready = root.join(crate::ready::FILE_NAME);
        assert!(
            read(&root, &root, &saved, Some(&ready))
                .await
                .unwrap()
                .movies
                .is_empty()
        );

        let local = root.join("downloads/Up.2009.1080p.mkv");
        fs::write(&local, b"x").unwrap();
        Ready::add(&ready, "Movies/Up (2009)/Up (2009).mkv", &local).unwrap();
        let library = read(&root, &root, &saved, Some(&ready)).await.unwrap();
        assert_eq!(
            library.movies[0].path,
            root.join("Movies/Up (2009)/Up (2009).mkv"),
            "the entry keeps its library path, so progress stays with it"
        );
        assert!(
            read(&root, &root, &saved, None).await.unwrap().movies.is_empty(),
            "only this TV has the copy"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
