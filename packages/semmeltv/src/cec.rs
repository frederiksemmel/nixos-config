use crate::app::Remote;
use crate::command::Command;
use cec_linux::{
    CecDevice, CecEvent, CecLogAddrType, CecLogAddrs, CecLogicalAddress, CecModeFollower, CecModeInitiator, CecOpcode,
    CecPhysicalAddress, CecPrimDevType, CecUserControlCode, PollFlags, PollTimeout, VendorID, Version,
};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

const DEVICE_NAME: &str = "SemmelTV";
const MENU_ACTIVATED: u8 = 0;
const POWER_ON: u8 = 0;
const REPEAT_WINDOW: Duration = Duration::from_millis(600);

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
    let mut keys = KeyFilter::default();
    loop {
        let ready = device.poll(PollFlags::POLLIN | PollFlags::POLLPRI, PollTimeout::NONE)?;
        if ready.contains(PollFlags::POLLPRI) {
            if let CecEvent::StateChange(state) = device.get_event()? {
                if state.phys_addr != CecPhysicalAddress::INVALID {
                    announce(&device)?;
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
            let own_address = device.get_phys()?;
            match react(opcode, message.parameters(), own_address) {
                Reaction::Ignore => {}
                Reaction::Press(command) => {
                    if let Some(command) = keys.press(command, Instant::now()) {
                        remote.send(command);
                    }
                }
                Reaction::Release => keys.release(),
                Reaction::Announce => announce(&device)?,
                Reaction::Reply(reply, status) => {
                    let from = own_logical_address(&device)?;
                    device.transmit_data(from, message.initiator(), reply, &[status])?;
                }
            }
        }
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
        CecOpcode::MenuRequest => Reaction::Reply(CecOpcode::MenuStatus, MENU_ACTIVATED),
        CecOpcode::GiveDevicePowerStatus => Reaction::Reply(CecOpcode::ReportPowerStatus, POWER_ON),
        _ => Reaction::Ignore,
    }
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
    fn stream_paths_only_matter_when_they_point_here() {
        assert_eq!(react(CecOpcode::SetStreamPath, &[0x10, 0x00], OWN), Reaction::Announce);
        assert_eq!(react(CecOpcode::SetStreamPath, &[0x20, 0x00], OWN), Reaction::Ignore);
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
