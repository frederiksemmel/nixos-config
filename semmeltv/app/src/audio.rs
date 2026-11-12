use crate::bus::Bus;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const TV_DEVICE: &str = "auto";
const COMMAND_TIMEOUT_SECONDS: &str = "20";
const HEADPHONE_ICONS: [&str; 2] = ["audio-headphones", "audio-headset"];
const MONITOR_RESTART: Duration = Duration::from_secs(2);
const SCANNING: &str = "Open the case near the TV and hold the button on its back until the light flashes white.";
const NOTHING_FOUND: &str =
    "No headphones showed up. Open the case near the TV, hold the button on its back until the light flashes white, and try again.";
const NONE_PAIRED: &str = "No headphones are paired. Pair them in Settings.";

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
    fn connect(&self, address: &str) -> Result<(), String>;
    fn disconnect(&self, address: &str);
    fn start_scan(&self);
    fn stop_scan(&self);
    fn found(&self) -> Option<Device>;
    fn pair(&self, address: &str) -> Result<(), String>;
    fn forget(&self, address: &str);
    fn sinks(&self) -> Vec<String>;
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

#[derive(Default)]
struct State {
    headphones: Option<Device>,
    output: Output,
    link: Link,
    take_sound: bool,
    error: Option<String>,
    attempt: u64,
    pairing: Option<PairingView>,
    pair_attempt: u64,
}

impl State {
    fn pairing(&mut self, state: Stage, message: String) {
        self.pairing = Some(PairingView { state, message });
    }
}

