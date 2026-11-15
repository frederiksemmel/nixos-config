//! The card the TV shows while a film or episode opens: the Semmel from the
//! logo, breathing like rising dough, beside what is starting. It is drawn in
//! mpv's OSD from the favicon's vector paths, over mpv's own background colour.

use crate::overlay::{self, escape, shorten_to};
use std::f64::consts::TAU;
use std::sync::LazyLock;
use std::time::Duration;

/// The oven: mpv paints it behind the card until the video starts.
pub const BACKGROUND: &str = "#1A0F09";

const LOGO: &str = include_str!("../../assets/logo/favicon.svg");
const INK: &str = "&H070C16&";
const CRUST: &str = "&H73C4F8&";
const CRUMB: &str = "&HBDE6FD&";
const SEMMEL_HEIGHT: f64 = 340.0;
const GAP: f64 = 0.16;
const TITLE_SIZE: f64 = 92.0;
const SMALLEST_TITLE: f64 = 64.0;
const LINE_SIZE: f64 = 48.0;
const LINE_ALPHA: f64 = 0.65;
const TITLE_EM: f64 = 0.47;
const LINE_EM: f64 = 0.45;
const MARGIN: f64 = 140.0;
const BREATH: f64 = 1.8;
const SWELL: f64 = 0.03;
const BLINK_EVERY: f64 = 3.6;
const BLINK_FOR: f64 = 0.14;
/// Drawings use `\p4`, so coordinates are in eighths of a pixel.
const SUBPIXELS: f64 = 8.0;

/// What the card says: the title and, under it, the episode and where it picks up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Billing {
    pub title: String,
    pub lines: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
enum Step {
    Move(f64, f64),
    Line(f64, f64),
    Curve([f64; 6]),
}

type Path = Vec<Vec<Step>>;

struct Group {
    fill: String,
    stroke: f64,
    paths: Vec<Path>,
}

struct Semmel {
    body: Path,
    cheeks: Vec<Path>,
    mouth: Vec<Path>,
    lines: Vec<Path>,
    eyes: Vec<Path>,
    smile: Vec<Path>,
    eye_stroke: f64,
    highlights: Vec<Path>,
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
}

static SEMMEL: LazyLock<Semmel> = LazyLock::new(|| semmel(LOGO).expect("the favicon has the Semmel's six layers"));

fn semmel(svg: &str) -> Option<Semmel> {
    let mut groups = svg
        .split("<g ")
        .skip(1)
        .map(group)
        .collect::<Option<Vec<_>>>()?
        .into_iter();
    let _silhouette = groups.next()?;
    let cheeks = groups.next()?;
    let mouth = groups.next()?;
    let lines = groups.next()?;
    let mut eyes = groups.next()?;
    let highlights = groups.next()?;
    if [&cheeks, &mouth, &lines, &eyes]
        .iter()
        .any(|group| group.fill.is_empty())
        || eyes.paths.len() < 3
    {
        return None;
    }
    let body = vec![lines.paths.first()?.first()?.clone()];
    let smile = eyes.paths.split_off(2);
    let points = lines.paths.iter().flatten().flatten().flat_map(|step| match *step {
        Step::Move(x, y) | Step::Line(x, y) => vec![(x, y)],
        Step::Curve(c) => vec![(c[0], c[1]), (c[2], c[3]), (c[4], c[5])],
    });
    let (mut left, mut right, mut top, mut bottom) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for (x, y) in points {
        left = left.min(x);
        right = right.max(x);
        top = top.min(y);
        bottom = bottom.max(y);
    }
    Some(Semmel {
        body,
        cheeks: cheeks.paths,
        mouth: mouth.paths,
        lines: lines.paths,
        eye_stroke: eyes.stroke,
        eyes: eyes.paths,
        smile,
        highlights: highlights.paths,
        left,
        right,
        top,
        bottom,
    })
}

fn attribute<'a>(attributes: &'a str, name: &str) -> Option<&'a str> {
    let start = attributes.find(&format!("{name}=\""))? + name.len() + 2;
    let end = attributes[start..].find('"')?;
    Some(&attributes[start..start + end])
}

