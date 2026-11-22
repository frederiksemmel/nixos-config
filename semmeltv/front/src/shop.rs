use crate::bakery::{Bakery, Probe, Scope, Status, Upstream};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Notify;

pub struct Shop {
    bakery: Mutex<Bakery>,
    file: Option<PathBuf>,
    pub core_changed: Notify,
    pub web_changed: Notify,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

impl Shop {
    pub fn open(file: Option<PathBuf>) -> Self {
        let bakery = file
            .as_ref()
            .and_then(|file| std::fs::read(file).ok())
            .and_then(|bytes| match serde_json::from_slice(&bytes) {
                Ok(bakery) => Some(bakery),
                Err(error) => {
                    eprintln!("forgetting an unreadable state file: {error}");
                    None
                }
            })
            .unwrap_or_default();
        Self {
            bakery: Mutex::new(bakery),
            file,
            core_changed: Notify::new(),
            web_changed: Notify::new(),
        }
    }

    pub fn status(&self, scope: Scope) -> Status {
        self.bakery.lock().unwrap().status(now(), scope)
    }

    pub fn is_down(&self, upstream: Upstream) -> bool {
        self.bakery.lock().unwrap().watch(upstream).is_down()
    }

    pub fn guess(&self, upstream: Upstream) -> u64 {
        self.bakery.lock().unwrap().watch(upstream).guess()
    }

    pub fn change<T>(&self, change: impl FnOnce(&mut Bakery) -> T) -> T {
        let mut bakery = self.bakery.lock().unwrap();
        let result = change(&mut bakery);
        if let Some(file) = &self.file {
            let written = serde_json::to_vec(&*bakery)
                .map_err(std::io::Error::other)
                .and_then(|bytes| {
                    let temporary = file.with_extension("new");
                    std::fs::write(&temporary, bytes)?;
                    std::fs::rename(&temporary, file)
                });
            if let Err(error) = written {
                eprintln!("{}: {error}", file.display());
            }
        }
        result
    }

    pub fn observe(&self, upstream: Upstream, probe: Probe) {
        let changed = {
            let mut bakery = self.bakery.lock().unwrap();
            bakery.observe(upstream, probe, now())
        };
        if changed {
            self.change(|_| ());
            let state = if self.is_down(upstream) { "down" } else { "back" };
            eprintln!("{upstream:?} is {state}");
        }
    }

    pub fn wake(&self, upstream: Upstream) {
        match upstream {
            Upstream::Core => self.core_changed.notify_one(),
            Upstream::Web => self.web_changed.notify_one(),
        }
    }
}
