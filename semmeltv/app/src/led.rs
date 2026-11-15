use std::fs;
use std::path::{Path, PathBuf};

pub struct Led {
    brightness: PathBuf,
}

impl Led {
    pub fn new(dir: &Path) -> Self {
        Self {
            brightness: dir.join("brightness"),
        }
    }

    pub fn set(&self, on: bool) {
        if let Err(error) = fs::write(&self.brightness, if on { "1" } else { "0" }) {
            eprintln!("{}: {error}", self.brightness.display());
        }
    }
}
