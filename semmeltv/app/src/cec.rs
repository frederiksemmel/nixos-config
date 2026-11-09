use crate::app::Remote;
use crate::command::Command;
use cec_linux::{
    CecDevice, CecEvent, CecLogAddrType, CecLogAddrs, CecLogicalAddress, CecModeFollower, CecModeInitiator, CecOpcode,
    CecPhysicalAddress, CecPrimDevType, CecUserControlCode, PollFlags, PollTimeout, VendorID, Version,
};
use std::fs;
use std::io;
use std::mem;
use std::path::{Path, PathBuf};
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

static INPUT_HELD_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);

pub fn spawn(remote: Remote) {
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

pub fn turn_tv_off() -> io::Result<()> {
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

fn listen(path: &Path, remote: &Remote) -> io::Result<()> {
    let device = CecDevice::open(path)?;
    device.set_mode(CecModeInitiator::Send, CecModeFollower::All)?;
    claim_address(&device)?;
    let plugged = device.get_phys()? != CecPhysicalAddress::INVALID;
    let mut wake = TvWake::start(plugged, uptime(), Instant::now());
    let mut keys = KeyFilter::default();
    loop {
        let ready = device.poll(PollFlags::POLLIN | PollFlags::POLLPRI, PollTimeout::NONE)?;
        if ready.contains(PollFlags::POLLPRI) {
            if let CecEvent::StateChange(state) = device.get_event()? {
                let plugged = state.phys_addr != CecPhysicalAddress::INVALID;
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
            match react(opcode, message.parameters(), own_address) {
                Reaction::Ignore => {}
                Reaction::Press(command) => {
                    if let Some(command) = keys.press(command, Instant::now()) {
                        remote.send(command);
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
                    remote.stop();
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
    )
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
        CecUserControlCode::F1Blue
        | CecUserControlCode::F2Red
        | CecUserControlCode::F3Green
        | CecUserControlCode::F4Yellow => Some(Command::Listen),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    const OWN: CecPhysicalAddress = CecPhysicalAddress::from_num(0x1000);

    fn press(code: u8) -> Reaction {
        react(CecOpcode::UserControlPressed, &[code], OWN)
    }

    #[test]
    fn remote_buttons_become_commands() {
        let cases = [
            (0x00, Command::Select),
            (0x01, Command::Up),
            (0x02, Command::Down),
            (0x03, Command::Left),
            (0x04, Command::Right),
            (0x0d, Command::Back),
            (0x44, Command::PlayPause),
            (0x45, Command::Back),
            (0x46, Command::PlayPause),
            (0x48, Command::Left),
            (0x49, Command::Right),
            (0x60, Command::PlayPause),
            (0x61, Command::PlayPause),
        ];
        for (code, command) in cases {
            assert_eq!(press(code), Reaction::Press(command), "{code:#x}");
        }
    }

    #[test]
    fn unknown_or_empty_buttons_are_ignored() {
        assert_eq!(press(0x40), Reaction::Ignore);
        assert_eq!(press(0xff), Reaction::Ignore);
        assert_eq!(react(CecOpcode::UserControlPressed, &[], OWN), Reaction::Ignore);
    }

    #[test]
    fn the_tv_gets_the_answers_it_needs_to_forward_keys() {
        assert_eq!(react(CecOpcode::RequestActiveSource, &[], OWN), Reaction::Announce);
        assert_eq!(
            react(CecOpcode::MenuRequest, &[0], OWN),
            Reaction::Reply(CecOpcode::MenuStatus, MENU_ACTIVATED)
        );
        assert_eq!(
            react(CecOpcode::GiveDevicePowerStatus, &[], OWN),
            Reaction::Reply(CecOpcode::ReportPowerStatus, POWER_ON)
        );
    }

    #[test]
    fn a_stream_path_here_announces_and_one_elsewhere_stops() {
        assert_eq!(react(CecOpcode::SetStreamPath, &[0x10, 0x00], OWN), Reaction::Announce);
        assert_eq!(react(CecOpcode::SetStreamPath, &[0x20, 0x00], OWN), Reaction::Stop);
    }

    #[test]
    fn switching_the_tv_to_another_input_stops_playback() {
        assert_eq!(
            react(CecOpcode::RoutingChange, &[0x10, 0x00, 0x20, 0x00], OWN),
            Reaction::Stop
        );
        assert_eq!(
            react(CecOpcode::RoutingChange, &[0x10, 0x00, 0x00, 0x00], OWN),
            Reaction::Stop
        );
        assert_eq!(react(CecOpcode::ActiveSource, &[0x30, 0x00], OWN), Reaction::Stop);
    }

    #[test]
    fn switching_the_tv_to_this_input_does_not_stop() {
        assert_eq!(
            react(CecOpcode::RoutingChange, &[0x20, 0x00, 0x10, 0x00], OWN),
            Reaction::Ignore
        );
        assert_eq!(react(CecOpcode::ActiveSource, &[0x10, 0x00], OWN), Reaction::Ignore);
    }

    #[test]
    fn truncated_routing_messages_are_ignored() {
        assert_eq!(
            react(CecOpcode::RoutingChange, &[0x10, 0x00, 0x20], OWN),
            Reaction::Ignore
        );
        assert_eq!(react(CecOpcode::ActiveSource, &[0x30], OWN), Reaction::Ignore);
        assert_eq!(react(CecOpcode::SetStreamPath, &[], OWN), Reaction::Ignore);
    }

    #[test]
    fn holding_select_does_not_select_twice() {
        let mut keys = KeyFilter::default();
        let start = Instant::now();
        assert_eq!(keys.press(Command::Select, start), Some(Command::Select));
        for step in 1..5 {
            let now = start + Duration::from_millis(step * 200);
            assert_eq!(keys.press(Command::Select, now), None);
        }
    }

    #[test]
    fn select_works_again_after_a_release_or_a_pause() {
        let mut keys = KeyFilter::default();
        let start = Instant::now();
        keys.press(Command::Select, start);
        keys.release();
        assert!(keys.press(Command::Select, start + Duration::from_millis(50)).is_some());
        assert!(keys.press(Command::Select, start + Duration::from_secs(2)).is_some());
    }

    #[test]
    fn holding_an_arrow_keeps_moving() {
        let mut keys = KeyFilter::default();
        let start = Instant::now();
        keys.press(Command::Right, start);
        assert!(keys.press(Command::Right, start + Duration::from_millis(200)).is_some());
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
    fn plugging_in_after_ten_seconds_wakes_the_tv_once() {
        let start = Instant::now();
        let mut wake = running(start);
        assert_eq!(replug(&mut wake, start, secs(10.0)), Greeting::Wake);
        assert_eq!(wake.link_changed(true, true, start + secs(11.0)), Greeting::Announce);
    }

    #[test]
    fn a_short_hot_plug_pulse_only_announces() {
        let start = Instant::now();
        let mut wake = running(start);
        assert_eq!(replug(&mut wake, start, secs(0.5)), Greeting::Announce);
        assert_eq!(replug(&mut wake, start + secs(1.0), secs(9.9)), Greeting::Announce);
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
    fn a_change_of_valid_address_never_wakes() {
        let start = Instant::now();
        let mut wake = running(start);
        assert_eq!(wake.link_changed(true, true, start + secs(60.0)), Greeting::Announce);
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
    fn nothing_is_sent_without_a_logical_address() {
        let start = Instant::now();
        let mut wake = running(start);
        assert_eq!(wake.link_changed(true, false, start + secs(1.0)), Greeting::Silent);
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
    fn a_cold_boot_wakes_the_tv_once() {
        let start = Instant::now();
        let mut wake = TvWake::start(true, COLD, start);
        assert_eq!(wake.link_changed(true, false, start), Greeting::Silent);
        assert_eq!(wake.link_changed(true, true, start), Greeting::Wake);
        assert_eq!(wake.link_changed(true, true, start), Greeting::Announce);
    }

    #[test]
    fn a_cold_boot_ends_at_three_minutes() {
        let start = Instant::now();
        let mut wake = TvWake::start(true, secs(179.9), start);
        assert_eq!(wake.link_changed(true, true, start), Greeting::Wake);
        let mut wake = TvWake::start(true, secs(180.0), start);
        assert_eq!(wake.link_changed(true, true, start), Greeting::Announce);
    }

    #[test]
    fn a_restart_announces_without_waking() {
        let start = Instant::now();
        let mut wake = TvWake::start(true, WARM, start);
        assert_eq!(wake.link_changed(true, true, start), Greeting::Announce);
    }

    #[test]
    fn an_unknown_uptime_counts_as_a_restart() {
        let start = Instant::now();
        let mut wake = TvWake::start(true, Duration::MAX, start);
        assert_eq!(wake.link_changed(true, true, start), Greeting::Announce);
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
    fn a_tv_that_is_on_again_can_be_woken_again() {
        for opcode in [CecOpcode::RequestActiveSource, CecOpcode::SetStreamPath] {
            let start = Instant::now();
            let mut wake = running(start);
            wake.heard(CecLogicalAddress::Tv, CecOpcode::Standby);
            wake.heard(CecLogicalAddress::Tv, opcode);
            assert_eq!(replug(&mut wake, start, secs(60.0)), Greeting::Wake, "{opcode:?}");
        }
    }

    #[test]
    fn only_the_tv_can_lift_the_suppression() {
        let start = Instant::now();
        let mut wake = running(start);
        wake.heard(CecLogicalAddress::Tv, CecOpcode::Standby);
        wake.heard(CecLogicalAddress::Playback1, CecOpcode::RequestActiveSource);
        wake.heard(CecLogicalAddress::Tv, CecOpcode::GiveDevicePowerStatus);
        assert_eq!(replug(&mut wake, start, secs(60.0)), Greeting::Announce);
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
    }
}
