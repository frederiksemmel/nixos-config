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
use epd_waveshare::epd2in7b::{HEIGHT, WIDTH};
use linux_embedded_hal::{
    CdevPin, SpidevDevice,
    gpio_cdev::{Chip, LineRequestFlags, chips},
    spidev::{SpiModeFlags, SpidevOptions, SpidevTransfer},
};

use crate::{
    animation::{self, Reel},
    render::{Canvas, Planes},
};

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

/// Panel frames the fast refresh of a clip drives a changing pixel for.
pub const CLIP_LUT_FRAMES: u8 = FAST_FRAMES;

pub trait Panel: Send {
    fn show(&mut self, canvas: &Canvas, planes: &Planes, refresh: Refresh) -> Result<()>;

    /// Plays a clip in its own time with fast refreshes; a panel that can't animate skips it.
    fn play(&mut self, _reel: &Reel) -> Result<()> {
        Ok(())
    }
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

    fn wait(&mut self, step: &str) -> Result<Duration> {
        let start = Instant::now();
        while self.busy.is_low()? {
            anyhow::ensure!(
                start.elapsed() < BUSY_TIMEOUT,
                "the panel stayed busy for {BUSY_TIMEOUT:?}"
            );
            sleep(BUSY_POLL);
        }
        tracing::debug!(step, ms = start.elapsed().as_millis(), "panel busy");
        Ok(start.elapsed())
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
        self.command(0x00, &[0xbf])?;
        self.command(0x61, &[0x00, 0xb0, 0x01, 0x08])?;
        self.command(0x82, &[0x12])?;
        match refresh {
            Refresh::Fast => self.load(&FAST),
            Refresh::Clean => self.load(&CLEAN),
            Refresh::Gray => self.load(&GRAY),
        }
    }

    fn load<const N: usize>(&mut self, waveform: &Waveform<N>) -> Result<()> {
        self.command(0x30, &[waveform.pll])?;
        self.command(0x50, &[waveform.border])?;
        self.command(0x20, &waveform.vcom)?;
        for (register, lut) in (0x21..).zip(&waveform.luts) {
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

    /// Draws `first` with a clean refresh, then keeps the panel powered for fast refreshes of
    /// `lut_frames` panel frames each, in the window that changed or, unless `partial`, the whole panel.
    pub fn animate(&mut self, first: &[u8], lut_frames: u8, partial: bool) -> Result<Animation<'_>> {
        self.shown = None;
        self.wake(Refresh::Clean)?;
        self.command(0x10, &first.iter().map(|byte| !byte).collect::<Vec<_>>())?;
        self.command(0x13, first)?;
        self.command(0x12, &[])?;
        sleep(Duration::from_millis(1));
        self.wait("refresh")?;
        self.load(&fast(lut_frames))?;
        Ok(Animation {
            hat: self,
            shown: first.to_vec(),
            partial,
        })
    }
}

/// A rectangle of the panel in its own portrait layout, 176 wide and 264 high; x and width are
/// multiples of 8, because the controller takes eight pixels to a byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

const ROW_BYTES: usize = WIDTH as usize / 8;

impl Window {
    const FULL: Window = Window {
        x: 0,
        y: 0,
        width: WIDTH as u16,
        height: HEIGHT as u16,
    };

    /// The smallest window around every byte that differs between two planes.
    pub fn changed(old: &[u8], new: &[u8]) -> Option<Window> {
        let (mut left, mut right, mut top, mut bottom) = (usize::MAX, 0, usize::MAX, 0);
        for (i, _) in old.iter().zip(new).enumerate().filter(|(_, (a, b))| a != b) {
            let (row, column) = (i / ROW_BYTES, i % ROW_BYTES);
            (left, right) = (left.min(column), right.max(column));
            (top, bottom) = (top.min(row), bottom.max(row));
        }
        (top != usize::MAX).then(|| Window {
            x: (left * 8) as u16,
            y: top as u16,
            width: ((right - left + 1) * 8) as u16,
            height: (bottom - top + 1) as u16,
        })
    }

    /// The window's bytes of a plane, row by row.
    fn cut(&self, plane: &[u8]) -> Vec<u8> {
        let (left, right) = (self.x as usize / 8, (self.x + self.width) as usize / 8);
        let rows = self.y as usize..(self.y + self.height) as usize;
        rows.flat_map(|row| &plane[row * ROW_BYTES + left..row * ROW_BYTES + right])
            .copied()
            .collect()
    }