fn group(part: &str) -> Option<Group> {
    let (attributes, body) = part.split_once('>')?;
    let paths = body
        .split("<path d=\"")
        .skip(1)
        .map(|rest| rest.split('"').next().map(parse))
        .collect::<Option<Vec<_>>>()?;
    Some(Group {
        fill: attribute(attributes, "fill")?.to_string(),
        stroke: attribute(attributes, "stroke-width")
            .and_then(|width| width.parse::<f64>().ok())
            .unwrap_or(0.0)
            / 10.0,
        paths,
    })
}

/// Reads a potrace path (`M`, then relative `m`, `l`, `c` and `z`) into
/// absolute points, applying potrace's `translate(0,738) scale(0.1,-0.1)`.
fn parse(data: &str) -> Path {
    let place = |x: f64, y: f64| (x * 0.1, 738.0 - y * 0.1);
    let mut tokens = Vec::new();
    let mut number = String::new();
    for character in data.chars() {
        if character.is_ascii_digit() || character == '.' || (character == '-' && number.is_empty()) {
            number.push(character);
            continue;
        }
        if !number.is_empty() {
            tokens.push(Token::Number(number.parse().unwrap_or(0.0)));
            number.clear();
        }
        if character == '-' {
            number.push('-');
        } else if character.is_ascii_alphabetic() {
            tokens.push(Token::Command(character));
        }
    }
    if !number.is_empty() {
        tokens.push(Token::Number(number.parse().unwrap_or(0.0)));
    }
    let mut path: Path = Vec::new();
    let (mut x, mut y, mut start) = (0.0, 0.0, (0.0, 0.0));
    let mut command = 'M';
    let mut numbers = Vec::new();
    for token in tokens {
        match token {
            Token::Command(letter) => {
                command = letter;
                numbers.clear();
                if letter.eq_ignore_ascii_case(&'z') {
                    (x, y) = start;
                }
            }
            Token::Number(value) => {
                numbers.push(value);
                let relative = command.is_ascii_lowercase();
                let (dx, dy) = if relative { (x, y) } else { (0.0, 0.0) };
                match command.to_ascii_lowercase() {
                    'm' if numbers.len() == 2 => {
                        (x, y) = (dx + numbers[0], dy + numbers[1]);
                        start = (x, y);
                        let (px, py) = place(x, y);
                        path.push(vec![Step::Move(px, py)]);
                        command = if relative { 'l' } else { 'L' };
                        numbers.clear();
                    }
                    'l' if numbers.len() == 2 => {
                        (x, y) = (dx + numbers[0], dy + numbers[1]);
                        let (px, py) = place(x, y);
                        if let Some(subpath) = path.last_mut() {
                            subpath.push(Step::Line(px, py));
                        }
                        numbers.clear();
                    }
                    'c' if numbers.len() == 6 => {
                        let n = &numbers;
                        let (ax, ay) = place(dx + n[0], dy + n[1]);
                        let (bx, by) = place(dx + n[2], dy + n[3]);
                        (x, y) = (dx + n[4], dy + n[5]);
                        let (cx, cy) = place(x, y);
                        if let Some(subpath) = path.last_mut() {
                            subpath.push(Step::Curve([ax, ay, bx, by, cx, cy]));
                        }
                        numbers.clear();
                    }
                    _ => {}
                }
            }
        }
    }
    path
}

enum Token {
    Command(char),
    Number(f64),
}

/// Where the Semmel stands and how it breathes: the base centre, its height,
/// and how far it is stretched across and squashed down.
struct Pose {
    centre: f64,
    base: f64,
    scale: f64,
    across: f64,
    down: f64,
}

impl Pose {
    fn place(&self, semmel: &Semmel, x: f64, y: f64) -> (i64, i64) {
        let middle = (semmel.left + semmel.right) / 2.0;
        let px = self.centre + (x - middle) * self.scale * self.across;
        let py = self.base - (semmel.bottom - y) * self.scale * self.down;
        ((px * SUBPIXELS).round() as i64, (py * SUBPIXELS).round() as i64)
    }

