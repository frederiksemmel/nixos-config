use crate::core::{Core, Request};
use crate::intent::Control;
use crate::command::Command;
use cec_linux::{
    CecDevice, CecEvent, CecLogAddrType, CecLogAddrs, CecLogicalAddress, CecModeFollower, CecModeInitiator, CecOpcode,
    CecPhysicalAddress, CecPrimDevType, CecUserControlCode, PollFlags, PollTimeout, VendorID, Version,
};
use std::fs;
use std::io;
use std::mem;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

const DEVICE_NAME: &str = "SemmelTV";
const MENU_ACTIVATED: u8 = 0;
const POWER_ON: u8 = 0;
const REPEAT_WINDOW: Duration = Duration::from_millis(600);
const MIN_UNPLUGGED: Duration = Duration::from_secs(10);
const COLD_BOOT_UPTIME: Duration = Duration::from_secs(180);
const POWER_ON_WAIT: Duration = Duration::from_secs(15);
const POWER_POLL: Duration = Duration::from_millis(500);
const HOLD_INPUT: Duration = Duration::from_secs(5);
const REPROBE_FIRST: Duration = Duration::from_secs(2);
const REPROBE_LONGEST: Duration = Duration::from_secs(300);
const REPROBE_EFFECT: Duration = Duration::from_secs(2);
const DRM_CONNECTORS: &str = "/sys/class/drm";
const CEC_ADAP_G_CONNECTOR_INFO: u32 = 0x8044_610a;
const CEC_CONNECTOR_TYPE_DRM: u32 = 1;

static INPUT_HELD_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);
static ACTIVE_SOURCE: AtomicBool = AtomicBool::new(false);

pub fn spawn(remote: Core) {
    for path in adapters(Path::new("/dev")) {
        let remote = remote.clone();
        thread::spawn(move || {
            if let Err(error) = listen(&path, &remote) {
                eprintln!("{}: {error}", path.display());
            }
        });
    }
}

pub fn show_semmeltv() -> io::Result<()> {
    each_adapter(show_on)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Showing,
    Elsewhere,
    Off,
}

pub fn screen() -> Screen {
    screen_of(tv_is_on().ok(), ACTIVE_SOURCE.load(Ordering::Relaxed))
}

fn screen_of(on: Option<bool>, active_source: bool) -> Screen {
    match (on, active_source) {
        (Some(true), true) => Screen::Showing,
        (Some(true), false) => Screen::Elsewhere,
        (Some(false) | None, _) => Screen::Off,
    }
}

fn tv_is_on() -> io::Result<bool> {
    let mut result = Err(io::Error::other("no HDMI-CEC adapter is connected"));
    for path in adapters(Path::new("/dev")) {
        let power = CecDevice::open(&path).and_then(|device| tv_power(&device));
        match power {
            Ok(status) => return Ok(status == POWER_ON),
            Err(error) if result.is_err() => result = Err(error),
            Err(_) => {}
        }
    }
    result
}

pub fn turn_tv_off() -> io::Result<()> {
    ACTIVE_SOURCE.store(false, Ordering::Relaxed);
    each_adapter(|_, device| device.transmit(own_logical_address(&device)?, CecLogicalAddress::Tv, CecOpcode::Standby))
}

fn each_adapter(send: impl Fn(&Path, CecDevice) -> io::Result<()>) -> io::Result<()> {
    let mut result = Err(io::Error::other("no HDMI-CEC adapter is connected"));
    for path in adapters(Path::new("/dev")) {
        let sent = CecDevice::open(&path).and_then(|device| send(&path, device));
        if result.is_err() {
            result = sent;
        }
    }
    result
}

fn show_on(path: &Path, device: CecDevice) -> io::Result<()> {
    turn_on_tv(&device)?;
    hold_input(Instant::now() + POWER_ON_WAIT + HOLD_INPUT);
    let path = path.to_path_buf();
    thread::spawn(move || {
        wait_for_power_on(&device);
        log_failure(&path, "Active Source", announce(&device));
        hold_input(Instant::now() + HOLD_INPUT);
    });
    Ok(())
}

