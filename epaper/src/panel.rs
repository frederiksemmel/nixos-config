use std::{
    path::PathBuf,
    thread::sleep,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use embedded_hal::{
    digital::{InputPin, OutputPin},
    spi::SpiDevice,
};
use linux_embedded_hal::{
    CdevPin, SpidevDevice,
    gpio_cdev::{Chip, LineRequestFlags, chips},
    spidev::{SpiModeFlags, SpidevOptions},
};

use crate::render::{Canvas, Planes};

const SPI_DEVICE: &str = "/dev/spidev0.0";
const SPI_HZ: u32 = 4_000_000;
const GPIO_LABEL: &str = "pinctrl-bcm2835";
const RST: u32 = 17;
const BUSY: u32 = 24;
const DC: u32 = 25;
const BUSY_POLL: Duration = Duration::from_millis(2);
const BUSY_TIMEOUT: Duration = Duration::from_secs(30);

/// Fast refreshes in a row before a clean one wipes their ghosts.
pub const CLEAN_EVERY: u32 = 10;

/// How the panel moves from the old picture to the new one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Refresh {
    /// Drives only the pixels that change, in a quarter second and without flashing; leaves faint ghosts.
    Fast,
    /// Flashes the whole panel black and white, in about two seconds, and leaves no ghosts.
    Clean,
}

pub trait Panel: Send {
    fn show(&mut self, canvas: &Canvas, planes: &Planes, refresh: Refresh) -> Result<()>;
}

/// The IL91874 controller of the 2.7" HAT, in black-and-white mode with waveforms from the LUT registers.
pub struct Hat {
    spi: SpidevDevice,
    busy: CdevPin,
    dc: CdevPin,
    rst: CdevPin,
    shown: Option<Vec<u8>>,
    fast_in_a_row: u32,
}

impl Hat {
    pub fn open() -> Result<Self> {
        let mut chip = gpio_chip()?;
        let mut line = |offset, flags, default, name| -> Result<CdevPin> {
            let handle = chip
                .get_line(offset)?
                .request(flags, default, name)
                .with_context(|| format!("requesting GPIO {offset} ({name})"))?;
            Ok(CdevPin::new(handle)?)
        };
        let busy = line(BUSY, LineRequestFlags::INPUT, 0, "epaper-busy")?;
        let dc = line(DC, LineRequestFlags::OUTPUT, 0, "epaper-dc")?;
        let rst = line(RST, LineRequestFlags::OUTPUT, 1, "epaper-rst")?;

        let mut spi = SpidevDevice::open(SPI_DEVICE).context(SPI_DEVICE)?;
        spi.configure(
            &SpidevOptions::new()
                .bits_per_word(8)
                .max_speed_hz(SPI_HZ)
                .mode(SpiModeFlags::SPI_MODE_0)
                .build(),
        )?;
        Ok(Hat {
            spi,
            busy,
            dc,
            rst,
            shown: None,
            fast_in_a_row: 0,
        })
    }

    fn command(&mut self, command: u8, data: &[u8]) -> Result<()> {
        self.dc.set_low()?;
        self.spi.write(&[command])?;
        if !data.is_empty() {
            self.dc.set_high()?;
            for chunk in data.chunks(4096) {
                self.spi.write(chunk)?;
            }
        }
        Ok(())
    }

    fn wait(&mut self) -> Result<()> {
        let start = Instant::now();
        while self.busy.is_low()? {
            anyhow::ensure!(
                start.elapsed() < BUSY_TIMEOUT,
                "the panel stayed busy for {BUSY_TIMEOUT:?}"
            );
            sleep(BUSY_POLL);
        }
        Ok(())
    }

    fn wake(&mut self, refresh: Refresh) -> Result<()> {
        self.rst.set_high()?;
        sleep(Duration::from_millis(10));
        self.rst.set_low()?;
        sleep(Duration::from_millis(2));
        self.rst.set_high()?;
        sleep(Duration::from_millis(20));
        self.wait()?;

        self.command(0x01, &[0x03, 0x00, 0x2b, 0x2b, 0x09])?;
        self.command(0x06, &[0x07, 0x07, 0x17])?;
        for option in [
            [0x60, 0xa5],
            [0x89, 0xa5],
            [0x90, 0x00],
            [0x93, 0x2a],
            [0xa0, 0xa5],
            [0xa1, 0x00],
            [0x73, 0x41],
        ] {
            self.command(0xf8, &option)?;
        }
        self.command(0x16, &[0x00])?;
        self.command(0x00, &[0xbf])?;
        self.command(0x30, &[0x3a])?;
        self.command(0x61, &[0x00, 0xb0, 0x01, 0x08])?;
        self.command(0x82, &[0x12])?;

        let waveform = match refresh {
            Refresh::Fast => &FAST,
            Refresh::Clean => &CLEAN,
        };
        self.command(0x50, &[waveform.border])?;
        self.command(0x20, &waveform.vcom)?;
        for (register, lut) in (0x21..).zip(&waveform.luts) {
            self.command(register, lut)?;
        }

        self.command(0x04, &[])?;
        self.wait()
    }

