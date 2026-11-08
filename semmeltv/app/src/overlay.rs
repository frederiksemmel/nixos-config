pub const WIDTH: u32 = 1920;
pub const HEIGHT: u32 = 1080;

const LEFT: f64 = 96.0;
const RIGHT: f64 = 1824.0;
const BAR_TOP: f64 = 942.0;
const BAR_HEIGHT: f64 = 10.0;
const KNOB: f64 = 17.0;
const LABEL_HALF_WIDTH: f64 = 140.0;
const PAUSE_WIDTH: f64 = 58.0;
const TITLE_LENGTH: usize = 56;
const CURVE: f64 = 0.45;
const CLOCK_BOTTOM: f64 = 1016.0;
const CLOCK_STYLE: &str = r"\fs38\1a&H10&";
const TOAST_TOP: f64 = 72.0;
const TOAST_PADDING_X: f64 = 48.0;
const TOAST_PADDING_Y: f64 = 24.0;
const TOAST_GAP: f64 = 6.0;
const TOAST_RADIUS: f64 = 32.0;
const TOAST_LENGTH: usize = 72;
const TOAST_MIN_WIDTH: f64 = 320.0;
const HEADING_SIZE: f64 = 34.0;
const MESSAGE_SIZE: f64 = 42.0;
const GLYPH_WIDTH: f64 = 0.41;

const SCRIM: &str =
    r"{\an7\pos(0,0)\bord0\shad0\blur90\1c&H000000&\1a&H20&\p1}m -200 770 l 2120 770 l 2120 1300 l -200 1300{\p0}";

pub struct View<'a> {
    pub title: &'a str,
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    pub jump: Option<i32>,
}

