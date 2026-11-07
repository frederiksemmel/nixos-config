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
    let radius = width.min(height) / 2.0;
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
    let title = title.replace(['\n', '\r'], " ");
    if title.chars().count() <= TITLE_LENGTH {
        return title;
    }
    let cut: String = title.chars().take(TITLE_LENGTH - 1).collect();
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
}
