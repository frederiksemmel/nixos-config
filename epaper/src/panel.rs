use std::path::PathBuf;

use anyhow::{Context, Result};
use epd_waveshare::{epd2in7b::Epd2in7b, prelude::*};
use linux_embedded_hal::{
    CdevPin, Delay, SpidevDevice,
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
const BUSY_POLL_US: u32 = 10_000;

pub trait Panel: Send {
    fn show(&mut self, canvas: &Canvas, planes: &Planes) -> Result<()>;
}

type Driver = Epd2in7b<SpidevDevice, CdevPin, CdevPin, CdevPin, Delay>;

pub struct Hat {
    spi: SpidevDevice,
    delay: Delay,
    epd: Driver,
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
        let mut delay = Delay;
        let mut epd = Epd2in7b::new(&mut spi, busy, dc, rst, &mut delay, Some(BUSY_POLL_US))?;
        epd.sleep(&mut spi, &mut delay)?;
        Ok(Hat { spi, delay, epd })
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
    fn show(&mut self, _canvas: &Canvas, planes: &Planes) -> Result<()> {
        let (spi, delay) = (&mut self.spi, &mut self.delay);
        self.epd.wake_up(spi, delay)?;
        self.epd
            .update_color_frame(spi, delay, planes.black.buffer(), planes.red.buffer())?;
        self.epd.display_frame(spi, delay)?;
        self.epd.sleep(spi, delay)?;
        Ok(())
    }
}

pub struct Preview(pub PathBuf);

impl Panel for Preview {
    fn show(&mut self, canvas: &Canvas, _planes: &Planes) -> Result<()> {
        std::fs::write(&self.0, canvas.to_ppm()).with_context(|| self.0.display().to_string())?;
        tracing::info!(path = %self.0.display(), "wrote preview");
        Ok(())
    }
}
