use super::fmp4::{self, Init};
use super::plan::Plan;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};
use tokio::sync::Notify;

const AHEAD: f64 = 60.0;
const RESUME_BELOW: f64 = 40.0;
const BEHIND: f64 = 45.0;
const REACH: f64 = 30.0;
const MATCH: f64 = 0.3;
const WAIT: Duration = Duration::from_secs(45);
const TICK: Duration = Duration::from_millis(150);
const NICENESS: i32 = 10;

#[derive(Clone, Debug)]
pub struct Spec {
    pub source: PathBuf,
    pub audio: Option<usize>,
    pub hevc: bool,
    pub copy_audio: bool,
}

#[derive(Clone, Debug)]
struct Fragment {
    start: f64,
    dts: i64,
    dts_end: i64,
    offset: i64,
    path: PathBuf,
    last: bool,
    opening: bool,
}

struct Run {
    number: u32,
    from: usize,
    pid: Option<u32>,
    produced: f64,
    paused: bool,
    exited: bool,
}

#[derive(Default)]
struct State {
    init: Option<Arc<Vec<u8>>>,
    timing: Option<Init>,
    codecs: Option<String>,
    fragments: BTreeMap<i64, Fragment>,
    run: Option<Run>,
    runs: u32,
    wanted: f64,
    failed: Option<String>,
    used: Option<Instant>,
}

pub struct Session {
    pub plan: Plan,
    dir: PathBuf,
    spec: Spec,
    state: Mutex<State>,
    changed: Notify,
}

impl Session {
    pub fn new(plan: Plan, dir: PathBuf, spec: Spec) -> Arc<Self> {
        Arc::new(Self {
            plan,
            dir,
            spec,
            state: Mutex::new(State {
                used: Some(Instant::now()),
                ..State::default()
            }),
            changed: Notify::new(),
        })
    }

    pub fn idle(&self) -> Duration {
        self.state
            .lock()
            .unwrap()
            .used
            .map_or(Duration::MAX, |used| used.elapsed())
    }

    pub fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.run = None;
        state.used = None;
        drop(state);
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    pub fn start(self: &Arc<Self>, at: f64) {
        let segment = self.plan.segment_at(at);
        let mut state = self.state.lock().unwrap();
        state.wanted = self.plan.start(segment);
        self.begin(&mut state, segment);
    }

    fn begin(self: &Arc<Self>, state: &mut State, segment: usize) {
        state.runs += 1;
        let number = state.runs;
        if let Err(error) = std::fs::create_dir_all(&self.dir) {
            state.failed = Some(format!("{}: {error}", self.dir.display()));
            return;
        }
        let mut command = self.command(number, self.plan.seek_for(segment));
        unsafe {
            command.pre_exec(|| {
                libc::setpriority(libc::PRIO_PROCESS, 0, NICENESS);
                Ok(())
            });
        }
        match command.spawn() {
            Ok(child) => {
                state.run = Some(Run {
                    number,
                    from: segment,
                    pid: child.id(),
                    produced: self.plan.start(segment),
                    paused: false,
                    exited: false,
                });
                tokio::spawn(self.clone().watch(number, segment > 0, child));
            }
            Err(error) => state.failed = Some(format!("ffmpeg: {error}")),
        }
    }

