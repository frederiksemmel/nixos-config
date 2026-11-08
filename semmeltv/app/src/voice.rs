use crate::wyoming;
use std::fs;
use std::io::{self, Read};
use std::process::{Child, Command as Process, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

pub const FRAME: usize = 480;
const RETRY: Duration = Duration::from_secs(3);
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

pub struct Voice {
    inputs: Sender<Input>,
}

impl Voice {
    pub fn listen(&self) {
        let _ = self.inputs.send(Input::Listen);
    }
}

pub fn spawn(whisper: String, microphone: Option<String>, heard: impl Fn(Heard) + Send + 'static) -> Voice {
    let (inputs, received) = mpsc::channel();
    let frames = inputs.clone();
    thread::spawn(move || capture(microphone, &frames));
    thread::spawn(move || run(&whisper, &received, &heard));
    Voice { inputs }
}

fn run(whisper: &str, received: &Receiver<Input>, heard: &impl Fn(Heard)) {
    let mut present = false;
    let mut recording: Option<(Endpointer, Vec<i16>)> = None;
    while let Ok(input) = received.recv() {
        match input {
            Input::Microphone(now) => {
                present = now;
                if !now && recording.take().is_some() {
                    heard(Heard::NoMicrophone);
                }
            }
            Input::Listen if !present => heard(Heard::NoMicrophone),
            Input::Listen => {
                if recording.is_none() {
                    recording = Some((Endpointer::default(), Vec::new()));
                    heard(Heard::Listening);
                }
            }
            Input::Frame(frame) => {
                let Some((endpointer, samples)) = recording.as_mut() else {
                    continue;
                };
                samples.extend_from_slice(&frame);
                match endpointer.push(&frame) {
                    Progress::Listening => {}
                    Progress::Nothing => {
                        recording = None;
                        heard(Heard::Nothing);
                    }
                    Progress::Ended => {
                        let samples = recording.take().map(|(_, samples)| samples).unwrap_or_default();
                        heard(match wyoming::transcribe(whisper, &samples) {
                            Ok(text) => Heard::Transcript(text),
                            Err(error) => Heard::Failed(format!("Whisper: {error}")),
                        });
                        for input in received.try_iter() {
                            if let Input::Microphone(now) = input {
                                present = now;
                            }
                        }
                    }
                }
            }
        }
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
}

impl Default for Endpointer {
    fn default() -> Self {
        Self {
            frames: 0,
            speech: 0,
            quiet: 0,
            floor: INITIAL_FLOOR,
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
