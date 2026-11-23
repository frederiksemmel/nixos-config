//! What a play needs from its files, found on a task of its own and within a
//! time limit, so the actor never waits on the pCloud mount.

use crate::offline::{Source, Store};
use crate::sidecar::{self, Sidecar};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::spawn_blocking;
use tokio::time::timeout;

/// How long finding a file and its subtitles may take.
pub const FIND_WITHIN: Duration = Duration::from_secs(4);

/// How long after the start of the finding its subtitle files must be read,
/// early enough to leave the play within `FIND_WITHIN`.
const SIDECARS_READ_BY: Duration = Duration::from_secs(3);
const _: () = assert!(SIDECARS_READ_BY.as_millis() < FIND_WITHIN.as_millis());

/// Where a file plays from, and the subtitle files beside it; `None` when it
/// is out of reach. Called with the file and whether the TV is online.
pub type Find = Arc<dyn Fn(&Path, bool) -> Option<(PathBuf, Vec<Sidecar>)> + Send + Sync>;

#[derive(Clone)]
pub struct Files {
    pub find: Find,
}

impl Files {
    /// The files of `store`, with `playable` naming the copy to play.
    pub fn of(store: Store, playable: impl Fn(&Path) -> PathBuf + Send + Sync + 'static) -> Self {
        Self {
            find: Arc::new(move |path, online| {
                let began = Instant::now();
                let here = playable(path);
                let source = match store.source(path, online) {
                    _ if here != path => here,
                    Source::Stream => path.to_path_buf(),
                    Source::Cached(file) => file,
                    Source::Unavailable => return None,
                };
                let sidecars = sidecar::warm(
                    sidecar::find(&source),
                    (began + SIDECARS_READ_BY).saturating_duration_since(Instant::now()),
                );
                Some((source, sidecars))
            }),
        }
    }
}

#[derive(Debug)]
pub enum Prepared {
    Ready(Box<Ready>),
    OutOfReach,
    Silent,
}

/// A file found: where it plays from and its subtitle files.
#[derive(Debug)]
pub struct Ready {
    pub source: PathBuf,
    pub sidecars: Vec<Sidecar>,
}

/// Finds `path`. A file that is not found `within` is `Silent`.
pub async fn prepare(files: Files, path: PathBuf, online: bool, within: Duration) -> Prepared {
    let find = files.find.clone();
    let looking = path.clone();
    match timeout(within, spawn_blocking(move || find(&looking, online))).await {
        Ok(Ok(Some((source, sidecars)))) => Prepared::Ready(Box::new(Ready { source, sidecars })),
        Ok(Ok(None)) => Prepared::OutOfReach,
        Ok(Err(_)) => Prepared::Silent,
        Err(_) => {
            eprintln!("tv player: {} was not found within {:?}", path.display(), within);
            Prepared::Silent
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHORT: Duration = Duration::from_millis(200);

    #[tokio::test]
    async fn a_mount_that_never_answers_is_given_up_on_in_time() {
        let (release, held) = std::sync::mpsc::channel::<()>();
        let held = std::sync::Mutex::new(held);
        let hung: Find = Arc::new(move |_, _| {
            let _ = held.lock().unwrap().recv();
            None
        });
        let asked = Instant::now();
        let prepared = prepare(Files { find: hung }, PathBuf::from("/lib/a.mkv"), true, SHORT).await;
        assert!(matches!(prepared, Prepared::Silent), "{prepared:?}");
        assert!(asked.elapsed() < Duration::from_millis(400), "{:?}", asked.elapsed());
        drop(release);
    }

    #[tokio::test]
    async fn a_file_found_is_ready_and_one_out_of_reach_says_so() {
        let found: Find = Arc::new(|path, _| Some((path.to_path_buf(), Vec::new())));
        let prepared = prepare(Files { find: found }, PathBuf::from("/lib/a.mkv"), true, SHORT).await;
        assert!(
            matches!(&prepared, Prepared::Ready(ready) if ready.source == Path::new("/lib/a.mkv")),
            "{prepared:?}"
        );
        let gone: Find = Arc::new(|_, _| None);
        let prepared = prepare(Files { find: gone }, PathBuf::from("/lib/a.mkv"), false, SHORT).await;
        assert!(matches!(prepared, Prepared::OutOfReach), "{prepared:?}");
    }
}