    fn command(&self, number: u32, seek: Option<f64>) -> Command {
        let mut command = Command::new("ffmpeg");
        command.args(["-nostdin", "-hide_banner", "-loglevel", "error"]);
        if let Some(seek) = seek {
            command.args(["-ss", &format!("{seek:.3}")]);
        }
        command.arg("-copyts").arg("-i").arg(&self.spec.source);
        command.args(["-map", "0:v:0"]);
        if let Some(audio) = self.spec.audio {
            command.args(["-map", &format!("0:a:{audio}")]);
        }
        command.args(["-c:v", "copy"]);
        if self.spec.hevc {
            command.args(["-tag:v", "hvc1"]);
        }
        if self.spec.copy_audio {
            command.args(["-c:a", "copy"]);
        } else {
            command.args(["-c:a", "aac", "-ac", "2", "-b:a", "192k"]);
        }
        command
            .args([
                "-sn",
                "-dn",
                "-avoid_negative_ts",
                "disabled",
                "-max_muxing_queue_size",
                "4096",
                "-f",
                "hls",
                "-hls_time",
                "0.1",
                "-hls_list_size",
                "0",
                "-hls_playlist_type",
                "event",
                "-hls_flags",
                "temp_file",
                "-hls_segment_type",
                "fmp4",
                "-hls_segment_options",
                "movflags=+frag_discont",
                "-hls_fmp4_init_filename",
                &format!("init-{number}.mp4"),
                "-hls_segment_filename",
            ])
            .arg(self.dir.join(format!("run{number}-%d.m4s")))
            .arg(self.playlist(number))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .kill_on_drop(true);
        if let Ok(log) = std::fs::File::create(self.log(number)) {
            command.stderr(log);
        }
        command
    }

    fn log(&self, number: u32) -> PathBuf {
        self.dir.join(format!("run{number}.log"))
    }

    fn playlist(&self, number: u32) -> PathBuf {
        self.dir.join(format!("run{number}.m3u8"))
    }

    async fn watch(self: Arc<Self>, number: u32, discard_first: bool, mut child: Child) {
        let mut listed = 0;
        loop {
            tokio::time::sleep(TICK).await;
            let current = self.state.lock().unwrap().run.as_ref().map(|run| run.number) == Some(number);
            if !current {
                let _ = child.kill().await;
                return;
            }
            let exited = match child.try_wait() {
                Ok(Some(status)) => Some(status.success()),
                Ok(None) => None,
                Err(_) => Some(false),
            };
            listed = self.ingest(number, listed, discard_first, exited == Some(true));
            if let Some(success) = exited {
                let error = std::fs::read_to_string(self.log(number)).unwrap_or_default();
                let mut state = self.state.lock().unwrap();
                if let Some(run) = state.run.as_mut().filter(|run| run.number == number) {
                    run.exited = true;
                    if !success && listed == 0 {
                        let line = error.lines().last().unwrap_or("ffmpeg stopped").to_string();
                        eprintln!("stream: {}: {line}", self.spec.source.display());
                        state.failed = Some(line);
                    }
                }
                drop(state);
                self.changed.notify_waiters();
                return;
            }
            self.throttle(number);
        }
    }

    fn listing(&self, number: u32) -> Vec<String> {
        std::fs::read_to_string(self.playlist(number))
            .unwrap_or_default()
            .lines()
            .filter(|line| !line.starts_with('#') && line.ends_with(".m4s"))
            .map(str::to_string)
            .collect()
    }