fn wait_for_power_on(device: &CecDevice) {
    let deadline = Instant::now() + POWER_ON_WAIT;
    while Instant::now() < deadline {
        if tv_power(device).is_ok_and(|status| status == POWER_ON) {
            return;
        }
        thread::sleep(POWER_POLL);
    }
}

fn tv_power(device: &CecDevice) -> io::Result<u8> {
    let reply = device.request_data(
        own_logical_address(device)?,
        CecLogicalAddress::Tv,
        CecOpcode::GiveDevicePowerStatus,
        &[],
        CecOpcode::ReportPowerStatus,
    )?;
    reply
        .first()
        .copied()
        .ok_or_else(|| io::Error::other("empty power status"))
}

fn hold_input(until: Instant) {
    *INPUT_HELD_UNTIL.lock().unwrap_or_else(PoisonError::into_inner) = Some(until);
}

fn holding_input(now: Instant) -> bool {
    INPUT_HELD_UNTIL
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_some_and(|until| now < until)
}

fn adapters(dev: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dev) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| is_adapter_name(&path.file_name().unwrap_or_default().to_string_lossy()))
        .collect()
}

fn is_adapter_name(name: &str) -> bool {
    name.strip_prefix("cec")
        .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
}

fn listen(path: &Path, remote: &Core) -> io::Result<()> {
    let device = CecDevice::open(path)?;
    device.set_mode(CecModeInitiator::Send, CecModeFollower::All)?;
    claim_address(&device)?;
    let plugged = device.get_phys()? != CecPhysicalAddress::INVALID;
    let mut wake = TvWake::start(plugged, uptime(), Instant::now());
    let mut reprobe = Reprobe::default();
    reprobe.link_changed(plugged, Instant::now());
    let mut keys = KeyFilter::default();
    loop {
        let ready = device.poll(PollFlags::POLLIN | PollFlags::POLLPRI, reprobe.timeout(Instant::now()))?;
        if reprobe.due(Instant::now()) {
            match probe_connector(&device) {
                Ok(true) => eprintln!(
                    "{}: no physical address while HDMI is connected, probing the connector",
                    path.display()
                ),
                Ok(false) => {}
                Err(error) => eprintln!("{}: probing the connector failed: {error}", path.display()),
            }
            reprobe.probed(Instant::now());
        }
        if ready.contains(PollFlags::POLLPRI) {
            if let CecEvent::StateChange(state) = device.get_event()? {
                let plugged = state.phys_addr != CecPhysicalAddress::INVALID;
                if reprobe.link_changed(plugged, Instant::now()) {
                    eprintln!(
                        "{}: physical address {:?} is back after probing",
                        path.display(),
                        state.phys_addr
                    );
                    wake.recovered();
                }
                match wake.link_changed(plugged, !state.log_addr_mask.is_empty(), Instant::now()) {
                    Greeting::Silent => {}
                    Greeting::Announce => log_failure(path, "Active Source", announce(&device)),
                    Greeting::Wake => log_failure(
                        path,
                        "switch to SemmelTV",
                        CecDevice::open(path).and_then(|device| show_on(path, device)),
                    ),
                }
            }
        }
        if ready.contains(PollFlags::POLLIN) {
            let message = device.rec()?;
            let Some(Ok(opcode)) = message.opcode() else {
                continue;
            };
            if !message.is_ok() {
                continue;
            }
            wake.heard(message.initiator(), opcode);
            let own_address = device.get_phys()?;
            let reaction = react(opcode, message.parameters(), own_address);
            if lost_the_screen(opcode, &reaction) {
                ACTIVE_SOURCE.store(false, Ordering::Relaxed);
            }
            match reaction {
                Reaction::Ignore => {}
                Reaction::Press(command) => {
                    if let Some(command) = keys.press(command, Instant::now()) {
                        remote.send(Request::Key(command));
                    }
                }
                Reaction::Release => keys.release(),
                Reaction::Announce => log_failure(path, "Active Source", announce(&device)),
                Reaction::Stop if holding_input(Instant::now()) => {
                    eprintln!(
                        "{}: {opcode:?} while switching to SemmelTV, claiming the input back",
                        path.display()
                    );
                    log_failure(path, "Active Source", announce(&device));
                }
                Reaction::Stop => {
                    eprintln!(
                        "{}: {opcode:?} moved the TV to another input, stopping playback",
                        path.display()
                    );
                    remote.send(Request::Control(Control::Stop));
                }
                Reaction::Reply(reply, status) => {
                    let sent = own_logical_address(&device)
                        .and_then(|from| device.transmit_data(from, message.initiator(), reply, &[status]));
                    log_failure(path, "reply", sent);
                }
            }
        }
    }
}