    fn drawing(&self, semmel: &Semmel, path: &Path) -> String {
        let mut out = Vec::new();
        for step in path.iter().flatten() {
            match *step {
                Step::Move(x, y) => {
                    let (a, b) = self.place(semmel, x, y);
                    out.push(format!("m {a} {b}"));
                }
                Step::Line(x, y) => {
                    let (a, b) = self.place(semmel, x, y);
                    out.push(format!("l {a} {b}"));
                }
                Step::Curve(c) => {
                    let (a, b) = self.place(semmel, c[0], c[1]);
                    let (d, e) = self.place(semmel, c[2], c[3]);
                    let (f, g) = self.place(semmel, c[4], c[5]);
                    out.push(format!("b {a} {b} {d} {e} {f} {g}"));
                }
            }
        }
        out.join(" ")
    }
}

fn shape(drawing: &str, color: &str, border: f64, alpha: &str) -> String {
    format!(r"{{\an7\pos(0,0)\bord{border:.2}\shad0\1c{color}\3c{INK}\alpha{alpha}\p4}}{drawing}{{\p0}}")
}

fn alpha(opacity: f64) -> String {
    format!("&H{:02X}&", (255.0 - opacity.clamp(0.0, 1.0) * 255.0).round() as u8)
}

/// A closed, smiling eye over where `eye` is open.
fn lid(eye: &Path) -> Path {
    let points = eye.iter().flatten().filter_map(|step| match *step {
        Step::Move(x, y) | Step::Line(x, y) => Some((x, y)),
        Step::Curve(c) => Some((c[4], c[5])),
    });
    let (mut left, mut right, mut top, mut bottom) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
    for (x, y) in points {
        left = left.min(x);
        right = right.max(x);
        top = top.min(y);
        bottom = bottom.max(y);
    }
    let (x, y, w) = ((left + right) / 2.0, (top + bottom) / 2.0, (right - left) * 0.62);
    vec![vec![
        Step::Move(x - w, y),
        Step::Curve([x - w * 0.5, y + w * 0.7, x + w * 0.5, y + w * 0.7, x + w, y]),
        Step::Curve([x + w * 0.5, y + w * 0.95, x - w * 0.5, y + w * 0.95, x - w, y]),
    ]]
}

fn text(x: f64, y: f64, size: f64, bold: bool, color: &str, alpha: &str, content: &str) -> String {
    let weight = if bold { 700 } else { 400 };
    format!(
        r"{{\an1\pos({x:.0},{y:.0})\bord0\shad0\b{weight}\fs{size:.0}\fsp-0.5\1c{color}\alpha{alpha}}}{}",
        escape(content)
    )
}

