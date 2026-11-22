//! Turns a picture into the panel's four shades.
//!
//! Line art, such as a comic, is scaled and rounded to the nearest shade, so
//! lines stay solid. Anything else, such as a sticker or a photo, has its
//! contrast stretched, its midtones lifted, and is dithered with Atkinson's
//! error diffusion.

use std::io::Cursor;

use anyhow::{Context, Result};
use embedded_graphics::prelude::Size;
use image::{ImageBuffer, ImageReader, Limits, Luma, RgbaImage, imageops::FilterType};

use crate::render::Shade;

/// The panel's shades from black to white, as sRGB values the preview uses too.
const LEVELS: [(f32, Shade); 4] = [
    (0.0, Shade::Black),
    (1.0 / 3.0, Shade::Dark),
    (2.0 / 3.0, Shade::Light),
    (1.0, Shade::White),
];
/// The largest picture `decode` accepts, in each direction.
const MAX_SIDE: u32 = 8192;
const MAX_BYTES: u64 = 256 << 20;
/// Darkens line art, so lines thinner than a pixel stay visible.
const LINE_GAMMA: f32 = 1.4;
/// Lifts the midtones of photos and stickers, which come out dark on the panel's grey paper.
const PHOTO_GAMMA: f32 = 0.8;

type Grey = ImageBuffer<Luma<f32>, Vec<f32>>;

/// Decodes a PNG, JPEG, GIF or WebP; an animation gives its first frame.
pub fn decode(bytes: &[u8]) -> Result<RgbaImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .context("reading the picture")?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_BYTES);
    reader.limits(limits);
    Ok(reader.decode().context("decoding the picture")?.into_rgba8())
}

/// The picture as shades, row by row, fitted into `size` and centred on white.
pub fn shades(image: &RgbaImage, size: Size) -> Vec<Shade> {
    let mut out = vec![Shade::White; (size.width * size.height) as usize];
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 || size.width == 0 || size.height == 0 {
        return out;
    }
    let scale = f64::min(size.width as f64 / width as f64, size.height as f64 / height as f64);
    let fit = |side: u32, limit: u32| ((side as f64 * scale).round() as u32).clamp(1, limit);
    let (fit_width, fit_height) = (fit(width, size.width), fit(height, size.height));

    let line_art = is_line_art(image);
    let grey = luminance(image);
    let mut grey = if (fit_width, fit_height) == (width, height) {
        grey
    } else {
        image::imageops::resize(&grey, fit_width, fit_height, FilterType::Lanczos3)
    };
    let shaded = if line_art {
        tone(&mut grey, 0.0, 1.0, LINE_GAMMA);
        grey.pixels().map(|p| nearest(p.0[0]).1).collect()
    } else {
        let (low, high) = spread(&grey);
        tone(&mut grey, low, high, PHOTO_GAMMA);
        atkinson(&mut grey)
    };

    let (left, top) = ((size.width - fit_width) / 2, (size.height - fit_height) / 2);
    for (y, row) in shaded.chunks(fit_width as usize).enumerate() {
        let start = ((top + y as u32) * size.width + left) as usize;
        out[start..start + row.len()].copy_from_slice(row);
    }
    out
}

fn to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// Each pixel's lightness on a white background, from 0 (black) to 1 (white), sRGB encoded.
fn luminance(image: &RgbaImage) -> Grey {
    Grey::from_fn(image.width(), image.height(), |x, y| {
        let [r, g, b, a] = image.get_pixel(x, y).0.map(|c| c as f32 / 255.0);
        let y = to_srgb(0.2126 * to_linear(r) + 0.7152 * to_linear(g) + 0.0722 * to_linear(b));
        Luma([a * y + (1.0 - a)])
    })
}

/// Whether nearly every pixel is grey and nearly black or white, as in a comic.
fn is_line_art(image: &RgbaImage) -> bool {
    let (mut grey, mut extreme) = (0usize, 0usize);
    for pixel in image.pixels() {
        let [r, g, b, a] = pixel.0.map(u32::from);
        let [r, g, b] = [r, g, b].map(|c| (c * a + 255 * (255 - a)) / 255);
        if r.max(g).max(b) - r.min(g).min(b) < 24 {
            grey += 1;
        }
        let luma = (299 * r + 587 * g + 114 * b) / 1000;
        if !(64..=192).contains(&luma) {
            extreme += 1;
        }
    }
    let total = image.pixels().len();
    grey * 50 >= total * 49 && extreme * 20 >= total * 17
}

/// The darkest and lightest values, ignoring the 1 % at either end.
fn spread(grey: &Grey) -> (f32, f32) {
    let mut values: Vec<f32> = grey.pixels().map(|p| p.0[0]).collect();
    values.sort_by(f32::total_cmp);
    let at = |fraction: f32| values[((values.len() - 1) as f32 * fraction) as usize];
    let (low, high) = (at(0.01), at(0.99));
    if high - low < 0.125 { (0.0, 1.0) } else { (low, high) }
}

/// Stretches `low..high` to `0..1`, then applies `gamma`.
fn tone(grey: &mut Grey, low: f32, high: f32, gamma: f32) {
    for pixel in grey.pixels_mut() {
        let v = ((pixel.0[0] - low) / (high - low)).clamp(0.0, 1.0);
        pixel.0[0] = v.powf(gamma);
    }
}

/// The level closest to `v`.
fn nearest(v: f32) -> (f32, Shade) {
    LEVELS
        .into_iter()
        .min_by(|a, b| (a.0 - v).abs().total_cmp(&(b.0 - v).abs()))
        .unwrap_or(LEVELS[3])
}

