use crate::wyoming;
use std::fs;
use std::io::{self, Read};
use std::process::{Child, Command as Process, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

pub const FRAME: usize = 480;
const RETRY: Duration = Duration::from_secs(3);
const FRAME_TIME: Duration = Duration::from_millis(30);
const INITIAL_FLOOR: f32 = 150.0;
const FLOOR_RISE: f32 = 1.005;
const SPEECH_OVER_FLOOR: f32 = 3.0;
const MIN_SPEECH_LEVEL: f32 = 400.0;
const MIN_SPEECH_FRAMES: usize = 4;
const END_QUIET_FRAMES: usize = 27;
const GIVE_UP_FRAMES: usize = 167;
const LONGEST_FRAMES: usize = 334;

#[derive(Debug, PartialEq, Eq)]
pub enum Heard {
    Listening,
    Transcribing,
    Nothing,
    Transcript(String),
    NoMicrophone,
    Failed(String),
}

enum Input {
    Listen,
    Frame(Vec<i16>),
    Microphone(bool),
}

pub enum Source {
    Microphone(Option<String>),
    Recording(Vec<i16>),
}

pub struct WakeWord {
    pub address: String,
    pub name: String,
}

pub struct Voice {
    inputs: Sender<Input>,
}

impl Voice {
    pub fn listen(&self) {
        let _ = self.inputs.send(Input::Listen);
    }
}

pub fn spawn(
    speech: String,
    wake_word: Option<WakeWord>,
    source: Source,
    heard: impl Fn(Heard) + Send + 'static,
) -> Voice {
    let (inputs, received) = mpsc::channel();
    let frames = inputs.clone();
    match source {
        Source::Microphone(device) => thread::spawn(move || capture(device, &frames)),
        Source::Recording(samples) => {
            let _ = frames.send(Input::Microphone(true));
            thread::spawn(move || replay(&samples, &frames))
        }
    };
    let waker = wake_word.map(|wake_word| Waker::new(wake_word, inputs.clone()));
    thread::spawn(move || run(&speech, waker, &received, &heard));
    Voice { inputs }
}

#[derive(Default)]
struct Recording {
    endpointer: Endpointer,
    samples: Vec<i16>,
}

impl Recording {
    fn push(&mut self, frame: &[i16]) -> Progress {
        self.samples.extend_from_slice(frame);
        self.endpointer.push(frame)
    }
}

fn run(speech: &str, mut waker: Option<Waker>, received: &Receiver<Input>, heard: &impl Fn(Heard)) {
    let mut present = false;
    let mut recording: Option<Recording> = None;
    while let Ok(input) = received.recv() {
        let progress = match input {
            Input::Microphone(now) => {
                present = now;
                if let Some(waker) = waker.as_mut() {
                    waker.restart();
                }
                if !now && recording.take().is_some() {
                    heard(Heard::NoMicrophone);
                }
                continue;
            }
            Input::Listen if !present => {
                heard(Heard::NoMicrophone);
                continue;
            }
            Input::Listen => {
                if recording.is_none() {
                    recording = Some(Recording::default());
                    heard(Heard::Listening);
                }
                continue;
            }
            Input::Frame(frame) => match recording.as_mut() {
                Some(recording) => recording.push(&frame),
                None => {
                    if let Some(waker) = waker.as_mut() {
                        waker.hear(&frame);
                    }
                    continue;
                }
            },
        };
        match progress {
            Progress::Listening => continue,
            Progress::Nothing => {
                if let Some(recording) = recording.take() {
                    eprintln!("voice: {}", recording.endpointer.summary());
                }
                heard(Heard::Nothing);
            }
            Progress::Ended => {
                let Some(Recording { endpointer, samples }) = recording.take() else {
                    continue;
                };
                heard(Heard::Transcribing);
                let started = Instant::now();
                let result = wyoming::transcribe(speech, &samples);
                eprintln!(
                    "voice: {}, recognised in {} ms",
                    endpointer.summary(),
                    started.elapsed().as_millis()
                );
                heard(match result {
                    Ok(text) => Heard::Transcript(text),
                    Err(error) => Heard::Failed(format!("Speech recognition: {error}")),
                });
                for input in received.try_iter() {
                    if let Input::Microphone(now) = input {
                        present = now;
                    }
                }
            }
        }
        if let Some(waker) = waker.as_mut() {
            waker.restart();
        }
    }
}

struct Waker {
    wake_word: WakeWord,
    inputs: Sender<Input>,
    detector: Option<wyoming::Detector>,
    next_attempt: Instant,
}

impl Waker {
    fn new(wake_word: WakeWord, inputs: Sender<Input>) -> Self {
        Self {
            wake_word,
            inputs,
            detector: None,
            next_attempt: Instant::now(),
        }
    }

    fn hear(&mut self, frame: &[i16]) {
        if self.detector.is_none() && Instant::now() >= self.next_attempt {
            self.connect();
        }
        let Some(detector) = self.detector.as_mut() else {
            return;
        };
        if let Err(error) = detector.send(frame) {
            self.fail(&error);
        }
    }

    fn connect(&mut self) {
        let inputs = self.inputs.clone();
        match wyoming::detect(&self.wake_word.address, &self.wake_word.name, move || {
            let _ = inputs.send(Input::Listen);
        }) {
            Ok(detector) => self.detector = Some(detector),
            Err(error) => self.fail(&error),
        }
    }

    fn restart(&mut self) {
        if let Some(detector) = self.detector.as_mut()
            && let Err(error) = detector.restart()
        {
            self.fail(&error);
        }
    }

    fn fail(&mut self, error: &io::Error) {
        eprintln!("wake word {}: {error}", self.wake_word.address);
        self.detector = None;
        self.next_attempt = Instant::now() + RETRY;
    }
}

fn replay(samples: &[i16], inputs: &Sender<Input>) {
    let silence = [0; FRAME];
    for frame in samples.chunks(FRAME).chain(std::iter::repeat(&silence[..])) {
        if inputs.send(Input::Frame(frame.to_vec())).is_err() {
            return;
        }
        thread::sleep(FRAME_TIME);
    }
}

fn capture(configured: Option<String>, inputs: &Sender<Input>) {
    loop {
        if let Some(device) = configured.clone().or_else(find_microphone) {
            if let Err(error) = record(&device, inputs) {
                eprintln!("microphone {device}: {error}");
            }
            if inputs.send(Input::Microphone(false)).is_err() {
                return;
            }
        }
        thread::sleep(RETRY);
    }
}

fn record(device: &str, inputs: &Sender<Input>) -> io::Result<()> {
    let mut child = Process::new("arecord")
        .args(["-q", "-D", device, "-t", "raw", "-f", "S16_LE", "-c", "1"])
        .arg(format!("-r{}", wyoming::RATE))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()?;
    let result = forward(&mut child, inputs);
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn forward(child: &mut Child, inputs: &Sender<Input>) -> io::Result<()> {
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("arecord has no output"))?;
    let mut buffer = [0; FRAME * 2];
    stdout.read_exact(&mut buffer)?;
    if inputs.send(Input::Microphone(true)).is_err() {
        return Ok(());
    }
    loop {
        if inputs.send(Input::Frame(samples(&buffer))).is_err() {
            return Ok(());
        }
        stdout.read_exact(&mut buffer)?;
    }
}

fn samples(bytes: &[u8]) -> Vec<i16> {
    bytes
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}

fn find_microphone() -> Option<String> {
    capture_device(&fs::read_to_string("/proc/asound/pcm").ok()?)
}

fn capture_device(pcm: &str) -> Option<String> {
    pcm.lines().find_map(|line| {
        let (id, description) = line.split_once(':')?;
        let (card, device) = id.split_once('-')?;
        let card: u32 = card.trim().parse().ok()?;
        let device: u32 = device.trim().parse().ok()?;
        description
            .contains("capture")
            .then(|| format!("plughw:{card},{device}"))
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum Progress {
    Listening,
    Nothing,
    Ended,
}

pub struct Endpointer {
    frames: usize,
    speech: usize,
    quiet: usize,
    floor: f32,
    first_speech: Option<usize>,
    last_speech: usize,
}

impl Default for Endpointer {
    fn default() -> Self {
        Self {
            frames: 0,
            speech: 0,
            quiet: 0,
            floor: INITIAL_FLOOR,
            first_speech: None,
            last_speech: 0,
        }
    }
}

impl Endpointer {
    pub fn push(&mut self, frame: &[i16]) -> Progress {
        let level = level(frame);
        self.frames += 1;
        if level > (self.floor * SPEECH_OVER_FLOOR).max(MIN_SPEECH_LEVEL) {
            self.speech += 1;
            self.quiet = 0;
            self.first_speech.get_or_insert(self.frames);
            self.last_speech = self.frames;
        } else {
            self.quiet += 1;
        }
        self.floor = (self.floor * FLOOR_RISE).min(level.max(1.0));
        let spoke = self.speech >= MIN_SPEECH_FRAMES;
        if spoke && (self.quiet >= END_QUIET_FRAMES || self.frames >= LONGEST_FRAMES) {
            Progress::Ended
        } else if !spoke && self.frames >= GIVE_UP_FRAMES {
            Progress::Nothing
        } else {
            Progress::Listening
        }
    }
}

impl Endpointer {
    fn summary(&self) -> String {
        let seconds = |frames: usize| frames as f64 * FRAME_TIME.as_secs_f64();
        let recorded = seconds(self.frames);
        match self.first_speech {
            Some(first) => format!(
                "{recorded:.1} s recorded, speech from {:.1} s to {:.1} s, noise floor {:.0}",
                seconds(first - 1),
                seconds(self.last_speech),
                self.floor
            ),
            None => format!("{recorded:.1} s recorded, no speech, noise floor {:.0}", self.floor),
        }
    }
}

fn level(frame: &[i16]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let energy: f64 = frame.iter().map(|sample| f64::from(*sample).powi(2)).sum();
    (energy / frame.len() as f64).sqrt() as f32
}

pub fn read_wav(bytes: &[u8]) -> Result<Vec<i16>, String> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV file".to_string());
    }
    let mut at = 12;
    let mut format_ok = false;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = bytes.get(at + 8..at + 8 + size).unwrap_or(&bytes[at + 8..]);
        match id {
            b"fmt " if body.len() >= 16 => {
                let channels = u16::from_le_bytes([body[2], body[3]]);
                let rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
                let bits = u16::from_le_bytes([body[14], body[15]]);
                format_ok = channels == 1 && rate == wyoming::RATE && bits == 16;
            }
            b"data" if format_ok => return Ok(samples(body)),
            b"data" => return Err(format!("needs 16-bit mono audio at {} Hz", wyoming::RATE)),
            _ => {}
        }
        at += 8 + size + size % 2;
    }
    Err("the WAV file has no audio".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(amplitude: f32) -> Vec<i16> {
        (0..FRAME)
            .map(|index| (amplitude * (index as f32 * 0.3).sin()) as i16)
            .collect()
    }

    fn feed(endpointer: &mut Endpointer, frame: &[i16], count: usize) -> Vec<Progress> {
        (0..count).map(|_| endpointer.push(frame)).collect()
    }

    #[test]
    fn speech_ends_after_a_pause() {
        let mut endpointer = Endpointer::default();
        assert!(
            feed(&mut endpointer, &tone(60.0), 15)
                .iter()
                .all(|p| *p == Progress::Listening)
        );
        assert!(
            feed(&mut endpointer, &tone(6000.0), 40)
                .iter()
                .all(|p| *p == Progress::Listening)
        );
        let after = feed(&mut endpointer, &tone(60.0), END_QUIET_FRAMES);
        assert_eq!(after.last(), Some(&Progress::Ended));
        assert!(after[..END_QUIET_FRAMES - 1].iter().all(|p| *p == Progress::Listening));
    }

    #[test]
    fn the_summary_says_when_speech_started_and_stopped() {
        let mut endpointer = Endpointer::default();
        feed(&mut endpointer, &tone(60.0), 10);
        feed(&mut endpointer, &tone(6000.0), 40);
        feed(&mut endpointer, &tone(60.0), END_QUIET_FRAMES);
        assert!(
            endpointer
                .summary()
                .starts_with("2.3 s recorded, speech from 0.3 s to 1.5 s, noise floor")
        );
    }

    #[test]
    fn speech_right_at_the_start_is_heard() {
        let mut endpointer = Endpointer::default();
        feed(&mut endpointer, &tone(6000.0), 30);
        let after = feed(&mut endpointer, &[0; FRAME], END_QUIET_FRAMES);
        assert_eq!(after.last(), Some(&Progress::Ended));
    }

    #[test]
    fn a_short_click_is_not_speech() {
        let mut endpointer = Endpointer::default();
        feed(&mut endpointer, &tone(6000.0), 2);
        let after = feed(&mut endpointer, &tone(60.0), GIVE_UP_FRAMES - 2);
        assert_eq!(after.last(), Some(&Progress::Nothing));
    }

    #[test]
    fn room_noise_is_not_speech() {
        let mut endpointer = Endpointer::default();
        let progress = feed(&mut endpointer, &tone(250.0), GIVE_UP_FRAMES);
        assert_eq!(progress.last(), Some(&Progress::Nothing));
    }

    #[test]
    fn endless_speech_is_cut_off() {
        let mut endpointer = Endpointer::default();
        let progress = feed(&mut endpointer, &tone(6000.0), LONGEST_FRAMES);
        assert_eq!(progress.last(), Some(&Progress::Ended));
    }

    #[test]
    fn the_first_capture_device_is_used() {
        let pcm = "00-00: MAI PCM i2s-hifi-0 : MAI PCM i2s-hifi-0 : playback 1\n\
                   02-00: USB Audio : USB Audio : playback 1 : capture 1\n";
        assert_eq!(capture_device(pcm), Some("plughw:2,0".to_string()));
        assert_eq!(capture_device("00-00: HDMI : HDMI : playback 1\n"), None);
    }

    #[test]
    fn wav_files_give_their_samples() {
        let mut bytes = b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\x3e\0\0\0\x7d\0\0\x02\0\x10\0".to_vec();
        bytes.extend_from_slice(b"data\x04\0\0\0\x01\0\xff\xff");
        assert_eq!(read_wav(&bytes), Ok(vec![1, -1]));
        bytes[24] = 0x44;
        assert!(read_wav(&bytes).is_err());
    }
}
