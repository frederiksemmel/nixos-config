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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_writes_the_brightness() {
        let dir = std::env::temp_dir().join(format!("semmeltv-led-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let led = Led::new(&dir);
        led.set(true);
        assert_eq!(fs::read_to_string(dir.join("brightness")).unwrap(), "1");
        led.set(false);
        assert_eq!(fs::read_to_string(dir.join("brightness")).unwrap(), "0");
        fs::remove_dir_all(dir).unwrap();
    }
}
