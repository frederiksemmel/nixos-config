use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "ready.json";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ready {
    files: BTreeMap<String, PathBuf>,
}

impl Ready {
    pub fn read(file: &Path) -> Self {
        fs::read(file)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn write(&self, file: &Path) -> Result<(), String> {
        let partial = file.with_extension("json.partial");
        let text = serde_json::to_vec_pretty(self).map_err(|error| error.to_string())?;
        fs::write(&partial, text)
            .and_then(|()| fs::rename(&partial, file))
            .map_err(|error| format!("{}: {error}", file.display()))
    }

    pub fn add(file: &Path, destination: &str, local: &Path) -> Result<(), String> {
        let mut ready = Self::read(file);
        ready.files.insert(destination.to_string(), local.to_path_buf());
        ready.write(file)
    }

    pub fn remove(file: &Path, destination: &str) -> Result<(), String> {
        let mut ready = Self::read(file);
        if ready.files.remove(destination).is_none() {
            return Ok(());
        }
        ready.write(file)
    }

    pub fn local(&self, root: &Path, path: &Path) -> Option<&Path> {
        let destination = path.strip_prefix(root).ok()?.to_str()?;
        self.files
            .get(destination)
            .map(PathBuf::as_path)
            .filter(|local| local.is_file())
    }

    pub fn present(&self, root: &Path, path: &Path) -> bool {
        path.is_file() || self.local(root, path).is_some()
    }

    pub fn playable(&self, root: &Path, path: &Path) -> PathBuf {
        if path.is_file() {
            return path.to_path_buf();
        }
        self.local(root, path).unwrap_or(path).to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("semmeltv-ready-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_downloaded_file_plays_from_here_until_its_library_copy_exists() {
        let dir = dir("plays");
        let (root, file) = (dir.join("library"), dir.join(FILE_NAME));
        let local = dir.join("downloads/job/Up.2009.mkv");
        fs::create_dir_all(local.parent().unwrap()).unwrap();
        fs::write(&local, "video").unwrap();
        let canonical = root.join("Movies/Up (2009)/Up (2009).mkv");

        assert!(!Ready::read(&file).present(&root, &canonical), "nothing is ready yet");
        Ready::add(&file, "Movies/Up (2009)/Up (2009).mkv", &local).unwrap();
        let ready = Ready::read(&file);
        assert!(ready.present(&root, &canonical));
        assert_eq!(ready.playable(&root, &canonical), local);
        assert!(
            !ready.present(&root, &root.join("Movies/Other.mkv")),
            "only the ready file"
        );

        fs::create_dir_all(canonical.parent().unwrap()).unwrap();
        fs::write(&canonical, "video").unwrap();
        assert_eq!(
            ready.playable(&root, &canonical),
            canonical,
            "the library copy wins once it exists"
        );

        Ready::remove(&file, "Movies/Up (2009)/Up (2009).mkv").unwrap();
        fs::remove_file(&canonical).unwrap();
        assert!(!Ready::read(&file).present(&root, &canonical));
    }

    #[test]
    fn a_ready_entry_whose_local_file_is_gone_is_not_present() {
        let dir = dir("gone");
        let (root, file) = (dir.join("library"), dir.join(FILE_NAME));
        Ready::add(&file, "Movies/Up (2009)/Up (2009).mkv", &dir.join("deleted.mkv")).unwrap();
        let canonical = root.join("Movies/Up (2009)/Up (2009).mkv");
        assert!(!Ready::read(&file).present(&root, &canonical));
        assert_eq!(Ready::read(&file).playable(&root, &canonical), canonical);
    }
}
