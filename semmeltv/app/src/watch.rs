use crate::library::{self, Library};
use crate::manifest;
use std::fs;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime};

const POLL: Duration = Duration::from_secs(30);
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

pub enum Change {
    Library(Result<Library, String>),
    Art,
    Poll { online: bool },
    Downloading(Option<PathBuf>),
}

pub struct Paths {
    pub root: PathBuf,
    pub saved: PathBuf,
    pub art: PathBuf,
    pub matches: PathBuf,
    pub remote: Option<String>,
}

type Stamp = Option<(SystemTime, u64)>;

fn stamp(path: &Path) -> Stamp {
    let metadata = fs::metadata(path).ok()?;
    Some((metadata.modified().ok()?, metadata.len()))
}

fn reachable(remote: &str) -> bool {
    remote
        .to_socket_addrs()
        .is_ok_and(|mut addresses| addresses.any(|address| TcpStream::connect_timeout(&address, PROBE_TIMEOUT).is_ok()))
}

pub fn read(root: &Path, saved: &Path) -> Result<Library, String> {
    match library::load(root) {
        Ok(library) => {
            library::save_copy(root, saved);
            Ok(library)
        }
        Err(error) => library::load_saved(root, saved).map_err(|_| error),
    }
}

pub fn spawn(paths: Paths, deliver: impl Fn(Change) + Send + 'static) {
    thread::spawn(move || {
        let manifest_file = paths.root.join(manifest::FILE_NAME);
        let mut manifest_seen = None;
        let mut art_seen = (stamp(&paths.art), stamp(&paths.matches));
        loop {
            let online = paths.remote.as_deref().is_none_or(reachable);
            deliver(Change::Poll { online });
            if online {
                let current = stamp(&manifest_file);
                if manifest_seen != Some(current) {
                    manifest_seen = Some(current);
                    deliver(Change::Library(read(&paths.root, &paths.saved)));
                }
            } else {
                manifest_seen = None;
            }
            let art = (stamp(&paths.art), stamp(&paths.matches));
            if art != art_seen {
                art_seen = art;
                deliver(Change::Art);
            }
            thread::sleep(POLL);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broken_manifest_falls_back_to_the_saved_copy() {
        let root = std::env::temp_dir().join(format!("semmeltv-watch-{}", std::process::id()));
        fs::create_dir_all(root.join("Movies")).unwrap();
        fs::write(root.join("Movies/Up.mkv"), b"x").unwrap();
        let saved = root.join("saved.toml");
        let good = "[[movie]]\ntitle = \"Up\"\npath = \"Movies/Up.mkv\"\nadded = 2024-01-01\n";
        fs::write(root.join(manifest::FILE_NAME), good).unwrap();
        assert_eq!(read(&root, &saved).unwrap().movies.len(), 1);
        assert_eq!(fs::read_to_string(&saved).unwrap(), good);

        fs::write(root.join(manifest::FILE_NAME), "not toml [").unwrap();
        assert_eq!(read(&root, &saved).unwrap().movies.len(), 1);
        fs::remove_file(root.join(manifest::FILE_NAME)).unwrap();
        fs::remove_file(root.join("Movies/Up.mkv")).unwrap();
        assert_eq!(read(&root, &saved).unwrap().movies.len(), 1);

        fs::remove_file(&saved).unwrap();
        assert!(read(&root, &saved).unwrap_err().contains("library.toml"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_closed_port_is_unreachable() {
        assert!(!reachable("127.0.0.1:1"));
        assert!(!reachable("no such host:443"));
    }
}
