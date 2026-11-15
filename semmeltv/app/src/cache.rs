use std::path::PathBuf;

pub struct Cache {
    root: PathBuf,
}

impl Cache {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn saved_manifest(&self) -> PathBuf {
        self.root.join("library.last-good.db")
    }
}