/// The card `since` the film began opening, at `opacity` (1 while it opens,
/// falling to 0 as the video takes over).
pub fn frame(billing: &Billing, since: Duration, opacity: f64) -> String {
    let semmel = &*SEMMEL;
    let seconds = since.as_secs_f64();
    let breath = (seconds / BREATH * TAU).sin();
    let blinking = seconds % BLINK_EVERY > BLINK_EVERY - BLINK_FOR;
    let scale = SEMMEL_HEIGHT / (semmel.bottom - semmel.top);
    let width = (semmel.right - semmel.left) * scale;
    let gap = width * GAP;

    let room = f64::from(overlay::WIDTH) - 2.0 * MARGIN - width - gap;
    let longest_line =
        billing.lines.iter().map(|line| line.chars().count()).max().unwrap_or(0) as f64 * LINE_SIZE * LINE_EM;
    let title_chars = billing.title.chars().count().max(1) as f64;
    let title_size = (room / (title_chars * TITLE_EM)).clamp(SMALLEST_TITLE, TITLE_SIZE);
    let fits = (room / (title_size * TITLE_EM)).floor().max(8.0) as usize;
    let title = shorten_to(&billing.title, fits);
    let text_width = (title.chars().count() as f64 * title_size * TITLE_EM)
        .max(longest_line)
        .min(room);
    let left = ((f64::from(overlay::WIDTH) - width - gap - text_width) / 2.0).max(MARGIN);

    let middle = f64::from(overlay::HEIGHT) / 2.0;
    let pose = Pose {
        centre: left + width / 2.0,
        base: middle + SEMMEL_HEIGHT / 2.0,
        scale,
        across: 1.0 + SWELL * breath,
        down: 1.0 - SWELL * breath,
    };
    let seen = alpha(opacity);
    let mut events = vec![shape(&pose.drawing(semmel, &semmel.body), CRUST, 2.0, &seen)];
    for path in semmel.cheeks.iter() {
        events.push(shape(&pose.drawing(semmel, path), "&H9C9AF5&", 0.0, &seen));
    }
    for path in semmel.mouth.iter() {
        events.push(shape(&pose.drawing(semmel, path), "&H4E47E0&", 0.0, &seen));
    }
    for path in semmel.lines.iter() {
        events.push(shape(&pose.drawing(semmel, path), INK, 0.0, &seen));
    }
    let eye_border = semmel.eye_stroke * scale / 2.0;
    for path in semmel.smile.iter() {
        events.push(shape(&pose.drawing(semmel, path), INK, eye_border, &seen));
    }
    if blinking {
        for eye in semmel.eyes.iter() {
            events.push(shape(&pose.drawing(semmel, &lid(eye)), INK, 0.0, &seen));
        }
    } else {
        for path in semmel.eyes.iter() {
            events.push(shape(&pose.drawing(semmel, path), INK, eye_border, &seen));
        }
        for path in semmel.highlights.iter() {
            events.push(shape(&pose.drawing(semmel, path), "&HFFFFFF&", 0.0, &seen));
        }
    }

    let x = left + width + gap;
    let block = title_size + billing.lines.len() as f64 * LINE_SIZE * 1.25;
    let mut y = middle - block / 2.0 + title_size * 0.95;
    events.push(text(x, y, title_size, true, CRUMB, &seen, &title));
    y += 14.0;
    let quiet = alpha(opacity * LINE_ALPHA);
    for line in &billing.lines {
        y += LINE_SIZE * 1.25;
        events.push(text(x, y, LINE_SIZE, false, CRUMB, &quiet, &shorten_to(line, 48)));
    }
    events.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_names_what_starts_and_stays_on_screen() {
        let billing = Billing {
            title: "Star Wars: Maul – Shadow Lord, an Extremely Long Title That Goes On".to_string(),
            lines: vec!["4. The Hunt Begins".to_string(), "Picks up at 3:12".to_string()],
        };
        let card = frame(&billing, Duration::from_millis(400), 1.0);
        assert!(card.contains("Star Wars: Maul"));
        assert!(card.contains('…'));
        assert!(card.contains("4. The Hunt Begins") && card.contains("Picks up at 3:12"));
        let positions = card
            .split(r"\pos(")
            .skip(1)
            .filter_map(|rest| rest.split(',').next()?.parse::<f64>().ok());
        assert!(positions.clone().all(|x| (0.0..1920.0).contains(&x)));
        let drawn_x = card
            .split(r"\p4}")
            .skip(1)
            .flat_map(|drawing| {
                drawing
                    .split("{\\p0}")
                    .next()
                    .unwrap_or("")
                    .split(' ')
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let coordinates = drawn_x
            .iter()
            .filter_map(|value| value.parse::<i64>().ok())
            .collect::<Vec<_>>();
        assert!(coordinates.iter().step_by(2).all(|x| (0..1920 * 8).contains(x)));
    }

    #[test]
    fn the_semmel_breathes_blinks_and_fades() {
        let billing = Billing {
            title: "Dune".to_string(),
            lines: Vec::new(),
        };
        let rest = frame(&billing, Duration::ZERO, 1.0);
        let swell = frame(&billing, Duration::from_secs_f64(BREATH / 4.0), 1.0);
        assert_ne!(rest, swell);
        let blink = frame(&billing, Duration::from_secs_f64(BLINK_EVERY - BLINK_FOR / 2.0), 1.0);
        assert!(blink.matches(r"\p4}").count() < rest.matches(r"\p4}").count());
        assert!(frame(&billing, Duration::ZERO, 0.0).contains(r"\alpha&HFF&"));
        assert!(rest.contains(r"\alpha&H00&"));
    }
}
