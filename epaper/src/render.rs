use anyhow::Result;
use embedded_graphics::prelude::*;
use epd_waveshare::{
    color::{Color, TriColor},
    epd2in7b::{Display2in7b, HEIGHT, WIDTH},
    graphics::DisplayRotation,
};

use crate::typeset::typeset;

const RING: &[u8] = include_bytes!("../assets/ring.raw");
const RING_WIDTH: u32 = 176;
const TEXT_TEMPLATE: &str = include_str!("text.typ");
const MAX_CHARS: usize = 1000;
const INK_BELOW: u32 = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scene {
    Ring,
    Text { text: String, ink: Ink },
    Clear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    Black,
    Red,
}

impl From<Ink> for TriColor {
    fn from(ink: Ink) -> Self {
        match ink {
            Ink::Black => TriColor::Black,
            Ink::Red => TriColor::Chromatic,
        }
    }
}

pub struct Canvas {
    size: Size,
    pixels: Vec<TriColor>,
}

pub struct Planes {
    pub black: Display2in7b,
    pub red: Display2in7b,
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
        Scene::Text { text, ink } => draw_text(&mut canvas, text, *ink)?,
    }
    Ok(canvas)
}

fn draw_ring(canvas: &mut Canvas) {
    let ring_height = (RING.len() * 8) as u32 / RING_WIDTH;
    canvas.fill(TriColor::Black);
    let offset = Point::new(
        (canvas.size.width as i32 - RING_WIDTH as i32) / 2,
        (canvas.size.height as i32 - ring_height as i32) / 2,
    );
    for y in 0..ring_height {
        for x in 0..RING_WIDTH {
            let bit = y * RING_WIDTH + x;
            if RING[(bit / 8) as usize] & (0x80 >> (bit % 8)) != 0 {
                canvas.set(offset + Point::new(x as i32, y as i32), TriColor::White);
            }
        }
    }
}

fn draw_text(canvas: &mut Canvas, text: &str, ink: Ink) -> Result<()> {
    let text: String = text.chars().take(MAX_CHARS).collect();
    let (width, height) = (canvas.size.width.to_string(), canvas.size.height.to_string());
    let ink_name = match ink {
        Ink::Black => "black",
        Ink::Red => "red",
    };
    let page = typeset(
        TEXT_TEMPLATE,
        &[
            ("text", text.as_str()),
            ("width", width.as_str()),
            ("height", height.as_str()),
            ("ink", ink_name),
        ],
    )?;
    for y in 0..page.height.min(canvas.size.height) {
        for x in 0..page.width.min(canvas.size.width) {
            if let Some(color) = ink_at(page.pixel(x, y)) {
                canvas.set(Point::new(x as i32, y as i32), color);
            }
        }
    }
    Ok(())
}

fn ink_at([r, g, b]: [u8; 3]) -> Option<TriColor> {
    let luma = (299 * r as u32 + 587 * g as u32 + 114 * b as u32) / 1000;
    if luma >= INK_BELOW {
        None
    } else if r > 120 && g < 90 {
        Some(TriColor::Chromatic)
    } else {
        Some(TriColor::Black)
    }
}

impl Canvas {
    pub fn new(size: Size) -> Self {
        Canvas {
            size,
            pixels: vec![TriColor::White; (size.width * size.height) as usize],
        }
    }

    #[cfg(test)]
    fn pixel(&self, point: Point) -> Option<TriColor> {
        self.index(point).map(|i| self.pixels[i])
    }

    fn set(&mut self, point: Point, color: TriColor) {
        if let Some(i) = self.index(point) {
            self.pixels[i] = color;
        }
    }

    fn fill(&mut self, color: TriColor) {
        self.pixels.fill(color);
    }

    fn index(&self, point: Point) -> Option<usize> {
        let inside = (0..self.size.width as i32).contains(&point.x) && (0..self.size.height as i32).contains(&point.y);
        inside.then(|| (point.y as u32 * self.size.width + point.x as u32) as usize)
    }

    fn points(&self) -> impl Iterator<Item = (Point, TriColor)> + '_ {
        let width = self.size.width as usize;
        self.pixels
            .iter()
            .enumerate()
            .map(move |(i, &color)| (Point::new((i % width) as i32, (i / width) as i32), color))
    }

    pub fn planes(&self, rotation: DisplayRotation) -> Planes {
        let mut black = Display2in7b::default();
        let mut red = Display2in7b::default();
        for plane in [&mut black, &mut red] {
            plane.set_rotation(rotation);
            plane.clear(Color::White).ok();
        }
        for (point, color) in self.points() {
            match color {
                TriColor::White => {}
                TriColor::Black => black.set_pixel(Pixel(point, Color::Black)),
                TriColor::Chromatic => red.set_pixel(Pixel(point, Color::Black)),
            }
        }
        Planes { black, red }
    }

    pub fn to_ppm(&self) -> Vec<u8> {
        let mut out = format!("P6\n{} {}\n255\n", self.size.width, self.size.height).into_bytes();
        for color in &self.pixels {
            out.extend_from_slice(match color {
                TriColor::White => &[255, 255, 255],
                TriColor::Black => &[0, 0, 0],
                TriColor::Chromatic => &[200, 0, 0],
            });
        }
        out
    }
}

