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
    spidev::{SpiModeFlags, SpidevOptions, SpidevTransfer},
};

use crate::render::{Canvas, Planes};

const SPI_DEVICE: &str = "/dev/spidev0.0";
const SPI_HZ: u32 = 4_000_000;
const GPIO_LABEL: &str = "pinctrl-bcm2835";
const RST: u32 = 17;
const BUSY: u32 = 24;
const DC: u32 = 25;
const BYTES_PER_MESSAGE: usize = 32;
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
    /// Flashes the whole panel and draws four shades: white, light grey, dark grey, black.
    Gray,
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
            for chunk in data.chunks(BYTES_PER_MESSAGE) {
                let mut transfers: Vec<_> = chunk
                    .iter()
                    .map(|byte| {
                        let mut transfer = SpidevTransfer::write(std::slice::from_ref(byte));
                        transfer.cs_change = 1;
                        transfer
                    })
                    .collect();
                if let Some(last) = transfers.last_mut() {
                    last.cs_change = 0;
                }
                self.spi.0.transfer_multiple(&mut transfers)?;
            }
        }
        Ok(())
    }

    fn wait(&mut self, step: &str) -> Result<()> {
        let start = Instant::now();
        while self.busy.is_low()? {
            anyhow::ensure!(
                start.elapsed() < BUSY_TIMEOUT,
                "the panel stayed busy for {BUSY_TIMEOUT:?}"
            );
            sleep(BUSY_POLL);
        }
        tracing::debug!(step, ms = start.elapsed().as_millis(), "panel busy");
        Ok(())
    }

    fn wake(&mut self, refresh: Refresh) -> Result<()> {
        self.rst.set_high()?;
        sleep(Duration::from_millis(10));
        self.rst.set_low()?;
        sleep(Duration::from_millis(2));
        self.rst.set_high()?;
        sleep(Duration::from_millis(20));
        self.wait("reset")?;

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
        self.command(0x04, &[])?;
        self.wait("power on")?;
        let waveform = match refresh {
            Refresh::Fast => &FAST,
            Refresh::Clean => &CLEAN,
            Refresh::Gray => &GRAY,
        };
        self.command(0x00, &[0xbf])?;
        self.command(0x30, &[waveform.pll])?;
        self.command(0x61, &[0x00, 0xb0, 0x01, 0x08])?;
        self.command(0x82, &[0x12])?;

        self.command(0x50, &[waveform.border])?;
        self.command(0x20, &waveform.vcom)?;
        for (register, lut) in (0x21..).zip(waveform.luts) {
            self.command(register, lut)?;
        }
        Ok(())
    }

    fn sleep(&mut self) -> Result<()> {
        self.command(0x50, &[0xf7])?;
        self.command(0x02, &[])?;
        self.wait("power off")?;
        self.command(0x07, &[0xa5])
    }

    /// Sends the two frames the waveform reads (the old and the new picture, or a grey picture's two bits) and refreshes.
    fn refresh(&mut self, first: &[u8], second: &[u8], refresh: Refresh) -> Result<()> {
        self.wake(refresh)?;
        self.command(0x10, first)?;
        self.command(0x13, second)?;
        self.command(0x12, &[])?;
        sleep(Duration::from_millis(1));
        self.wait("refresh")?;
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
        let new = planes.high.buffer().to_vec();
        let refresh = match refresh {
            Refresh::Fast if self.fast_in_a_row >= CLEAN_EVERY => Refresh::Clean,
            refresh => refresh,
        };
        let (first, second) = match (&self.shown, refresh) {
            (Some(shown), Refresh::Fast) => (shown.clone(), new.clone()),
            (_, Refresh::Gray) => (new.clone(), planes.low.buffer().to_vec()),
            _ => (new.iter().map(|byte| !byte).collect(), new.clone()),
        };
        let start = Instant::now();
        self.shown = None;
        self.refresh(&first, &second, refresh)?;
        tracing::info!(?refresh, ms = start.elapsed().as_millis(), "drew on the panel");
        self.fast_in_a_row = match refresh {
            Refresh::Fast => self.fast_in_a_row + 1,
            Refresh::Clean | Refresh::Gray => 0,
        };
        self.shown = Some(new);
        Ok(())
    }
}

struct Waveform {
    pll: u8,
    border: u8,
    /// Two leading bytes, then the same rows as the LUTs.
    vcom: [u8; 44],
    /// For registers 0x21 on: one LUT per pair of bits in the two frames.
    luts: &'static [[u8; 42]],
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
        pll: 0x3a,
        border: 0x17,
        vcom: lut(&still),
        luts: &[lut(&still), lut(&to_white), lut(&to_black), lut(&still)],
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
        pll: 0x3a,
        border: 0x97,
        vcom: lut(&timing),
        luts: &[lut(&to_white), lut(&to_white), lut(&to_black), lut(&to_black)],
    }
};

/// How often the grey refresh drives every pixel black and white before it sets the shades.
const GRAY_SHAKES: u8 = 5;

const GRAY: Waveform = {
    let timing = [
        [0x00, 0x0a, 0x00, 0x00, 0x00, 0x01],
        [0x60, 0x14, 0x14, 0x00, 0x00, GRAY_SHAKES],
        [0x00, 0x14, 0x00, 0x00, 0x00, 0x01],
        [0x00, 0x13, 0x0a, 0x01, 0x00, 0x01],
    ];
    let ww = [
        [0x40, 0x0a, 0x00, 0x00, 0x00, 0x01],
        [0x90, 0x14, 0x14, 0x00, 0x00, GRAY_SHAKES],
        [0x10, 0x14, 0x0a, 0x00, 0x00, 0x01],
        [0xa0, 0x13, 0x01, 0x00, 0x00, 0x01],
    ];
    let bw = [
        [0x40, 0x0a, 0x00, 0x00, 0x00, 0x01],
        [0x90, 0x14, 0x14, 0x00, 0x00, GRAY_SHAKES],
        [0x00, 0x14, 0x0a, 0x00, 0x00, 0x01],
        [0x99, 0x0c, 0x01, 0x03, 0x04, 0x01],
    ];
    let wb = [
        [0x40, 0x0a, 0x00, 0x00, 0x00, 0x01],
        [0x90, 0x14, 0x14, 0x00, 0x00, GRAY_SHAKES],
        [0x00, 0x14, 0x0a, 0x00, 0x00, 0x01],
        [0x99, 0x0b, 0x04, 0x04, 0x01, 0x01],
    ];
    let bb = [
        [0x80, 0x0a, 0x00, 0x00, 0x00, 0x01],
        [0x90, 0x14, 0x14, 0x00, 0x00, GRAY_SHAKES],
        [0x20, 0x14, 0x0a, 0x00, 0x00, 0x01],
        [0x50, 0x13, 0x01, 0x00, 0x00, 0x01],
    ];
    Waveform {
        pll: 0x90,
        border: 0x57,
        vcom: lut(&timing),
        luts: &[lut(&ww), lut(&bw), lut(&wb), lut(&bb), lut(&ww)],
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
        assert_eq!(GRAY.luts.len(), 5);
        assert_eq!(&GRAY.luts[1][18..24], &[0x99, 0x0c, 0x01, 0x03, 0x04, 0x01]);
        assert!(GRAY.luts.iter().all(|lut| lut[11] == GRAY_SHAKES));
        assert_eq!(GRAY.vcom[2 + 11], GRAY_SHAKES);
    }
}
