use crate::progress::Progress;
use std::fs;
use std::io;
use std::path::{self, Path, PathBuf};

pub struct Cache {
    root: PathBuf,
}

impl Cache {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn saved_manifest(&self) -> PathBuf {
        self.root.join("library.last-good.toml")
    }

    pub fn tv_token_path(&self) -> PathBuf {
        self.root.join("tv-token")
    }

    pub fn subtitles_path(&self) -> PathBuf {
        self.root.join("subtitles.json")
    }

    fn progress_path(&self) -> PathBuf {
        self.root.join("progress.json")
    }

    pub fn watch_later_dir(&self, file: &Path) -> PathBuf {
        let parts = file
            .components()
            .filter(|part| matches!(part, path::Component::Normal(_)));
        self.root.join("watch-later").join(parts.collect::<PathBuf>())
    }

    pub fn load_progress(&self) -> Progress {
        fs::read(self.progress_path())
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save_progress(&self, progress: &Progress) -> io::Result<()> {
        let path = self.progress_path();
        let temporary = path.with_extension("json.tmp");
        fs::create_dir_all(&self.root)?;
        fs::write(&temporary, serde_json::to_vec_pretty(progress)?)?;
        fs::rename(temporary, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_with_the_same_name_keep_their_saved_positions_apart() {
        let cache = Cache::new("/cache".into());
        let one = cache.watch_later_dir(Path::new("/lib/Series/One/S01/E01.mkv"));
        let two = cache.watch_later_dir(Path::new("/lib/Series/Two/S01/E01.mkv"));
        assert_eq!(one, Path::new("/cache/watch-later/lib/Series/One/S01/E01.mkv"));
        assert_ne!(one, two);
    }
}
