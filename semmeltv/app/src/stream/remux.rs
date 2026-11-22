//! One phone stream: ffmpeg copies the video into HLS with its own muxer,
//! from a point in the file to its end, and writes the playlist itself.

use super::codecs;
use super::vtt::{self, Cue};
use crate::audio;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::sync::Notify;

const WAIT: Duration = Duration::from_secs(45);
/// The media segments a stream has before it is handed to a player. hls.js
/// reads an EVENT playlist again only a target duration (10 s on most files)
/// after the first read, so it plays only what that first read listed.
pub const READY_SEGMENTS: usize = 3;
const POLL: Duration = Duration::from_millis(100);
const NICENESS: i32 = 10;
pub const PLAYLIST: &str = "video.m3u8";
const INIT: &str = "init.mp4";
/// How much of the film a stream keeps behind the newest segment its viewer
/// fetched, and how far ffmpeg may run ahead of it before it waits.
const BEHIND: f64 = 90.0;
const AHEAD: f64 = 180.0;
const SLACK: f64 = 30.0;
const KEEPING: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub source: PathBuf,
    pub audio: Option<usize>,
    pub hevc: bool,
    pub copy_audio: bool,
    pub dialogue: bool,
    pub surround: bool,
    /// The embedded text subtitle tracks to write out as WebVTT on the way.
    pub subtitles: Vec<usize>,
}

pub struct Remux {
    dir: PathBuf,
    pub spec: Spec,
    /// The file time at which the stream's own timeline starts.
    pub offset: f64,
    pub codecs: Option<String>,
    exit: Arc<Mutex<Option<bool>>>,
    stop: Arc<Notify>,
    used: Mutex<Instant>,
    window: Arc<Mutex<Window>>,
    closed: Arc<std::sync::atomic::AtomicBool>,
}

/// Which segments a stream keeps: those from `BEHIND` seconds before the
/// newest one its viewer fetched, and ffmpeg waits while it is more than
/// `AHEAD` seconds past it. Segments are numbered from 0 in playlist order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Window {
    fetched: usize,
    dropped: usize,
    dropped_seconds: f64,
    held: bool,
}

/// What a stream's keeper does next: delete these segments, and stop or
/// let go of ffmpeg.
#[derive(Debug, PartialEq)]
pub struct Plan {
    pub drop: std::ops::Range<usize>,
    pub hold: Option<bool>,
}

impl Window {
    pub fn fetched(&mut self, index: usize) {
        self.fetched = self.fetched.max(index);
    }

    /// The stream time from which segments are still there.
    pub fn kept_from(&self) -> f64 {
        self.dropped_seconds
    }

    /// The plan for a playlist whose segments last `lengths` seconds.
    pub fn plan(&mut self, lengths: &[f64], running: bool) -> Plan {
        let fetched = self.fetched.min(lengths.len().saturating_sub(1));
        let start = |index: usize| lengths[..index].iter().sum::<f64>();
        let ahead: f64 = lengths.get(fetched + 1..).map_or(0.0, |rest| rest.iter().sum());
        let hold = match (self.held, running) {
            (false, true) if ahead > AHEAD => Some(true),
            (true, _) if ahead < AHEAD - SLACK || !running => Some(false),
            _ => None,
        };
        if let Some(held) = hold {
            self.held = held;
        }
        let keep_from = (self.dropped..fetched)
            .find(|&index| start(fetched) - start(index + 1) < BEHIND)
            .unwrap_or(fetched);
        let drop = self.dropped..keep_from.max(self.dropped);
        self.dropped_seconds += lengths[drop.clone()].iter().sum::<f64>();
        self.dropped = drop.end;
        Plan { drop, hold }
    }
}

/// How long each segment of `playlist` lasts, in order.
pub fn lengths(playlist: &str) -> Vec<f64> {
    playlist
        .lines()
        .filter_map(|line| line.strip_prefix("#EXTINF:"))
        .filter_map(|rest| rest.split(',').next()?.parse().ok())
        .collect()
}