    fn sleep(&mut self) -> Result<()> {
        self.command(0x50, &[0xf7])?;
        self.command(0x02, &[])?;
        self.wait()?;
        self.command(0x07, &[0xa5])
    }

    fn refresh(&mut self, old: &[u8], new: &[u8], refresh: Refresh) -> Result<()> {
        self.wake(refresh)?;
        self.command(0x10, old)?;
        self.command(0x13, new)?;
        self.command(0x12, &[])?;
        sleep(Duration::from_millis(1));
        self.wait()?;
        self.sleep()
    }
}

fn gpio_chip() -> Result<Chip> {
    for chip in chips()? {
        let chip = chip?;
        if chip.label() == GPIO_LABEL {
            return Ok(chip);
        }
    }
    anyhow::bail!("no GPIO chip labelled {GPIO_LABEL}")
}

impl Panel for Hat {
    fn show(&mut self, _canvas: &Canvas, planes: &Planes, refresh: Refresh) -> Result<()> {
        let new = planes.black.buffer().to_vec();
        let refresh = if self.fast_in_a_row >= CLEAN_EVERY {
            Refresh::Clean
        } else {
            refresh
        };
        let old = match (&self.shown, refresh) {
            (Some(shown), Refresh::Fast) => shown.clone(),
            _ => new.iter().map(|byte| !byte).collect(),
        };
        let start = Instant::now();
        self.shown = None;
        self.refresh(&old, &new, refresh)?;
        tracing::info!(?refresh, ms = start.elapsed().as_millis(), "drew on the panel");
        self.fast_in_a_row = match refresh {
            Refresh::Fast => self.fast_in_a_row + 1,
            Refresh::Clean => 0,
        };
        self.shown = Some(new);
        Ok(())
    }
}

struct Waveform {
    border: u8,
    /// Two leading bytes, then the same rows as the LUTs.
    vcom: [u8; 44],
    /// White to white, black to white, white to black, black to black.
    luts: [[u8; 42]; 4],
}

const fn lut<const N: usize>(rows: &[[u8; 6]]) -> [u8; N] {
    let skip = N - 42;
    let mut lut = [0; N];
    let mut row = 0;
    while row < rows.len() {
        let mut byte = 0;
        while byte < 6 {
            lut[skip + row * 6 + byte] = rows[row][byte];
            byte += 1;
        }
        row += 1;
    }
    lut
}

const FAST_FRAMES: u8 = 25;

const FAST: Waveform = {
    let still = [[0x00, FAST_FRAMES, 0x01, 0x00, 0x00, 0x01]];
    let to_white = [[0x80, FAST_FRAMES, 0x01, 0x00, 0x00, 0x01]];
    let to_black = [[0x40, FAST_FRAMES, 0x01, 0x00, 0x00, 0x01]];
    Waveform {
        border: 0x17,
        vcom: lut(&still),
        luts: [lut(&still), lut(&to_white), lut(&to_black), lut(&still)],
    }
};

const CLEAN: Waveform = {
    let timing = [
        [0x00, 0x08, 0x00, 0x00, 0x00, 0x02],
        [0x60, 0x28, 0x28, 0x00, 0x00, 0x01],
        [0x00, 0x14, 0x00, 0x00, 0x00, 0x01],
        [0x00, 0x12, 0x12, 0x00, 0x00, 0x01],
    ];
    let to_white = [
        [0x40, 0x08, 0x00, 0x00, 0x00, 0x02],
        [0x90, 0x28, 0x28, 0x00, 0x00, 0x01],
        [0x40, 0x14, 0x00, 0x00, 0x00, 0x01],
        [0xa0, 0x12, 0x12, 0x00, 0x00, 0x01],
    ];
    let to_black = [
        [0x80, 0x08, 0x00, 0x00, 0x00, 0x02],
        [0x90, 0x28, 0x28, 0x00, 0x00, 0x01],
        [0x80, 0x14, 0x00, 0x00, 0x00, 0x01],
        [0x50, 0x12, 0x12, 0x00, 0x00, 0x01],
    ];
    Waveform {
        border: 0x97,
        vcom: lut(&timing),
        luts: [lut(&to_white), lut(&to_white), lut(&to_black), lut(&to_black)],
    }
};

pub struct Preview(pub PathBuf);

impl Panel for Preview {
    fn show(&mut self, canvas: &Canvas, _planes: &Planes, _refresh: Refresh) -> Result<()> {
        std::fs::write(&self.0, canvas.to_ppm()).with_context(|| self.0.display().to_string())?;
        tracing::info!(path = %self.0.display(), "wrote preview");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luts_hold_their_rows_and_pad_with_zeros() {
        assert_eq!(&FAST.luts[1][..6], &[0x80, FAST_FRAMES, 0x01, 0x00, 0x00, 0x01]);
        assert!(FAST.luts[1][6..].iter().all(|&byte| byte == 0));
        assert_eq!(&CLEAN.vcom[8..14], &[0x60, 0x28, 0x28, 0x00, 0x00, 0x01]);
        assert_eq!(CLEAN.vcom[..2], [0, 0]);
    }
}
