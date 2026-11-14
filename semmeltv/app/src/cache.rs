use std::path::{Path, PathBuf};

pub struct Cache {
    root: PathBuf,
}

impl Cache {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn saved_manifest(&self) -> PathBuf {
        self.root.join("library.last-good.toml")
    }
}
