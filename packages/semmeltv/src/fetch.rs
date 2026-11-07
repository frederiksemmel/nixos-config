use crate::offline::Store;
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

const CHUNK_BYTES: usize = 1 << 20;
const CHUNK_TIME: Duration = Duration::from_millis(100);

enum Outcome {
    Finished,
    Replaced(Vec<PathBuf>),
    Closed,
    Failed(io::Error),
}

pub fn spawn(store: Store, report: impl Fn(Option<PathBuf>) + Send + 'static) -> Sender<Vec<PathBuf>> {
    let (sender, receiver) = mpsc::channel::<Vec<PathBuf>>();
    thread::spawn(move || {
        for paths in receiver.iter() {
            let mut queue = VecDeque::from(paths);
            while let Some(path) = queue.pop_front() {
                match read_one(&store, &path, &receiver, &report) {
                    Some(Outcome::Replaced(paths)) => queue = paths.into(),
                    Some(Outcome::Failed(error)) => {
                        eprintln!("{}: stopped downloading: {error}", path.display());
                        queue.clear();
                    }
                    Some(Outcome::Closed) => return,
                    Some(Outcome::Finished) | None => {}
                }
            }
            report(None);
        }
    });
    sender
}

fn read_one(
    store: &Store,
    path: &Path,
    jobs: &Receiver<Vec<PathBuf>>,
    report: &impl Fn(Option<PathBuf>),
) -> Option<Outcome> {
    if store.is_downloaded(path) {
        return None;
    }
    let size = match fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(error) => return Some(Outcome::Failed(error)),
    };
    if !store.fits(size) {
        eprintln!("{}: larger than the offline cache, not downloading", path.display());
        return None;
    }
    report(Some(path.to_path_buf()));
    Some(match File::open(path) {
        Ok(file) => read_paced(file, jobs),
        Err(error) => Outcome::Failed(error),
    })
}

fn read_paced(mut file: File, jobs: &Receiver<Vec<PathBuf>>) -> Outcome {
    let mut buffer = vec![0; CHUNK_BYTES];
    loop {
        let started = Instant::now();
        match jobs.try_recv() {
            Ok(paths) => return Outcome::Replaced(paths),
            Err(TryRecvError::Disconnected) => return Outcome::Closed,
            Err(TryRecvError::Empty) => {}
        }
        match file.read(&mut buffer) {
            Ok(0) => return Outcome::Finished,
            Ok(_) => thread::sleep(CHUNK_TIME.saturating_sub(started.elapsed())),
            Err(error) => return Outcome::Failed(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn wait_until(mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < deadline, "timed out");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn it_reads_each_file_and_reports_when_idle() {
        let root = std::env::temp_dir().join(format!("semmeltv-fetch-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("big.mkv"), vec![0; 2048]).unwrap();
        fs::write(root.join("small.mkv"), vec![0; 16]).unwrap();
        let store = Store::new(root.clone(), Some(root.join("vfs")), Some(1024));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let jobs = spawn(store, move |path| log.lock().unwrap().push(path));
        jobs.send(vec![root.join("big.mkv"), root.join("small.mkv")]).unwrap();
        wait_until(|| seen.lock().unwrap().last().is_some_and(Option::is_none));
        assert_eq!(*seen.lock().unwrap(), [Some(root.join("small.mkv")), None]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_missing_file_stops_the_queue_without_a_panic() {
        let root = std::env::temp_dir().join(format!("semmeltv-fetch-gone-{}", std::process::id()));
        let store = Store::new(root.clone(), Some(root.join("vfs")), None);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let jobs = spawn(store, move |path| log.lock().unwrap().push(path));
        jobs.send(vec![root.join("gone.mkv"), root.join("next.mkv")]).unwrap();
        wait_until(|| !seen.lock().unwrap().is_empty());
        assert_eq!(*seen.lock().unwrap(), [None]);
    }
}
