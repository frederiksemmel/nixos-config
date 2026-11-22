//! The bake-off on the TV: drawn over the paused film as an mpv overlay,
//! with the remote's OK as the one button. Like a phone, the TV times the
//! oven on its own clock and sends its presses when the oven is done.

use super::bake::{BURNS_AFTER_MS, Baked, GAME, Input, REST_MS, SEMMELS, WINDOW, crust};
use super::{Ending, Side, View};
use crate::command::Command;
use crate::overlay::{self, HEIGHT, WIDTH};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

const DOUGH: (f64, f64, f64) = (243.0, 227.0, 192.0);
const GOLDEN: (f64, f64, f64) = (217.0, 154.0, 58.0);
const BURNT: (f64, f64, f64) = (59.0, 36.0, 18.0);
const CENTRE: f64 = WIDTH as f64 / 2.0;
const BAR_LEFT: f64 = 560.0;
const BAR_WIDTH: f64 = 800.0;
const BAR_TOP: f64 = 740.0;

/// What the TV answers in a bake-off it is challenged to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "kebab-case")]
pub enum Answer {
    Accept,
    DropOff,
    Presses {
        presses: [Option<f64>; SEMMELS],
    },
    /// The result was read; the film goes on or ends.
    Dismiss,
}

/// The bake-off as the TV shows it, from the moment it is challenged until
/// the result is dismissed.
pub struct Stand {
    view: View,
    since: Instant,
    focus: usize,
    presses: [Option<f64>; SEMMELS],
    sent: bool,
}

impl Stand {
    pub fn new(view: View, now: Instant) -> Self {
        Self {
            view,
            since: now,
            focus: 0,
            presses: [None; SEMMELS],
            sent: false,
        }
    }

    /// A newer view of the same bake-off. The oven keeps its own start and
    /// presses once it bakes.
    pub fn update(&mut self, view: View, now: Instant) {
        let baking_already = matches!(self.view, View::Baking { .. }) && matches!(view, View::Baking { .. });
        if !baking_already {
            self.since = now;
            self.view = view;
        }
    }

    /// Whether the TV draws it again and again: the countdown to answer
    /// and the oven both move.
    pub fn animates(&self) -> bool {
        match self.view {
            View::Asked { .. } => true,
            View::Baking { .. } => !self.sent,
            View::Done { .. } => false,
        }
    }

    fn slots(golden_ms: &[u32; SEMMELS]) -> [(f64, f64); SEMMELS] {
        let mut at = 0.0;
        golden_ms.map(|golden| {
            let slot = (at, f64::from(golden));
            at += f64::from(golden + BURNS_AFTER_MS + REST_MS);
            slot
        })
    }

    /// Milliseconds since the first Semmel went in, negative before.
    fn elapsed(&self, starts_in_ms: i64, now: Instant) -> f64 {
        let since = now.saturating_duration_since(self.since).as_secs_f64() * 1000.0;
        since - starts_in_ms.max(0) as f64
    }

    pub fn key(&mut self, command: Command, now: Instant) -> Option<Answer> {
        match (&self.view, command) {
            (View::Asked { .. }, Command::Left | Command::Right) => {
                self.focus = 1 - self.focus;
                None
            }
            (View::Asked { .. }, Command::Select) if self.focus == 0 => Some(Answer::Accept),
            (View::Asked { .. }, Command::Select) => Some(Answer::DropOff),
            (
                View::Baking {
                    starts_in_ms,
                    golden_ms,
                    ..
                },
                Command::Select,
            ) => {
                let elapsed = self.elapsed(*starts_in_ms, now);
                let burns = f64::from(BURNS_AFTER_MS);
                let found = Self::slots(golden_ms)
                    .iter()
                    .position(|(start, golden)| elapsed >= *start && elapsed < start + golden + burns);
                if let Some(index) = found
                    && self.presses[index].is_none()
                {
                    let (start, golden) = Self::slots(golden_ms)[index];
                    self.presses[index] = Some((elapsed - start - golden).round());
                }
                None
            }
            (View::Done { .. }, Command::Select | Command::Back) => Some(Answer::Dismiss),
            _ => None,
        }
    }

