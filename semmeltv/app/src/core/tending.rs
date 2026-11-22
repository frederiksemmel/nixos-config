use super::Outcome;
use super::library;
use crate::library::Library;
use crate::offline::Store;
use crate::progress::Progress;
use crate::storage::{self, Cached, Keep};
use crate::watching;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::task::spawn_blocking;

pub struct Job {
    pub store: Store,
    pub cache: Option<storage::Cache>,
    pub library: Arc<Library>,
    pub progress: Arc<Progress>,
    pub playing: Option<PathBuf>,
    pub queued: Vec<PathBuf>,
    pub stuck: bool,
}

pub struct Tended {
    pub downloaded: HashSet<PathBuf>,
    pub report: Option<storage::Report>,
    pub stuck: bool,
}

impl Job {
    fn titles(&self, cache: &storage::Cache, everything: &Library) -> Vec<Cached> {
        let activity = storage::Activity {
            playing: self.playing.as_deref(),
            queued: &self.queued,
        };
        storage::titles(&cache.library, everything, &self.progress, cache.scan(), &activity)
    }

    fn downloaded(&self) -> HashSet<PathBuf> {
        self.store.downloaded(self.library.paths())
    }

    fn report(&self, cache: &storage::Cache, everything: &Library) -> Option<storage::Report> {
        match cache.disk() {
            Ok(disk) => Some(storage::Report::new(disk, cache.marks, self.titles(cache, everything))),
            Err(error) => {
                eprintln!("storage: {}: {error}", cache.data.display());
                None
            }
        }
    }

    fn tend(&self, everything: Option<Library>) -> Tended {
        let untended = |job: &Self| Tended {
            downloaded: job.downloaded(),
            report: None,
            stuck: job.stuck,
        };
        let (Some(cache), Some(everything)) = (&self.cache, everything) else {
            return untended(self);
        };
        let disk = match cache.disk() {
            Ok(disk) => disk,
            Err(error) => {
                eprintln!("storage: {}: {error}", cache.data.display());
                return untended(self);
            }
        };
        let titles = self.titles(cache, &everything);
        let used: u64 = titles.iter().map(|title| title.bytes).sum();
        let excess = storage::excess(used, disk.free, cache.marks);
        let chosen = storage::plan(&titles, used, disk.free, cache.marks);
        let freeing: u64 = chosen.iter().map(|&index| titles[index].bytes).sum();
        let now = watching::now();
        let why = format!(
            "the offline cache holds {} of {} with {} free on the disk",
            storage::size(used),
            storage::size(cache.marks.high),
            storage::size(disk.free)
        );
        for &index in &chosen {
            let title = &titles[index];
            match cache.remove(&title.files) {
                Ok(()) => eprintln!(
                    "storage: removed {}, {}: {why}",
                    title.describe(now),
                    storage::size(title.bytes)
                ),
                Err(error) => eprintln!("storage: could not remove {}: {error}", title.describe(now)),
            }
        }
        let stuck = freeing < excess;
        if stuck && !self.stuck {
            eprintln!("storage: {why}, and every other title is kept");
        }
        let report = if chosen.is_empty() {
            Some(storage::Report::new(disk, cache.marks, titles))
        } else {
            self.report(cache, &everything)
        };
        Tended {
            downloaded: self.downloaded(),
            report,
            stuck,
        }
    }

    fn remove(&self, everything: &Library, title: &storage::Title) -> Outcome {
        let Some(cache) = &self.cache else {
            return Outcome::refused("This TV keeps no downloads");
        };
        let titles = self.titles(cache, everything);
        let Some(cached) = titles.iter().find(|cached| &cached.title == title) else {
            return Outcome::refused("That title is not downloaded");
        };
        match cached.keep {
            Some(Keep::Playing) => return Outcome::refused(format!("{} is playing", cached.name)),
            Some(Keep::UpNext) => {
                return Outcome::refused(format!(
                    "{} is up next, so it would download again straight away",
                    cached.name
                ));
            }
            _ => {}
        }
        if let Err(error) = cache.remove(&cached.files) {
            eprintln!(
                "storage: could not remove {}: {error}",
                cached.describe(watching::now())
            );
            return Outcome::refused(format!("Could not remove {}: {error}", cached.name));
        }
        eprintln!(
            "storage: removed {}, {}: asked to from the web app",
            cached.describe(watching::now()),
            storage::size(cached.bytes)
        );
        Outcome::done(format!(
            "Removed {}, {} freed",
            cached.name,
            storage::size(cached.bytes)
        ))
    }
}

async fn everything(cache: Option<&storage::Cache>) -> Result<Option<Library>, String> {
    match cache {
        Some(cache) => library::everything(&cache.library, &cache.manifest).await.map(Some),
        None => Ok(None),
    }
}

pub async fn tend(job: Job) -> Tended {
    let everything = everything(job.cache.as_ref()).await.unwrap_or_else(|error| {
        eprintln!("storage: not tending the offline cache: {error}");
        None
    });
    let stuck = job.stuck;
    spawn_blocking(move || job.tend(everything))
        .await
        .unwrap_or_else(|error| {
            eprintln!("storage: {error}");
            Tended {
                downloaded: HashSet::new(),
                report: None,
                stuck,
            }
        })
}

/// Removes a title from pCloud and the offline cache.
pub type Remove = std::sync::Arc<
    dyn Fn(Job, storage::Title) -> std::pin::Pin<Box<dyn std::future::Future<Output = Outcome> + Send>> + Send + Sync,
>;

/// The removal that asks pCloud.
pub fn remover() -> Remove {
    std::sync::Arc::new(|job, title| Box::pin(remove(job, title)))
}

pub async fn remove(job: Job, title: storage::Title) -> Outcome {
    let everything = match everything(job.cache.as_ref()).await {
        Ok(Some(everything)) => everything,
        Ok(None) => return Outcome::refused("This TV keeps no downloads"),
        Err(error) => return Outcome::refused(error),
    };
    spawn_blocking(move || job.remove(&everything, &title))
        .await
        .unwrap_or_else(|error| Outcome::refused(error.to_string()))
}