    fn ingest(&self, number: u32, listed: usize, discard_first: bool, ended: bool) -> usize {
        let names = self.listing(number);
        if names.len() <= listed {
            return listed;
        }
        let Some((own, bytes)) = std::fs::read(self.dir.join(format!("init-{number}.mp4")))
            .ok()
            .and_then(|bytes| Some((fmp4::init(&bytes)?, bytes)))
        else {
            return listed;
        };
        let mut state = self.state.lock().unwrap();
        if state.init.is_none() {
            state.codecs = fmp4::codecs(&bytes);
            state.timing = Some(own);
            state.init = Some(Arc::new(bytes));
        }
        let Some(init) = state.timing else {
            return listed;
        };
        if own.timescale != init.timescale || own.video != init.video {
            state.failed = Some("the video changed its timing between runs".to_string());
            return listed;
        }
        let offset = own.shift - init.shift;
        for (position, name) in names.iter().enumerate().skip(listed) {
            let path = self.dir.join(name);
            if position == 0 && discard_first {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            let Some(timing) = std::fs::read(&path).ok().and_then(|bytes| fmp4::timing(&bytes, &own)) else {
                continue;
            };
            let fragment = Fragment {
                start: timing.start,
                dts: timing.dts - offset,
                dts_end: timing.dts_end - offset,
                offset,
                path,
                last: ended && position + 1 == names.len(),
                opening: position == 0 && !discard_first,
            };
            let end = timing.start + (timing.dts_end - timing.dts) as f64 / f64::from(init.timescale);
            if let Some(run) = state.run.as_mut().filter(|run| run.number == number) {
                run.produced = run.produced.max(end);
            }
            let kept = fragment.path.clone();
            if let Some(old) = state.fragments.insert(timing.dts - offset, fragment)
                && old.path != kept
            {
                let _ = std::fs::remove_file(old.path);
            }
        }
        self.tidy(&mut state, init);
        drop(state);
        self.changed.notify_waiters();
        names.len()
    }

    fn tidy(&self, state: &mut State, init: Init) {
        let wanted = state.wanted;
        let scale = f64::from(init.timescale);
        let stale: Vec<i64> = state
            .fragments
            .values()
            .filter(|fragment| {
                let end = fragment.start + (fragment.dts_end - fragment.dts) as f64 / scale;
                end < wanted - BEHIND || fragment.start > wanted + AHEAD + REACH
            })
            .map(|fragment| fragment.dts)
            .collect();
        for dts in stale {
            if let Some(fragment) = state.fragments.remove(&dts) {
                let _ = std::fs::remove_file(fragment.path);
            }
        }
    }

    fn throttle(&self, number: u32) {
        let mut state = self.state.lock().unwrap();
        let wanted = state.wanted;
        let Some(run) = state.run.as_mut().filter(|run| run.number == number && !run.exited) else {
            return;
        };
        let Some(pid) = run.pid else {
            return;
        };
        let ahead = run.produced - wanted;
        if !run.paused && ahead > AHEAD {
            unsafe { libc::kill(pid as i32, libc::SIGSTOP) };
            run.paused = true;
        } else if run.paused && ahead < RESUME_BELOW {
            unsafe { libc::kill(pid as i32, libc::SIGCONT) };
            run.paused = false;
        }
    }

    fn assemble(&self, state: &State, segment: usize) -> Option<Vec<u8>> {
        let start = self.plan.start(segment);
        let last_segment = segment + 1 == self.plan.len();
        let first = state.fragments.values().find(|fragment| {
            if segment == 0 {
                fragment.opening
            } else {
                (fragment.start - start).abs() < MATCH
            }
        })?;
        let mut chain = vec![first];
        loop {
            let tail = chain.last().unwrap();
            if tail.last {
                if !last_segment {
                    return None;
                }
                break;
            }
            let next = state.fragments.get(&tail.dts_end)?;
            if self.plan.segment_at(next.start + MATCH) != segment {
                break;
            }
            chain.push(next);
        }
        let mut sequence = segment as u32 * 64 + 1;
        let mut bytes = Vec::new();
        for fragment in chain {
            let video = state.timing?.video;
            bytes.extend(fmp4::media(
                &std::fs::read(&fragment.path).ok()?,
                &mut sequence,
                video,
                fragment.offset,
            ));
        }
        Some(bytes)
    }

    fn should_restart(&self, state: &State, segment: usize) -> bool {
        let start = self.plan.start(segment);
        match &state.run {
            None => true,
            Some(run) => run.from > segment || run.exited || run.produced < start - REACH,
        }
    }

    fn touch(&self, state: &mut State, segment: usize) {
        state.used = Some(Instant::now());
        state.wanted = self.plan.start(segment);
    }

    pub async fn segment(self: &Arc<Self>, segment: usize) -> Result<Vec<u8>, String> {
        if segment >= self.plan.len() {
            return Err(format!("no segment {segment}"));
        }
        let deadline = Instant::now() + WAIT;
        loop {
            let changed = self.changed.notified();
            {
                let mut state = self.state.lock().unwrap();
                self.touch(&mut state, segment);
                if let Some(bytes) = self.assemble(&state, segment) {
                    return Ok(bytes);
                }
                if let Some(error) = state.failed.take() {
                    return Err(error);
                }
                if self.should_restart(&state, segment) {
                    self.begin(&mut state, segment);
                }
            }
            let running = self.state.lock().unwrap().run.as_ref().map(|run| run.number);
            if let Some(number) = running {
                self.throttle(number);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(format!("segment {segment} took too long"));
            }
            let _ = tokio::time::timeout(left.min(Duration::from_secs(1)), changed).await;
        }
    }

    pub async fn init(self: &Arc<Self>, start: f64) -> Result<(Arc<Vec<u8>>, Option<String>), String> {
        let deadline = Instant::now() + WAIT;
        loop {
            let changed = self.changed.notified();
            {
                let mut state = self.state.lock().unwrap();
                state.used = Some(Instant::now());
                if let Some(init) = &state.init {
                    return Ok((init.clone(), state.codecs.clone()));
                }
                if let Some(error) = state.failed.take() {
                    return Err(error);
                }
                if state.run.as_ref().is_none_or(|run| run.exited) {
                    let segment = self.plan.segment_at(start);
                    self.begin(&mut state, segment);
                }
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("the video did not start in time".to_string());
            }
            let _ = tokio::time::timeout(left.min(Duration::from_secs(1)), changed).await;
        }
    }
}

pub fn clear(root: &Path) {
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stream::index;

    async fn joined(session: &Arc<Session>, at: f64, order: &[usize], out: &std::path::Path) {
        let (init, codecs) = session.init(at).await.unwrap();
        eprintln!("codecs {codecs:?}");
        let mut parts = vec![Vec::new(); session.plan.len()];
        for segment in order {
            parts[*segment] = session.segment(*segment).await.unwrap();
            assert!(!parts[*segment].is_empty(), "segment {segment} is empty");
        }
        std::fs::create_dir_all(out).unwrap();
        let mut joined = init.as_ref().clone();
        for (segment, bytes) in parts.iter().enumerate() {
            joined.extend_from_slice(bytes);
            std::fs::write(out.join(format!("{segment}.m4s")), bytes).unwrap();
        }
        std::fs::write(out.join("init.mp4"), init.as_ref()).unwrap();
        std::fs::write(out.join("joined.mp4"), joined).unwrap();
        std::fs::write(out.join("video.m3u8"), session.plan.media_playlist(0.0)).unwrap();
    }

    #[tokio::test]
    #[ignore = "needs ffmpeg and SEMMELTV_STREAM_SAMPLE"]
    async fn a_session_serves_every_segment_whichever_point_playback_starts_from() {
        let sample = PathBuf::from(std::env::var("SEMMELTV_STREAM_SAMPLE").unwrap());
        let plan = Plan::new(&index::read(&sample).unwrap());
        let count = plan.len();
        assert!(count > 3, "{count} segments");
        let spec = Spec {
            source: sample.clone(),
            audio: Some(0),
            hevc: true,
            copy_audio: true,
        };
        let root = std::env::temp_dir().join(format!("semmeltv-session-{}", std::process::id()));
        let out = std::env::var("SEMMELTV_STREAM_OUT")
            .map(PathBuf::from)
            .unwrap_or(root.join("out"));
        let straight = Session::new(plan.clone(), root.join("straight"), spec.clone());
        straight.start(0.0);
        joined(&straight, 0.0, &(0..count).collect::<Vec<_>>(), &out.join("straight")).await;
        let jumped = Session::new(plan.clone(), root.join("jumped"), spec);
        let middle = count / 2;
        jumped.start(plan.start(middle));
        let order: Vec<usize> = (middle..count).chain(0..middle).collect();
        joined(&jumped, plan.start(middle), &order, &out.join("jumped")).await;
        straight.close();
        jumped.close();
    }
}
