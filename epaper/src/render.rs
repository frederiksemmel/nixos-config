use std::sync::Arc;

use anyhow::Result;
use embedded_graphics::prelude::*;
use epd_waveshare::{
    color::Color,
    epd2in7b::{Display2in7b, HEIGHT, WIDTH},
    graphics::DisplayRotation,
};

use image::RgbaImage;

use crate::{markdown, panel::Refresh, picture, typeset::typeset};

const RING: &[u8] = include_bytes!("../assets/ring.raw");
const RING_WIDTH: u32 = 176;
const TEXT_TEMPLATE: &str = include_str!("text.typ");
const MAX_CHARS: usize = 1000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scene {
    Ring,
    Text {
        text: String,
    },
    Clear,
    /// A picture, fitted into the panel and centred on white.
    Picture(Arc<RgbaImage>),
}

impl Scene {
    pub fn refresh(&self) -> Refresh {
        match self {
            Scene::Clear => Refresh::Clean,
            Scene::Ring | Scene::Text { .. } | Scene::Picture(_) => Refresh::Gray,
        }
    }
}

/// The four levels the panel can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shade {
    White,
    Light,
    Dark,
    Black,
}

impl Shade {
    fn of([r, g, b]: [u8; 3]) -> Shade {
        match (299 * r as u32 + 587 * g as u32 + 114 * b as u32) / 1000 {
            192.. => Shade::White,
            128.. => Shade::Light,
            64.. => Shade::Dark,
            _ => Shade::Black,
        }
    }
}

pub struct Canvas {
    size: Size,
    pixels: Vec<Shade>,
}

/// The two bits of each pixel's shade, as the panel takes them; a cleared bit
/// is a 0. White is 11, light 10, dark 01, black 00, so `high` alone is the
/// picture in black and white.
pub struct Planes {
    pub high: Display2in7b,
    pub low: Display2in7b,
}

pub fn canvas_size(rotation: DisplayRotation) -> Size {
    match rotation {
        DisplayRotation::Rotate0 | DisplayRotation::Rotate180 => Size::new(WIDTH, HEIGHT),
        DisplayRotation::Rotate90 | DisplayRotation::Rotate270 => Size::new(HEIGHT, WIDTH),
    }
}

pub fn render(scene: &Scene, size: Size) -> Result<Canvas> {
    let mut canvas = Canvas::new(size);
    match scene {
        Scene::Clear => {}
        Scene::Ring => draw_ring(&mut canvas),
        Scene::Text { text } => draw_text(&mut canvas, text)?,
        Scene::Picture(image) => canvas.pixels = picture::shades(image, size),
    }
    Ok(canvas)
}

fn draw_ring(canvas: &mut Canvas) {
    let ring_height = (RING.len() * 8) as u32 / RING_WIDTH;
    canvas.fill(Shade::Black);
    let offset = Point::new(
        (canvas.size.width as i32 - RING_WIDTH as i32) / 2,
        (canvas.size.height as i32 - ring_height as i32) / 2,
    );
    for y in 0..ring_height {
        for x in 0..RING_WIDTH {
            let bit = y * RING_WIDTH + x;
            if RING[(bit / 8) as usize] & (0x80 >> (bit % 8)) != 0 {
                canvas.set(offset + Point::new(x as i32, y as i32), Shade::White);
            }
        }
    }
}

fn draw_text(canvas: &mut Canvas, text: &str) -> Result<()> {
    let text: String = text.chars().take(MAX_CHARS).collect();
    let doc = serde_json::to_string(&markdown::parse(&text))?;
    let (width, height) = (canvas.size.width.to_string(), canvas.size.height.to_string());
    let page = typeset(
        TEXT_TEMPLATE,
        &[
            ("doc", doc.as_str()),
            ("width", width.as_str()),
            ("height", height.as_str()),
        ],
    )?;
    for y in 0..page.height.min(canvas.size.height) {
        for x in 0..page.width.min(canvas.size.width) {
            canvas.set(Point::new(x as i32, y as i32), Shade::of(page.pixel(x, y)));
        }
    }
    Ok(())
}

impl Canvas {
    pub fn new(size: Size) -> Self {
        Canvas {
            size,
            pixels: vec![Shade::White; (size.width * size.height) as usize],
        }
    }

    #[cfg(test)]
    fn pixel(&self, point: Point) -> Option<Shade> {
        self.index(point).map(|i| self.pixels[i])
    }

