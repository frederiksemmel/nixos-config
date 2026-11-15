use super::fmp4::{self, Init};
use super::plan::Plan;
use super::vtt::{self, Cue};
use crate::audio;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
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
    pub dialogue: Arc<AtomicBool>,
    /// The embedded text subtitle tracks to write out as WebVTT while remuxing.
    pub subtitles: Vec<usize>,
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
    dialogue: bool,
}

struct Run {
    number: u32,
    from: usize,
    starts: f64,
    pid: Option<u32>,
    produced: f64,
    paused: bool,
    exited: bool,
    dialogue: bool,
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
    reach: f64,
    covered: Vec<(f64, f64)>,
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

    pub fn keep_alive(&self) {
        self.state.lock().unwrap().used = Some(Instant::now());
    }

    pub fn rest(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        retire(&mut state);
        let busy = state.run.take().is_some() || !state.fragments.is_empty();
        for (_, fragment) in std::mem::take(&mut state.fragments) {
            let _ = std::fs::remove_file(fragment.path);
        }
        busy
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
        retire(state);
        state.runs += 1;
        let number = state.runs;
        if let Err(error) = std::fs::create_dir_all(&self.dir) {
            state.failed = Some(format!("{}: {error}", self.dir.display()));
            return;
        }
        let dialogue = self.dialogue();
        let mut command = self.command(number, self.plan.seek_for(segment), dialogue);
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
                    starts: self.plan.start(segment),
                    pid: child.id(),
                    produced: self.plan.start(segment),
                    paused: false,
                    exited: false,
                    dialogue,
                });
                tokio::spawn(self.clone().watch(number, segment > 0, child));
            }
            Err(error) => state.failed = Some(format!("ffmpeg: {error}")),
        }
    }

    fn command(&self, number: u32, seek: Option<f64>, dialogue: bool) -> Command {
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
            command.args(["-af", audio::stereo(dialogue), "-c:a", "aac", "-b:a", "256k"]);
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
            .arg(self.playlist(number));
        for track in &self.spec.subtitles {
            command
                .args(["-map", &format!("0:s:{track}"), "-c:s", "webvtt", "-flush_packets", "1", "-f", "webvtt"])
                .arg(self.subtitle_file(number, *track));
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .kill_on_drop(true);
        if let Ok(log) = std::fs::File::create(self.log(number)) {
            command.stderr(log);
        }
        command
    }

    fn subtitle_file(&self, number: u32, track: usize) -> PathBuf {
        self.dir.join(format!("run{number}-sub{track}.vtt"))
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
                    if success {
                        run.produced = self.plan.duration;
                    }
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
        let Some(run_dialogue) = state
            .run
            .as_ref()
            .filter(|run| run.number == number)
            .map(|run| run.dialogue)
        else {
            return listed;
        };
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
                dialogue: run_dialogue,
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
                end < wanted - BEHIND
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
        let target = (state.wanted + AHEAD).max(state.reach);
        let Some(run) = state.run.as_mut().filter(|run| run.number == number && !run.exited) else {
            return;
        };
        let Some(pid) = run.pid else {
            return;
        };
        if !run.paused && run.produced > target {
            unsafe { libc::kill(pid as i32, libc::SIGSTOP) };
            run.paused = true;
        } else if run.paused && run.produced < target - (AHEAD - RESUME_BELOW) {
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

    /// Whether a segment that could not be assembled needs a new run. A run
    /// that has already passed the segment's end left a hole there, so
    /// waiting for it would never help.
    fn should_restart(&self, state: &State, segment: usize) -> bool {
        let start = self.plan.start(segment);
        match &state.run {
            None => true,
            Some(run) => {
                run.from > segment
                    || run.exited
                    || run.produced < start - REACH
                    || run.produced > self.plan.end(segment) + MATCH
            }
        }
    }

    fn dialogue(&self) -> bool {
        !self.spec.copy_audio && self.spec.dialogue.load(Ordering::SeqCst)
    }

    fn follow_dialogue(&self, state: &mut State, segment: usize) {
        let on = self.dialogue();
        let from = self.plan.start(segment) - MATCH;
        let stale: Vec<i64> = state
            .fragments
            .values()
            .filter(|fragment| fragment.dialogue != on && fragment.start >= from)
            .map(|fragment| fragment.dts)
            .collect();
        for dts in stale {
            if let Some(fragment) = state.fragments.remove(&dts) {
                let _ = std::fs::remove_file(fragment.path);
            }
        }
        if state.run.as_ref().is_some_and(|run| run.dialogue != on) {
            retire(state);
            state.run = None;
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
                self.follow_dialogue(&mut state, segment);
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

    /// The cues of embedded subtitle `track` that start between `from` and
    /// `to`, once the remux has passed `to` (or, after a while, whatever it has).
    pub async fn subtitles(self: &Arc<Self>, track: usize, from: f64, to: f64) -> Vec<Cue> {
        let deadline = Instant::now() + WAIT;
        let runs = loop {
            let changed = self.changed.notified();
            let running = {
                let mut state = self.state.lock().unwrap();
                state.used = Some(Instant::now());
                if covers(&coverage(&state), from, to) || Instant::now() >= deadline {
                    state.reach = 0.0;
                    break state.runs;
                }
                state.reach = state.reach.max(to);
                if state.run.as_ref().is_none_or(|run| run.exited && run.produced < to) {
                    self.begin(&mut state, self.plan.segment_at(from));
                }
                state.run.as_ref().map(|run| run.number)
            };
            if let Some(number) = running {
                self.throttle(number);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            let _ = tokio::time::timeout(left.min(Duration::from_secs(1)), changed).await;
        };
        let mut seen = std::collections::HashSet::new();
        let mut found = Vec::new();
        for number in 1..=runs {
            let Ok(text) = std::fs::read_to_string(self.subtitle_file(number, track)) else {
                continue;
            };
            for cue in vtt::cues(&text) {
                if cue.start >= from && cue.start < to && seen.insert(((cue.start * 1000.0).round() as i64, cue.text.join("\n"))) {
                    found.push(cue);
                }
            }
        }
        found.sort_by(|a, b| a.start.total_cmp(&b.start));
        found
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

/// Keeps what a run that is being replaced or stopped has remuxed.
fn retire(state: &mut State) {
    if let Some(run) = &state.run {
        state.covered.push((run.starts, run.produced));
    }
}

/// Every stretch of the video the remux has passed, in this run and before.
fn coverage(state: &State) -> Vec<(f64, f64)> {
    let mut stretches = state.covered.clone();
    if let Some(run) = &state.run {
        stretches.push((run.starts, run.produced));
    }
    stretches
}

/// Whether `stretches` together cover `from` to `to`.
fn covers(stretches: &[(f64, f64)], from: f64, to: f64) -> bool {
    let mut sorted = stretches.to_vec();
    sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut reached = from;
    for (start, end) in sorted {
        if start > reached + MATCH {
            break;
        }
        reached = reached.max(end);
        if reached >= to - MATCH {
            return true;
        }
    }
    reached >= to - MATCH
}

pub fn clear(root: &Path) {
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rounders() -> Arc<Session> {
        converted(true, Arc::default())
    }

    fn converted(copy_audio: bool, dialogue: Arc<AtomicBool>) -> Arc<Session> {
        let keyframes: Vec<f64> = (0..400).map(|n| f64::from(n) * 10.0).collect();
        let plan = Plan::new(&index::Index {
            duration: 4000.0,
            keyframes,
        });
        let spec = Spec {
            source: PathBuf::from("/nonexistent.mp4"),
            audio: Some(0),
            hevc: true,
            copy_audio,
            dialogue,
            subtitles: Vec::new(),
        };
        Session::new(plan, PathBuf::from("/nonexistent"), spec)
    }

    fn run_through(produced: f64) -> Run {
        Run {
            number: 1,
            from: 281,
            starts: 2810.0,
            pid: None,
            produced,
            paused: true,
            exited: false,
            dialogue: false,
        }
    }

    fn remuxed(state: &mut State, from: u32, to: u32, dialogue: bool) {
        for n in from..to {
            let start = f64::from(n) * 10.0;
            let dts = (start * 1000.0) as i64;
            state.fragments.insert(
                dts,
                Fragment {
                    start,
                    dts,
                    dts_end: dts + 10_000,
                    offset: 0,
                    path: PathBuf::from(format!("/nonexistent/run1-{n}.m4s")),
                    last: false,
                    opening: false,
                    dialogue,
                },
            );
        }
    }

    #[test]
    fn switching_dialogue_mode_drops_what_lies_ahead_and_stops_the_old_remux() {
        let dialogue = Arc::new(AtomicBool::new(false));
        let session = converted(false, dialogue.clone());
        let mut state = State {
            run: Some(run_through(3000.0)),
            ..State::default()
        };
        remuxed(&mut state, 281, 300, false);
        let segment = session.plan.segment_at(2900.0);
        session.follow_dialogue(&mut state, segment);
        assert_eq!(state.fragments.len(), 19, "nothing changed, so nothing goes");
        assert!(state.run.is_some());
        dialogue.store(true, Ordering::SeqCst);
        session.follow_dialogue(&mut state, segment);
        let from = session.plan.start(segment) - MATCH;
        assert!(state.fragments.values().all(|fragment| fragment.start < from));
        assert!(!state.fragments.is_empty(), "what was served before the switch stays");
        assert!(state.run.is_none());
        assert!(session.should_restart(&state, segment));
    }

    #[test]
    fn a_copied_e_ac3_stream_ignores_dialogue_mode() {
        let session = converted(true, Arc::new(AtomicBool::new(true)));
        let mut state = State {
            run: Some(run_through(3000.0)),
            ..State::default()
        };
        remuxed(&mut state, 281, 300, false);
        session.follow_dialogue(&mut state, session.plan.segment_at(2900.0));
        assert_eq!(state.fragments.len(), 19);
        assert!(state.run.is_some());
    }

    #[test]
    fn a_remux_that_runs_far_ahead_keeps_every_fragment_until_playback_has_passed_it() {
        let session = rounders();
        let timing = Init {
            video: 1,
            timescale: 1000,
            shift: 0,
        };
        let mut state = State {
            wanted: 2820.0,
            ..State::default()
        };
        for n in 281..303 {
            let start = f64::from(n) * 10.0;
            let dts = (start * 1000.0) as i64;
            state.fragments.insert(
                dts,
                Fragment {
                    start,
                    dts,
                    dts_end: dts + 10_000,
                    offset: 0,
                    path: PathBuf::from(format!("/nonexistent/run1-{n}.m4s")),
                    last: false,
                    opening: false,
                    dialogue: false,
                },
            );
        }
        session.tidy(&mut state, timing);
        assert_eq!(state.fragments.len(), 22, "a fragment ahead of playback was dropped");
        state.wanted = 2900.0;
        session.tidy(&mut state, timing);
        assert_eq!(state.fragments.values().next().unwrap().start, 2850.0);
    }

    #[test]
    fn a_segment_the_run_has_already_passed_restarts_the_remux_instead_of_waiting() {
        let session = rounders();
        let state = State {
            run: Some(run_through(3022.0)),
            ..State::default()
        };
        assert!(session.should_restart(&state, 291));
        let state = State {
            run: Some(run_through(2905.0)),
            ..State::default()
        };
        assert!(!session.should_restart(&state, 291), "the run is still making segment 291");
        assert!(!session.should_restart(&state, 293), "the run is close enough to reach segment 293");
    }

    #[test]
    fn subtitles_wait_until_the_remux_has_passed_their_piece() {
        assert!(covers(&[(0.0, 65.0)], 30.0, 60.0));
        assert!(!covers(&[(0.0, 45.0)], 30.0, 60.0));
        assert!(covers(&[(0.0, 40.0), (39.9, 90.0)], 30.0, 60.0));
        assert!(!covers(&[(0.0, 35.0), (50.0, 90.0)], 30.0, 60.0));
        assert!(!covers(&[(32.0, 90.0)], 30.0, 60.0));
        assert!(covers(&[(600.0, 700.0), (0.0, 61.0)], 30.0, 60.0));
    }
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
            dialogue: Arc::default(),
            subtitles: Vec::new(),
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
        jumped.rest();
        assert!(!jumped.segment(middle).await.unwrap().is_empty());
        assert!(!jumped.segment(0).await.unwrap().is_empty());
        straight.close();
        jumped.close();
    }

    #[tokio::test]
    #[ignore = "needs ffmpeg and SEMMELTV_STREAM_SAMPLE"]
    async fn switching_dialogue_mode_remakes_the_segments_not_yet_served() {
        let sample = PathBuf::from(std::env::var("SEMMELTV_STREAM_SAMPLE").unwrap());
        let plan = Plan::new(&index::read(&sample).unwrap());
        let count = plan.len();
        assert!(count > 2, "{count} segments");
        let dialogue = Arc::new(AtomicBool::new(false));
        let spec = Spec {
            source: sample,
            audio: Some(0),
            hevc: true,
            copy_audio: false,
            dialogue: dialogue.clone(),
            subtitles: Vec::new(),
        };
        let root = std::env::temp_dir().join(format!("semmeltv-dialogue-{}", std::process::id()));
        let session = Session::new(plan, root.clone(), spec);
        session.start(0.0);
        session.init(0.0).await.unwrap();
        assert!(!session.segment(0).await.unwrap().is_empty());
        assert!(!session.segment(1).await.unwrap().is_empty());
        let made_before = |state: &State| state.fragments.values().filter(|fragment| !fragment.dialogue).count();
        assert!(made_before(&session.state.lock().unwrap()) > 0);
        dialogue.store(true, Ordering::SeqCst);
        assert!(!session.segment(2).await.unwrap().is_empty());
        {
            let state = session.state.lock().unwrap();
            assert!(state.run.as_ref().is_some_and(|run| run.dialogue && run.from == 2));
            let from = session.plan.start(2) - MATCH;
            assert!(
                state
                    .fragments
                    .values()
                    .filter(|fragment| fragment.start >= from)
                    .all(|fragment| fragment.dialogue)
            );
        }
        assert!(!session.segment(1).await.unwrap().is_empty());
        session.close();
        let _ = std::fs::remove_dir_all(root);
    }
}