/// ffmpeg's arguments for a stream of `spec` from file time `at` into `dir`:
/// every track starts at the keyframe before `at` and keeps the file's own
/// times, and the file is read at twice the playback pace after a minute.
pub fn arguments(spec: &Spec, dir: &Path, at: f64) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    let mut push = |items: &[&str]| args.extend(items.iter().map(OsString::from));
    push(&["-nostdin", "-hide_banner", "-loglevel", "error"]);
    if at > 0.0 {
        push(&["-ss", &format!("{at:.3}"), "-noaccurate_seek"]);
    }
    push(&["-copyts", "-readrate", "2", "-readrate_initial_burst", "60", "-i"]);
    args.push(spec.source.clone().into());
    let mut push = |items: &[&str]| args.extend(items.iter().map(OsString::from));
    push(&["-map", "0:v:0"]);
    if let Some(audio) = spec.audio {
        push(&["-map", &format!("0:a:{audio}")]);
    }
    push(&["-c:v", "copy"]);
    if spec.hevc {
        push(&["-tag:v", "hvc1"]);
    }
    if spec.copy_audio {
        push(&["-c:a", "copy"]);
    } else if spec.surround && !spec.dialogue {
        push(&["-af", audio::SURROUND, "-c:a", "aac", "-b:a", "384k"]);
    } else {
        push(&["-af", audio::stereo(spec.dialogue), "-c:a", "aac", "-b:a", "256k"]);
    }
    push(&[
        "-sn",
        "-dn",
        "-max_muxing_queue_size",
        "4096",
        "-f",
        "hls",
        "-hls_time",
        "6",
        "-hls_list_size",
        "0",
        "-hls_playlist_type",
        "event",
        "-hls_segment_type",
        "fmp4",
        "-hls_flags",
        "independent_segments+temp_file",
        "-hls_fmp4_init_filename",
        INIT,
        "-hls_segment_filename",
    ]);
    args.push(dir.join("%d.m4s").into());
    args.push(dir.join(PLAYLIST).into());
    for track in &spec.subtitles {
        let mut push = |items: &[&str]| args.extend(items.iter().map(OsString::from));
        push(&[
            "-map",
            &format!("0:s:{track}"),
            "-c:s",
            "webvtt",
            "-flush_packets",
            "1",
            "-f",
            "webvtt",
        ]);
        args.push(subtitle_file(dir, *track).into());
    }
    args
}

fn subtitle_file(dir: &Path, track: usize) -> PathBuf {
    dir.join(format!("sub{track}.vtt"))
}

/// The segments a playlist lists so far.
pub fn segments(playlist: &str) -> usize {
    playlist
        .lines()
        .filter(|line| !line.starts_with('#') && line.ends_with(".m4s"))
        .count()
}

/// `playlist` with the point a player should start at, in the stream's own
/// time, which players read before they pick a segment.
pub fn starting_at(playlist: &str, at: f64) -> String {
    if at < 0.5 {
        return playlist.to_string();
    }
    match playlist.split_once('\n') {
        Some((head, rest)) => format!("{head}\n#EXT-X-START:TIME-OFFSET={at:.3},PRECISE=YES\n{rest}"),
        None => playlist.to_string(),
    }
}

