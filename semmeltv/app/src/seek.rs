use std::time::{Duration, Instant};

const STEPS: [i32; 6] = [10, 10, 20, 30, 60, 120];
const STREAK: Duration = Duration::from_millis(1200);
const REPEAT: Duration = Duration::from_millis(180);
const SEND_EVERY: Duration = Duration::from_millis(250);
const SETTLE: Duration = Duration::from_millis(600);
const END_MARGIN: f64 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Back,
    Forward,
}

impl Direction {
    fn signed(self, seconds: i32) -> i32 {
        match self {
            Self::Back => -seconds,
            Self::Forward => seconds,
        }
    }
}

struct Streak {
    direction: Direction,
    presses: usize,
    last: Instant,
}

#[derive(Default)]
struct Ramp {
    streak: Option<Streak>,
}

impl Ramp {
    fn press(&mut self, direction: Direction, now: Instant) -> Option<i32> {
        let presses = match &self.streak {
            Some(streak) if streak.direction == direction && now - streak.last < REPEAT => return None,
            Some(streak) if streak.direction == direction && now - streak.last < STREAK => streak.presses,
            _ => 0,
        };
        self.streak = Some(Streak {
            direction,
            presses: presses + 1,
            last: now,
        });
        Some(direction.signed(STEPS[presses.min(STEPS.len() - 1)]))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Order {
    pub to: f64,
    pub exact: bool,
}

#[derive(Clone, Copy)]
struct Pending {
    to: f64,
    pressed: Instant,
}

#[derive(Default)]
pub struct Scrub {
    ramp: Ramp,
    pending: Option<Pending>,
    sent: Option<(Order, Instant)>,
    jump: Option<(i32, Instant)>,
}

impl Scrub {
    pub fn step(&mut self, direction: Direction, position: f64, duration: f64, now: Instant) -> Option<Order> {
        let seconds = self.ramp.press(direction, now)?;
        self.go(seconds, position, duration, now)
    }

    pub fn jump(&mut self, seconds: i32, position: f64, duration: f64, now: Instant) -> Option<Order> {
        self.ramp = Ramp::default();
        self.go(seconds, position, duration, now)
    }

    pub fn poll(&mut self, now: Instant) -> Option<Order> {
        let Pending { to, pressed } = self.pending?;
        if now - pressed >= SETTLE {
            let order = self.send(Order { to, exact: true }, now);
            self.pending = None;
            self.sent = None;
            return order;
        }
        let due = self.sent.is_none_or(|(_, at)| now - at >= SEND_EVERY);
        due.then(|| self.send(Order { to, exact: false }, now)).flatten()
    }

    pub fn cancel(&mut self) {
        *self = Self::default();
    }

    pub fn target(&self) -> Option<f64> {
        self.pending.map(|pending| pending.to)
    }

    pub fn jump_shown(&self, now: Instant) -> Option<i32> {
        self.jump
            .filter(|(_, at)| now - *at < STREAK)
            .map(|(seconds, _)| seconds)
    }

    fn go(&mut self, seconds: i32, position: f64, duration: f64, now: Instant) -> Option<Order> {
        let fresh = self.pending.is_none();
        let to = land(self.target().unwrap_or(position), seconds, duration);
        self.pending = Some(Pending { to, pressed: now });
        self.jump = Some((seconds, now));
        if fresh {
            self.send(Order { to, exact: true }, now)
        } else {
            self.poll(now)
        }
    }

    fn send(&mut self, order: Order, now: Instant) -> Option<Order> {
        let covered = self
            .sent
            .is_some_and(|(sent, _)| sent.to == order.to && (sent.exact || !order.exact));
        if covered {
            return None;
        }
        self.sent = Some((order, now));
        Some(order)
    }
}

fn land(from: f64, seconds: i32, duration: f64) -> f64 {
    let to = from + f64::from(seconds);
    if seconds < 0 {
        to.max(0.0)
    } else if duration > 0.0 {
        to.min((duration - END_MARGIN).max(from))
    } else {
        to
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILM: f64 = 5400.0;

    fn after(start: Instant, millis: u64) -> Instant {
        start + Duration::from_millis(millis)
    }

    fn steps(direction: Direction, gap: u64, count: u64) -> Vec<Option<i32>> {
        let mut ramp = Ramp::default();
        let start = Instant::now();
        (0..count)
            .map(|press| ramp.press(direction, after(start, press * gap)))
            .collect()
    }

    #[test]
    fn repeated_presses_grow_and_then_hold_the_largest_step() {
        let expected = [10, 10, 20, 30, 60, 120, 120, 120].map(Some);
        assert_eq!(steps(Direction::Forward, 400, 8), expected);
    }

    #[test]
    fn going_back_grows_the_same_way() {
        let expected = [-10, -10, -20, -30].map(Some);
        assert_eq!(steps(Direction::Back, 1000, 4), expected);
    }

    #[test]
    fn the_steps_start_over_after_a_rest() {
        let mut ramp = Ramp::default();
        let start = Instant::now();
        for press in 0..4 {
            ramp.press(Direction::Forward, after(start, press * 300));
        }
        assert_eq!(ramp.press(Direction::Forward, after(start, 900 + 1199)), Some(60));
        assert_eq!(ramp.press(Direction::Forward, after(start, 2099 + 1200)), Some(10));
    }

    #[test]
    fn the_steps_start_over_when_the_direction_changes() {
        let mut ramp = Ramp::default();
        let start = Instant::now();
        for press in 0..4 {
            ramp.press(Direction::Forward, after(start, press * 300));
        }
        assert_eq!(ramp.press(Direction::Back, after(start, 1000)), Some(-10));
        assert_eq!(ramp.press(Direction::Forward, after(start, 1300)), Some(10));
    }

    #[test]
    fn a_fast_keyboard_repeat_climbs_no_quicker_than_a_tv_remote() {
        let accepted: Vec<_> = steps(Direction::Forward, 40, 26).into_iter().flatten().collect();
        assert_eq!(accepted, [10, 10, 20, 30, 60, 120]);
    }

    #[test]
    fn a_lone_press_seeks_exactly_at_once() {
        let mut scrub = Scrub::default();
        let start = Instant::now();
        let order = scrub.step(Direction::Forward, 100.0, FILM, start);
        assert_eq!(order, Some(Order { to: 110.0, exact: true }));
        assert_eq!(scrub.poll(after(start, 300)), None);
        assert_eq!(scrub.poll(after(start, 600)), None);
        assert_eq!(scrub.target(), None);
    }

    #[test]
    fn a_burst_sends_keyframe_seeks_at_a_steady_pace_and_lands_exactly() {
        let mut scrub = Scrub::default();
        let start = Instant::now();
        let mut orders = Vec::new();
        for millis in (0..=900).step_by(50) {
            let now = after(start, millis);
            orders.extend(scrub.poll(now));
            if millis % 200 == 0 {
                orders.extend(scrub.step(Direction::Forward, 100.0, FILM, now));
            }
        }
        for millis in (950..=1600).step_by(50) {
            orders.extend(scrub.poll(after(start, millis)));
        }
        let keyframe = |to| Order { to, exact: false };
        assert_eq!(
            orders,
            [
                Order { to: 110.0, exact: true },
                keyframe(120.0),
                keyframe(140.0),
                keyframe(170.0),
                keyframe(230.0),
                Order { to: 230.0, exact: true },
            ]
        );
    }

    #[test]
    fn presses_build_on_the_target_while_mpv_catches_up() {
        let mut scrub = Scrub::default();
        let start = Instant::now();
        scrub.step(Direction::Forward, 100.0, FILM, start);
        scrub.step(Direction::Forward, 100.0, FILM, after(start, 300));
        scrub.step(Direction::Forward, 100.0, FILM, after(start, 600));
        assert_eq!(scrub.target(), Some(140.0));
    }

    #[test]
    fn once_settled_a_press_starts_from_the_playing_position() {
        let mut scrub = Scrub::default();
        let start = Instant::now();
        scrub.step(Direction::Forward, 100.0, FILM, start);
        assert_eq!(scrub.poll(after(start, 600)), None);
        let order = scrub.step(Direction::Back, 110.0, FILM, after(start, 5000));
        assert_eq!(order, Some(Order { to: 100.0, exact: true }));
    }

    #[test]
    fn a_seek_returns_to_a_target_it_already_visited() {
        let mut scrub = Scrub::default();
        let start = Instant::now();
        scrub.step(Direction::Forward, 100.0, FILM, start);
        scrub.poll(after(start, 600));
        let order = scrub.step(Direction::Back, 120.0, FILM, after(start, 5000));
        assert_eq!(order, Some(Order { to: 110.0, exact: true }));
    }

    #[test]
    fn seeks_stop_at_the_start() {
        let mut scrub = Scrub::default();
        let order = scrub.step(Direction::Back, 4.0, FILM, Instant::now());
        assert_eq!(order, Some(Order { to: 0.0, exact: true }));
    }

    #[test]
    fn seeks_stop_just_before_the_end() {
        let mut scrub = Scrub::default();
        let order = scrub.jump(60, FILM - 20.0, FILM, Instant::now());
        assert_eq!(
            order,
            Some(Order {
                to: FILM - END_MARGIN,
                exact: true
            })
        );
    }

    #[test]
    fn a_forward_seek_never_moves_back_from_the_last_seconds() {
        assert_eq!(land(FILM - 2.0, 10, FILM), FILM - 2.0);
    }

    #[test]
    fn a_video_of_unknown_length_seeks_without_an_end() {
        assert_eq!(land(100.0, 10, 0.0), 110.0);
    }

    #[test]
    fn up_and_down_jump_a_fixed_minute_and_end_a_streak() {
        let mut scrub = Scrub::default();
        let start = Instant::now();
        for press in 0..4 {
            scrub.step(Direction::Forward, 100.0, FILM, after(start, press * 300));
        }
        scrub.jump(60, 100.0, FILM, after(start, 1000));
        assert_eq!(scrub.jump_shown(after(start, 1000)), Some(60));
        scrub.step(Direction::Forward, 100.0, FILM, after(start, 1300));
        assert_eq!(scrub.jump_shown(after(start, 1300)), Some(10));
    }

    #[test]
    fn the_jump_size_shows_as_long_as_a_streak_can_continue() {
        let mut scrub = Scrub::default();
        let start = Instant::now();
        scrub.step(Direction::Back, 100.0, FILM, start);
        assert_eq!(scrub.jump_shown(after(start, 1199)), Some(-10));
        assert_eq!(scrub.jump_shown(after(start, 1200)), None);
    }

    #[test]
    fn cancelling_forgets_a_seek_in_progress() {
        let mut scrub = Scrub::default();
        let start = Instant::now();
        scrub.step(Direction::Forward, 100.0, FILM, start);
        scrub.step(Direction::Forward, 100.0, FILM, after(start, 100 + 100));
        scrub.cancel();
        assert_eq!(scrub.target(), None);
        assert_eq!(scrub.poll(after(start, 1000)), None);
    }
}