impl View<'_> {
    pub fn render(&self) -> String {
        let mut events = vec![SCRIM.to_string(), self.title_line(), self.track(), self.fill()];
        events.extend(self.pause_mark());
        events.push(text(r"\an1", LEFT, CLOCK_BOTTOM, CLOCK_STYLE, &clock(self.position)));
        if self.duration > 0.0 {
            let left = clock((self.duration - self.position).max(0.0));
            events.push(text(r"\an3", RIGHT, CLOCK_BOTTOM, CLOCK_STYLE, &format!("-{left}")));
        }
        events.extend(self.seek_marks());
        events.join("\n")
    }

    fn title_line(&self) -> String {
        let x = if self.paused { LEFT + PAUSE_WIDTH } else { LEFT };
        text(r"\an1", x, 870.0, r"\fs46\b1", &escape(&shorten(self.title)))
    }

    fn pause_mark(&self) -> Option<String> {
        self.paused.then(|| {
            let bar = |x: f64| shape(x, 826.0, &pill(14.0, 40.0), "");
            format!("{}\n{}", bar(LEFT), bar(LEFT + 24.0))
        })
    }

    fn progress(&self) -> f64 {
        if self.duration > 0.0 {
            (self.position / self.duration).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    fn head(&self) -> f64 {
        LEFT + (RIGHT - LEFT) * self.progress()
    }

    fn track(&self) -> String {
        shape(LEFT, BAR_TOP, &pill(RIGHT - LEFT, BAR_HEIGHT), r"\1a&HA0&")
    }

    fn fill(&self) -> String {
        shape(
            LEFT,
            BAR_TOP,
            &pill((self.head() - LEFT).max(BAR_HEIGHT), BAR_HEIGHT),
            "",
        )
    }

    fn seek_marks(&self) -> Vec<String> {
        let Some(jump) = self.jump else {
            return Vec::new();
        };
        let head = self.head();
        let centre = BAR_TOP + BAR_HEIGHT / 2.0;
        let knob = shape(
            head - KNOB,
            centre - KNOB,
            &pill(KNOB * 2.0, KNOB * 2.0),
            r"\shad2\4a&H80&",
        );
        let label = format!(r"{}{{\b0\fs36\1a&H30&}}   {}", clock(self.position), jump_label(jump));
        let x = head.clamp(LEFT + LABEL_HALF_WIDTH, RIGHT - LABEL_HALF_WIDTH);
        vec![knob, text(r"\an2", x, 924.0, r"\fs44\b1", &label)]
    }
}

pub fn toast(heading: &str, message: &str) -> String {
    let heading = shorten_to(heading, TOAST_LENGTH);
    let message = shorten_to(message, TOAST_LENGTH);
    let estimate = |line: &str, size: f64| line.chars().count() as f64 * size * GLYPH_WIDTH;
    let width = (estimate(&heading, HEADING_SIZE).max(estimate(&message, MESSAGE_SIZE)) + 2.0 * TOAST_PADDING_X)
        .clamp(TOAST_MIN_WIDTH, RIGHT - LEFT);
    let heading_height = if heading.is_empty() {
        0.0
    } else {
        HEADING_SIZE + TOAST_GAP
    };
    let height = 2.0 * TOAST_PADDING_Y + heading_height + MESSAGE_SIZE;
    let centre = f64::from(WIDTH) / 2.0;
    let top = TOAST_TOP + TOAST_PADDING_Y;
    let mut events = vec![shape(
        centre - width / 2.0,
        TOAST_TOP,
        &rounded(width, height, TOAST_RADIUS),
        r"\1c&H332723&",
    )];
    if !heading.is_empty() {
        let style = format!(r"\shad0\fs{HEADING_SIZE}\1c&HB8ADA8&");
        events.push(text(r"\an8", centre, top, &style, &escape(&heading)));
    }
    let style = format!(r"\shad0\fs{MESSAGE_SIZE}");
    events.push(text(r"\an8", centre, top + heading_height, &style, &escape(&message)));
    events.join("\n")
}

fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn jump_label(seconds: i32) -> String {
    format!("{seconds:+}s")
}

fn text(anchor: &str, x: f64, y: f64, style: &str, content: &str) -> String {
    format!(r"{{{anchor}\pos({x:.0},{y:.0})\bord0\shad1.5\4c&H000000&\4a&HA0&{style}}}{content}")
}

fn shape(x: f64, y: f64, path: &str, style: &str) -> String {
    format!(r"{{\an7\pos({x:.1},{y:.1})\bord0\shad0\1c&HFFFFFF&{style}\p1}}{path}{{\p0}}")
}

pub fn pill(width: f64, height: f64) -> String {
    rounded(width, height, width.min(height) / 2.0)
}

fn rounded(width: f64, height: f64, radius: f64) -> String {
    let (w, h, r, c) = (width, height, radius, radius * CURVE);
    let mut path = format!("m {r:.1} 0 l {:.1} 0 ", w - r);
    path += &format!("b {:.1} 0 {w:.1} {c:.1} {w:.1} {r:.1} l {w:.1} {:.1} ", w - c, h - r);
    path += &format!(
        "b {w:.1} {:.1} {:.1} {h:.1} {:.1} {h:.1} l {r:.1} {h:.1} ",
        h - c,
        w - c,
        w - r
    );
    path += &format!("b {c:.1} {h:.1} 0 {:.1} 0 {:.1} l 0 {r:.1} ", h - c, h - r);
    path += &format!("b 0 {c:.1} {c:.1} 0 {r:.1} 0");
    path
}

fn shorten(title: &str) -> String {
    shorten_to(title, TITLE_LENGTH)
}

fn shorten_to(title: &str, length: usize) -> String {
    let title = title.replace(['\n', '\r'], " ");
    if title.chars().count() <= length {
        return title;
    }
    let cut: String = title.chars().take(length - 1).collect();
    format!("{}…", cut.trim_end())
}

fn escape(content: &str) -> String {
    let mut escaped = String::new();
    let mut after_backslash = false;
    for character in content.chars() {
        if after_backslash {
            escaped.push('\u{2060}');
        }
        if character == '{' {
            escaped.push('\\');
        }
        escaped.push(character);
        after_backslash = character == '\\';
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> View<'static> {
        View {
            title: "Sintel",
            position: 754.0,
            duration: 5400.0,
            paused: false,
            jump: None,
        }
    }

    #[test]
    fn clocks_read_like_a_video_player() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(59.9), "0:59");
        assert_eq!(clock(754.0), "12:34");
        assert_eq!(clock(3725.0), "1:02:05");
    }

    #[test]
    fn jumps_carry_their_direction() {
        assert_eq!(jump_label(30), "+30s");
        assert_eq!(jump_label(-120), "-120s");
    }

    #[test]
    fn the_view_shows_title_elapsed_and_remaining_time() {
        let rendered = view().render();
        assert!(rendered.contains("Sintel"));
        assert!(rendered.contains("12:34"));
        assert!(rendered.contains("-1:17:26"));
    }

    #[test]
    fn a_seek_shows_the_target_and_the_jump_size() {
        let rendered = View {
            jump: Some(30),
            ..view()
        }
        .render();
        assert!(rendered.contains("+30s"));
        assert_eq!(rendered.matches("12:34").count(), 2);
    }

    #[test]
    fn only_a_paused_view_has_the_pause_mark() {
        let bars = |view: &View| view.render().matches(&pill(14.0, 40.0)).count();
        assert_eq!(bars(&view()), 0);
        assert_eq!(bars(&View { paused: true, ..view() }), 2);
    }

    #[test]
    fn the_bar_follows_the_position_and_stays_on_the_track() {
        let at = |position| View { position, ..view() }.head();
        assert_eq!(at(0.0), LEFT);
        assert_eq!(at(2700.0), (LEFT + RIGHT) / 2.0);
        assert_eq!(at(9000.0), RIGHT);
    }

    #[test]
    fn a_video_of_unknown_length_has_no_remaining_time() {
        let rendered = View {
            duration: 0.0,
            ..view()
        }
        .render();
        assert!(rendered.contains("12:34"));
        assert!(!rendered.contains(r"\an3"));
    }

    #[test]
    fn long_titles_are_shortened() {
        let long = "x".repeat(100);
        let short = shorten(&long);
        assert_eq!(short.chars().count(), TITLE_LENGTH);
        assert!(short.ends_with('…'));
    }

    #[test]
    fn a_title_cannot_become_ass_markup() {
        assert_eq!(escape(r"{\b1}Pilot"), "\\{\\\u{2060}b1}Pilot");
        assert!(!shorten("one\ntwo").contains('\n'));
    }

    fn toast_box(rendered: &str) -> (f64, f64) {
        let start = rendered.find(r"\pos(").unwrap() + 5;
        let end = start + rendered[start..].find(',').unwrap();
        let left: f64 = rendered[start..end].parse().unwrap();
        let path_start = rendered.find(r"\p1}").unwrap() + 4;
        let path_end = path_start + rendered[path_start..].find('{').unwrap();
        let right = rendered[path_start..path_end]
            .split(['m', 'l', 'b'])
            .flat_map(|part| part.split_whitespace().step_by(2).map(|x| x.parse::<f64>().unwrap()))
            .fold(0.0, f64::max);
        (left, left + right)
    }

    #[test]
    fn a_toast_is_centred_and_fits_its_text() {
        let short = toast("", "Listening…");
        let long = toast("“Play White Lotus”", "Playing The White Lotus · season 1, episode 1");
        let (left, right) = toast_box(&long);
        assert!((left + right - f64::from(WIDTH)).abs() < 0.2);
        assert!(toast_box(&short).1 - toast_box(&short).0 < right - left);
        assert!(long.contains(r"\an8\pos(960,"));
    }

    #[test]
    fn a_long_transcript_is_shortened_to_one_line() {
        let rendered = toast(&"word ".repeat(40), "Playing");
        let (left, right) = toast_box(&rendered);
        assert!(left >= LEFT && right <= RIGHT);
        assert!(rendered.contains('…'));
    }
}