    /// The presses, once the oven is done.
    pub fn tick(&mut self, now: Instant) -> Option<Answer> {
        let View::Baking {
            starts_in_ms,
            length_ms,
            ..
        } = &self.view
        else {
            return None;
        };
        if self.sent || self.elapsed(*starts_in_ms, now) < f64::from(*length_ms) {
            return None;
        }
        self.sent = true;
        Some(Answer::Presses { presses: self.presses })
    }

    pub fn frame(&self, now: Instant) -> String {
        let mut events = vec![shape(
            0.0,
            0.0,
            &box_path(f64::from(WIDTH), f64::from(HEIGHT)),
            r"\1c&H000000&\1a&H30&",
        )];
        match &self.view {
            View::Asked {
                challenger,
                answer_within_ms,
                ..
            } => {
                events.push(heading(&format!("{GAME}!")));
                events.push(line(
                    300.0,
                    40.0,
                    &format!("{challenger} fordert dich zum {GAME} heraus. The better baker keeps watching."),
                ));
                for (index, (label, x)) in [("Bake", CENTRE - 220.0), ("Stop watching", CENTRE + 220.0)]
                    .into_iter()
                    .enumerate()
                {
                    events.extend(pill(x, 560.0, label, index == self.focus));
                }
                let left =
                    Duration::from_millis(*answer_within_ms).saturating_sub(now.saturating_duration_since(self.since));
                events.push(line(
                    700.0,
                    34.0,
                    &format!("Answer within {}", overlay::clock(left.as_secs_f64())),
                ));
            }
            View::Baking {
                starts_in_ms,
                golden_ms,
                bias_ms,
                ..
            } => {
                events.push(heading(GAME));
                let elapsed = self.elapsed(*starts_in_ms, now);
                let bias = bias_ms.get(&Input::Remote).copied().unwrap_or_default() / 1000.0;
                events.extend(self.oven(elapsed, golden_ms, bias));
            }
            View::Done {
                challenger,
                winner,
                ending,
                baked,
                ..
            } => {
                let (title, detail) = verdict(challenger, *winner, *ending);
                events.push(heading(&title));
                events.push(line(300.0, 40.0, &detail));
                events.extend(table(baked));
                let hint = if *winner == Some(Side::Challenger) {
                    "OK"
                } else {
                    "OK: back to the film"
                };
                events.push(line(850.0, 34.0, hint));
            }
        }
        events.join("\n")
    }

    fn oven(&self, elapsed: f64, golden_ms: &[u32; SEMMELS], bias: f64) -> Vec<String> {
        let mut events = Vec::new();
        let burns = f64::from(BURNS_AFTER_MS);
        if elapsed < 0.0 {
            events.push(text(CENTRE, 300.0, 96.0, &format!("{}", (-elapsed / 1000.0).ceil())));
            events.push(semmel(DOUGH, false));
        } else {
            let slots = Self::slots(golden_ms);
            let index = slots.iter().rposition(|(start, _)| elapsed >= *start).unwrap_or(0);
            let (start, golden) = slots[index];
            let into = elapsed - start;
            let stopped = self.presses[index].map_or(golden + burns, |press| golden + press);
            let shown = into.min(stopped);
            let colour = if shown <= golden {
                mix(DOUGH, GOLDEN, shown / golden)
            } else {
                mix(GOLDEN, BURNT, (shown - golden) / burns)
            };
            events.push(semmel(colour, into >= stopped));
            let share = |at: f64| (at / (golden + burns)).clamp(0.0, 1.0) * BAR_WIDTH;
            events.push(shape(BAR_LEFT, BAR_TOP, &box_path(BAR_WIDTH, 24.0), r"\1c&H3A3A3A&"));
            let band = share(golden - WINDOW * 500.0);
            events.push(shape(
                BAR_LEFT + band,
                BAR_TOP - 6.0,
                &box_path(share(golden + WINDOW * 500.0) - band, 36.0),
                r"\1c&H18C5F5&\1a&H60&",
            ));
            events.push(shape(
                BAR_LEFT + share(into) - 3.0,
                BAR_TOP - 12.0,
                &box_path(6.0, 48.0),
                "",
            ));
        }
        let scores: Vec<String> = self
            .presses
            .iter()
            .flatten()
            .map(|press| format!("+{:.0}", crust(press / 1000.0 - bias)))
            .collect();
        events.push(text(CENTRE, 810.0, 44.0, &scores.join("   ")));
        events.push(line(870.0, 34.0, "Press OK when the Semmel is golden"));
        events
    }
}