/// Atkinson's error diffusion, in serpentine order: each pixel passes six
/// eighths of its error on to its neighbours.
fn atkinson(grey: &mut Grey) -> Vec<Shade> {
    const SPREAD: [(i64, i64); 6] = [(1, 0), (2, 0), (-1, 1), (0, 1), (1, 1), (0, 2)];
    let (width, height) = (grey.width() as i64, grey.height() as i64);
    let mut out = vec![Shade::White; (width * height) as usize];
    for y in 0..height {
        let backwards = y % 2 == 1;
        for i in 0..width {
            let x = if backwards { width - 1 - i } else { i };
            let old = grey.get_pixel(x as u32, y as u32).0[0];
            let (level, shade) = nearest(old);
            out[(y * width + x) as usize] = shade;
            let error = (old - level) / 8.0;
            for (dx, dy) in SPREAD {
                let (nx, ny) = (if backwards { x - dx } else { x + dx }, y + dy);
                if (0..width).contains(&nx) && ny < height {
                    grey.get_pixel_mut(nx as u32, ny as u32).0[0] += error;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    const LANDSCAPE: Size = Size::new(264, 176);
    const PORTRAIT: Size = Size::new(176, 264);

    fn filled(width: u32, height: u32, color: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba(color))
    }

    /// The first and last column and row that are not white.
    fn inked(shades: &[Shade], size: Size) -> Option<(u32, u32, u32, u32)> {
        let mut bounds: Option<(u32, u32, u32, u32)> = None;
        for (i, &shade) in shades.iter().enumerate() {
            if shade == Shade::White {
                continue;
            }
            let (x, y) = (i as u32 % size.width, i as u32 / size.width);
            bounds = Some(match bounds {
                None => (x, y, x, y),
                Some((l, t, r, b)) => (l.min(x), t.min(y), r.max(x), b.max(y)),
            });
        }
        bounds
    }

    fn count(shades: &[Shade], shade: Shade) -> usize {
        shades.iter().filter(|&&s| s == shade).count()
    }

    #[test]
    fn transparency_is_white() {
        let shades = shades(&filled(512, 512, [200, 30, 30, 0]), LANDSCAPE);
        assert_eq!(count(&shades, Shade::White), shades.len());
    }

    #[test]
    fn a_wide_picture_fills_the_width_and_is_centred() {
        let shades = shades(&filled(400, 200, [0, 0, 0, 255]), LANDSCAPE);
        assert_eq!(inked(&shades, LANDSCAPE), Some((0, 22, 263, 153)));
        assert_eq!(count(&shades, Shade::Black), 264 * 132);
    }

    #[test]
    fn a_square_picture_fills_the_height_of_a_landscape_panel() {
        let shades = shades(&filled(512, 512, [0, 0, 0, 255]), LANDSCAPE);
        assert_eq!(inked(&shades, LANDSCAPE), Some((44, 0, 219, 175)));
    }

    #[test]
    fn a_tiny_picture_is_scaled_up() {
        let shades = shades(&filled(8, 8, [0, 0, 0, 255]), PORTRAIT);
        assert_eq!(inked(&shades, PORTRAIT), Some((0, 44, 175, 219)));
    }

    #[test]
    fn mid_grey_dithers_between_the_greys() {
        let shades = shades(&filled(300, 200, [119, 119, 119, 255]), LANDSCAPE);
        let (dark, light) = (count(&shades, Shade::Dark), count(&shades, Shade::Light));
        assert!(dark > 1000 && light > 1000, "dark {dark}, light {light}");
        assert_eq!(count(&shades, Shade::Black) + count(&shades, Shade::White), 0);
    }

    #[test]
    fn line_art_keeps_its_lines_black_without_dots() {
        let mut comic = filled(528, 352, [255, 255, 255, 255]);
        for y in 100..104 {
            for x in 50..478 {
                comic.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        assert!(is_line_art(&comic));
        let shades = shades(&comic, LANDSCAPE);
        let column = |x: u32| -> Vec<Shade> { (0..176).map(|y| shades[(y * 264 + x) as usize]).collect() };
        assert!(count(&column(30), Shade::Black) >= 2);
        for x in 30..230 {
            assert_eq!(column(x), column(30), "column {x}");
        }
        let (left, top, right, bottom) = inked(&shades, LANDSCAPE).unwrap();
        assert!(
            left >= 21 && top >= 47 && right <= 242 && bottom <= 54,
            "inked {left},{top}..{right},{bottom}"
        );
    }

    #[test]
    fn colour_is_not_line_art() {
        assert!(!is_line_art(&filled(10, 10, [255, 200, 0, 255])));
        assert!(!is_line_art(&filled(10, 10, [128, 128, 128, 255])));
        assert!(is_line_art(&filled(10, 10, [255, 255, 255, 0])));
    }

    #[test]
    fn a_png_decodes_with_its_alpha() {
        let mut png = Vec::new();
        filled(3, 2, [10, 20, 30, 40])
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let image = decode(&png).unwrap();
        assert_eq!(image.dimensions(), (3, 2));
        assert_eq!(image.get_pixel(2, 1).0, [10, 20, 30, 40]);
    }

    #[test]
    fn an_animated_webp_gives_its_first_frame() {
        let image = decode(include_bytes!("../assets/two-frames.webp")).unwrap();
        assert_eq!(image.dimensions(), (16, 16));
        let [r, g, b, a] = image.get_pixel(8, 8).0;
        assert!(r > 200 && g < 60 && b < 60 && a == 255, "{:?}", image.get_pixel(8, 8));
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(decode(b"not a picture").is_err());
    }
}