/// Whether `name` is a file ffmpeg writes for players: the init segment or a
/// numbered media segment.
pub fn servable(name: &str) -> bool {
    name == INIT
        || name
            .strip_suffix(".m4s")
            .is_some_and(|number| !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
}

impl Remux {
    /// Starts ffmpeg and waits until the init segment and READY_SEGMENTS
    /// media segments are on disk, or fewer when the file ends first.
    pub async fn start(dir: PathBuf, spec: Spec, at: f64) -> Result<Self, String> {
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|error| format!("{}: {error}", dir.display()))?;
        let log = dir.join("ffmpeg.log");
        let mut command = Command::new("ffmpeg");
        command
            .args(arguments(&spec, &dir, at))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .kill_on_drop(true);
        if let Ok(file) = std::fs::File::create(&log) {
            command.stderr(file);
        }
        unsafe {
            command.pre_exec(|| {
                libc::setpriority(libc::PRIO_PROCESS, 0, NICENESS);
                Ok(())
            });
        }
        let mut child = command.spawn().map_err(|error| format!("ffmpeg: {error}"))?;
        let pid = child.id();
        let exit = Arc::new(Mutex::new(None));
        let stop = Arc::new(Notify::new());
        {
            let (exit, stop, source) = (exit.clone(), stop.clone(), spec.source.clone());
            let log = log.clone();
            tokio::spawn(async move {
                tokio::select! {
                    status = child.wait() => {
                        let success = status.is_ok_and(|status| status.success());
                        if !success {
                            eprintln!("stream: {}: {}", source.display(), last_line(&log));
                        }
                        *exit.lock().unwrap() = Some(success);
                    }
                    () = stop.notified() => {
                        let _ = child.kill().await;
                    }
                }
            });
        }
        let deadline = Instant::now() + WAIT;
        loop {
            let listed = tokio::fs::read_to_string(dir.join(PLAYLIST)).await.unwrap_or_default();
            let finished = *exit.lock().unwrap() == Some(true);
            if segments(&listed) >= READY_SEGMENTS || (finished && segments(&listed) > 0) {
                break;
            }
            let failed = *exit.lock().unwrap() == Some(false);
            if failed || Instant::now() >= deadline {
                let reason = if failed {
                    last_line(&log)
                } else {
                    "the video did not start in time".to_string()
                };
                stop.notify_one();
                let _ = tokio::fs::remove_dir_all(&dir).await;
                return Err(reason);
            }
            tokio::time::sleep(POLL).await;
        }
        let init = tokio::fs::read(dir.join(INIT)).await.unwrap_or_default();
        let offset = first_time(&dir).await.unwrap_or(at);
        let window = Arc::new(Mutex::new(Window::default()));
        let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        tokio::spawn(keep(dir.clone(), pid, window.clone(), exit.clone(), closed.clone()));
        Ok(Self {
            codecs: codecs::codecs(&init),
            offset,
            dir,
            spec,
            exit,
            stop,
            used: Mutex::new(Instant::now()),
            window,
            closed,
        })
    }

    /// The file time from which this stream still has its segments.
    pub fn kept_from(&self) -> f64 {
        self.offset + self.window.lock().unwrap().kept_from()
    }

    fn exited(&self) -> Option<bool> {
        *self.exit.lock().unwrap()
    }

    pub fn idle(&self) -> Duration {
        self.used.lock().unwrap().elapsed()
    }

    pub fn touch(&self) {
        *self.used.lock().unwrap() = Instant::now();
    }

    /// ffmpeg's playlist as it stands, starting players at file time `at`.
    pub async fn playlist(&self, at: f64) -> Option<String> {
        self.touch();
        let text = tokio::fs::read_to_string(self.dir.join(PLAYLIST)).await.ok()?;
        Some(starting_at(&text, at - self.offset))
    }

    pub async fn file(&self, name: &str) -> Option<Vec<u8>> {
        self.touch();
        if !servable(name) {
            return None;
        }
        if let Some(index) = name.strip_suffix(".m4s").and_then(|number| number.parse().ok()) {
            self.window.lock().unwrap().fetched(index);
        }
        tokio::fs::read(self.dir.join(name)).await.ok()
    }

    /// The cues of embedded track `track` that the remux has written so far,
    /// and whether it has written all of them.
    pub async fn cues(&self, track: usize) -> (Vec<Cue>, bool) {
        self.touch();
        let complete = self.exited() == Some(true);
        let text = tokio::fs::read_to_string(subtitle_file(&self.dir, track))
            .await
            .unwrap_or_default();
        let cues = if complete {
            vtt::all_cues(&text)
        } else {
            vtt::cues(&text)
        };
        (cues, complete)
    }

    pub fn close(&self) {
        self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
        self.stop.notify_one();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Drop for Remux {
    fn drop(&mut self) {
        self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
        self.stop.notify_one();
    }
}

/// Keeps a stream's segments to its window until the stream closes: deletes
/// what its viewer left behind, and stops ffmpeg while it runs too far ahead.
async fn keep(
    dir: PathBuf,
    pid: Option<u32>,
    window: Arc<Mutex<Window>>,
    exit: Arc<Mutex<Option<bool>>>,
    closed: Arc<std::sync::atomic::AtomicBool>,
) {
    while !closed.load(std::sync::atomic::Ordering::SeqCst) {
        tokio::time::sleep(KEEPING).await;
        let Ok(listed) = tokio::fs::read_to_string(dir.join(PLAYLIST)).await else {
            continue;
        };
        let running = exit.lock().unwrap().is_none();
        let plan = window.lock().unwrap().plan(&lengths(&listed), running);
        for index in plan.drop {
            let _ = tokio::fs::remove_file(dir.join(format!("{index}.m4s"))).await;
        }
        if let (Some(hold), Some(pid)) = (plan.hold, pid.and_then(|pid| i32::try_from(pid).ok())) {
            let signal = if hold { libc::SIGSTOP } else { libc::SIGCONT };
            unsafe {
                libc::kill(pid, signal);
            }
        }
    }
}

fn last_line(log: &Path) -> String {
    std::fs::read_to_string(log)
        .ok()
        .and_then(|text| text.lines().last().map(str::to_string))
        .unwrap_or_else(|| "ffmpeg stopped".to_string())
}

/// The earliest time in the first segment, where players put the start of
/// the stream's timeline.
async fn first_time(dir: &Path) -> Option<f64> {
    let joined = format!("concat:{}|{}", dir.join(INIT).display(), dir.join("0.m4s").display());
    let output = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=start_time", "-of", "csv=p=0"])
        .arg(joined)
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

pub fn clear(root: &Path) {
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> Spec {
        Spec {
            source: PathBuf::from("/library/Film.mkv"),
            audio: Some(1),
            hevc: true,
            copy_audio: false,
            dialogue: false,
            surround: false,
            subtitles: vec![0, 2],
        }
    }

    fn words(args: &[OsString]) -> String {
        args.iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn a_stream_from_the_middle_seeks_to_the_keyframe_before_and_keeps_the_files_times() {
        let args = words(&arguments(&spec(), Path::new("/run/s/abc"), 2829.25));
        assert!(
            args.contains(
                "-ss 2829.250 -noaccurate_seek -copyts -readrate 2 -readrate_initial_burst 60 -i /library/Film.mkv"
            ),
            "{args}"
        );
        assert!(args.contains("-map 0:v:0 -map 0:a:1 -c:v copy -tag:v hvc1"), "{args}");
        assert!(
            args.contains("-hls_playlist_type event -hls_segment_type fmp4"),
            "{args}"
        );
        assert!(
            args.contains("-hls_segment_filename /run/s/abc/%d.m4s /run/s/abc/video.m3u8"),
            "{args}"
        );
        assert!(args.ends_with("-map 0:s:0 -c:s webvtt -flush_packets 1 -f webvtt /run/s/abc/sub0.vtt -map 0:s:2 -c:s webvtt -flush_packets 1 -f webvtt /run/s/abc/sub2.vtt"), "{args}");
        let from_start = words(&arguments(&spec(), Path::new("/run/s/abc"), 0.0));
        assert!(!from_start.contains("-ss"), "{from_start}");
    }

    #[test]
    fn a_stream_keeps_ninety_seconds_behind_its_viewer_and_waits_three_minutes_ahead() {
        let mut window = Window::default();
        let six = |count: usize| vec![6.0; count];
        assert_eq!(window.plan(&six(10), true), Plan { drop: 0..0, hold: None });
        assert_eq!(
            window.plan(&six(32), true),
            Plan {
                drop: 0..0,
                hold: Some(true)
            }
        );
        assert_eq!(window.plan(&six(32), true), Plan { drop: 0..0, hold: None });
        window.fetched(20);
        assert_eq!(
            window.plan(&six(32), true),
            Plan {
                drop: 0..5,
                hold: Some(false)
            }
        );
        assert_eq!(window.kept_from(), 30.0);
        window.fetched(22);
        assert_eq!(window.plan(&six(40), true), Plan { drop: 5..7, hold: None });
        assert_eq!(window.kept_from(), 42.0);
        window.fetched(3);
        assert_eq!(window.plan(&six(40), true), Plan { drop: 7..7, hold: None });
    }

    #[test]
    fn a_held_ffmpeg_is_let_go_when_it_ends_or_its_viewer_catches_up() {
        let mut window = Window::default();
        assert_eq!(window.plan(&[6.0; 40], true).hold, Some(true));
        assert_eq!(window.plan(&[6.0; 40], false).hold, Some(false));
        assert_eq!(window.plan(&[6.0; 40], false).hold, None);
    }

    #[test]
    fn segment_lengths_come_from_the_playlist() {
        let playlist = "#EXTM3U\n#EXT-X-TARGETDURATION:7\n#EXTINF:6.006000,\n0.m4s\n#EXTINF:4.170833,\n1.m4s\n";
        assert_eq!(lengths(playlist), vec![6.006, 4.170833]);
    }

    #[test]
    fn audio_is_copied_or_converted_through_the_chosen_stereo_chain() {
        let copied = words(&arguments(
            &Spec {
                copy_audio: true,
                ..spec()
            },
            Path::new("/d"),
            0.0,
        ));
        assert!(copied.contains("-c:a copy") && !copied.contains("-af"), "{copied}");
        let plain = words(&arguments(&spec(), Path::new("/d"), 0.0));
        assert!(
            plain.contains(&format!("-af {} -c:a aac -b:a 256k", audio::STEREO)),
            "{plain}"
        );
        let dialogue = words(&arguments(
            &Spec {
                dialogue: true,
                ..spec()
            },
            Path::new("/d"),
            0.0,
        ));
        assert!(
            dialogue.contains(&format!("-af {} -c:a aac", audio::DIALOGUE)),
            "{dialogue}"
        );
        let surround = Spec {
            surround: true,
            ..spec()
        };
        let kept = words(&arguments(&surround, Path::new("/d"), 0.0));
        assert!(
            kept.contains(&format!("-af {} -c:a aac -b:a 384k", audio::SURROUND)),
            "{kept}"
        );
        let folded = words(&arguments(
            &Spec {
                dialogue: true,
                ..surround
            },
            Path::new("/d"),
            0.0,
        ));
        assert!(
            folded.contains(&format!("-af {} -c:a aac -b:a 256k", audio::DIALOGUE)),
            "{folded}"
        );
        let h264 = words(&arguments(
            &Spec {
                hevc: false,
                audio: None,
                ..spec()
            },
            Path::new("/d"),
            0.0,
        ));
        assert!(!h264.contains("hvc1") && !h264.contains("0:a:"), "{h264}");
    }

    #[test]
    fn players_start_where_the_viewer_left_off() {
        let playlist = "#EXTM3U\n#EXT-X-VERSION:7\n#EXTINF:6.000000,\n0.m4s\n";
        assert_eq!(
            starting_at(playlist, 12.5),
            "#EXTM3U\n#EXT-X-START:TIME-OFFSET=12.500,PRECISE=YES\n#EXT-X-VERSION:7\n#EXTINF:6.000000,\n0.m4s\n"
        );
        assert_eq!(starting_at(playlist, 0.2), playlist);
        assert_eq!(segments(playlist), 1);
        assert_eq!(segments("#EXTM3U\n"), 0);
    }

    fn sample(dir: &Path) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let subtitles = dir.join("film.srt");
        std::fs::write(
            &subtitles,
            "1\n00:00:02,000 --> 00:00:03,000\ntwo\n\n2\n00:00:09,000 --> 00:00:10,500\nnine\n\n3\n00:00:17,000 --> 00:00:18,000\nseventeen\n",
        )
        .unwrap();
        let source = dir.join("film.mkv");
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=24",
                "-f",
                "lavfi",
            ])
            .args(["-i", "sine=frequency=440:sample_rate=48000", "-i"])
            .arg(&subtitles)
            .args([
                "-t",
                "20",
                "-map",
                "0:v",
                "-map",
                "1:a",
                "-map",
                "2:s",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
            ])
            .args([
                "-g",
                "48",
                "-keyint_min",
                "48",
                "-sc_threshold",
                "0",
                "-bf",
                "2",
                "-c:a",
                "aac",
                "-c:s",
                "srt",
            ])
            .arg(&source)
            .status()
            .unwrap();
        assert!(status.success());
        source
    }

    #[tokio::test]
    async fn a_stream_from_the_middle_plays_from_the_keyframe_before_with_the_files_times() {
        let dir = std::env::temp_dir().join(format!("semmeltv-remux-{}", std::process::id()));
        let source = sample(&dir);
        let spec = Spec {
            source,
            audio: Some(0),
            hevc: false,
            copy_audio: false,
            dialogue: true,
            surround: false,
            subtitles: vec![0],
        };
        let remux = Remux::start(dir.join("stream"), spec, 7.3).await.unwrap();
        assert!(
            (5.8..=6.0).contains(&remux.offset),
            "starts at the keyframe at 6 s: {}",
            remux.offset
        );
        let codecs = remux.codecs.clone().unwrap_or_default();
        assert!(
            codecs.starts_with("avc1.") && codecs.ends_with(",mp4a.40.2"),
            "{codecs}"
        );
        let deadline = Instant::now() + Duration::from_secs(30);
        while remux.exited().is_none() && Instant::now() < deadline {
            tokio::time::sleep(POLL).await;
        }
        assert_eq!(remux.exited(), Some(true));
        let playlist = remux.playlist(7.3).await.unwrap();
        assert!(playlist.contains("#EXT-X-PLAYLIST-TYPE:EVENT"), "{playlist}");
        assert!(
            playlist.contains(&format!(
                "#EXT-X-START:TIME-OFFSET={:.3},PRECISE=YES",
                7.3 - remux.offset
            )),
            "{playlist}"
        );
        assert!(playlist.trim_end().ends_with("#EXT-X-ENDLIST"), "{playlist}");
        assert!(segments(&playlist) >= 2, "{playlist}");
        assert!(remux.file("init.mp4").await.is_some_and(|bytes| !bytes.is_empty()));
        assert!(remux.file("0.m4s").await.is_some_and(|bytes| !bytes.is_empty()));
        assert!(remux.file("ffmpeg.log").await.is_none());
        let (cues, complete) = remux.cues(0).await;
        assert!(complete);
        let found: Vec<(f64, &str)> = cues.iter().map(|cue| (cue.start, cue.text.as_str())).collect();
        assert_eq!(found, vec![(9.0, "nine"), (17.0, "seventeen")]);
        remux.close();
        assert!(!dir.join("stream").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_file_ffmpeg_cannot_read_fails_to_open_with_ffmpegs_reason() {
        let dir = std::env::temp_dir().join(format!("semmeltv-remux-broken-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("broken.mkv");
        std::fs::write(&source, b"not a video").unwrap();
        let spec = Spec { source, ..spec() };
        let error = Remux::start(dir.join("stream"), spec, 0.0).await.err().unwrap();
        assert!(error.contains("Invalid data"), "{error}");
        assert!(!dir.join("stream").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_init_and_numbered_segments_are_served() {
        assert!(servable("init.mp4") && servable("0.m4s") && servable("293.m4s"));
        for name in [
            "video.m3u8",
            "ffmpeg.log",
            "sub0.vtt",
            ".m4s",
            "../x.m4s",
            "1.m4s.tmp",
            "a1.m4s",
        ] {
            assert!(!servable(name), "{name}");
        }
    }
}