/// The result's heading and line, for the TV, which is always the one
/// challenged.
fn verdict(challenger: &str, winner: Option<Side>, ending: Ending) -> (String, String) {
    let resting = "You can challenge again in 10 minutes.".to_string();
    match (winner, ending) {
        (None, _) => ("Nobody baked".to_string(), format!("{challenger} went away.")),
        (Some(Side::Defender), Ending::Baked) => (
            "You keep the oven".to_string(),
            "Fresh out of the oven: enjoy the film.".to_string(),
        ),
        (Some(Side::Defender), _) => (
            "You keep the oven".to_string(),
            format!("{challenger} left the bake-off."),
        ),
        (Some(Side::Challenger), Ending::Baked) => (format!("{challenger} bakes better"), resting),
        (Some(Side::Challenger), Ending::NoAnswer) => ("You didn't answer in time".to_string(), resting),
        (Some(Side::Challenger), _) => ("You handed over the oven".to_string(), String::new()),
    }
}

fn table(baked: &[Option<Baked>; 2]) -> Vec<String> {
    let [Some(challenger), Some(defender)] = baked else {
        return Vec::new();
    };
    let mut events = vec![
        text(CENTRE - 260.0, 440.0, 34.0, "Them"),
        text(CENTRE + 260.0, 440.0, 34.0, "You"),
    ];
    for index in 0..SEMMELS {
        let y = 500.0 + 60.0 * index as f64;
        let row = |baked: &Baked| format!("{:.0} crust · {:.0} rise", baked.crust[index], baked.rise[index]);
        events.push(text(CENTRE - 260.0, y, 36.0, &row(challenger)));
        events.push(text(CENTRE + 260.0, y, 36.0, &row(defender)));
    }
    events.push(text(CENTRE - 260.0, 700.0, 44.0, &format!("{:.0}", challenger.total)));
    events.push(text(CENTRE + 260.0, 700.0, 44.0, &format!("{:.0}", defender.total)));
    events
}

fn mix(from: (f64, f64, f64), to: (f64, f64, f64), share: f64) -> (f64, f64, f64) {
    let share = share.clamp(0.0, 1.0);
    (
        from.0 + (to.0 - from.0) * share,
        from.1 + (to.1 - from.1) * share,
        from.2 + (to.2 - from.2) * share,
    )
}

fn colour((red, green, blue): (f64, f64, f64)) -> String {
    format!(r"\1c&H{:02X}{:02X}{:02X}&", blue as u8, green as u8, red as u8)
}

fn semmel(fill: (f64, f64, f64), out: bool) -> String {
    let (rx, ry) = (200.0, 140.0);
    let lift = if out { 24.0 } else { 0.0 };
    shape(CENTRE - rx, 520.0 - ry - lift, &ellipse(rx, ry), &colour(fill))
}

fn ellipse(rx: f64, ry: f64) -> String {
    let (kx, ky) = (rx * 0.5523, ry * 0.5523);
    let (w, h) = (2.0 * rx, 2.0 * ry);
    format!(
        "m {rx:.1} 0 b {:.1} 0 {w:.1} {:.1} {w:.1} {ry:.1} b {w:.1} {:.1} {:.1} {h:.1} {rx:.1} {h:.1} b {:.1} {h:.1} 0 {:.1} 0 {ry:.1} b 0 {:.1} {:.1} 0 {rx:.1} 0",
        rx + kx,
        ry - ky,
        ry + ky,
        rx + kx,
        rx - kx,
        ry + ky,
        ry - ky,
        rx - kx,
    )
}

fn box_path(width: f64, height: f64) -> String {
    format!("m 0 0 l {width:.1} 0 l {width:.1} {height:.1} l 0 {height:.1}")
}

fn shape(x: f64, y: f64, path: &str, style: &str) -> String {
    format!(r"{{\an7\pos({x:.1},{y:.1})\bord0\shad0\1c&HFFFFFF&{style}\p1}}{path}{{\p0}}")
}

