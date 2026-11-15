use crate::bus::Bus;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const TV_DEVICE: &str = "auto";
pub const STEREO: &str = "aformat=channel_layouts=stereo,alimiter=limit=-1dB:level=false";
const COMMAND_TIMEOUT_SECONDS: &str = "20";
const HEADPHONE_ICONS: [&str; 2] = ["audio-headphones", "audio-headset"];
const MONITOR_RESTART: Duration = Duration::from_secs(2);
const SCANNING: &str = "Open the case near the TV and hold the button on its back until the light flashes white.";
const NOTHING_FOUND: &str =
    "No headphones showed up. Open the case near the TV, hold the button on its back until the light flashes white, and try again.";
const NONE_PAIRED: &str = "No headphones are paired. Pair them in Settings.";
const MAX_VOLUME: u8 = 127;
const VOLUME_STEP: u8 = 8;
const MAX_AUDIO_DELAY: i32 = 500;
const FIND_MY: &str = " - Find My";
const APPLE: &str = "bluetooth:v004C";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub address: String,
    pub name: String,
}

impl Device {
    fn alsa(&self) -> String {
        format!("alsa/bluealsa:DEV={},PROFILE=a2dp", self.address)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Output {
    #[default]
    Tv,
    Headphones,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Link {
    #[default]
    Disconnected,
    Connecting,
    Connected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    Scanning,
    Pairing,
    Paired,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct View {
    pub output: Output,
    pub headphones: Option<HeadphonesView>,
    pub pairing: Option<PairingView>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HeadphonesView {
    pub name: String,
    pub address: String,
    pub state: Link,
    pub volume: Option<VolumeView>,
    pub audio_delay: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct VolumeView {
    pub level: u8,
    pub muted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Level {
    pub value: u8,
    pub muted: bool,
}

impl Level {
    fn from_property(raw: u16) -> Self {
        let left = (raw >> 8) as u8;
        Self {
            value: left & 0x7f,
            muted: left & 0x80 != 0,
        }
    }

    fn percent(self) -> u8 {
        ((u32::from(self.value) * 100 + u32::from(MAX_VOLUME) / 2) / u32::from(MAX_VOLUME)) as u8
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolumeChange {
    Up,
    Down,
    Mute,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PairingView {
    pub state: Stage,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub device: String,
    pub said: String,
}

pub trait Radio: Send + Sync {
    fn paired(&self) -> Option<Device>;
    fn bonded(&self, address: &str) -> Option<Device>;
    fn connect(&self, address: &str) -> Result<(), String>;
    fn disconnect(&self, address: &str);
    fn start_scan(&self);
    fn stop_scan(&self);
    fn found(&self) -> Option<Device>;
    fn pair(&self, address: &str) -> Result<(), String>;
    fn forget(&self, address: &str);
    fn sinks(&self) -> Vec<String>;
    fn volume(&self, address: &str) -> Option<Level>;
    fn delay_adjustment(&self, address: &str) -> Option<i32>;
    fn set_delay_adjustment(&self, address: &str, milliseconds: i32) -> Result<(), String>;
    fn set_volume(&self, address: &str, level: Level) -> Result<(), String>;
}

pub type Mover = Box<dyn Fn(Route) + Send + Sync>;

#[derive(Clone, Copy)]
struct Timing {
    sink_wait: Duration,
    scan: Duration,
    poll: Duration,
}

const TIMING: Timing = Timing {
    sink_wait: Duration::from_secs(8),
    scan: Duration::from_secs(60),
    poll: Duration::from_millis(500),
};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Sound {
    Tv,
    Untested,
    Trying(Instant),
    Headphones,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Wire {
    #[default]
    Idle,
    Connecting {
        take_sound: bool,
    },
    Connected(Sound),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Idle,
    Silent,
    Flowing,
    Stalled,
}

pub const TRIAL: Duration = Duration::from_secs(3);
const STALL: Duration = Duration::from_secs(2);
const SLOWEST: f64 = 0.5;

#[derive(Default)]
pub struct Pace {
    last: Option<(Instant, f64)>,
    slow_since: Option<Instant>,
}

impl Pace {
    pub fn observe(&mut self, now: Instant, playing: Option<(f64, f64)>) -> Flow {
        let Some((position, speed)) = playing else {
            *self = Self::default();
            return Flow::Idle;
        };
        let last = self.last.replace((now, position));
        let Some((then, before)) = last else {
            return Flow::Silent;
        };
        let wall = now.duration_since(then).as_secs_f64();
        let moved = position - before;
        if wall <= 0.0 || moved < 0.0 || moved > wall * speed * 4.0 + 1.0 {
            self.slow_since = None;
            return Flow::Silent;
        }
        if moved < wall * speed * SLOWEST {
            let since = *self.slow_since.get_or_insert(then);
            if now.duration_since(since) >= STALL {
                return Flow::Stalled;
            }
            return Flow::Silent;
        }
        self.slow_since = None;
        Flow::Flowing
    }
}

struct Known {
    device: Device,
    wire: Wire,
    volume: Option<Level>,
    audio_delay: Option<i32>,
}

impl Known {
    fn new(device: Device, wire: Wire) -> Self {
        Self {
            device,
            wire,
            volume: None,
            audio_delay: None,
        }
    }
}

#[derive(Default)]
struct State {
    headphones: Option<Known>,
    video: bool,
    error: Option<String>,
    attempt: u64,
    pairing: Option<PairingView>,
    pair_attempt: u64,
}

impl State {
    fn wire(&self) -> Wire {
        self.headphones.as_ref().map_or(Wire::Idle, |known| known.wire)
    }

    fn set_wire(&mut self, wire: Wire) {
        if let Some(known) = self.headphones.as_mut() {
            known.wire = wire;
        }
    }

    fn device(&self) -> Option<&Device> {
        self.headphones.as_ref().map(|known| &known.device)
    }

    fn pairing(&mut self, state: Stage, message: String) {
        self.pairing = Some(PairingView { state, message });
    }

    fn output(&self) -> Output {
        match self.wire() {
            Wire::Connected(Sound::Untested | Sound::Trying(_) | Sound::Headphones) => Output::Headphones,
            _ => Output::Tv,
        }
    }

    fn connected(&self) -> bool {
        matches!(self.wire(), Wire::Connected(_))
    }

    fn shown(&self) -> (Output, Link) {
        match self.wire() {
            Wire::Idle => (Output::Tv, Link::Disconnected),
            Wire::Connecting { .. } | Wire::Connected(Sound::Trying(_)) => (Output::Tv, Link::Connecting),
            Wire::Connected(Sound::Tv) => (Output::Tv, Link::Connected),
            Wire::Connected(Sound::Untested) if self.video => (Output::Tv, Link::Connecting),
            Wire::Connected(Sound::Untested | Sound::Headphones) => (Output::Headphones, Link::Connected),
        }
    }
}

struct Inner {
    state: Mutex<State>,
    kept: Mutex<HashSet<String>>,
    radio: Box<dyn Radio>,
    bus: Bus,
    mover: Mover,
    timing: Timing,
}

#[derive(Clone)]
pub struct Audio {
    inner: Arc<Inner>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Busy;

impl Audio {
    pub fn spawn(bus: Bus, mover: Mover) -> Self {
        let audio = Self::new(Box::new(Bluetoothctl::default()), bus, mover, TIMING);
        let watching = audio.clone();
        let spawned = thread::Builder::new()
            .name("headphones".to_string())
            .spawn(move || watching.watch_sinks());
        if let Err(error) = spawned {
            eprintln!("headphones: {error}");
        }
        audio
    }

    fn new(radio: Box<dyn Radio>, bus: Bus, mover: Mover, timing: Timing) -> Self {
        let headphones = radio.paired().map(|device| Known::new(device, Wire::Idle));
        let audio = Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    headphones,
                    ..State::default()
                }),
                kept: Mutex::new(HashSet::new()),
                radio,
                bus,
                mover,
                timing,
            }),
        };
        audio.publish(&audio.inner.state.lock().unwrap());
        audio
    }

    pub fn view(&self) -> View {
        view_of(&self.inner.state.lock().unwrap())
    }

    pub fn device(&self) -> String {
        let state = self.inner.state.lock().unwrap();
        device_for(state.output(), state.device())
    }

    pub fn choose(&self, output: Output) -> View {
        let mut state = self.inner.state.lock().unwrap();
        state.error = None;
        let mut route = None;
        let mut connect = None;
        let mut disconnect = None;
        match (output, state.device().cloned()) {
            (Output::Headphones, None) => state.error = Some(NONE_PAIRED.to_string()),
            (Output::Headphones, Some(headphones)) => match state.wire() {
                Wire::Connected(Sound::Untested | Sound::Trying(_) | Sound::Headphones) => {}
                Wire::Connecting { .. } => state.set_wire(Wire::Connecting { take_sound: true }),
                Wire::Connected(Sound::Tv) => {
                    state.set_wire(Wire::Connected(Sound::Untested));
                    route = Some(Output::Headphones);
                }
                Wire::Idle => {
                    state.set_wire(Wire::Connecting { take_sound: true });
                    state.attempt += 1;
                    connect = Some((state.attempt, headphones.address));
                }
            },
            (Output::Tv, headphones) => {
                if state.output() == Output::Headphones {
                    route = Some(Output::Tv);
                }
                match state.wire() {
                    Wire::Connecting { .. } => {
                        state.set_wire(Wire::Idle);
                        state.attempt += 1;
                    }
                    Wire::Connected(_) => {
                        state.set_wire(Wire::Connected(Sound::Tv));
                        disconnect = headphones.map(|headphones| headphones.address);
                    }
                    Wire::Idle => {}
                }
            }
        }
        self.publish(&state);
        let view = view_of(&state);
        let headphones = state.device().cloned();
        drop(state);
        if let Some(output) = route {
            self.route(output, headphones.as_ref());
        }
        if let Some((attempt, address)) = connect {
            let audio = self.clone();
            thread::spawn(move || audio.connect(attempt, &address));
        }
        if let Some(address) = disconnect {
            let audio = self.clone();
            thread::spawn(move || audio.inner.radio.disconnect(&address));
        }
        view
    }

    pub fn pair(&self) -> Result<View, Busy> {
        let mut state = self.inner.state.lock().unwrap();
        if matches!(
            state.pairing.as_ref().map(|pairing| pairing.state),
            Some(Stage::Scanning | Stage::Pairing)
        ) {
            return Err(Busy);
        }
        state.error = None;
        state.pair_attempt += 1;
        state.pairing(Stage::Scanning, SCANNING.to_string());
        let attempt = state.pair_attempt;
        self.publish(&state);
        let view = view_of(&state);
        drop(state);
        let audio = self.clone();
        thread::spawn(move || audio.scan_and_pair(attempt));
        Ok(view)
    }

    pub fn cancel_pairing(&self) -> View {
        let mut state = self.inner.state.lock().unwrap();
        if state.pairing.as_ref().is_some_and(|pairing| pairing.state == Stage::Scanning) {
            state.pair_attempt += 1;
            state.pairing = None;
            self.publish(&state);
        }
        view_of(&state)
    }

    pub fn set_kept(&self, addresses: impl IntoIterator<Item = String>) {
        *self.inner.kept.lock().unwrap() = addresses.into_iter().collect();
    }

    fn kept(&self, address: &str) -> bool {
        self.inner.kept.lock().unwrap().contains(address)
    }

    pub fn bonded(&self, address: &str) -> Option<Device> {
        self.inner.radio.bonded(address)
    }

    pub fn headphones(&self) -> Option<Device> {
        self.inner.state.lock().unwrap().device().cloned()
    }

    pub fn select(&self, headphones: Option<Device>) {
        let mut state = self.inner.state.lock().unwrap();
        let same = state.device().map(|known| &known.address) == headphones.as_ref().map(|new| &new.address);
        if same {
            return;
        }
        let old = state.device().cloned();
        let route = state.output() == Output::Headphones;
        let disconnect = old
            .as_ref()
            .filter(|_| state.wire() != Wire::Idle)
            .map(|old| old.address.clone());
        state.headphones = headphones.clone().map(|device| Known::new(device, Wire::Idle));
        state.attempt += 1;
        state.error = None;
        self.publish(&state);
        drop(state);
        if route {
            self.route(Output::Tv, old.as_ref());
        }
        if let Some(address) = disconnect {
            let audio = self.clone();
            thread::spawn(move || audio.inner.radio.disconnect(&address));
        }
        if let Some(headphones) = headphones
            && self.inner.radio.sinks().contains(&headphones.address)
        {
            self.sink(&headphones.address, true);
        }
    }

    pub fn heard(&self, flow: Flow, now: Instant) {
        let mut state = self.inner.state.lock().unwrap();
        let video = flow != Flow::Idle;
        let next = match (state.wire(), flow) {
            (Wire::Connected(Sound::Untested), Flow::Flowing) => Some(Sound::Trying(now)),
            (Wire::Connected(Sound::Trying(since)), Flow::Flowing) if now.duration_since(since) >= TRIAL => {
                Some(Sound::Headphones)
            }
            (Wire::Connected(Sound::Trying(_)), Flow::Idle) => Some(Sound::Untested),
            (Wire::Connected(Sound::Untested | Sound::Trying(_)), Flow::Stalled) => Some(Sound::Tv),
            _ => None,
        };
        let before = view_of(&state);
        state.video = video;
        let Some(next) = next else {
            if view_of(&state) != before {
                self.publish(&state);
            }
            return;
        };
        state.set_wire(Wire::Connected(next));
        let headphones = state.device().cloned();
        if next == Sound::Tv {
            let name = headphones.as_ref().map_or("The headphones", |headphones| &headphones.name);
            state.error = Some(format!("{name} didn't play. The sound is back on the TV."));
        }
        self.publish(&state);
        drop(state);
        if next == Sound::Tv {
            self.route(Output::Tv, headphones.as_ref());
        }
    }

    pub fn forget(&self) -> View {
        let mut state = self.inner.state.lock().unwrap();
        state.error = None;
        let route = state.output() == Output::Headphones;
        let forgotten = state.headphones.take().map(|known| known.device);
        state.attempt += 1;
        if state
            .pairing
            .as_ref()
            .is_some_and(|pairing| matches!(pairing.state, Stage::Paired | Stage::Failed))
        {
            state.pairing = None;
        }
        self.publish(&state);
        let view = view_of(&state);
        drop(state);
        if route {
            self.route(Output::Tv, forgotten.as_ref());
        }
        if let Some(forgotten) = forgotten.filter(|forgotten| !self.kept(&forgotten.address)) {
            self.inner.radio.forget(&forgotten.address);
        }
        view
    }

    fn scan_and_pair(&self, attempt: u64) {
        let radio = &self.inner.radio;
        let still = || self.inner.state.lock().unwrap().pair_attempt == attempt;
        radio.start_scan();
        let deadline = Instant::now() + self.inner.timing.scan;
        let mut forgotten = false;
        let found = loop {
            if !still() {
                radio.stop_scan();
                return;
            }
            if let Some(device) = radio.found() {
                let earlier = self.inner.state.lock().unwrap().device().cloned();
                if forgotten || earlier.is_none_or(|earlier| earlier.address != device.address) {
                    break Some(device);
                }
                radio.forget(&device.address);
                forgotten = true;
            }
            if Instant::now() >= deadline {
                break None;
            }
            thread::sleep(self.inner.timing.poll);
        };
        radio.stop_scan();
        let Some(device) = found else {
            self.paired(attempt, Stage::Failed, NOTHING_FOUND.to_string());
            return;
        };
        if !self.paired(attempt, Stage::Pairing, format!("Pairing with {}…", device.name)) {
            return;
        }
        if let Err(error) = radio.pair(&device.address) {
            eprintln!("headphones: {error}");
            let message = format!(
                "{} didn't pair. Hold the button on the case until the light flashes white and try again.",
                device.name
            );
            self.paired(attempt, Stage::Failed, message);
            return;
        }
        let mut state = self.inner.state.lock().unwrap();
        let route = state.output() == Output::Headphones;
        let replaced = state
            .headphones
            .replace(Known::new(device.clone(), Wire::Connecting { take_sound: false }))
            .map(|known| known.device);
        state.attempt += 1;
        let connect = state.attempt;
        state.pairing(Stage::Paired, format!("Paired with {}.", device.name));
        self.publish(&state);
        drop(state);
        if route {
            self.route(Output::Tv, replaced.as_ref());
        }
        if let Some(replaced) =
            replaced.filter(|replaced| replaced.address != device.address && !self.kept(&replaced.address))
        {
            radio.forget(&replaced.address);
        }
        self.connect(connect, &device.address);
    }

    fn paired(&self, attempt: u64, stage: Stage, message: String) -> bool {
        let mut state = self.inner.state.lock().unwrap();
        if state.pair_attempt != attempt {
            return false;
        }
        state.pairing(stage, message);
        self.publish(&state);
        true
    }

    fn connect(&self, attempt: u64, address: &str) {
        let radio = &self.inner.radio;
        let connected = radio.connect(address);
        if let Err(error) = &connected {
            eprintln!("headphones: {error}");
        }
        let deadline = Instant::now() + self.inner.timing.sink_wait;
        loop {
            if radio.sinks().iter().any(|sink| sink == address) {
                self.sink(address, true);
            }
            {
                let mut state = self.inner.state.lock().unwrap();
                if state.attempt != attempt {
                    let abandoned = state.output() == Output::Tv;
                    drop(state);
                    if abandoned {
                        radio.disconnect(address);
                    }
                    return;
                }
                let Wire::Connecting { take_sound } = state.wire() else {
                    return;
                };
                if connected.is_err() || Instant::now() >= deadline {
                    state.set_wire(Wire::Idle);
                    let name = state.device().map_or("The headphones", |headphones| &headphones.name);
                    if take_sound {
                        state.error = Some(format!(
                            "{name} didn't connect. Open the case near the TV and try again."
                        ));
                    }
                    self.publish(&state);
                    break;
                }
            }
            thread::sleep(self.inner.timing.poll);
        }
        radio.disconnect(address);
    }

    fn sink(&self, address: &str, present: bool) {
        let level = if present { self.inner.radio.volume(address) } else { None };
        let adjustment = if present {
            self.inner.radio.delay_adjustment(address)
        } else {
            None
        };
        let mut state = self.inner.state.lock().unwrap();
        let had_sound = state.output() == Output::Headphones;
        let Some(known) = state.headphones.as_mut().filter(|known| known.device.address == address) else {
            return;
        };
        let headphones = known.device.clone();
        let mut route = None;
        if level.is_some() {
            known.volume = level;
        }
        if let Some(adjustment) = adjustment {
            known.audio_delay = Some(-adjustment);
        }
        match (present, known.wire) {
            (true, Wire::Connecting { take_sound: true }) => {
                known.wire = Wire::Connected(Sound::Untested);
                route = Some(Output::Headphones);
            }
            (true, Wire::Connecting { take_sound: false } | Wire::Idle) => known.wire = Wire::Connected(Sound::Tv),
            (true, Wire::Connected(_)) => {}
            (false, Wire::Connecting { .. }) => {}
            (false, _) => {
                known.wire = Wire::Idle;
                if had_sound {
                    state.error = Some(format!(
                        "{} disconnected. The sound is back on the TV.",
                        headphones.name
                    ));
                    route = Some(Output::Tv);
                }
            }
        }
        self.publish(&state);
        drop(state);
        if let Some(output) = route {
            self.route(output, Some(&headphones));
        }
    }

    pub fn change_volume(&self, change: VolumeChange) -> Option<Result<String, String>> {
        let (headphones, level) = {
            let state = self.inner.state.lock().unwrap();
            if state.output() != Output::Headphones {
                return None;
            }
            let known = state.headphones.as_ref()?;
            (known.device.clone(), known.volume)
        };
        let level = level
            .or_else(|| self.inner.radio.volume(&headphones.address))
            .unwrap_or(Level {
                value: MAX_VOLUME / 2,
                muted: false,
            });
        let changed = match change {
            VolumeChange::Up => Level {
                value: level.value.saturating_add(VOLUME_STEP).min(MAX_VOLUME),
                muted: false,
            },
            VolumeChange::Down => Level {
                value: level.value.saturating_sub(VOLUME_STEP),
                muted: false,
            },
            VolumeChange::Mute => Level {
                muted: !level.muted,
                ..level
            },
        };
        if let Err(error) = self.inner.radio.set_volume(&headphones.address, changed) {
            eprintln!("headphones: {error}");
            return Some(Err(format!("{} didn't take the volume change", headphones.name)));
        }
        self.volume_changed(&headphones.address, changed);
        Some(Ok(match change {
            VolumeChange::Mute if changed.muted => format!("{} muted", headphones.name),
            VolumeChange::Mute => format!("{} sound on", headphones.name),
            _ => format!("{} volume {}%", headphones.name, changed.percent()),
        }))
    }

    pub fn delay_sound(&self, by: i32) -> Result<View, String> {
        let (headphones, delay) = {
            let state = self.inner.state.lock().unwrap();
            match &state.headphones {
                Some(known) if state.connected() => (known.device.clone(), known.audio_delay.unwrap_or(0)),
                _ => return Err("The headphones aren't connected".to_string()),
            }
        };
        let delay = (delay + by).clamp(-MAX_AUDIO_DELAY, MAX_AUDIO_DELAY);
        self.inner.radio.set_delay_adjustment(&headphones.address, -delay).map_err(|error| {
            eprintln!("headphones: {error}");
            format!("{} didn't take the timing change", headphones.name)
        })?;
        let mut state = self.inner.state.lock().unwrap();
        if let Some(known) = state
            .headphones
            .as_mut()
            .filter(|known| known.device.address == headphones.address)
        {
            known.audio_delay = Some(delay);
        }
        self.publish(&state);
        Ok(view_of(&state))
    }

    fn volume_changed(&self, address: &str, level: Level) {
        let mut state = self.inner.state.lock().unwrap();
        if let Some(known) = state.headphones.as_mut().filter(|known| known.device.address == address) {
            known.volume = Some(level);
            self.publish(&state);
        }
    }

    fn sinks_listed(&self, sinks: &[String]) {
        let current = self.inner.state.lock().unwrap().device().cloned();
        if let Some(headphones) = current {
            self.sink(&headphones.address, sinks.contains(&headphones.address));
        }
    }

    fn route(&self, output: Output, headphones: Option<&Device>) {
        let said = match (output, headphones) {
            (Output::Headphones, Some(headphones)) => format!("Sound on {}", headphones.name),
            _ => "Sound on the TV".to_string(),
        };
        eprintln!("headphones: {said}");
        (self.inner.mover)(Route {
            device: device_for(output, headphones),
            said,
        });
    }

    fn publish(&self, state: &State) {
        self.inner.bus.audio(view_of(state));
    }

    fn watch_sinks(&self) {
        loop {
            let monitor = Command::new("bluealsa-cli")
                .args(["monitor", "--properties=Volume"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();
            match monitor {
                Ok(mut monitor) => {
                    self.sinks_listed(&self.inner.radio.sinks());
                    if let Some(lines) = monitor.stdout.take() {
                        for line in BufReader::new(lines).lines().map_while(Result::ok) {
                            match sink_change(&line) {
                                Some(Change::Added(address)) => self.sink(&address, true),
                                Some(Change::Removed(address)) => self.sink(&address, false),
                                Some(Change::Stopped) => self.sinks_listed(&[]),
                                Some(Change::Volume(address, level)) => self.volume_changed(&address, level),
                                None => {}
                            }
                        }
                    }
                    let _ = monitor.kill();
                    let _ = monitor.wait();
                }
                Err(error) => eprintln!("headphones: bluealsa-cli: {error}"),
            }
            self.sinks_listed(&[]);
            thread::sleep(MONITOR_RESTART);
        }
    }
}

fn device_for(output: Output, headphones: Option<&Device>) -> String {
    match (output, headphones) {
        (Output::Headphones, Some(headphones)) => headphones.alsa(),
        _ => TV_DEVICE.to_string(),
    }
}

fn view_of(state: &State) -> View {
    let (output, link) = state.shown();
    View {
        output,
        headphones: state.headphones.as_ref().map(|known| HeadphonesView {
            name: known.device.name.clone(),
            address: known.device.address.clone(),
            state: link,
            volume: known.volume.filter(|_| link == Link::Connected).map(|level| VolumeView {
                level: level.percent(),
                muted: level.muted,
            }),
            audio_delay: known.audio_delay.filter(|_| link == Link::Connected),
        }),
        pairing: state.pairing.clone(),
        error: state.error.clone(),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Change {
    Added(String),
    Removed(String),
    Stopped,
    Volume(String, Level),
}

fn sink_address(path: &str) -> Option<String> {
    let rest = path.split("/dev_").nth(1)?;
    let (device, profile) = rest.split_once('/')?;
    (profile == "a2dpsrc/sink").then(|| device.replace('_', ":"))
}

fn sink_change(line: &str) -> Option<Change> {
    let mut words = line.split_whitespace();
    match (words.next(), words.next()) {
        (Some("PCMAdded"), Some(path)) => sink_address(path).map(Change::Added),
        (Some("PCMRemoved"), Some(path)) => sink_address(path).map(Change::Removed),
        (Some("ServiceStopped"), _) => Some(Change::Stopped),
        (Some("PropertyChanged"), Some(path)) => match (words.next(), words.next()) {
            (Some("Volume"), Some(raw)) => {
                let raw = u16::from_str_radix(raw.trim_start_matches("0x"), 16).ok()?;
                sink_address(path).map(|address| Change::Volume(address, Level::from_property(raw)))
            }
            _ => None,
        },
        _ => None,
    }
}

fn pcm_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix(key)?.trim();
        let rest = rest.strip_prefix("L:").unwrap_or(rest);
        rest.split_whitespace().next().map(str::to_string)
    })
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Info {
    name: Option<String>,
    headphones: bool,
    in_range: bool,
    bonded: bool,
    apple: bool,
}

fn parse_info(text: &str) -> Info {
    let mut info = Info::default();
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once(": ") else {
            continue;
        };
        match key {
            "Alias" => info.name = Some(value.trim().to_string()),
            "Name" if info.name.is_none() => info.name = Some(value.trim().to_string()),
            "Icon" => info.headphones = HEADPHONE_ICONS.contains(&value.trim()),
            "RSSI" => info.in_range = true,
            "Bonded" => info.bonded = value.trim() == "yes",
            "Modalias" => info.apple = value.trim().starts_with(APPLE),
            _ => {}
        }
    }
    info
}

fn shown_name(info: &Info) -> String {
    match (&info.name, info.apple && info.headphones) {
        (_, true) => "AirPods".to_string(),
        (Some(name), false) => name.strip_suffix(FIND_MY).unwrap_or(name).to_string(),
        (None, false) => "Headphones".to_string(),
    }
}

fn parse_devices(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            (words.next() == Some("Device")).then(|| words.next()).flatten()
        })
        .map(str::to_string)
        .collect()
}

#[derive(Default)]
struct Bluetoothctl {
    scan: Mutex<Option<Child>>,
}

impl Bluetoothctl {
    fn run(args: &[&str]) -> String {
        Command::new("bluetoothctl")
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_else(|error| {
                eprintln!("headphones: bluetoothctl: {error}");
                String::new()
            })
    }

    fn device(address: &str) -> (Device, Info) {
        let info = parse_info(&Self::run(&["info", address]));
        let device = Device {
            address: address.to_string(),
            name: shown_name(&info),
        };
        (device, info)
    }
}

impl Radio for Bluetoothctl {
    fn bonded(&self, address: &str) -> Option<Device> {
        let (device, info) = Self::device(address);
        (info.headphones && info.bonded).then_some(device)
    }

    fn paired(&self) -> Option<Device> {
        parse_devices(&Self::run(&["devices", "Bonded"]))
            .iter()
            .map(|address| Self::device(address))
            .find(|(_, info)| info.headphones && info.bonded)
            .map(|(device, _)| device)
    }

    fn connect(&self, address: &str) -> Result<(), String> {
        let said = Self::run(&["--timeout", COMMAND_TIMEOUT_SECONDS, "connect", address]);
        if said.contains("Connection successful") {
            Ok(())
        } else {
            Err(format!("bluetoothctl connect {address}: {}", said.trim()))
        }
    }

    fn disconnect(&self, address: &str) {
        Self::run(&["--timeout", COMMAND_TIMEOUT_SECONDS, "disconnect", address]);
    }

    fn start_scan(&self) {
        let scan = Command::new("bluetoothctl")
            .args(["--timeout", "3600", "scan", "bredr"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match scan {
            Ok(child) => *self.scan.lock().unwrap() = Some(child),
            Err(error) => eprintln!("headphones: bluetoothctl scan: {error}"),
        }
    }

    fn stop_scan(&self) {
        if let Some(mut child) = self.scan.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn found(&self) -> Option<Device> {
        parse_devices(&Self::run(&["devices"]))
            .iter()
            .map(|address| Self::device(address))
            .find(|(_, info)| info.headphones && info.in_range)
            .map(|(device, _)| device)
    }

    fn pair(&self, address: &str) -> Result<(), String> {
        Self::run(&["pairable", "on"]);
        let said = Self::run(&[
            "--agent",
            "NoInputNoOutput",
            "--timeout",
            COMMAND_TIMEOUT_SECONDS,
            "pair",
            address,
        ]);
        Self::run(&["pairable", "off"]);
        if !said.contains("Pairing successful") {
            return Err(format!("bluetoothctl pair {address}: {}", said.trim()));
        }
        if !Self::device(address).1.bonded {
            return Err(format!("bluetoothctl pair {address}: paired without bonding"));
        }
        let trusted = Self::run(&["trust", address]);
        if trusted.contains("trust succeeded") {
            Ok(())
        } else {
            Err(format!("bluetoothctl trust {address}: {}", trusted.trim()))
        }
    }

    fn forget(&self, address: &str) {
        Self::run(&["remove", address]);
    }

    fn sinks(&self) -> Vec<String> {
        bluealsa_cli(&["list-pcms"]).lines().filter_map(sink_address).collect()
    }

    fn volume(&self, address: &str) -> Option<Level> {
        let path = sink_path(address)?;
        let value = pcm_value(&bluealsa_cli(&["volume", &path]), "Volume:")?.parse().ok()?;
        let muted = pcm_value(&bluealsa_cli(&["mute", &path]), "Muted:")? == "true";
        Some(Level { value, muted })
    }

    fn delay_adjustment(&self, address: &str) -> Option<i32> {
        let path = sink_path(address)?;
        let said = bluealsa_cli(&["delay-adjustment", &path]);
        let milliseconds: f64 = pcm_value(&said, "DelayAdjustment:")?.parse().ok()?;
        Some(milliseconds.round() as i32)
    }

    fn set_delay_adjustment(&self, address: &str, milliseconds: i32) -> Result<(), String> {
        let path = sink_path(address).ok_or_else(|| format!("no BlueALSA sink for {address}"))?;
        let value = format!("{milliseconds:+}");
        let status = Command::new("bluealsa-cli")
            .args(["delay-adjustment", &path, &value])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| format!("bluealsa-cli: {error}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("bluealsa-cli delay-adjustment {path} {value}: {status}"))
        }
    }

    fn set_volume(&self, address: &str, level: Level) -> Result<(), String> {
        let path = sink_path(address).ok_or_else(|| format!("no BlueALSA sink for {address}"))?;
        let value = level.value.to_string();
        let muted = if level.muted { "on" } else { "off" };
        for args in [["volume", path.as_str(), value.as_str()], ["mute", path.as_str(), muted]] {
            let status = Command::new("bluealsa-cli")
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|error| format!("bluealsa-cli: {error}"))?;
            if !status.success() {
                return Err(format!("bluealsa-cli {}: {status}", args.join(" ")));
            }
        }
        Ok(())
    }
}

fn bluealsa_cli(args: &[&str]) -> String {
    Command::new("bluealsa-cli")
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default()
}

fn sink_path(address: &str) -> Option<String> {
    bluealsa_cli(&["list-pcms"])
        .lines()
        .find(|line| sink_address(line).as_deref() == Some(address))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Event;
    use std::sync::mpsc;

    const AIRPODS: &str = "34:0E:22:67:03:38";
    const OTHER: &str = "00:11:22:33:44:55";
    const SINK: &str = "/org/bluealsa/hci0/dev_34_0E_22_67_03_38/a2dpsrc/sink";

    fn airpods() -> Device {
        Device {
            address: AIRPODS.to_string(),
            name: "AirPods".to_string(),
        }
    }

    #[derive(Default)]
    struct World {
        paired: Option<Device>,
        bonded: Vec<Device>,
        connects: bool,
        sinks: Vec<String>,
        in_range: Option<Device>,
        pairs: bool,
        volume: Option<Level>,
        adjustment: i32,
    }

    struct FakeRadio {
        world: Arc<Mutex<World>>,
        calls: mpsc::Sender<String>,
    }

    impl FakeRadio {
        fn call(&self, call: String) {
            let _ = self.calls.send(call);
        }
    }

    impl Radio for FakeRadio {
        fn paired(&self) -> Option<Device> {
            self.world.lock().unwrap().paired.clone()
        }

        fn bonded(&self, address: &str) -> Option<Device> {
            let world = self.world.lock().unwrap();
            world.paired.iter().chain(&world.bonded).find(|device| device.address == address).cloned()
        }

        fn connect(&self, address: &str) -> Result<(), String> {
            self.call(format!("connect {address}"));
            if self.world.lock().unwrap().connects {
                Ok(())
            } else {
                Err("Failed to connect".to_string())
            }
        }

        fn disconnect(&self, address: &str) {
            self.call(format!("disconnect {address}"));
        }

        fn start_scan(&self) {
            self.call("scan".to_string());
        }

        fn stop_scan(&self) {
            self.call("stop scan".to_string());
        }

        fn found(&self) -> Option<Device> {
            self.world.lock().unwrap().in_range.clone()
        }

        fn pair(&self, address: &str) -> Result<(), String> {
            self.call(format!("pair {address}"));
            if self.world.lock().unwrap().pairs {
                Ok(())
            } else {
                Err("Failed to pair".to_string())
            }
        }

        fn forget(&self, address: &str) {
            self.call(format!("forget {address}"));
        }

        fn sinks(&self) -> Vec<String> {
            self.world.lock().unwrap().sinks.clone()
        }

        fn volume(&self, _address: &str) -> Option<Level> {
            self.world.lock().unwrap().volume
        }

        fn delay_adjustment(&self, _address: &str) -> Option<i32> {
            Some(self.world.lock().unwrap().adjustment)
        }

        fn set_delay_adjustment(&self, address: &str, milliseconds: i32) -> Result<(), String> {
            self.call(format!("delay {address} {milliseconds}"));
            self.world.lock().unwrap().adjustment = milliseconds;
            Ok(())
        }

        fn set_volume(&self, address: &str, level: Level) -> Result<(), String> {
            self.call(format!("volume {address} {} {}", level.value, level.muted));
            self.world.lock().unwrap().volume = Some(level);
            Ok(())
        }
    }

    struct Rig {
        audio: Audio,
        bus: Bus,
        world: Arc<Mutex<World>>,
        calls: mpsc::Receiver<String>,
        routes: mpsc::Receiver<Route>,
    }

    fn rig(world: World) -> Rig {
        let bus = Bus::default();
        let world = Arc::new(Mutex::new(world));
        let (calls_sent, calls) = mpsc::channel();
        let (routes_sent, routes) = mpsc::channel();
        let routes_sent = Mutex::new(routes_sent);
        let audio = Audio::new(
            Box::new(FakeRadio {
                world: world.clone(),
                calls: calls_sent,
            }),
            bus.clone(),
            Box::new(move |route| {
                let _ = routes_sent.lock().unwrap().send(route);
            }),
            Timing {
                sink_wait: Duration::from_millis(300),
                scan: Duration::from_millis(300),
                poll: Duration::from_millis(10),
            },
        );
        Rig {
            audio,
            bus,
            world,
            calls,
            routes,
        }
    }

    fn paired_rig() -> Rig {
        rig(World {
            paired: Some(airpods()),
            connects: true,
            pairs: true,
            ..World::default()
        })
    }

    fn call(rig: &Rig) -> String {
        rig.calls.recv_timeout(Duration::from_secs(2)).unwrap()
    }

    fn calls_until(rig: &Rig, last: &str) -> Vec<String> {
        let mut calls = Vec::new();
        while let Ok(call) = rig.calls.recv_timeout(Duration::from_secs(2)) {
            let done = call == last;
            calls.push(call);
            if done {
                break;
            }
        }
        calls
    }

    fn route(rig: &Rig) -> Route {
        rig.routes.recv_timeout(Duration::from_secs(2)).unwrap()
    }

    fn settled(rig: &Rig, wanted: impl Fn(&View) -> bool) -> View {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let view = rig.audio.view();
            if wanted(&view) || Instant::now() > deadline {
                return view;
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn link(view: &View) -> Option<Link> {
        view.headphones.as_ref().map(|headphones| headphones.state)
    }

    #[test]
    fn the_view_is_the_json_the_web_app_reads() {
        let rig = paired_rig();
        assert_eq!(
            serde_json::to_value(Event::Audio(rig.audio.view())).unwrap(),
            serde_json::json!({
                "type": "audio",
                "output": "tv",
                "headphones": { "name": "AirPods", "address": AIRPODS, "state": "disconnected", "volume": null, "audio_delay": null },
                "pairing": null,
                "error": null,
            })
        );
        assert_eq!(rig.bus.audio_state(), Some(rig.audio.view()));
    }

    #[test]
    fn choosing_the_headphones_connects_them_and_moves_the_sound_once_they_play() {
        let rig = paired_rig();
        let view = rig.audio.choose(Output::Headphones);
        assert_eq!((view.output, link(&view)), (Output::Tv, Some(Link::Connecting)));
        assert_eq!(rig.audio.device(), TV_DEVICE);
        assert_eq!(call(&rig), format!("connect {AIRPODS}"));
        rig.audio.sink(AIRPODS, true);
        assert_eq!(
            route(&rig),
            Route {
                device: "alsa/bluealsa:DEV=34:0E:22:67:03:38,PROFILE=a2dp".to_string(),
                said: "Sound on AirPods".to_string(),
            }
        );
        let view = rig.audio.view();
        assert_eq!((view.output, link(&view)), (Output::Headphones, Some(Link::Connected)));
        assert_eq!(rig.audio.device(), "alsa/bluealsa:DEV=34:0E:22:67:03:38,PROFILE=a2dp");
        assert_eq!(rig.bus.audio_state(), Some(view));
    }

    fn sony() -> Device {
        Device {
            address: OTHER.to_string(),
            name: "WH-1000XM5".to_string(),
        }
    }

    #[test]
    fn selecting_another_profiles_headphones_brings_the_sound_back_to_the_tv() {
        let rig = paired_rig();
        rig.world.lock().unwrap().sinks = vec![AIRPODS.to_string()];
        rig.audio.choose(Output::Headphones);
        assert_eq!(route(&rig).said, "Sound on AirPods");
        rig.audio.select(Some(sony()));
        assert_eq!(route(&rig).device, TV_DEVICE);
        let view = rig.audio.view();
        assert_eq!(view.output, Output::Tv);
        assert_eq!(view.headphones.unwrap().name, "WH-1000XM5");
        assert!(calls_until(&rig, &format!("disconnect {AIRPODS}")).contains(&format!("disconnect {AIRPODS}")));
        rig.audio.select(Some(sony()));
        assert!(rig.routes.recv_timeout(Duration::from_millis(100)).is_err(), "the same headphones change nothing");
    }

    fn tick(pace: &mut Pace, start: Instant, millis: u64, position: f64) -> Flow {
        pace.observe(start + Duration::from_millis(millis), Some((position, 1.0)))
    }

    #[test]
    fn a_video_keeping_pace_flows_and_one_that_crawls_stalls() {
        let start = Instant::now();
        let mut pace = Pace::default();
        assert_eq!(tick(&mut pace, start, 0, 100.0), Flow::Silent);
        assert_eq!(tick(&mut pace, start, 500, 100.5), Flow::Flowing);
        assert_eq!(tick(&mut pace, start, 1000, 101.0), Flow::Flowing);
        assert_eq!(tick(&mut pace, start, 1500, 101.05), Flow::Silent);
        assert_eq!(tick(&mut pace, start, 2000, 101.1), Flow::Silent);
        assert_eq!(tick(&mut pace, start, 3000, 101.2), Flow::Stalled);
        assert_eq!(pace.observe(start + Duration::from_millis(3500), None), Flow::Idle);
        let mut seeking = Pace::default();
        tick(&mut seeking, start, 0, 100.0);
        assert_eq!(tick(&mut seeking, start, 500, 700.0), Flow::Silent, "a seek forward is no verdict");
        assert_eq!(tick(&mut seeking, start, 1000, 50.0), Flow::Silent, "nor is one back");
    }

    fn playing_rig() -> Rig {
        let rig = paired_rig();
        rig.world.lock().unwrap().sinks = vec![AIRPODS.to_string()];
        rig.audio.heard(Flow::Silent, Instant::now());
        rig
    }

    #[test]
    fn headphones_that_take_the_sound_during_a_video_count_only_once_it_keeps_playing() {
        let rig = playing_rig();
        rig.audio.choose(Output::Headphones);
        assert_eq!(route(&rig).said, "Sound on AirPods");
        let view = rig.audio.view();
        assert_eq!((view.output, link(&view)), (Output::Tv, Some(Link::Connecting)), "no success while untested");
        let start = Instant::now();
        rig.audio.heard(Flow::Flowing, start);
        assert_eq!(rig.audio.view().output, Output::Tv);
        rig.audio.heard(Flow::Flowing, start + TRIAL);
        let view = rig.audio.view();
        assert_eq!((view.output, link(&view)), (Output::Headphones, Some(Link::Connected)));
        rig.audio.heard(Flow::Stalled, start + TRIAL * 2);
        assert_eq!(rig.audio.view().output, Output::Headphones, "proven headphones aren't second-guessed");
    }

    #[test]
    fn headphones_that_stall_the_video_give_the_sound_back_to_the_tv_at_once() {
        let rig = playing_rig();
        rig.audio.choose(Output::Headphones);
        assert_eq!(route(&rig).said, "Sound on AirPods");
        rig.audio.heard(Flow::Stalled, Instant::now());
        assert_eq!(route(&rig).device, TV_DEVICE);
        assert_eq!(rig.audio.device(), TV_DEVICE);
        let view = rig.audio.view();
        assert_eq!(view.output, Output::Tv);
        assert_eq!(view.error.as_deref(), Some("AirPods didn't play. The sound is back on the TV."));
    }

    #[test]
    fn with_nothing_playing_connected_headphones_have_the_sound_until_a_video_tests_them() {
        let rig = paired_rig();
        rig.world.lock().unwrap().sinks = vec![AIRPODS.to_string()];
        rig.audio.choose(Output::Headphones);
        route(&rig);
        assert_eq!(rig.audio.view().output, Output::Headphones);
        rig.audio.heard(Flow::Silent, Instant::now());
        assert_eq!(rig.audio.view().output, Output::Tv, "a video starting puts them on trial");
        rig.audio.heard(Flow::Stalled, Instant::now());
        assert_eq!(route(&rig).device, TV_DEVICE);
    }

    #[test]
    fn headphones_that_never_play_leave_the_sound_on_the_tv() {
        for (name, connects) in [("connecting fails", false), ("no sink before the deadline", true)] {
            let rig = paired_rig();
            rig.world.lock().unwrap().connects = connects;
            rig.audio.choose(Output::Headphones);
            let view = settled(&rig, |view| link(view) != Some(Link::Connecting));
            assert_eq!(
                (view.output, link(&view)),
                (Output::Tv, Some(Link::Disconnected)),
                "{name}"
            );
            assert_eq!(
                view.error.as_deref(),
                Some("AirPods didn't connect. Open the case near the TV and try again."),
                "{name}"
            );
            assert_eq!(
                calls_until(&rig, &format!("disconnect {AIRPODS}")),
                [format!("connect {AIRPODS}"), format!("disconnect {AIRPODS}")],
                "{name}"
            );
            assert!(rig.routes.try_recv().is_err(), "{name}");
            let view = rig.audio.choose(Output::Headphones);
            assert_eq!((link(&view), view.error), (Some(Link::Connecting), None), "{name}");
        }
    }

    #[test]
    fn headphones_that_drop_send_the_sound_back_to_the_tv() {
        let rig = paired_rig();
        rig.audio.choose(Output::Headphones);
        rig.audio.sink(AIRPODS, true);
        route(&rig);
        rig.audio.sink(OTHER, false);
        assert_eq!(rig.audio.view().output, Output::Headphones);
        rig.audio.sink(AIRPODS, false);
        assert_eq!(route(&rig).device, TV_DEVICE);
        let view = rig.audio.view();
        assert_eq!((view.output, link(&view)), (Output::Tv, Some(Link::Disconnected)));
        assert_eq!(
            view.error.as_deref(),
            Some("AirPods disconnected. The sound is back on the TV.")
        );
    }

    #[test]
    fn choosing_the_tv_moves_the_sound_and_lets_the_headphones_go() {
        let rig = paired_rig();
        rig.audio.choose(Output::Headphones);
        call(&rig);
        rig.audio.sink(AIRPODS, true);
        route(&rig);
        let view = rig.audio.choose(Output::Tv);
        assert_eq!(view.output, Output::Tv);
        assert_eq!(route(&rig).device, TV_DEVICE);
        assert_eq!(call(&rig), format!("disconnect {AIRPODS}"));
        rig.audio.sink(AIRPODS, false);
        let view = rig.audio.view();
        assert_eq!((link(&view), view.error), (Some(Link::Disconnected), None));
    }

    #[test]
    fn headphones_already_connected_take_the_sound_at_once() {
        let rig = paired_rig();
        rig.audio.sinks_listed(&[AIRPODS.to_string()]);
        assert_eq!(rig.audio.view().output, Output::Tv);
        let view = rig.audio.choose(Output::Headphones);
        assert_eq!((view.output, link(&view)), (Output::Headphones, Some(Link::Connected)));
        assert_eq!(route(&rig).said, "Sound on AirPods");
        assert!(rig.calls.try_recv().is_err());
    }

    #[test]
    fn choosing_the_tv_while_connecting_gives_up_and_lets_the_headphones_go() {
        let rig = paired_rig();
        rig.audio.choose(Output::Headphones);
        call(&rig);
        let view = rig.audio.choose(Output::Tv);
        assert_eq!((view.output, link(&view)), (Output::Tv, Some(Link::Disconnected)));
        assert_eq!(call(&rig), format!("disconnect {AIRPODS}"));
        assert!(rig.routes.try_recv().is_err());
    }

    #[test]
    fn without_paired_headphones_the_sound_stays_on_the_tv() {
        let rig = rig(World::default());
        let view = rig.audio.choose(Output::Headphones);
        assert_eq!((view.output, view.headphones), (Output::Tv, None));
        assert_eq!(view.error.as_deref(), Some(NONE_PAIRED));
        assert!(rig.calls.try_recv().is_err());
    }

    #[test]
    fn pairing_scans_pairs_the_headphones_found_and_connects_them_without_taking_the_sound() {
        let rig = rig(World {
            connects: true,
            pairs: true,
            ..World::default()
        });
        let view = rig.audio.pair().unwrap();
        assert_eq!(view.pairing.unwrap().state, Stage::Scanning);
        assert_eq!(rig.audio.pair(), Err(Busy));
        rig.world.lock().unwrap().sinks = vec![AIRPODS.to_string()];
        rig.world.lock().unwrap().in_range = Some(airpods());
        assert_eq!(
            calls_until(&rig, &format!("connect {AIRPODS}")),
            [
                "scan".to_string(),
                "stop scan".to_string(),
                format!("pair {AIRPODS}"),
                format!("connect {AIRPODS}")
            ]
        );
        let view = settled(&rig, |view| link(view) == Some(Link::Connected));
        assert_eq!(view.output, Output::Tv);
        assert_eq!(view.headphones.unwrap().name, "AirPods");
        assert_eq!(
            view.pairing,
            Some(PairingView {
                state: Stage::Paired,
                message: "Paired with AirPods.".to_string()
            })
        );
        assert!(rig.routes.try_recv().is_err());
    }

    #[test]
    fn pairing_new_headphones_forgets_the_old_ones() {
        let rig = paired_rig();
        let other = Device {
            address: OTHER.to_string(),
            name: "Other".to_string(),
        };
        rig.world.lock().unwrap().in_range = Some(other);
        rig.audio.pair().unwrap();
        let calls = calls_until(&rig, &format!("connect {OTHER}"));
        assert!(calls.contains(&format!("forget {AIRPODS}")));
        assert_eq!(rig.audio.view().headphones.unwrap().address, OTHER);
    }

    #[test]
    fn pairing_the_same_headphones_again_forgets_them_and_pairs_them_once_found_again() {
        let rig = paired_rig();
        rig.world.lock().unwrap().in_range = Some(airpods());
        rig.audio.pair().unwrap();
        assert_eq!(
            calls_until(&rig, &format!("pair {AIRPODS}")),
            [
                "scan".to_string(),
                format!("forget {AIRPODS}"),
                "stop scan".to_string(),
                format!("pair {AIRPODS}")
            ]
        );
    }

    #[test]
    fn pairing_that_fails_says_why_and_keeps_the_old_headphones() {
        let refused =
            "AirPods didn't pair. Hold the button on the case until the light flashes white and try again.".to_string();
        let cases = [
            (
                "nothing found",
                World {
                    paired: Some(airpods()),
                    connects: true,
                    pairs: true,
                    ..World::default()
                },
                NOTHING_FOUND.to_string(),
                Some(AIRPODS),
                vec!["scan".to_string(), "stop scan".to_string()],
            ),
            (
                "pairing refused",
                World {
                    in_range: Some(airpods()),
                    ..World::default()
                },
                refused,
                None,
                vec!["scan".to_string(), "stop scan".to_string(), format!("pair {AIRPODS}")],
            ),
        ];
        for (name, world, message, headphones, calls) in cases {
            let rig = rig(world);
            rig.audio.pair().unwrap();
            let view = settled(&rig, |view| {
                view.pairing
                    .as_ref()
                    .is_some_and(|pairing| pairing.state == Stage::Failed)
            });
            assert_eq!(view.pairing.map(|pairing| pairing.message), Some(message), "{name}");
            assert_eq!(
                view.headphones.map(|known| known.address).as_deref(),
                headphones,
                "{name}"
            );
            assert_eq!(calls_until(&rig, calls.last().unwrap()), calls, "{name}");
        }
    }

    #[test]
    fn a_cancelled_scan_stops_and_pairs_nothing() {
        let rig = rig(World::default());
        rig.audio.pair().unwrap();
        assert_eq!(call(&rig), "scan");
        assert_eq!(rig.audio.cancel_pairing().pairing, None);
        rig.world.lock().unwrap().in_range = Some(airpods());
        assert_eq!(call(&rig), "stop scan");
        thread::sleep(Duration::from_millis(50));
        assert!(rig.calls.try_recv().is_err());
        assert_eq!(rig.audio.view().headphones, None);
    }

    #[test]
    fn forgetting_headphones_returns_the_sound_and_unbonds_unless_kept() {
        let cases = [
            ("on the tv, kept by another profile", true, false),
            ("on the tv", false, false),
            ("playing, kept by another profile", true, true),
            ("playing", false, true),
        ];
        for (name, kept, playing) in cases {
            let rig = paired_rig();
            if kept {
                rig.audio.set_kept([AIRPODS.to_string()]);
            }
            if playing {
                rig.audio.sinks_listed(&[AIRPODS.to_string()]);
                rig.audio.choose(Output::Headphones);
                route(&rig);
            }
            let view = rig.audio.forget();
            assert_eq!((view.output, view.headphones), (Output::Tv, None), "{name}");
            if playing {
                assert_eq!(route(&rig).device, TV_DEVICE, "{name}");
            } else {
                assert!(rig.routes.try_recv().is_err(), "{name}");
            }
            assert_eq!(rig.audio.device(), TV_DEVICE, "{name}");
            if kept {
                assert!(rig.calls.recv_timeout(Duration::from_millis(100)).is_err(), "{name}");
            } else {
                assert_eq!(call(&rig), format!("forget {AIRPODS}"), "{name}");
            }
            assert_eq!(rig.audio.view().headphones, None, "{name}");
        }
    }

    #[test]
    fn volume_keys_change_the_headphones_only_while_they_have_the_sound() {
        let rig = paired_rig();
        rig.world.lock().unwrap().volume = Some(Level {
            value: 102,
            muted: false,
        });
        rig.audio.sinks_listed(&[AIRPODS.to_string()]);
        assert_eq!(
            rig.audio.view().headphones.unwrap().volume,
            Some(VolumeView {
                level: 80,
                muted: false
            })
        );
        assert_eq!(rig.audio.change_volume(VolumeChange::Up), None);
        rig.audio.choose(Output::Headphones);
        route(&rig);
        assert_eq!(
            rig.audio.change_volume(VolumeChange::Up),
            Some(Ok("AirPods volume 87%".to_string()))
        );
        assert_eq!(call(&rig), format!("volume {AIRPODS} 110 false"));
        for _ in 0..3 {
            rig.audio.change_volume(VolumeChange::Up);
        }
        assert_eq!(
            rig.audio.change_volume(VolumeChange::Up),
            Some(Ok("AirPods volume 100%".to_string()))
        );
        assert_eq!(
            rig.audio.change_volume(VolumeChange::Mute),
            Some(Ok("AirPods muted".to_string()))
        );
        assert_eq!(
            rig.audio.view().headphones.unwrap().volume,
            Some(VolumeView {
                level: 100,
                muted: true
            })
        );
        assert_eq!(
            rig.audio.change_volume(VolumeChange::Down),
            Some(Ok("AirPods volume 94%".to_string()))
        );
        assert_eq!(rig.bus.audio_state(), Some(rig.audio.view()));
    }

    #[test]
    fn the_sound_on_the_headphones_can_be_nudged_later_or_earlier() {
        let rig = paired_rig();
        rig.world.lock().unwrap().adjustment = 20;
        assert_eq!(
            rig.audio.delay_sound(-20),
            Err("The headphones aren't connected".to_string())
        );
        rig.audio.sinks_listed(&[AIRPODS.to_string()]);
        assert_eq!(rig.audio.view().headphones.unwrap().audio_delay, Some(-20));
        let view = rig.audio.delay_sound(50).unwrap();
        assert_eq!(view.headphones.unwrap().audio_delay, Some(30));
        assert_eq!(call(&rig), format!("delay {AIRPODS} -30"));
        assert_eq!(rig.audio.delay_sound(900).unwrap().headphones.unwrap().audio_delay, Some(500));
        assert_eq!(rig.bus.audio_state(), Some(rig.audio.view()));
    }

    #[test]
    fn apple_headphones_are_called_airpods_and_others_keep_their_name() {
        let airpods = parse_info(
            "\tName: airpodsfs - Find My\n\tAlias: airpodsfs - Find My\n\tIcon: audio-headphones\n\tModalias: bluetooth:v004Cp2027d242F\n",
        );
        assert_eq!(shown_name(&airpods), "AirPods");
        let other = parse_info("\tAlias: Sony WH - Find My\n\tIcon: audio-headphones\n\tModalias: bluetooth:v0039p4EF0d0001\n");
        assert_eq!(shown_name(&other), "Sony WH");
        assert_eq!(shown_name(&Info::default()), "Headphones");
    }

    #[test]
    fn the_headphones_own_volume_changes_show_up() {
        let rig = paired_rig();
        rig.audio.sinks_listed(&[AIRPODS.to_string()]);
        let line = format!("PropertyChanged {SINK} Volume 0x4040");
        let Some(Change::Volume(address, level)) = sink_change(&line) else {
            panic!("a volume change");
        };
        rig.audio.volume_changed(&address, level);
        assert_eq!(
            rig.audio.view().headphones.unwrap().volume,
            Some(VolumeView {
                level: 50,
                muted: false
            })
        );
        assert_eq!(
            sink_change(&format!("PropertyChanged {SINK} Volume 0xE6E6")),
            Some(Change::Volume(
                AIRPODS.to_string(),
                Level {
                    value: 102,
                    muted: true
                }
            ))
        );
        rig.audio.sinks_listed(&[]);
        assert_eq!(rig.audio.view().headphones.unwrap().volume, None);
    }

    #[test]
    fn bluealsa_cli_volume_and_mute_are_read_from_the_left_channel() {
        assert_eq!(pcm_value("Volume: L: 102 R: 100\n", "Volume:"), Some("102".to_string()));
        assert_eq!(pcm_value("Muted: L: true R: true\n", "Muted:"), Some("true".to_string()));
        assert_eq!(pcm_value("Volume: 90\n", "Volume:"), Some("90".to_string()));
    }

    #[test]
    fn the_monitor_reports_headphone_sinks_by_address() {
        assert_eq!(sink_change(&format!("PCMAdded {SINK}")), Some(Change::Added(AIRPODS.to_string())));
        assert_eq!(sink_change(&format!("PCMRemoved {SINK}")), Some(Change::Removed(AIRPODS.to_string())));
        assert_eq!(sink_change("ServiceStopped org.bluealsa"), Some(Change::Stopped));
        assert_eq!(sink_change("PCMAdded /org/bluealsa/hci0/dev_34_0E_22_67_03_38/a2dpsnk/source"), None);
        assert_eq!(sink_change(&format!("PropertyChanged {SINK} Running true")), None);
    }

    #[test]
    fn bluetoothctl_info_tells_headphones_in_range() {
        let info = parse_info(
            "Device 34:0E:22:67:03:38 (public)\n\tName: AirPods\n\tAlias: AirPods von Frederik\n\tClass: 0x00240418 (2360344)\n\tIcon: audio-headphones\n\tPaired: no\n\tBonded: no\n\tTrusted: no\n\tRSSI: 0xffffffc4 (-60)\n",
        );
        assert_eq!(
            info,
            Info {
                name: Some("AirPods von Frederik".to_string()),
                headphones: true,
                in_range: true,
                bonded: false,
                apple: false,
            }
        );
        let tv = parse_info("Device D0:C2:4E:8A:F1:37 (public)\n\tName: [TV] Samsung\n\tIcon: video-display\n\tRSSI: -70\n");
        assert!(!tv.headphones);
        assert_eq!(
            parse_devices("Device 34:0E:22:67:03:38 AirPods\nDevice D0:C2:4E:8A:F1:37 [TV] Samsung\n"),
            ["34:0E:22:67:03:38", "D0:C2:4E:8A:F1:37"]
        );
    }
}