struct Inner {
    state: Mutex<State>,
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
        let headphones = radio.paired();
        let audio = Self {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    headphones,
                    ..State::default()
                }),
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
        device_for(state.output, state.headphones.as_ref())
    }

    pub fn choose(&self, output: Output) -> View {
        let mut state = self.inner.state.lock().unwrap();
        state.error = None;
        let mut route = None;
        let mut connect = None;
        let mut disconnect = None;
        match (output, state.headphones.clone()) {
            (Output::Headphones, None) => state.error = Some(NONE_PAIRED.to_string()),
            (Output::Headphones, Some(headphones)) => match (state.output, state.link) {
                (Output::Headphones, _) => {}
                (Output::Tv, Link::Connecting) => state.take_sound = true,
                (Output::Tv, Link::Connected) => {
                    state.output = Output::Headphones;
                    route = Some(Output::Headphones);
                }
                (Output::Tv, Link::Disconnected) => {
                    state.link = Link::Connecting;
                    state.take_sound = true;
                    state.attempt += 1;
                    connect = Some((state.attempt, headphones.address));
                }
            },
            (Output::Tv, headphones) => {
                if state.output == Output::Headphones {
                    route = Some(Output::Tv);
                }
                state.output = Output::Tv;
                if state.link == Link::Connecting {
                    state.link = Link::Disconnected;
                    state.attempt += 1;
                } else if state.link == Link::Connected {
                    disconnect = headphones.map(|headphones| headphones.address);
                }
            }
        }
        self.publish(&state);
        let view = view_of(&state);
        let headphones = state.headphones.clone();
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

    pub fn forget(&self) -> View {
        let mut state = self.inner.state.lock().unwrap();
        state.error = None;
        let forgotten = state.headphones.take();
        let route = state.output == Output::Headphones;
        state.output = Output::Tv;
        state.link = Link::Disconnected;
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
        if let Some(forgotten) = forgotten {
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
                let earlier = self.inner.state.lock().unwrap().headphones.clone();
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
        let replaced = state.headphones.replace(device.clone());
        let route = state.output == Output::Headphones;
        state.output = Output::Tv;
        state.link = Link::Connecting;
        state.take_sound = false;
        state.attempt += 1;
        let connect = state.attempt;
        state.pairing(Stage::Paired, format!("Paired with {}.", device.name));
        self.publish(&state);
        drop(state);
        if route {
            self.route(Output::Tv, replaced.as_ref());
        }
        if let Some(replaced) = replaced.filter(|replaced| replaced.address != device.address) {
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
                    let abandoned = state.output == Output::Tv;
                    drop(state);
                    if abandoned {
                        radio.disconnect(address);
                    }
                    return;
                }
                if state.link != Link::Connecting {
                    return;
                }
                if connected.is_err() || Instant::now() >= deadline {
                    state.link = Link::Disconnected;
                    let name = state.headphones.as_ref().map_or("The headphones", |headphones| &headphones.name);
                    if state.take_sound {
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
        let mut state = self.inner.state.lock().unwrap();
        let Some(headphones) = state.headphones.clone().filter(|headphones| headphones.address == address) else {
            return;
        };
        let mut route = None;
        match (present, state.link) {
            (true, Link::Connecting) => {
                state.link = Link::Connected;
                if state.take_sound {
                    state.output = Output::Headphones;
                    route = Some(Output::Headphones);
                }
            }
            (true, _) => state.link = Link::Connected,
            (false, Link::Connecting) => {}
            (false, _) => {
                state.link = Link::Disconnected;
                if state.output == Output::Headphones {
                    state.output = Output::Tv;
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

    fn sinks_listed(&self, sinks: &[String]) {
        let current = self.inner.state.lock().unwrap().headphones.clone();
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
                .arg("monitor")
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
    View {
        output: state.output,
        headphones: state.headphones.as_ref().map(|headphones| HeadphonesView {
            name: headphones.name.clone(),
            address: headphones.address.clone(),
            state: state.link,
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
        _ => None,
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Info {
    name: Option<String>,
    headphones: bool,
    in_range: bool,
    bonded: bool,
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
            _ => {}
        }
    }
    info
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
            name: info.name.clone().unwrap_or_else(|| "Headphones".to_string()),
        };
        (device, info)
    }
}

impl Radio for Bluetoothctl {
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
        Command::new("bluealsa-cli")
            .arg("list-pcms")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .filter_map(sink_address)
                    .collect()
            })
            .unwrap_or_default()
    }
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
        connects: bool,
        sinks: Vec<String>,
        in_range: Option<Device>,
        pairs: bool,
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
                "headphones": { "name": "AirPods", "address": AIRPODS, "state": "disconnected" },
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

    #[test]
    fn a_sink_already_there_after_connecting_takes_the_sound() {
        let rig = paired_rig();
        rig.world.lock().unwrap().sinks = vec![AIRPODS.to_string()];
        rig.audio.choose(Output::Headphones);
        assert_eq!(route(&rig).said, "Sound on AirPods");
        assert_eq!(rig.audio.view().output, Output::Headphones);
    }

    #[test]
    fn headphones_that_do_not_connect_leave_the_sound_on_the_tv_with_an_error() {
        let rig = paired_rig();
        rig.world.lock().unwrap().connects = false;
        rig.audio.choose(Output::Headphones);
        let view = settled(&rig, |view| link(view) != Some(Link::Connecting));
        assert_eq!((view.output, link(&view)), (Output::Tv, Some(Link::Disconnected)));
        assert_eq!(
            view.error.as_deref(),
            Some("AirPods didn't connect. Open the case near the TV and try again.")
        );
        assert_eq!(
            calls_until(&rig, &format!("disconnect {AIRPODS}")),
            [format!("connect {AIRPODS}"), format!("disconnect {AIRPODS}")]
        );
        assert!(rig.routes.try_recv().is_err());
        let view = rig.audio.choose(Output::Headphones);
        assert_eq!((link(&view), view.error), (Some(Link::Connecting), None));
    }

    #[test]
    fn headphones_that_connect_without_playing_count_as_failed() {
        let rig = paired_rig();
        rig.audio.choose(Output::Headphones);
        let view = settled(&rig, |view| link(view) != Some(Link::Connecting));
        assert_eq!(link(&view), Some(Link::Disconnected));
        assert!(view.error.is_some());
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
    fn pairing_that_finds_nothing_fails_and_keeps_the_old_headphones() {
        let rig = paired_rig();
        rig.audio.pair().unwrap();
        let view = settled(&rig, |view| {
            view.pairing.as_ref().is_some_and(|pairing| pairing.state == Stage::Failed)
        });
        assert_eq!(view.pairing.unwrap().message, NOTHING_FOUND);
        assert_eq!(view.headphones.unwrap().address, AIRPODS);
        assert_eq!(calls_until(&rig, "stop scan"), ["scan", "stop scan"]);
    }

    #[test]
    fn headphones_that_refuse_to_pair_fail_with_their_name() {
        let rig = rig(World {
            in_range: Some(airpods()),
            ..World::default()
        });
        rig.audio.pair().unwrap();
        let view = settled(&rig, |view| {
            view.pairing.as_ref().is_some_and(|pairing| pairing.state == Stage::Failed)
        });
        assert!(view.pairing.unwrap().message.starts_with("AirPods didn't pair."));
        assert_eq!(view.headphones, None);
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
    fn forgetting_the_headphones_brings_the_sound_back_to_the_tv() {
        let rig = paired_rig();
        rig.audio.sinks_listed(&[AIRPODS.to_string()]);
        rig.audio.choose(Output::Headphones);
        route(&rig);
        let view = rig.audio.forget();
        assert_eq!((view.output, view.headphones), (Output::Tv, None));
        assert_eq!(route(&rig).device, TV_DEVICE);
        assert_eq!(call(&rig), format!("forget {AIRPODS}"));
        assert_eq!(rig.audio.device(), TV_DEVICE);
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
