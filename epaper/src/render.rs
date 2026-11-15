use embedded_graphics::{
    mono_font::{MonoFont, MonoTextStyle, iso_8859_1},
    prelude::*,
    text::{Baseline, Text},
};
use epd_waveshare::{
    color::{Color, TriColor},
    epd2in7b::{Display2in7b, HEIGHT, WIDTH},
    graphics::DisplayRotation,
};

const RING: &[u8] = include_bytes!("../assets/ring.raw");
const RING_WIDTH: u32 = 176;
const MARGIN: u32 = 4;
const FONTS: [&MonoFont; 4] = [
    &iso_8859_1::FONT_10X20,
    &iso_8859_1::FONT_9X15,
    &iso_8859_1::FONT_7X13,
    &iso_8859_1::FONT_6X10,
];

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

pub fn render(scene: &Scene, size: Size) -> Canvas {
    let mut canvas = Canvas::new(size);
    match scene {
        Scene::Clear => {}
        Scene::Ring => draw_ring(&mut canvas),
        Scene::Text { text, ink } => draw_text(&mut canvas, text, (*ink).into()),
    }
    canvas
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

fn draw_text(canvas: &mut Canvas, text: &str, color: TriColor) {
    let (font, lines) = layout(text, canvas.size);
    let line_height = font.character_size.height;
    let block_height = line_height * lines.len() as u32;
    let top = (canvas.size.height.saturating_sub(block_height) / 2) as i32;
    let style = MonoTextStyle::new(font, color);
    for (i, line) in lines.iter().enumerate() {
        let origin = Point::new(MARGIN as i32, top + (i as u32 * line_height) as i32);
        Text::with_baseline(line, origin, style, Baseline::Top)
            .draw(canvas)
            .ok();
    }
}

pub fn layout(text: &str, size: Size) -> (&'static MonoFont<'static>, Vec<String>) {
    let inner = size.saturating_sub(Size::new_equal(2 * MARGIN));
    for font in FONTS {
        let (cols, rows) = grid(font, inner);
        let lines = wrap(text, cols);
        if lines.len() <= rows {
            return (font, lines);
        }
    }
    let font = FONTS[FONTS.len() - 1];
    let (cols, rows) = grid(font, inner);
    let mut lines = wrap(text, cols);
    lines.truncate(rows);
    if let Some(last) = lines.last_mut() {
        let keep = cols.saturating_sub(3).min(last.chars().count());
        *last = last.chars().take(keep).collect::<String>() + "...";
    }
    (font, lines)
}

fn grid(font: &MonoFont, area: Size) -> (usize, usize) {
    let advance = font.character_size.width + font.character_spacing;
    (
        (area.width / advance).max(1) as usize,
        (area.height / font.character_size.height) as usize,
    )
}

pub fn wrap(text: &str, cols: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.trim().lines() {
        let mut line = String::new();
        let mut len = 0;
        for word in paragraph.split_whitespace() {
            let mut word: Vec<char> = word.chars().collect();
            if len > 0 && len + 1 + word.len() <= cols {
                line.push(' ');
                line.extend(&word);
                len += 1 + word.len();
                continue;
            }
            if len > 0 {
                lines.push(std::mem::take(&mut line));
            }
            while word.len() > cols {
                lines.push(word.drain(..cols).collect());
            }
            len = word.len();
            line.extend(word);
        }
        lines.push(line);
    }
    lines
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

    #[test]
    fn wrap_fills_lines_greedily() {
        assert_eq!(wrap("ine gluegt du bisch dra", 10), ["ine gluegt", "du bisch", "dra"]);
    }

    #[test]
    fn wrap_keeps_newlines_and_splits_long_words() {
        assert_eq!(
            wrap("Grüezi\n\nChuchichäschtli", 6),
            ["Grüezi", "", "Chuchi", "chäsch", "tli"]
        );
    }

    #[test]
    fn layout_uses_the_largest_font_that_fits() {
        let (font, lines) = layout("Hoi", PORTRAIT);
        assert_eq!(font.character_size, iso_8859_1::FONT_10X20.character_size);
        assert_eq!(lines, ["Hoi"]);

        let long = "wort ".repeat(120);
        let (font, _) = layout(&long, PORTRAIT);
        assert_eq!(font.character_size, iso_8859_1::FONT_6X10.character_size);
    }

    #[test]
    fn layout_truncates_text_that_never_fits() {
        let (_, lines) = layout(&"x".repeat(5000), PORTRAIT);
        assert_eq!(lines.len(), 25);
        assert!(lines.last().unwrap().ends_with("..."));
        assert!(lines.iter().all(|l| l.chars().count() <= 28));
    }

    #[test]
    fn text_is_drawn_in_its_ink() {
        let text = |ink| Scene::Text {
            text: "Hoi".into(),
            ink,
        };
        let black = render(&text(Ink::Black), PORTRAIT);
        assert!(count(&black, TriColor::Black) > 0);
        assert_eq!(count(&black, TriColor::Chromatic), 0);

        let red = render(&text(Ink::Red), PORTRAIT);
        assert!(count(&red, TriColor::Chromatic) > 0);
        assert_eq!(count(&red, TriColor::Black), 0);
    }

    #[test]
    fn clear_is_all_white() {
        let canvas = render(&Scene::Clear, PORTRAIT);
        assert_eq!(count(&canvas, TriColor::White), (WIDTH * HEIGHT) as usize);
    }

    #[test]
    fn ring_is_a_white_hand_on_black() {
        let canvas = render(&Scene::Ring, PORTRAIT);
        let white = count(&canvas, TriColor::White);
        assert!(white > 0);
        assert!(count(&canvas, TriColor::Black) > 3 * white);
        assert_eq!(canvas.pixel(Point::zero()), Some(TriColor::Black));
    }

    #[test]
    fn ring_is_centred_in_landscape() {
        let canvas = render(&Scene::Ring, Size::new(HEIGHT, WIDTH));
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