    /// X, y, width and height as the partial commands 0x14 to 0x16 take them: two bytes each, high byte first.
    fn position(&self) -> [u8; 8] {
        let [x, y, width, height] = [self.x, self.y, self.width, self.height].map(u16::to_be_bytes);
        [x[0], x[1], y[0], y[1], width[0], width[1], height[0], height[1]]
    }
}

/// How long one frame of an animation took.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub window: Window,
    /// Sending the old and new pixels over SPI.
    pub transfer: Duration,
    /// From the refresh command until the panel is no longer busy.
    pub refresh: Duration,
}

/// A run of fast refreshes on a panel that stays powered between them.
pub struct Animation<'a> {
    hat: &'a mut Hat,
    shown: Vec<u8>,
    partial: bool,
}

impl Animation<'_> {
    /// Moves the panel to `new`, driving only the pixels that change; nothing when none do.
    pub fn show(&mut self, new: &[u8]) -> Result<Option<Frame>> {
        let Some(changed) = Window::changed(&self.shown, new) else {
            return Ok(None);
        };
        let start = Instant::now();
        let window = if self.partial {
            let position = changed.position();
            self.hat
                .command(0x14, &[&position[..], &changed.cut(&self.shown)].concat())?;
            self.hat.command(0x15, &[&position[..], &changed.cut(new)].concat())?;
            changed
        } else {
            self.hat.command(0x10, &self.shown)?;
            self.hat.command(0x13, new)?;
            Window::FULL
        };
        let transfer = start.elapsed();
        if self.partial {
            self.hat.command(0x16, &window.position())?;
        } else {
            self.hat.command(0x12, &[])?;
        }
        sleep(Duration::from_millis(1));
        self.hat.wait("refresh")?;
        self.shown = new.to_vec();
        Ok(Some(Frame {
            window,
            transfer,
            refresh: start.elapsed() - transfer,
        }))
    }

    /// Powers the panel down; the picture stays.
    pub fn finish(self) -> Result<()> {
        self.hat.sleep()
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

    fn play(&mut self, reel: &Reel) -> Result<()> {
        let mut animation = self.animate(&reel.pictures[0].1, CLIP_LUT_FRAMES, true)?;
        let played = animation::in_time(&mut animation, reel);
        animation.finish()?;
        played
    }
}

struct Waveform<const N: usize> {
    pll: u8,
    border: u8,
    /// Two leading bytes, then the same rows as the LUTs.
    vcom: [u8; 44],
    /// For registers 0x21 on: one LUT per pair of bits in the two frames.
    luts: [[u8; 42]; N],
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

/// Panel frames the fast refresh drives a changing pixel for; at the PLL's 81.8 Hz, 25 are about 0.3 s.
pub const FAST_FRAMES: u8 = 25;

const FAST: Waveform<4> = fast(FAST_FRAMES);

const fn fast(frames: u8) -> Waveform<4> {
    let still = [[0x00, frames, 0x01, 0x00, 0x00, 0x01]];
    let to_white = [[0x80, frames, 0x01, 0x00, 0x00, 0x01]];
    let to_black = [[0x40, frames, 0x01, 0x00, 0x00, 0x01]];
    Waveform {
        pll: 0x3a,
        border: 0x17,
        vcom: lut(&still),
        luts: [lut(&still), lut(&to_white), lut(&to_black), lut(&still)],
    }
}

const CLEAN: Waveform<4> = {
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
        luts: [lut(&to_white), lut(&to_white), lut(&to_black), lut(&to_black)],
    }
};

/// How often the grey refresh drives every pixel black and white before it sets the shades.
const GRAY_SHAKES: u8 = 5;

const GRAY: Waveform<5> = {
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
        luts: [lut(&ww), lut(&bw), lut(&wb), lut(&bb), lut(&ww)],
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
        assert_eq!(&fast(9).luts[2][..2], &[0x40, 9]);
    }

    fn blank() -> Vec<u8> {
        vec![0xff; ROW_BYTES * HEIGHT as usize]
    }

    #[test]
    fn the_window_spans_the_bytes_that_changed() {
        let (old, mut new) = (blank(), blank());
        assert_eq!(Window::changed(&old, &new), None);
        new[10 * ROW_BYTES + 3] = 0x7f;
        new[12 * ROW_BYTES + 5] = 0xfe;
        let window = Window::changed(&old, &new).unwrap();
        assert_eq!(
            window,
            Window {
                x: 24,
                y: 10,
                width: 24,
                height: 3
            }
        );
        assert_eq!(window.cut(&new), [0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe]);
    }

    #[test]
    fn the_partial_commands_take_the_window_high_byte_first() {
        let window = Window {
            x: 168,
            y: 260,
            width: 8,
            height: 4,
        };
        assert_eq!(window.position(), [0, 168, 1, 4, 0, 8, 0, 4]);
        assert_eq!(Window::FULL.cut(&blank()).len(), blank().len());
    }
}
