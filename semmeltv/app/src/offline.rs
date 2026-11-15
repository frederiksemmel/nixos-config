use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

const ST_BLOCK_BYTES: u64 = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    Unknown,
    Remote,
    Downloading,
    Partial,
    Downloaded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Stream,
    Cached(PathBuf),
    Unavailable,
}

#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
    vfs: Option<PathBuf>,
    capacity: Option<u64>,
}

pub fn covers(size: u64, allocated: u64) -> bool {
    size > 0 && allocated >= size
}

pub fn fits(size: u64, capacity: Option<u64>) -> bool {
    capacity.is_none_or(|capacity| size <= capacity)
}

pub fn choose(online: bool, mounted: bool, cached: Option<PathBuf>) -> Source {
    match cached {
        _ if online && mounted => Source::Stream,
        Some(path) => Source::Cached(path),
        None => Source::Unavailable,
    }
}

pub fn single(downloaded: bool, downloading: bool) -> (Presence, String) {
    if downloaded {
        (Presence::Downloaded, "Downloaded".to_string())
    } else if downloading {
        (Presence::Downloading, "Downloading".to_string())
    } else {
        (Presence::Remote, "Not downloaded".to_string())
    }
}

pub fn group(done: usize, total: usize, downloading: bool) -> (Presence, String) {
    let text = if done == 0 && !downloading {
        "Not downloaded".to_string()
    } else {
        format!("{done} of {total} downloaded")
    };
    let presence = match done {
        _ if downloading => Presence::Downloading,
        0 => Presence::Remote,
        done if done == total => Presence::Downloaded,
        _ => Presence::Partial,
    };
    (presence, text)
}

impl Store {
    pub fn new(root: PathBuf, vfs: Option<PathBuf>, capacity: Option<u64>) -> Self {
        Self { root, vfs, capacity }
    }

    pub fn is_enabled(&self) -> bool {
        self.vfs.is_some()
    }

    fn cache_file(&self, path: &Path) -> Option<PathBuf> {
        Some(self.vfs.as_ref()?.join(path.strip_prefix(&self.root).ok()?))
    }

    pub fn is_downloaded(&self, path: &Path) -> bool {
        self.cache_file(path)
            .and_then(|file| fs::metadata(file).ok())
            .is_some_and(|metadata| covers(metadata.len(), metadata.blocks() * ST_BLOCK_BYTES))
    }

    pub fn downloaded<'a>(&self, paths: impl Iterator<Item = &'a PathBuf>) -> HashSet<PathBuf> {
        paths.filter(|path| self.is_downloaded(path)).cloned().collect()
    }

    pub fn source(&self, path: &Path, online: bool) -> Source {
        if !self.is_enabled() {
            return Source::Stream;
        }
        let cached = self.cache_file(path).filter(|_| self.is_downloaded(path));
        choose(online, online && path.is_file(), cached)
    }

    pub fn fits(&self, size: u64) -> bool {
        fits(size, self.capacity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn streaming_needs_the_network_and_the_mount() {
        let cached = PathBuf::from("/cache/a.mkv");
        assert_eq!(choose(true, true, None), Source::Stream);
        assert_eq!(choose(true, true, Some(cached.clone())), Source::Stream);
        assert_eq!(
            choose(false, false, Some(cached.clone())),
            Source::Cached(cached.clone())
        );
        assert_eq!(choose(true, false, Some(cached.clone())), Source::Cached(cached));
        assert_eq!(choose(false, false, None), Source::Unavailable);
        assert_eq!(choose(true, false, None), Source::Unavailable);
    }

    #[test]
    fn presence_says_what_is_downloaded() {
        let cases = [
            ("single downloaded", single(true, true), Presence::Downloaded, None),
            (
                "single downloading",
                single(false, true),
                Presence::Downloading,
                Some("Downloading"),
            ),
            (
                "single remote",
                single(false, false),
                Presence::Remote,
                Some("Not downloaded"),
            ),
            ("0 of 10", group(0, 10, false), Presence::Remote, Some("Not downloaded")),
            (
                "3 of 10",
                group(3, 10, false),
                Presence::Partial,
                Some("3 of 10 downloaded"),
            ),
            (
                "10 of 10",
                group(10, 10, false),
                Presence::Downloaded,
                Some("10 of 10 downloaded"),
            ),
            (
                "0 of 10 downloading",
                group(0, 10, true),
                Presence::Downloading,
                Some("0 of 10 downloaded"),
            ),
        ];
        for (name, (presence, label), want, want_label) in cases {
            assert_eq!(presence, want, "{name}");
            if let Some(want_label) = want_label {
                assert_eq!(label, want_label, "{name}");
            }
        }
    }

    #[test]
    fn sparse_cache_files_are_not_downloaded_until_they_are_full() {
        let root = std::env::temp_dir().join(format!("semmeltv-vfs-{}", std::process::id()));
        fs::create_dir_all(root.join("vfs/Movies")).unwrap();
        let store = Store::new("/lib".into(), Some(root.join("vfs")), None);
        let path = Path::new("/lib/Movies/Up.mkv");
        assert!(!store.is_downloaded(path));

        let mut file = File::create(root.join("vfs/Movies/Up.mkv")).unwrap();
        file.set_len(1 << 24).unwrap();
        assert!(!store.is_downloaded(path));

        file.write_all(&vec![1; 1 << 24]).unwrap();
        file.sync_all().unwrap();
        assert!(store.is_downloaded(path));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn without_a_cache_everything_streams() {
        let store = Store::new("/lib".into(), None, None);
        assert!(!store.is_enabled());
        assert_eq!(store.source(Path::new("/lib/Movies/Up.mkv"), false), Source::Stream);
    }
}