    fn set(&mut self, point: Point, color: Shade) {
        if let Some(i) = self.index(point) {
            self.pixels[i] = color;
        }
    }

    fn fill(&mut self, color: Shade) {
        self.pixels.fill(color);
    }

    fn index(&self, point: Point) -> Option<usize> {
        let inside = (0..self.size.width as i32).contains(&point.x) && (0..self.size.height as i32).contains(&point.y);
        inside.then(|| (point.y as u32 * self.size.width + point.x as u32) as usize)
    }

    fn points(&self) -> impl Iterator<Item = (Point, Shade)> + '_ {
        let width = self.size.width as usize;
        self.pixels
            .iter()
            .enumerate()
            .map(move |(i, &color)| (Point::new((i % width) as i32, (i / width) as i32), color))
    }

    pub fn planes(&self, rotation: DisplayRotation) -> Planes {
        let plane = || {
            let mut plane = Display2in7b::default();
            plane.set_rotation(rotation);
            plane.clear(Color::White).ok();
            plane
        };
        let (mut high, mut low) = (plane(), plane());
        for (point, shade) in self.points() {
            if matches!(shade, Shade::Dark | Shade::Black) {
                high.set_pixel(Pixel(point, Color::Black));
            }
            if matches!(shade, Shade::Light | Shade::Black) {
                low.set_pixel(Pixel(point, Color::Black));
            }
        }
        Planes { high, low }
    }

    pub fn to_ppm(&self) -> Vec<u8> {
        let mut out = format!("P6\n{} {}\n255\n", self.size.width, self.size.height).into_bytes();
        for color in &self.pixels {
            out.extend_from_slice(match color {
                Shade::White => &[255, 255, 255],
                Shade::Light => &[170, 170, 170],
                Shade::Dark => &[85, 85, 85],
                Shade::Black => &[0, 0, 0],
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORTRAIT: Size = Size::new(WIDTH, HEIGHT);

    fn count(canvas: &Canvas, color: Shade) -> usize {
        canvas.points().filter(|&(_, c)| c == color).count()
    }

    fn panel_bit(plane: &Display2in7b, x: u32, y: u32) -> bool {
        let byte = plane.buffer()[(y * WIDTH / 8 + x / 8) as usize];
        byte & (0x80 >> (x % 8)) != 0
    }

    const LANDSCAPE: Size = Size::new(HEIGHT, WIDTH);

    fn inked(canvas: &Canvas) -> Option<(Point, Point)> {
        let mut points = canvas.points().filter(|&(_, c)| c != Shade::White).map(|(p, _)| p);
        let first = points.next()?;
        Some(points.fold((first, first), |(lo, hi), p| {
            (
                Point::new(lo.x.min(p.x), lo.y.min(p.y)),
                Point::new(hi.x.max(p.x), hi.y.max(p.y)),
            )
        }))
    }

    fn text(text: &str) -> Scene {
        Scene::Text { text: text.into() }
    }

    #[test]
    fn a_short_word_fills_the_panel() {
        let canvas = render(&text("Hoi"), LANDSCAPE).unwrap();
        let (lo, hi) = inked(&canvas).unwrap();
        assert!(hi.x - lo.x > 200, "ink spans {lo:?}..{hi:?}");
        assert!(hi.y - lo.y > 80, "ink spans {lo:?}..{hi:?}");
    }

    #[test]
    fn text_is_centred() {
        let canvas = render(&text("michel ist ganz toll"), LANDSCAPE).unwrap();
        let (lo, hi) = inked(&canvas).unwrap();
        let (left, right) = (lo.x, LANDSCAPE.width as i32 - 1 - hi.x);
        assert!((left - right).abs() <= 8, "left {left}, right {right}");
        let (top, bottom) = (lo.y, LANDSCAPE.height as i32 - 1 - hi.y);
        assert!((top - bottom).abs() <= 30, "top {top}, bottom {bottom}");
    }

    #[test]
    fn text_stays_inside_the_margin() {
        let long = "Chuchichäschtli ".repeat(40);
        let messages = [
            "Das isch absolute nervig, oder?",
            "Absolute nervtötend",
            "Absolute\u{a0}nervig",
            "Absolut nerv**tötend**, gäll",
            "eis zwei drüü vier füüf sächs siebe acht nüün zäh",
            "Hoi 👋 wie gaht's?",
            long.as_str(),
        ];
        for message in messages {
            for size in [LANDSCAPE, PORTRAIT] {
                let canvas = render(&text(message), size).unwrap();
                let (lo, hi) = inked(&canvas).unwrap();
                let (right, bottom) = (size.width as i32 - 1 - hi.x, size.height as i32 - 1 - hi.y);
                assert!(
                    lo.x >= 3 && right >= 3 && lo.y >= 3 && bottom >= 3,
                    "{message:?} in {size:?} inks {lo:?}..{hi:?}"
                );
            }
        }
    }

    #[test]
    fn markup_in_a_message_is_drawn_as_text() {
        assert!(render(&text("#panic(\"boom\") *bold* $x^2$ <label> @ref"), LANDSCAPE).is_ok());
    }

    #[test]
    fn emoji_are_drawn() {
        let canvas = render(&text("👍"), LANDSCAPE).unwrap();
        assert!(
            count(&canvas, Shade::Black) > 3000,
            "{} black pixels",
            count(&canvas, Shade::Black)
        );
    }

    #[test]
    fn strong_text_is_white_on_black() {
        let plain = count(&render(&text("Hoi"), LANDSCAPE).unwrap(), Shade::Black);
        let strong = count(&render(&text("**Hoi**"), LANDSCAPE).unwrap(), Shade::Black);
        assert!(2 * strong > 3 * plain, "plain {plain}, strong {strong}");
    }

    #[test]
    fn blank_text_draws_nothing() {
        assert!(inked(&render(&text("  \n "), LANDSCAPE).unwrap()).is_none());
    }

    #[test]
    fn text_edges_are_grey() {
        let canvas = render(&text("Hoi"), PORTRAIT).unwrap();
        assert!(count(&canvas, Shade::Black) > 0);
        assert!(count(&canvas, Shade::Dark) + count(&canvas, Shade::Light) > 0);
    }

    #[test]
    fn clear_is_all_white() {
        let canvas = render(&Scene::Clear, PORTRAIT).unwrap();
        assert_eq!(count(&canvas, Shade::White), (WIDTH * HEIGHT) as usize);
    }

    #[test]
    fn ring_is_a_white_hand_on_black() {
        let canvas = render(&Scene::Ring, PORTRAIT).unwrap();
        let white = count(&canvas, Shade::White);
        assert!(white > 0);
        assert!(count(&canvas, Shade::Black) > 3 * white);
        assert_eq!(canvas.pixel(Point::zero()), Some(Shade::Black));
    }

    #[test]
    fn ring_is_centred_in_landscape() {
        let canvas = render(&Scene::Ring, LANDSCAPE).unwrap();
        assert!(count(&canvas, Shade::White) > 0);
    }

    #[test]
    fn a_picture_is_drawn_in_grey() {
        let black = RgbaImage::from_pixel(100, 100, image::Rgba([0, 0, 0, 255]));
        let canvas = render(&Scene::Picture(Arc::new(black)), LANDSCAPE).unwrap();
        assert_eq!(count(&canvas, Shade::Black), 176 * 176);
        assert_eq!(Scene::Picture(Arc::new(RgbaImage::new(1, 1))).refresh(), Refresh::Gray);
    }

    #[test]
    fn planes_hold_the_two_bits_of_each_shade() {
        let mut canvas = Canvas::new(PORTRAIT);
        canvas.set(Point::new(1, 0), Shade::Light);
        canvas.set(Point::new(2, 0), Shade::Dark);
        canvas.set(Point::new(3, 0), Shade::Black);
        let planes = canvas.planes(DisplayRotation::Rotate0);
        let bits = |x| (panel_bit(&planes.high, x, 0), panel_bit(&planes.low, x, 0));

        assert_eq!(bits(0), (true, true));
        assert_eq!(bits(1), (true, false));
        assert_eq!(bits(2), (false, true));
        assert_eq!(bits(3), (false, false));
    }

    #[test]
    fn planes_apply_the_rotation() {
        let mut canvas = Canvas::new(PORTRAIT);
        canvas.set(Point::new(0, 0), Shade::Black);
        let planes = canvas.planes(DisplayRotation::Rotate180);
        assert!(panel_bit(&planes.high, 0, 0));
        assert!(!panel_bit(&planes.high, WIDTH - 1, HEIGHT - 1));
    }

    #[test]
    fn ppm_has_header_and_one_rgb_triple_per_pixel() {
        let ppm = Canvas::new(Size::new(3, 2)).to_ppm();
        let header = b"P6\n3 2\n255\n";
        assert_eq!(&ppm[..header.len()], header);
        assert_eq!(ppm.len(), header.len() + 3 * 2 * 3);
    }
}
