//! What a play needs from its files, found on tasks of their own and within a
//! time limit, so the actor never waits on the pCloud mount.

use crate::offline::{Source, Store};
use crate::sidecar::{self, Sidecar};
use crate::stream::probe::{self, Media};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::spawn_blocking;
use tokio::time::timeout;

/// How long finding a file and its subtitles may take.
pub const FIND_WITHIN: Duration = Duration::from_secs(4);
/// How long reading what a file holds may take.
pub const PROBE_WITHIN: Duration = Duration::from_secs(4);

/// Where a file plays from, and the subtitle files beside it; `None` when it
/// is out of reach. Called with the file and whether the TV is online.
pub type Find = Arc<dyn Fn(&Path, bool) -> Option<(PathBuf, Vec<Sidecar>)> + Send + Sync>;
/// What a file holds.
pub type Probe = Arc<dyn Fn(PathBuf) -> Pin<Box<dyn Future<Output = Result<Media, String>> + Send>> + Send + Sync>;

#[derive(Clone)]
pub struct Files {
    pub find: Find,
    pub probe: Probe,
}

impl Files {
    /// The files of `store`, with `playable` naming the copy to play.
    pub fn of(store: Store, playable: impl Fn(&Path) -> PathBuf + Send + Sync + 'static) -> Self {
        Self {
            find: Arc::new(move |path, online| {
                let here = playable(path);
                let source = match store.source(path, online) {
                    _ if here != path => here,
                    Source::Stream => path.to_path_buf(),
                    Source::Cached(file) => file,
                    Source::Unavailable => return None,
                };
                let sidecars = sidecar::find(&source);
                Some((source, sidecars))
            }),
            probe: Arc::new(|path| Box::pin(async move { probe::probe(&path).await })),
        }
    }
}

#[derive(Debug)]
pub enum Prepared {
    Ready(Box<Ready>),
    OutOfReach,
    Silent,
}

/// A file found: where it plays from, its subtitle files, and what it holds
/// when that was asked for and read in time.
#[derive(Debug)]
pub struct Ready {
    pub source: PathBuf,
    pub sidecars: Vec<Sidecar>,
    pub media: Option<Media>,
}

/// Finds `path`, and with `probing` reads what it holds. A file that is not
/// found within `within.0` is `Silent`; one whose reading takes longer than
/// `within.1` is ready without what it holds.
pub async fn prepare(
    files: Files,
    path: PathBuf,
    online: bool,
    probing: bool,
    within: (Duration, Duration),
) -> Prepared {
    let find = files.find.clone();
    let looking = path.clone();
    let (source, sidecars) = match timeout(within.0, spawn_blocking(move || find(&looking, online))).await {
        Ok(Ok(Some(found))) => found,
        Ok(Ok(None)) => return Prepared::OutOfReach,
        Ok(Err(_)) => return Prepared::Silent,
        Err(_) => {
            eprintln!("tv player: {} was not found within {:?}", path.display(), within.0);
            return Prepared::Silent;
        }
    };
    let media = if probing {
        match timeout(within.1, (files.probe)(source.clone())).await {
            Ok(Ok(media)) => Some(media),
            Ok(Err(error)) => {
                eprintln!("tv player: {}: {error}", source.display());
                None
            }
            Err(_) => {
                eprintln!(
                    "tv player: {} said nothing about what it holds within {:?}",
                    source.display(),
                    within.1
                );
                None
            }
        }
    } else {
        None
    };
    Prepared::Ready(Box::new(Ready {
        source,
        sidecars,
        media,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const SHORT: (Duration, Duration) = (Duration::from_millis(200), Duration::from_millis(200));

    fn files(find: Find, probe: Probe) -> Files {
        Files { find, probe }
    }

    fn found() -> Find {
        Arc::new(|path, _| Some((path.to_path_buf(), Vec::new())))
    }

    fn never_probed() -> Probe {
        Arc::new(|_| Box::pin(std::future::pending()))
    }

    #[tokio::test]
    async fn a_mount_that_never_answers_is_given_up_on_in_time() {
        let (release, held) = std::sync::mpsc::channel::<()>();
        let held = std::sync::Mutex::new(held);
        let hung: Find = Arc::new(move |_, _| {
            let _ = held.lock().unwrap().recv();
            None
        });
        let asked = Instant::now();
        let prepared = prepare(
            files(hung, never_probed()),
            PathBuf::from("/lib/a.mkv"),
            true,
            true,
            SHORT,
        )
        .await;
        assert!(matches!(prepared, Prepared::Silent), "{prepared:?}");
        assert!(asked.elapsed() < Duration::from_millis(400), "{:?}", asked.elapsed());
        drop(release);
    }

    #[tokio::test]
    async fn a_probe_that_never_answers_leaves_the_file_to_mpv() {
        let asked = Instant::now();
        let prepared = prepare(
            files(found(), never_probed()),
            PathBuf::from("/lib/a.mkv"),
            true,
            true,
            SHORT,
        )
        .await;
        let Prepared::Ready(ready) = prepared else {
            panic!("{prepared:?}");
        };
        let Ready { source, media, .. } = *ready;
        assert_eq!(source, PathBuf::from("/lib/a.mkv"));
        assert!(media.is_none());
        assert!(asked.elapsed() < Duration::from_millis(400), "{:?}", asked.elapsed());
    }

    #[tokio::test]
    async fn nothing_is_probed_unless_asked_and_a_file_out_of_reach_says_so() {
        let prepared = prepare(
            files(found(), never_probed()),
            PathBuf::from("/lib/a.mkv"),
            true,
            false,
            SHORT,
        )
        .await;
        assert!(
            matches!(&prepared, Prepared::Ready(ready) if ready.media.is_none()),
            "{prepared:?}"
        );
        let gone: Find = Arc::new(|_, _| None);
        let prepared = prepare(
            files(gone, never_probed()),
            PathBuf::from("/lib/a.mkv"),
            false,
            true,
            SHORT,
        )
        .await;
        assert!(matches!(prepared, Prepared::OutOfReach), "{prepared:?}");
    }
}
