//! Plays a moving picture, a GIF or a short video such as a messenger's GIF, in black and white.
//!
//! Each picture is fitted into the panel and dithered with a fixed 4×4 pattern
//! (ordered dithering), so a still area dithers the same in every picture and
//! only what moves has to be redrawn.

use std::{
    io::Cursor,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
    thread::sleep,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use embedded_graphics::prelude::*;
use epd_waveshare::graphics::DisplayRotation;
use image::{
    AnimationDecoder, DynamicImage, GrayImage, ImageDecoder, RgbaImage, codecs::gif::GifDecoder, imageops::FilterType,
};

use crate::{
    panel::{Animation, Frame},
    picture::{self, Grey},
    render::{Canvas, Shade, canvas_size},
};

/// How long a clip plays at most; a shorter one plays once.
pub const LIMIT: Duration = Duration::from_secs(15);
/// Pictures per second taken from a video. The panel shows fewer, so more would only be skipped.
const VIDEO_FPS: u32 = 10;
/// The most pictures a clip keeps, so a long GIF with tiny delays can't fill the Pi's memory.
const MAX_PICTURES: usize = 300;
/// Browsers show a GIF picture with a shorter delay than this for 100 ms instead.
const MIN_DELAY: Duration = Duration::from_millis(20);
const DEFAULT_DELAY: Duration = Duration::from_millis(100);
/// Thresholds of the 4×4 Bayer pattern, from 0 to 1.
const BAYER: [[f32; 4]; 4] = {
    let order = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    let mut thresholds = [[0.0; 4]; 4];
    let mut i = 0;
    while i < 16 {
        thresholds[i / 4][i % 4] = (order[i / 4][i % 4] as f32 + 0.5) / 16.0;
        i += 1;
    }
    thresholds
};

/// A clip ready for the panel.
pub struct Reel {
    /// Each picture as the panel's black-and-white plane, with when it starts in the clip.
    pub pictures: Vec<(Duration, Vec<u8>)>,
    /// How long the clip runs, cut at the limit it was decoded with.
    pub length: Duration,
    /// The first picture, for drawing in four shades once the clip has played.
    pub cover: RgbaImage,
}

impl Reel {
    /// Adds a picture shown for `delay`; false once the reel is full.
    fn push(&mut self, picture: &Grey, delay: Duration, rotation: DisplayRotation, limit: Duration) -> bool {
        self.pictures.push((self.length, plane(picture, rotation)));
        self.length = (self.length + delay).min(limit);
        self.length < limit && self.pictures.len() < MAX_PICTURES
    }
}

/// Decodes a GIF, or any video ffmpeg reads, for at most `limit`.
pub fn decode(clip: &[u8], rotation: DisplayRotation, limit: Duration) -> Result<Reel> {
    let reel = if clip.starts_with(b"GIF8") {
        decode_gif(clip, rotation, limit)
    } else {
        decode_video(clip, rotation, limit)
    }?;
    if reel.pictures.is_empty() {
        bail!("the clip has no pictures");
    }
    Ok(reel)
}

fn decode_gif(gif: &[u8], rotation: DisplayRotation, limit: Duration) -> Result<Reel> {
    let mut decoder = GifDecoder::new(Cursor::new(gif)).context("reading the GIF")?;
    decoder.set_limits(picture::limits())?;
    let mut reel = empty();
    for frame in decoder.into_frames() {
        let frame = frame.context("decoding the GIF")?;
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay = Duration::from_micros(1000 * numerator as u64 / denominator.max(1) as u64);
        let delay = if delay < MIN_DELAY { DEFAULT_DELAY } else { delay };
        let picture = frame.into_buffer();
        if reel.pictures.is_empty() {
            reel.cover = picture.clone();
        }
        if !reel.push(&picture::luminance(&picture), delay, rotation, limit) {
            break;
        }
    }
    Ok(reel)
}

fn decode_video(video: &[u8], rotation: DisplayRotation, limit: Duration) -> Result<Reel> {
    static CLIPS: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "epaper-clip-{}-{}",
        std::process::id(),
        CLIPS.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, video).with_context(|| path.display().to_string())?;
    let size = canvas_size(rotation);
    let (width, height) = (size.width, size.height);
    let output = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-v",
            "error",
            "-t",
            &format!("{}", limit.as_secs_f32()),
            "-i",
        ])
        .arg(&path)
        .args([
            "-an",
            "-vf",
            &format!(
                "fps={VIDEO_FPS},scale={width}:{height}:force_original_aspect_ratio=decrease,\
                 pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=white"
            ),
            "-frames:v",
            &MAX_PICTURES.to_string(),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "gray",
            "pipe:1",
        ])
        .output();
    std::fs::remove_file(&path).ok();
    let output = output.context("running ffmpeg")?;
    if !output.status.success() {
        bail!("ffmpeg: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    let mut reel = empty();
    for bytes in output.stdout.chunks_exact((width * height) as usize) {
        let picture = GrayImage::from_raw(width, height, bytes.to_vec()).context("a picture from ffmpeg")?;
        if reel.pictures.is_empty() {
            reel.cover = DynamicImage::ImageLuma8(picture.clone()).into_rgba8();
        }
        let grey = Grey::from_fn(width, height, |x, y| {
            image::Luma([picture.get_pixel(x, y).0[0] as f32 / 255.0])
        });
        if !reel.push(&grey, Duration::from_secs(1) / VIDEO_FPS, rotation, limit) {
            break;
        }
    }
    Ok(reel)
}

fn empty() -> Reel {
    Reel {
        pictures: Vec::new(),
        length: Duration::ZERO,
        cover: RgbaImage::new(1, 1),
    }
}

/// The picture fitted into the panel, centred on white, in black and white as the panel takes it.
fn plane(picture: &Grey, rotation: DisplayRotation) -> Vec<u8> {
    let size = canvas_size(rotation);
    let mut canvas = Canvas::new(size);
    let (width, height) = picture::fit(picture.width().max(1), picture.height().max(1), size);
    let resized;
    let picture = if picture.dimensions() == (width, height) {
        picture
    } else {
        resized = image::imageops::resize(picture, width, height, FilterType::Triangle);
        &resized
    };
    let (left, top) = ((size.width - width) / 2, (size.height - height) / 2);
    for (x, y, pixel) in picture.enumerate_pixels() {
        let (x, y) = (left + x, top + y);
        if pixel.0[0] < BAYER[(y % 4) as usize][(x % 4) as usize] {
            canvas.set(Point::new(x as i32, y as i32), Shade::Black);
        }
    }
    canvas.planes(rotation).high.buffer().to_vec()
}

/// Plays the reel in its own time, skipping pictures the panel is too slow for.
pub fn in_time(animation: &mut Animation<'_>, reel: &Reel) -> Result<()> {
    let start = Instant::now();
    let mut tally = Tally::default();
    let mut current = 0;
    loop {
        let elapsed = start.elapsed();
        if elapsed >= reel.length {
            break;
        }
        let due = reel.pictures.partition_point(|(at, _)| *at <= elapsed).max(1) - 1;
        if due == current {
            let next = reel.pictures.get(current + 1).map_or(reel.length, |(at, _)| *at);
            sleep(next.saturating_sub(elapsed));
            continue;
        }
        tally.add(animation.show(&reel.pictures[due].1)?);
        current = due;
    }
    tally.log(reel.pictures.len(), start.elapsed());
    Ok(())
}

/// Shows `count` pictures in order, looping the reel, each no sooner than its delay; for measuring the panel.
pub fn every(animation: &mut Animation<'_>, reel: &Reel, count: usize) -> Result<()> {
    let start = Instant::now();
    let mut tally = Tally::default();
    let mut due = start;
    let ends = reel.pictures.iter().skip(1).map(|(at, _)| *at).chain([reel.length]);
    let delays: Vec<Duration> = reel.pictures.iter().zip(ends).map(|((at, _), end)| end - *at).collect();
    for i in (0..reel.pictures.len()).cycle().skip(1).take(count.saturating_sub(1)) {
        due += delays[(i + reel.pictures.len() - 1) % reel.pictures.len()];
        sleep(due.saturating_duration_since(Instant::now()));
        tally.add(animation.show(&reel.pictures[i].1)?);
    }
    tally.log(count, start.elapsed());
    Ok(())
}

#[derive(Default)]
struct Tally {
    drawn: u32,
    unchanged: u32,
    transfer: Duration,
    refresh: Duration,
}

impl Tally {
    fn add(&mut self, frame: Option<Frame>) {
        let Some(frame) = frame else {
            self.unchanged += 1;
            return;
        };
        tracing::debug!(
            x = frame.window.x,
            y = frame.window.y,
            width = frame.window.width,
            height = frame.window.height,
            transfer_ms = frame.transfer.as_millis(),
            refresh_ms = frame.refresh.as_millis(),
            "frame"
        );
        self.drawn += 1;
        self.transfer += frame.transfer;
        self.refresh += frame.refresh;
    }

    fn log(&self, pictures: usize, took: Duration) {
        let seconds = took.as_secs_f32();
        tracing::info!(
            pictures,
            drawn = self.drawn,
            unchanged = self.unchanged,
            seconds,
            fps = self.drawn as f32 / seconds,
            transfer_ms = (self.transfer / self.drawn.max(1)).as_millis(),
            refresh_ms = (self.refresh / self.drawn.max(1)).as_millis(),
            "played a clip"
        );
    }
}

#[cfg(test)]
mod tests {
    use epd_waveshare::epd2in7b::{HEIGHT, WIDTH};
    use image::{Delay, Rgba, codecs::gif::GifEncoder};

    use super::*;
    use crate::panel::Window;

    fn black_pixels(plane: &[u8]) -> u32 {
        plane.iter().map(|byte| byte.count_zeros()).sum()
    }

    fn gif(frames: Vec<(RgbaImage, u32)>) -> Vec<u8> {
        let mut gif = Vec::new();
        GifEncoder::new(&mut gif)
            .encode_frames(
                frames
                    .into_iter()
                    .map(|(picture, ms)| image::Frame::from_parts(picture, 0, 0, Delay::from_numer_denom_ms(ms, 1))),
            )
            .unwrap();
        gif
    }

    /// A landscape picture the size of the panel, white with a black `side`-pixel square at `x`, `y`.
    fn square(x: u32, y: u32, side: u32) -> RgbaImage {
        RgbaImage::from_fn(HEIGHT, WIDTH, |px, py| {
            let inside = (x..x + side).contains(&px) && (y..y + side).contains(&py);
            Rgba(if inside { [0, 0, 0, 255] } else { [255; 4] })
        })
    }

    #[test]
    fn only_the_moving_square_changes() {
        let clip = gif(vec![(square(100, 40, 16), 100), (square(120, 40, 16), 100)]);
        let reel = decode(&clip, DisplayRotation::Rotate90, LIMIT).unwrap();
        assert_eq!(reel.pictures.len(), 2);
        assert_eq!(reel.pictures[1].0, Duration::from_millis(100));
        assert_eq!(reel.length, Duration::from_millis(200));
        let [(_, first), (_, second)] = &reel.pictures[..] else {
            unreachable!()
        };
        assert_eq!(black_pixels(first), 16 * 16);
        let window = Window::changed(first, second).unwrap();
        assert_eq!(window.height, 36, "{window:?}");
        assert!(window.width <= 24, "{window:?}");
        assert_eq!(reel.cover.dimensions(), (HEIGHT, WIDTH));
    }

    #[test]
    fn a_long_clip_is_cut_at_the_limit() {
        let frames = (0..20).map(|i| (square(10 * i, 40, 16), 1000)).collect();
        let reel = decode(&gif(frames), DisplayRotation::Rotate90, Duration::from_millis(2500)).unwrap();
        assert_eq!(reel.pictures.len(), 3);
        assert_eq!(reel.length, Duration::from_millis(2500));
    }

    #[test]
    fn tiny_delays_mean_a_tenth_of_a_second() {
        let reel = decode(
            &gif(vec![(square(0, 0, 8), 0), (square(8, 0, 8), 0)]),
            DisplayRotation::Rotate90,
            LIMIT,
        )
        .unwrap();
        assert_eq!(reel.length, Duration::from_millis(200));
    }

    #[test]
    fn grey_dithers_to_half_black_in_a_fixed_pattern() {
        let grey = Grey::from_pixel(40, 40, image::Luma([0.5]));
        let plane = plane(&grey, DisplayRotation::Rotate0);
        assert_eq!(black_pixels(&plane), WIDTH * WIDTH / 2);
        assert_eq!(plane, super::plane(&grey, DisplayRotation::Rotate0));
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(decode(b"GIF89a broken", DisplayRotation::Rotate0, LIMIT).is_err());
    }

    #[test]
    fn a_video_plays_through_ffmpeg() {
        let dir = std::env::temp_dir().join(format!("epaper-video-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mp4 = dir.join("black.mp4");
        let made = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=black:s=64x32:d=1:r=25",
                "-c:v",
                "mpeg4",
            ])
            .arg(&mp4)
            .status()
            .unwrap();
        assert!(made.success());
        let reel = decode(&std::fs::read(&mp4).unwrap(), DisplayRotation::Rotate90, LIMIT).unwrap();
        std::fs::remove_dir_all(dir).ok();
        assert!(
            (9..=11).contains(&reel.pictures.len()),
            "{} pictures",
            reel.pictures.len()
        );
        assert_eq!(reel.length, reel.pictures.len() as u32 * Duration::from_millis(100));
        let (black, fitted) = (black_pixels(&reel.pictures[0].1), HEIGHT * HEIGHT / 2);
        assert!(
            black <= fitted && black >= fitted / 16 * 15,
            "{black} of {fitted} black"
        );
    }
}
