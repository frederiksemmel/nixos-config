//! Database writes behind the core's actor: one thread runs them in the order
//! they were sent, so a write that waits on the SD card never holds up the
//! actor, and later writes still land after earlier ones. The thread runs
//! them on the core's runtime, so what a write starts there (toasty opens a
//! connection with a task of its own) keeps running after the write.

use crate::db::Db;
use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc::{self, Sender};

type Job = Box<dyn FnOnce(Db) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

#[derive(Clone)]
pub struct Saves {
    jobs: Sender<Job>,
}

impl Saves {
    /// Starts the writer for the runtime this is called on.
    pub fn start(db: Db) -> Self {
        let (jobs, queued) = mpsc::channel::<Job>();
        let runtime = tokio::runtime::Handle::current();
        let spawned = std::thread::Builder::new().name("saves".to_string()).spawn(move || {
            for job in queued {
                runtime.block_on(job(db.clone()));
            }
        });
        if let Err(error) = spawned {
            eprintln!("saves: {error}");
        }
        Self { jobs }
    }

    /// Runs `job` after every job sent before it.
    pub fn save<F, Done>(&self, job: F)
    where
        F: FnOnce(Db) -> Done + Send + 'static,
        Done: Future<Output = ()> + Send + 'static,
    {
        let boxed: Job = Box::new(move |db| Box::pin(job(db)));
        if self.jobs.send(boxed).is_err() {
            eprintln!("saves: the writer is gone, a write is lost");
        }
    }

    /// Returns once every write sent before has landed.
    #[cfg(test)]
    pub async fn flushed(&self) {
        let (done, landed) = tokio::sync::oneshot::channel::<()>();
        self.save(move |_| async move {
            let _ = done.send(());
        });
        let _ = landed.await;
    }

    /// Holds every later write until the returned sender is dropped, as an SD
    /// card that stops answering does.
    #[cfg(test)]
    pub fn hold(&self) -> Sender<()> {
        let (release, held) = mpsc::channel::<()>();
        self.save(move |_| async move {
            let _ = tokio::task::spawn_blocking(move || held.recv()).await;
        });
        release
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn writes_run_in_order_and_a_held_one_holds_only_the_later_ones() {
        let saves = Saves::start(Db::memory().await);
        let done = Arc::new(Mutex::new(Vec::new()));
        let first = done.clone();
        saves.save(move |_| async move { first.lock().unwrap().push(1) });
        let release = saves.hold();
        let second = done.clone();
        let asked = Instant::now();
        saves.save(move |_| async move { second.lock().unwrap().push(2) });
        assert!(asked.elapsed() < Duration::from_millis(10));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(*done.lock().unwrap(), [1]);
        drop(release);
        let deadline = Instant::now() + Duration::from_secs(2);
        while done.lock().unwrap().len() < 2 {
            assert!(Instant::now() < deadline, "the held write never landed");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(*done.lock().unwrap(), [1, 2]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn connections_first_opened_inside_a_write_still_answer_reads_after_it() {
        let dir = std::env::temp_dir().join(format!("semmeltv-saves-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Db::open(&dir.join("semmeltv.db")).await.unwrap();
        let saves = Saves::start(db.clone());
        saves.save(move |db| async move {
            let _ = tokio::join!(db.profiles(), db.profiles(), db.profiles(), db.profiles());
        });
        saves.flushed().await;
        let reads = async {
            tokio::join!(
                db.profiles(),
                db.profiles(),
                db.profiles(),
                db.profiles(),
                db.profiles(),
                db.profiles(),
                db.profiles(),
                db.profiles()
            )
        };
        let read = tokio::time::timeout(Duration::from_secs(5), reads).await;
        assert!(read.is_ok(), "a read waited on a connection the write opened");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_task_a_write_starts_keeps_running_after_the_write() {
        let saves = Saves::start(Db::memory().await);
        let (started, starting) = tokio::sync::oneshot::channel();
        let (go, going) = tokio::sync::oneshot::channel::<()>();
        let (answer, answered) = tokio::sync::oneshot::channel();
        saves.save(move |_| async move {
            tokio::spawn(async move {
                let _ = going.await;
                let _ = answer.send("still running");
            });
            let _ = started.send(());
        });
        starting.await.unwrap();
        saves.flushed().await;
        go.send(()).unwrap();
        let said = tokio::time::timeout(Duration::from_secs(2), answered).await;
        assert_eq!(said.map(Result::ok), Ok(Some("still running")));
    }
}