impl OriginDimensions for Canvas {
    fn size(&self) -> Size {
        self.size
    }
}

impl DrawTarget for Canvas {
    type Color = TriColor;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<TriColor>>,
    {
        for Pixel(point, color) in pixels {
            self.set(point, color);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORTRAIT: Size = Size::new(WIDTH, HEIGHT);

    fn count(canvas: &Canvas, color: TriColor) -> usize {
        canvas.points().filter(|&(_, c)| c == color).count()
    }

    fn panel_bit(plane: &Display2in7b, x: u32, y: u32) -> bool {
        let byte = plane.buffer()[(y * WIDTH / 8 + x / 8) as usize];
        byte & (0x80 >> (x % 8)) != 0
    }

    const LANDSCAPE: Size = Size::new(HEIGHT, WIDTH);

    fn inked(canvas: &Canvas) -> Option<(Point, Point)> {
        let mut points = canvas.points().filter(|&(_, c)| c != TriColor::White).map(|(p, _)| p);
        let first = points.next()?;
        Some(points.fold((first, first), |(lo, hi), p| {
            (
                Point::new(lo.x.min(p.x), lo.y.min(p.y)),
                Point::new(hi.x.max(p.x), hi.y.max(p.y)),
            )
        }))
    }

    fn text(text: &str) -> Scene {
        Scene::Text {
            text: text.into(),
            ink: Ink::Black,
        }
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
    fn long_text_stays_on_the_panel() {
        let canvas = render(&text(&"Chuchichäschtli ".repeat(40)), LANDSCAPE).unwrap();
        let (lo, hi) = inked(&canvas).unwrap();
        assert!(lo.x >= 0 && lo.y >= 0);
        assert!(hi.x < LANDSCAPE.width as i32 && hi.y < LANDSCAPE.height as i32);
    }

    #[test]
    fn markup_in_a_message_is_drawn_as_text() {
        assert!(render(&text("#panic(\"boom\") *bold* $x^2$ <label> @ref"), LANDSCAPE).is_ok());
    }

    #[test]
    fn blank_text_draws_nothing() {
        assert!(inked(&render(&text("  \n "), LANDSCAPE).unwrap()).is_none());
    }

    #[test]
    fn text_is_drawn_in_its_ink() {
        let text = |ink| Scene::Text {
            text: "Hoi".into(),
            ink,
        };
        let black = render(&text(Ink::Black), PORTRAIT).unwrap();
        assert!(count(&black, TriColor::Black) > 0);
        assert_eq!(count(&black, TriColor::Chromatic), 0);

        let red = render(&text(Ink::Red), PORTRAIT).unwrap();
        assert!(count(&red, TriColor::Chromatic) > 0);
        assert_eq!(count(&red, TriColor::Black), 0);
    }

    #[test]
    fn clear_is_all_white() {
        let canvas = render(&Scene::Clear, PORTRAIT).unwrap();
        assert_eq!(count(&canvas, TriColor::White), (WIDTH * HEIGHT) as usize);
    }

    #[test]
    fn ring_is_a_white_hand_on_black() {
        let canvas = render(&Scene::Ring, PORTRAIT).unwrap();
        let white = count(&canvas, TriColor::White);
        assert!(white > 0);
        assert!(count(&canvas, TriColor::Black) > 3 * white);
        assert_eq!(canvas.pixel(Point::zero()), Some(TriColor::Black));
    }

    #[test]
    fn ring_is_centred_in_landscape() {
        let canvas = render(&Scene::Ring, LANDSCAPE).unwrap();
        assert!(count(&canvas, TriColor::White) > 0);
    }

    #[test]
    fn planes_encode_black_and_red_as_cleared_bits() {
        let mut canvas = Canvas::new(PORTRAIT);
        canvas.set(Point::new(0, 0), TriColor::Black);
        canvas.set(Point::new(9, 3), TriColor::Chromatic);
        let planes = canvas.planes(DisplayRotation::Rotate0);

        assert!(!panel_bit(&planes.black, 0, 0));
        assert!(panel_bit(&planes.red, 0, 0));
        assert!(panel_bit(&planes.black, 9, 3));
        assert!(!panel_bit(&planes.red, 9, 3));
        assert!(panel_bit(&planes.black, 1, 0));
        assert!(panel_bit(&planes.red, 1, 0));
    }

    #[test]
    fn planes_apply_the_rotation() {
        let mut canvas = Canvas::new(PORTRAIT);
        canvas.set(Point::new(0, 0), TriColor::Black);
        let planes = canvas.planes(DisplayRotation::Rotate180);
        assert!(panel_bit(&planes.black, 0, 0));
        assert!(!panel_bit(&planes.black, WIDTH - 1, HEIGHT - 1));
    }

    #[test]
    fn ppm_has_header_and_one_rgb_triple_per_pixel() {
        let ppm = Canvas::new(Size::new(3, 2)).to_ppm();
        let header = b"P6\n3 2\n255\n";
        assert_eq!(&ppm[..header.len()], header);
        assert_eq!(ppm.len(), header.len() + 3 * 2 * 3);
    }
}