fn text(x: f64, y: f64, size: f64, content: &str) -> String {
    format!(
        r"{{\an8\pos({x:.0},{y:.0})\bord0\shad1.5\4c&H000000&\4a&HA0&\fs{size:.0}}}{}",
        overlay::escape(content)
    )
}

fn heading(content: &str) -> String {
    text(CENTRE, 180.0, 80.0, content).replacen(r"\fs80", r"\fs80\b1", 1)
}

fn line(y: f64, size: f64, content: &str) -> String {
    text(CENTRE, y, size, content)
}

fn pill(x: f64, y: f64, label: &str, focused: bool) -> Vec<String> {
    let (width, height) = (380.0, 96.0);
    let (fill, ink) = if focused {
        (r"\1c&HFFFFFF&", r"\1c&H120D0B&")
    } else {
        (r"\1c&HFFFFFF&\1a&HD0&", r"\1c&HFFFFFF&")
    };
    vec![
        shape(x - width / 2.0, y, &overlay::pill(width, height), fill),
        format!(
            r"{{\an5\pos({x:.0},{:.0})\bord0\shad0\b1\fs44{ink}}}{}",
            y + height / 2.0,
            overlay::escape(label)
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn asked() -> View {
        View::Asked {
            challenger: "Mami".to_string(),
            defender: "Frederik".to_string(),
            title: "Dune".to_string(),
            answer_within_ms: 60_000,
        }
    }

    fn baking() -> View {
        View::Baking {
            challenger: "Mami".to_string(),
            defender: "Frederik".to_string(),
            starts_in_ms: 3000,
            golden_ms: [2000, 1600, 2400],
            length_ms: 9300,
            bias_ms: HashMap::from([(Input::Remote, 220.0)]),
            pressed: false,
        }
    }

    #[test]
    fn the_remote_chooses_between_baking_and_stopping_and_ok_answers() {
        let now = Instant::now();
        let mut stand = Stand::new(asked(), now);
        assert!(stand.frame(now).contains("Mami fordert dich zum Semmelstechen heraus"));
        assert_eq!(stand.key(Command::Back, now), None);
        assert_eq!(stand.key(Command::Select, now), Some(Answer::Accept));
        stand.key(Command::Right, now);
        assert_eq!(stand.key(Command::Select, now), Some(Answer::DropOff));
    }

    #[test]
    fn ok_pulls_each_semmel_out_on_the_tvs_own_clock_and_the_presses_go_when_the_oven_is_done() {
        let start = Instant::now();
        let mut stand = Stand::new(baking(), start);
        let at = |ms: u64| start + Duration::from_millis(ms);
        assert!(stand.animates());
        assert_eq!(stand.key(Command::Select, at(1000)), None);
        stand.key(Command::Select, at(3000 + 2220));
        stand.key(Command::Select, at(3000 + 2300));
        stand.key(Command::Select, at(3000 + 3100 + 1500));
        assert!(stand.frame(at(3000 + 3100 + 1600)).contains("+100"));
        stand.update(baking(), at(9000));
        assert_eq!(stand.tick(at(3000 + 9299)), None);
        assert_eq!(
            stand.tick(at(3000 + 9300)),
            Some(Answer::Presses {
                presses: [Some(220.0), Some(-100.0), None]
            })
        );
        assert_eq!(stand.tick(at(3000 + 9400)), None);
        assert!(!stand.animates());
    }

    #[test]
    fn the_result_says_who_keeps_the_oven_and_ok_dismisses_it() {
        let now = Instant::now();
        let mut stand = Stand::new(
            View::Done {
                challenger: "Mami".to_string(),
                defender: "Frederik".to_string(),
                winner: Some(Side::Challenger),
                ending: Ending::Baked,
                baked: [None, None],
            },
            now,
        );
        let frame = stand.frame(now);
        assert!(
            frame.contains("Mami bakes better") && frame.contains("10 minutes"),
            "{frame}"
        );
        assert_eq!(stand.key(Command::Select, now), Some(Answer::Dismiss));
        assert_eq!(
            verdict("Mami", Some(Side::Defender), Ending::Withdrawn).1,
            "Mami left the bake-off."
        );
    }
}