fn log_failure(path: &Path, action: &str, result: io::Result<()>) {
    if let Err(error) = result {
        eprintln!("{}: {action} failed: {error}", path.display());
    }
}

fn claim_address(device: &CecDevice) -> io::Result<()> {
    let addresses = CecLogAddrs::new(
        VendorID::NONE,
        Version::V1_4,
        DEVICE_NAME
            .to_string()
            .try_into()
            .map_err(|_| io::Error::other("device name is too long"))?,
        &[CecPrimDevType::PLAYBACK],
        &[CecLogAddrType::PLAYBACK],
    );
    match device.set_log(addresses) {
        Err(error) if error.kind() == io::ErrorKind::ResourceBusy => Ok(()),
        result => result,
    }
}

fn own_logical_address(device: &CecDevice) -> io::Result<CecLogicalAddress> {
    device
        .get_log()?
        .addresses()
        .first()
        .copied()
        .ok_or_else(|| io::Error::other("no logical address claimed"))
}

fn turn_on_tv(device: &CecDevice) -> io::Result<()> {
    device.turn_on(own_logical_address(device)?, CecLogicalAddress::Tv)
}

fn announce(device: &CecDevice) -> io::Result<()> {
    device.transmit_data(
        own_logical_address(device)?,
        CecLogicalAddress::UnregisteredBroadcast,
        CecOpcode::ActiveSource,
        &device.get_phys()?.to_bytes(),
    )?;
    ACTIVE_SOURCE.store(true, Ordering::Relaxed);
    Ok(())
}

fn lost_the_screen(opcode: CecOpcode, reaction: &Reaction) -> bool {
    opcode == CecOpcode::Standby || *reaction == Reaction::Stop
}

#[derive(Debug, PartialEq, Eq)]
enum Reaction {
    Ignore,
    Press(Command),
    Release,
    Announce,
    Stop,
    Reply(CecOpcode, u8),
}

fn react(opcode: CecOpcode, parameters: &[u8], own_address: CecPhysicalAddress) -> Reaction {
    match opcode {
        CecOpcode::UserControlPressed => parameters
            .first()
            .and_then(|code| CecUserControlCode::try_from(*code).ok())
            .and_then(key_command)
            .map_or(Reaction::Ignore, Reaction::Press),
        CecOpcode::UserControlReleased => Reaction::Release,
        CecOpcode::RequestActiveSource => Reaction::Announce,
        CecOpcode::SetStreamPath if own_address == parameters => Reaction::Announce,
        CecOpcode::SetStreamPath | CecOpcode::ActiveSource if elsewhere(parameters.get(..2), own_address) => {
            Reaction::Stop
        }
        CecOpcode::RoutingChange if elsewhere(parameters.get(2..4), own_address) => Reaction::Stop,
        CecOpcode::MenuRequest => Reaction::Reply(CecOpcode::MenuStatus, MENU_ACTIVATED),
        CecOpcode::GiveDevicePowerStatus => Reaction::Reply(CecOpcode::ReportPowerStatus, POWER_ON),
        _ => Reaction::Ignore,
    }
}

fn elsewhere(address: Option<&[u8]>, own_address: CecPhysicalAddress) -> bool {
    address.is_some_and(|address| own_address != address)
}

fn key_command(code: CecUserControlCode) -> Option<Command> {
    match code {
        CecUserControlCode::Up => Some(Command::Up),
        CecUserControlCode::Down => Some(Command::Down),
        CecUserControlCode::Left | CecUserControlCode::Rewind => Some(Command::Left),
        CecUserControlCode::Right | CecUserControlCode::FastForward => Some(Command::Right),
        CecUserControlCode::Select | CecUserControlCode::Enter => Some(Command::Select),
        CecUserControlCode::Exit | CecUserControlCode::Stop => Some(Command::Back),
        CecUserControlCode::Play
        | CecUserControlCode::Pause
        | CecUserControlCode::PlayFunction
        | CecUserControlCode::PausePlayFunction => Some(Command::PlayPause),
        CecUserControlCode::F1Blue | CecUserControlCode::F2Red => Some(Command::Listen),
        CecUserControlCode::F3Green => Some(Command::Dialogue),
        CecUserControlCode::F4Yellow => Some(Command::Subtitles),
        _ => None,
    }
}

#[derive(Default)]
struct KeyFilter {
    held: Option<(Command, Instant)>,
}

impl KeyFilter {
    fn press(&mut self, command: Command, now: Instant) -> Option<Command> {
        let repeated = self
            .held
            .is_some_and(|(held, since)| held == command && now - since < REPEAT_WINDOW);
        self.held = Some((command, now));
        let may_repeat = matches!(command, Command::Up | Command::Down | Command::Left | Command::Right);
        (!repeated || may_repeat).then_some(command)
    }

    fn release(&mut self) {
        self.held = None;
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Greeting {
    Silent,
    Announce,
    Wake,
}

struct TvWake {
    unplugged_since: Option<Instant>,
    wake_owed: bool,
    tv_off: bool,
}

impl TvWake {
    fn start(plugged: bool, uptime: Duration, now: Instant) -> Self {
        Self {
            unplugged_since: (!plugged).then_some(now),
            wake_owed: plugged && uptime < COLD_BOOT_UPTIME,
            tv_off: false,
        }
    }

    fn link_changed(&mut self, plugged: bool, claimed: bool, now: Instant) -> Greeting {
        if !plugged {
            self.unplugged_since.get_or_insert(now);
            self.wake_owed = false;
            return Greeting::Silent;
        }
        if let Some(since) = self.unplugged_since.take() {
            self.wake_owed = now - since >= MIN_UNPLUGGED && !self.tv_off;
        }
        if !claimed {
            return Greeting::Silent;
        }
        if mem::take(&mut self.wake_owed) {
            Greeting::Wake
        } else {
            Greeting::Announce
        }
    }

    fn recovered(&mut self) {
        self.unplugged_since = None;
        self.wake_owed = false;
    }

    fn heard(&mut self, initiator: CecLogicalAddress, opcode: CecOpcode) {
        match opcode {
            CecOpcode::Standby => self.tv_off = true,
            CecOpcode::RequestActiveSource | CecOpcode::SetStreamPath if initiator == CecLogicalAddress::Tv => {
                self.tv_off = false;
            }
            _ => {}
        }
    }
}

fn uptime() -> Duration {
    fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|contents| parse_uptime(&contents))
        .unwrap_or(Duration::MAX)
}

fn parse_uptime(contents: &str) -> Option<Duration> {
    let seconds = contents.split_whitespace().next()?.parse().ok()?;
    Duration::try_from_secs_f64(seconds).ok()
}

#[derive(Default)]
struct Reprobe {
    next: Option<Instant>,
    wait: Duration,
    probed_at: Option<Instant>,
}

impl Reprobe {
    fn link_changed(&mut self, plugged: bool, now: Instant) -> bool {
        if !plugged {
            if self.next.is_none() {
                self.wait = REPROBE_FIRST;
                self.next = Some(now + REPROBE_FIRST);
            }
            return false;
        }
        self.next = None;
        self.probed_at
            .take()
            .is_some_and(|at| now.saturating_duration_since(at) <= REPROBE_EFFECT)
    }

    fn due(&self, now: Instant) -> bool {
        self.next.is_some_and(|next| now >= next)
    }

    fn probed(&mut self, now: Instant) {
        self.probed_at = Some(now);
        self.wait = (self.wait * 2).min(REPROBE_LONGEST);
        self.next = Some(now + self.wait);
    }

    fn timeout(&self, now: Instant) -> PollTimeout {
        match self.next {
            None => PollTimeout::NONE,
            Some(next) => PollTimeout::try_from(next.saturating_duration_since(now)).unwrap_or(PollTimeout::MAX),
        }
    }
}

#[repr(C)]
struct ConnectorInfo {
    kind: u32,
    raw: [u32; 16],
}

fn probe_connector(device: &CecDevice) -> io::Result<bool> {
    let (card, connector) = drm_connector(device)?;
    let status = connector_status(Path::new(DRM_CONNECTORS), card, connector)
        .ok_or_else(|| io::Error::other(format!("no DRM connector {connector} on card {card}")))?;
    if fs::read_to_string(&status)?.trim() != "connected" {
        return Ok(false);
    }
    fs::write(&status, "detect")?;
    Ok(true)
}

fn drm_connector(device: &CecDevice) -> io::Result<(u32, u32)> {
    let mut info = ConnectorInfo { kind: 0, raw: [0; 16] };
    let result = unsafe { libc::ioctl(device.as_raw_fd(), CEC_ADAP_G_CONNECTOR_INFO as _, &raw mut info) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    if info.kind != CEC_CONNECTOR_TYPE_DRM {
        return Err(io::Error::other("the adapter is not tied to a DRM connector"));
    }
    Ok((info.raw[0], info.raw[1]))
}

fn connector_status(drm: &Path, card: u32, connector: u32) -> Option<PathBuf> {
    let prefix = format!("card{card}-");
    let id = connector.to_string();
    fs::read_dir(drm)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(&prefix))
        })
        .find(|path| fs::read_to_string(path.join("connector_id")).is_ok_and(|read| read.trim() == id))
        .map(|path| path.join("status"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    const OWN: CecPhysicalAddress = CecPhysicalAddress::from_num(0x1000);

    fn press(code: u8) -> Reaction {
        react(CecOpcode::UserControlPressed, &[code], OWN)
    }

    #[test]
    fn remote_buttons_become_commands_or_nothing() {
        let cases: [(&[u8], Reaction); 20] = [
            (&[0x00], Reaction::Press(Command::Select)),
            (&[0x01], Reaction::Press(Command::Up)),
            (&[0x02], Reaction::Press(Command::Down)),
            (&[0x03], Reaction::Press(Command::Left)),
            (&[0x04], Reaction::Press(Command::Right)),
            (&[0x0d], Reaction::Press(Command::Back)),
            (&[0x44], Reaction::Press(Command::PlayPause)),
            (&[0x45], Reaction::Press(Command::Back)),
            (&[0x46], Reaction::Press(Command::PlayPause)),
            (&[0x48], Reaction::Press(Command::Left)),
            (&[0x49], Reaction::Press(Command::Right)),
            (&[0x60], Reaction::Press(Command::PlayPause)),
            (&[0x61], Reaction::Press(Command::PlayPause)),
            (&[0x71], Reaction::Press(Command::Listen)),
            (&[0x72], Reaction::Press(Command::Listen)),
            (&[0x73], Reaction::Press(Command::Dialogue)),
            (&[0x74], Reaction::Press(Command::Subtitles)),
            (&[0x40], Reaction::Ignore),
            (&[0xff], Reaction::Ignore),
            (&[], Reaction::Ignore),
        ];
        for (code, reaction) in cases {
            assert_eq!(react(CecOpcode::UserControlPressed, code, OWN), reaction, "{code:x?}");
        }
    }

    #[test]
    fn the_screen_shows_semmeltv_only_when_the_tv_is_on_and_semmeltv_is_the_active_source() {
        assert_eq!(screen_of(Some(true), true), Screen::Showing);
        assert_eq!(screen_of(Some(true), false), Screen::Elsewhere);
        assert_eq!(screen_of(Some(false), true), Screen::Off, "standby");
        assert_eq!(screen_of(None, true), Screen::Off, "no answer over CEC");
    }

    #[test]
    fn semmeltv_stops_being_shown_on_standby_or_another_input() {
        let elsewhere = react(CecOpcode::ActiveSource, &[0x20, 0x00], OWN);
        assert!(lost_the_screen(CecOpcode::ActiveSource, &elsewhere));
        assert!(lost_the_screen(CecOpcode::Standby, &Reaction::Ignore));
        let ours = react(CecOpcode::SetStreamPath, &[0x10, 0x00], OWN);
        assert!(!lost_the_screen(CecOpcode::SetStreamPath, &ours));
        assert!(!lost_the_screen(CecOpcode::UserControlPressed, &press(0x00)));
    }

    #[test]
    fn each_cec_message_gets_its_reaction() {
        let cases: [(CecOpcode, &[u8], Reaction); 13] = [
            (CecOpcode::RequestActiveSource, &[], Reaction::Announce),
            (
                CecOpcode::MenuRequest,
                &[0],
                Reaction::Reply(CecOpcode::MenuStatus, MENU_ACTIVATED),
            ),
            (
                CecOpcode::GiveDevicePowerStatus,
                &[],
                Reaction::Reply(CecOpcode::ReportPowerStatus, POWER_ON),
            ),
            (CecOpcode::SetStreamPath, &[0x10, 0x00], Reaction::Announce),
            (CecOpcode::SetStreamPath, &[0x20, 0x00], Reaction::Stop),
            (CecOpcode::RoutingChange, &[0x10, 0x00, 0x20, 0x00], Reaction::Stop),
            (CecOpcode::RoutingChange, &[0x10, 0x00, 0x00, 0x00], Reaction::Stop),
            (CecOpcode::ActiveSource, &[0x30, 0x00], Reaction::Stop),
            (CecOpcode::RoutingChange, &[0x20, 0x00, 0x10, 0x00], Reaction::Ignore),
            (CecOpcode::ActiveSource, &[0x10, 0x00], Reaction::Ignore),
            (CecOpcode::RoutingChange, &[0x10, 0x00, 0x20], Reaction::Ignore),
            (CecOpcode::ActiveSource, &[0x30], Reaction::Ignore),
            (CecOpcode::SetStreamPath, &[], Reaction::Ignore),
        ];
        for (opcode, parameters, reaction) in cases {
            assert_eq!(react(opcode, parameters, OWN), reaction, "{opcode:?} {parameters:x?}");
        }
    }

    #[test]
    fn key_repeats_pass_only_for_arrows() {
        enum Key {
            Press(Command, u64, Option<Command>),
            Release,
        }
        use Key::{Press, Release};
        let select = Some(Command::Select);
        let cases: [(&str, &[Key]); 3] = [
            (
                "holding select",
                &[
                    Press(Command::Select, 0, select),
                    Press(Command::Select, 200, None),
                    Press(Command::Select, 400, None),
                    Press(Command::Select, 600, None),
                    Press(Command::Select, 800, None),
                ],
            ),
            (
                "select after a release or a pause",
                &[
                    Press(Command::Select, 0, select),
                    Release,
                    Press(Command::Select, 50, select),
                    Press(Command::Select, 2000, select),
                ],
            ),
            (
                "holding an arrow",
                &[
                    Press(Command::Right, 0, Some(Command::Right)),
                    Press(Command::Right, 200, Some(Command::Right)),
                ],
            ),
        ];
        for (name, keys) in cases {
            let mut filter = KeyFilter::default();
            let start = Instant::now();
            for (step, key) in keys.iter().enumerate() {
                match *key {
                    Press(command, millis, passed) => assert_eq!(
                        filter.press(command, start + Duration::from_millis(millis)),
                        passed,
                        "{name}, step {step}"
                    ),
                    Release => filter.release(),
                }
            }
        }
    }

    const COLD: Duration = Duration::from_secs(45);
    const WARM: Duration = Duration::from_secs(3600);

    fn secs(seconds: f64) -> Duration {
        Duration::from_secs_f64(seconds)
    }

    fn running(start: Instant) -> TvWake {
        let mut wake = TvWake::start(true, WARM, start);
        assert_eq!(wake.link_changed(true, true, start), Greeting::Announce);
        wake
    }

    fn replug(wake: &mut TvWake, at: Instant, absent: Duration) -> Greeting {
        assert_eq!(wake.link_changed(false, false, at), Greeting::Silent);
        wake.link_changed(true, true, at + absent)
    }

    #[test]
    fn replugging_wakes_only_after_ten_seconds_away() {
        let cases: [(&str, &[(f64, f64, Greeting)]); 2] = [
            ("ten seconds away", &[(0.0, 10.0, Greeting::Wake)]),
            (
                "short hot plug pulses",
                &[(0.0, 0.5, Greeting::Announce), (1.0, 9.9, Greeting::Announce)],
            ),
        ];
        for (name, replugs) in cases {
            let start = Instant::now();
            let mut wake = running(start);
            for (at, absent, greeting) in replugs {
                assert_eq!(
                    replug(&mut wake, start + secs(*at), secs(*absent)),
                    *greeting,
                    "{name}, at {at}"
                );
            }
            assert_eq!(
                wake.link_changed(true, true, start + secs(60.0)),
                Greeting::Announce,
                "{name}, a change of valid address"
            );
        }
    }

    #[test]
    fn repeated_unplug_events_keep_the_first_timestamp() {
        let start = Instant::now();
        let mut wake = running(start);
        wake.link_changed(false, false, start);
        wake.link_changed(false, false, start + secs(9.0));
        assert_eq!(wake.link_changed(true, true, start + secs(10.0)), Greeting::Wake);
    }

    #[test]
    fn the_wake_waits_for_a_logical_address() {
        let start = Instant::now();
        let mut wake = running(start);
        wake.link_changed(false, false, start);
        assert_eq!(wake.link_changed(true, false, start + secs(20.0)), Greeting::Silent);
        assert_eq!(wake.link_changed(true, true, start + secs(20.1)), Greeting::Wake);
    }

    #[test]
    fn unplugging_before_the_claim_cancels_the_wake() {
        let start = Instant::now();
        let mut wake = running(start);
        wake.link_changed(false, false, start);
        wake.link_changed(true, false, start + secs(20.0));
        wake.link_changed(false, false, start + secs(20.1));
        assert_eq!(wake.link_changed(true, true, start + secs(20.2)), Greeting::Announce);
    }

    #[test]
    fn start_wakes_only_after_a_cold_boot() {
        let cases = [
            (COLD, Greeting::Wake),
            (secs(179.9), Greeting::Wake),
            (secs(180.0), Greeting::Announce),
            (WARM, Greeting::Announce),
            (Duration::MAX, Greeting::Announce),
        ];
        for (uptime, greeting) in cases {
            let start = Instant::now();
            let mut wake = TvWake::start(true, uptime, start);
            assert_eq!(wake.link_changed(true, false, start), Greeting::Silent, "{uptime:?}");
            assert_eq!(wake.link_changed(true, true, start), greeting, "{uptime:?}");
            assert_eq!(
                wake.link_changed(true, true, start),
                Greeting::Announce,
                "{uptime:?}, once"
            );
        }
    }

    #[test]
    fn starting_unplugged_counts_as_absent_from_the_start() {
        let start = Instant::now();
        let mut soon = TvWake::start(false, COLD, start);
        assert_eq!(soon.link_changed(true, true, start + secs(5.0)), Greeting::Announce);
        let mut later = TvWake::start(false, WARM, start);
        assert_eq!(later.link_changed(true, true, start + secs(10.0)), Greeting::Wake);
    }

    #[test]
    fn standby_suppresses_hot_plug_wakes() {
        let start = Instant::now();
        let mut wake = running(start);
        wake.heard(CecLogicalAddress::Tv, CecOpcode::Standby);
        assert_eq!(replug(&mut wake, start, secs(60.0)), Greeting::Announce);
        assert_eq!(replug(&mut wake, start + secs(70.0), secs(60.0)), Greeting::Announce);
    }

    #[test]
    fn only_the_tv_turning_on_lifts_the_standby_suppression() {
        let cases = [
            (CecLogicalAddress::Tv, CecOpcode::RequestActiveSource, Greeting::Wake),
            (CecLogicalAddress::Tv, CecOpcode::SetStreamPath, Greeting::Wake),
            (
                CecLogicalAddress::Playback1,
                CecOpcode::RequestActiveSource,
                Greeting::Announce,
            ),
            (
                CecLogicalAddress::Tv,
                CecOpcode::GiveDevicePowerStatus,
                Greeting::Announce,
            ),
        ];
        for (initiator, opcode, greeting) in cases {
            let start = Instant::now();
            let mut wake = running(start);
            wake.heard(CecLogicalAddress::Tv, CecOpcode::Standby);
            wake.heard(initiator, opcode);
            assert_eq!(
                replug(&mut wake, start, secs(60.0)),
                greeting,
                "{initiator:?} {opcode:?}"
            );
        }
    }

    #[test]
    fn a_cold_boot_wakes_even_after_standby() {
        let start = Instant::now();
        let mut wake = TvWake::start(true, COLD, start);
        wake.heard(CecLogicalAddress::Tv, CecOpcode::Standby);
        assert_eq!(wake.link_changed(true, true, start), Greeting::Wake);
    }

    #[test]
    fn uptime_is_the_first_field_of_proc_uptime() {
        assert_eq!(parse_uptime("123.50 4567.89\n"), Some(secs(123.5)));
        assert_eq!(parse_uptime(""), None);
        assert_eq!(parse_uptime("nonsense 1.0"), None);
        assert_eq!(parse_uptime("-1.0 1.0"), None);
    }

    #[test]
    fn only_cec_devices_count_as_adapters() {
        assert!(is_adapter_name("cec0"));
        assert!(is_adapter_name("cec12"));
        assert!(!is_adapter_name("cec"));
        assert!(!is_adapter_name("cecil"));
    }

    #[test]
    fn structures_match_the_kernel_abi() {
        assert_eq!(size_of::<cec_linux::CecMsg>(), 56);
        assert_eq!(size_of::<CecLogAddrs>(), 92);
        assert_eq!(size_of::<ConnectorInfo>(), 68);
        let read = 2 << 30;
        assert_eq!(
            CEC_ADAP_G_CONNECTOR_INFO,
            read | ((size_of::<ConnectorInfo>() as u32) << 16) | (u32::from(b'a') << 8) | 10
        );
    }

    #[test]
    fn a_lost_address_is_probed_with_growing_waits() {
        let start = Instant::now();
        let mut reprobe = Reprobe::default();
        assert!(!reprobe.due(start + secs(3600.0)));
        assert!(!reprobe.link_changed(false, start));
        assert!(!reprobe.due(start + secs(1.9)));
        assert!(reprobe.due(start + secs(2.0)));
        let mut now = start + secs(2.0);
        let mut waits = Vec::new();
        for _ in 0..10 {
            reprobe.probed(now);
            let next = reprobe.next.unwrap();
            waits.push((next - now).as_secs());
            now = next;
        }
        assert_eq!(waits, [4, 8, 16, 32, 64, 128, 256, 300, 300, 300]);
    }

    #[test]
    fn an_address_back_right_after_a_probe_counts_as_recovered() {
        let start = Instant::now();
        let mut reprobe = Reprobe::default();
        reprobe.link_changed(false, start);
        reprobe.probed(start + secs(2.0));
        assert!(reprobe.link_changed(true, start + secs(2.1)));
        assert!(!reprobe.due(start + secs(3600.0)));

        reprobe.link_changed(false, start + secs(10.0));
        reprobe.probed(start + secs(12.0));
        assert!(!reprobe.link_changed(true, start + secs(20.0)));

        let mut unprobed = Reprobe::default();
        unprobed.link_changed(false, start);
        assert!(!unprobed.link_changed(true, start + secs(1.0)));
    }

    #[test]
    fn a_recovered_address_announces_without_waking_the_tv() {
        let start = Instant::now();
        let mut wake = TvWake::start(true, WARM, start);
        assert_eq!(wake.link_changed(true, true, start), Greeting::Announce);
        assert_eq!(wake.link_changed(false, false, start + secs(1.0)), Greeting::Silent);
        wake.recovered();
        assert_eq!(wake.link_changed(true, true, start + secs(3600.0)), Greeting::Announce);
    }

    #[test]
    fn the_connector_status_is_found_by_card_and_connector_id() {
        let drm = std::env::temp_dir().join(format!("semmeltv-cec-drm-{}", std::process::id()));
        let _ = fs::remove_dir_all(&drm);
        for (name, id) in [
            ("card0-HDMI-A-1", "35"),
            ("card1-HDMI-A-1", "36"),
            ("card1-HDMI-A-2", "35"),
        ] {
            fs::create_dir_all(drm.join(name)).unwrap();
            fs::write(drm.join(name).join("connector_id"), format!("{id}\n")).unwrap();
        }
        fs::create_dir_all(drm.join("card1")).unwrap();
        assert_eq!(connector_status(&drm, 1, 35), Some(drm.join("card1-HDMI-A-2/status")));
        assert_eq!(connector_status(&drm, 0, 35), Some(drm.join("card0-HDMI-A-1/status")));
        assert_eq!(connector_status(&drm, 1, 99), None);
        fs::remove_dir_all(&drm).unwrap();
    }
}
