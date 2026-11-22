//! The bake-off: three Semmels go into the oven one after the other, and each
//! player pulls each one out when it is golden. The crust counts how close to
//! the golden moment a press came; the rise is the yeast's luck. Half of what
//! separates two players comes from each (see docs/streaming.md).

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

pub const SEMMELS: usize = 3;
/// Seconds off the golden moment at which the crust is worth nothing.
pub const WINDOW: f64 = 0.25;
pub const CRUST: f64 = 100.0;
/// The most a Semmel's rise adds, drawn evenly from 0 up to this.
pub const RISE: f64 = 80.0;
pub const SOONEST_GOLDEN_MS: u32 = 1600;
pub const LATEST_GOLDEN_MS: u32 = 2600;
/// How long after the golden moment a Semmel burns and comes out by itself.
pub const BURNS_AFTER_MS: u32 = 600;
pub const REST_MS: u32 = 500;
const REMEMBERED: usize = 60;
const LEARNS_FROM: usize = 15;
const LOWEST_BIAS: f64 = -0.1;
const HIGHEST_BIAS: f64 = 0.4;

/// A small seeded generator, so a bake-off is the same wherever it is drawn.
#[derive(Clone, Debug)]
pub struct Dough(u64);

impl Dough {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Evenly in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[cfg(test)]
impl Dough {
    pub fn normal(&mut self) -> f64 {
        let first = self.unit().max(f64::MIN_POSITIVE);
        let second = self.unit();
        (-2.0 * first.ln()).sqrt() * (std::f64::consts::TAU * second).cos()
    }
}

/// When each Semmel turns golden, the same for both players.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Oven {
    pub golden_ms: [u32; SEMMELS],
}

impl Oven {
    pub fn from_seed(seed: u64) -> Self {
        let mut dough = Dough::new(seed);
        let spread = f64::from(LATEST_GOLDEN_MS - SOONEST_GOLDEN_MS);
        let golden_ms = std::array::from_fn(|_| SOONEST_GOLDEN_MS + (dough.unit() * spread) as u32);
        Self { golden_ms }
    }

    /// How long the whole bake takes, the same however anyone presses.
    pub fn length_ms(&self) -> u32 {
        self.golden_ms
            .iter()
            .map(|golden| golden + BURNS_AFTER_MS + REST_MS)
            .sum()
    }
}

/// Each player's yeast: how much each of their Semmels rises.
pub fn rises(seed: u64) -> [f64; SEMMELS] {
    let mut dough = Dough::new(seed ^ 0x5EED_7E57);
    std::array::from_fn(|_| (dough.unit() * RISE).round())
}

/// What a player pressed with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Input {
    Touch,
    Mouse,
    Keys,
    Remote,
}

impl Input {
    /// How late this input registers a press after the player saw the
    /// moment, until enough presses taught the oven better, and how much that
    /// lateness wobbles from press to press, in seconds.
    fn measured(self) -> (f64, f64) {
        match self {
            Self::Touch => (0.05, 0.008),
            Self::Mouse | Self::Keys => (0.04, 0.008),
            Self::Remote => (0.22, 0.035),
        }
    }

    pub fn jitter(self) -> f64 {
        self.measured().1
    }
}

/// The crust points for a press `error` seconds after the golden moment,
/// once the input's own lateness is taken off.
pub fn crust(error: f64) -> f64 {
    (CRUST * (1.0 - (error / WINDOW).powi(2))).max(0.0)
}

/// Points an input's wobble costs on average: the crust falls with the
/// square of the error, so a wobble of `jitter` costs its square, and handing
/// that back makes a wobbly input as good as a steady one.
pub fn wobble(input: Input) -> f64 {
    CRUST * (input.jitter() / WINDOW).powi(2)
}

/// How late each input presses, learnt from the presses of past bake-offs.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Calibration {
    seen: HashMap<Input, VecDeque<f64>>,
}

impl Calibration {
    pub fn bias(&self, input: Input) -> f64 {
        match self.seen.get(&input) {
            Some(errors) if errors.len() >= LEARNS_FROM => {
                let mut sorted: Vec<f64> = errors.iter().copied().collect();
                sorted.sort_by(f64::total_cmp);
                sorted[sorted.len() / 2].clamp(LOWEST_BIAS, HIGHEST_BIAS)
            }
            _ => input.measured().0,
        }
    }

    pub fn learn(&mut self, input: Input, errors: &[Option<f64>]) {
        let seen = self.seen.entry(input).or_default();
        for error in errors.iter().flatten() {
            if error.abs() < 1.0 {
                seen.push_back(*error);
            }
        }
        while seen.len() > REMEMBERED {
            seen.pop_front();
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Baked {
    pub crust: [f64; SEMMELS],
    pub rise: [f64; SEMMELS],
    pub total: f64,
}

/// A player's result from their presses, each in seconds after the golden
/// moment as their own screen showed it, or None for a Semmel left to burn.
pub fn bake(presses: &[Option<f64>; SEMMELS], input: Input, bias: f64, rise: [f64; SEMMELS]) -> Baked {
    let crust = std::array::from_fn(|index| match presses[index] {
        Some(error) => (crust(error - bias) + wobble(input)).min(CRUST).round(),
        None => 0.0,
    });
    let total = crust.iter().sum::<f64>() + rise.iter().sum::<f64>();
    Baked { crust, rise, total }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_semmel_pulled_at_the_golden_moment_has_the_full_crust_and_one_pulled_late_burns() {
        assert_eq!(crust(0.0), 100.0);
        assert_eq!(crust(0.125), 75.0);
        assert_eq!(crust(-0.125), 75.0);
        assert_eq!(crust(WINDOW), 0.0);
        assert_eq!(crust(0.6), 0.0);
    }

    #[test]
    fn the_oven_is_the_same_for_both_players_and_lasts_about_ten_seconds() {
        let oven = Oven::from_seed(42);
        assert_eq!(oven, Oven::from_seed(42));
        assert_ne!(oven, Oven::from_seed(43));
        for seed in 0..1000 {
            let oven = Oven::from_seed(seed);
            assert!(
                oven.golden_ms
                    .iter()
                    .all(|ms| (SOONEST_GOLDEN_MS..LATEST_GOLDEN_MS).contains(ms))
            );
            assert!((8100..=11_100).contains(&oven.length_ms()), "{}", oven.length_ms());
        }
    }

    #[test]
    fn a_press_counts_from_the_inputs_own_lateness_and_a_burnt_semmel_has_no_crust() {
        let baked = bake(
            &[Some(0.22), Some(0.345), None],
            Input::Remote,
            0.22,
            [10.0, 20.0, 30.0],
        );
        assert_eq!(baked.crust, [100.0, 77.0, 0.0]);
        assert_eq!(baked.total, 237.0);
    }

    #[test]
    fn the_oven_learns_how_late_an_input_presses_once_it_has_seen_enough() {
        let mut calibration = Calibration::default();
        assert_eq!(calibration.bias(Input::Remote), 0.22);
        calibration.learn(Input::Remote, &[Some(0.30), Some(0.31), None]);
        assert_eq!(calibration.bias(Input::Remote), 0.22);
        for _ in 0..10 {
            calibration.learn(Input::Remote, &[Some(0.29), Some(0.30), Some(5.0)]);
        }
        assert_eq!(calibration.bias(Input::Remote), 0.30);
        assert_eq!(calibration.bias(Input::Touch), 0.05);
    }

    /// A simulated player: how far their presses scatter around the moment
    /// they aim at. People spread from about 35 ms to 150 ms, 70 ms typical.
    fn spread(dough: &mut Dough) -> f64 {
        (-2.65 + 0.45 * dough.normal()).exp()
    }

    /// Lateness and wobble the simulated inputs really have: the remote's
    /// as measured, so the oven's starting guesses are what is tested.
    fn play(dough: &mut Dough, spread: f64, input: Input, luck: bool) -> f64 {
        let (late, jitter) = input.measured();
        let presses: [Option<f64>; SEMMELS] =
            std::array::from_fn(|_| Some(spread * dough.normal() + late + jitter * dough.normal()));
        let rise = if luck { rises(dough.next()) } else { [0.0; SEMMELS] };
        bake(&presses, input, late, rise).total
    }

    fn variance(values: &[f64]) -> f64 {
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        values.iter().map(|value| (value - mean).powi(2)).sum::<f64>() / values.len() as f64
    }

    fn wins(dough: &mut Dough, first: (f64, Input), second: (f64, Input), games: usize) -> f64 {
        let won = (0..games)
            .filter(|_| play(dough, first.0, first.1, true) > play(dough, second.0, second.1, true))
            .count();
        won as f64 / games as f64
    }

    #[test]
    fn half_of_what_decides_a_bake_off_is_skill_and_half_is_the_yeast() {
        let mut dough = Dough::new(7);
        let games = 40_000;
        let mut skill = Vec::with_capacity(games);
        let mut luck = Vec::with_capacity(games);
        for _ in 0..games {
            let (first, second) = (spread(&mut dough), spread(&mut dough));
            skill.push(play(&mut dough, first, Input::Touch, false) - play(&mut dough, second, Input::Touch, false));
            let rise = |dough: &mut Dough| rises(dough.next()).iter().sum::<f64>();
            luck.push(rise(&mut dough) - rise(&mut dough));
        }
        let share = variance(&skill) / (variance(&skill) + variance(&luck));
        assert!((0.45..=0.55).contains(&share), "skill share {share:.3}");
    }

    #[test]
    fn a_better_baker_wins_more_often_but_not_always() {
        let mut dough = Dough::new(11);
        let steady = wins(&mut dough, (0.04, Input::Touch), (0.08, Input::Touch), 20_000);
        assert!((0.58..=0.70).contains(&steady), "40 ms against 80 ms wins {steady:.3}");
        let master = wins(&mut dough, (0.035, Input::Touch), (0.15, Input::Touch), 20_000);
        assert!((0.78..=0.92).contains(&master), "35 ms against 150 ms wins {master:.3}");
    }

    #[test]
    fn the_same_player_wins_as_often_with_the_remote_as_with_a_touch_screen() {
        let mut dough = Dough::new(13);
        for spread in [0.04, 0.07, 0.1] {
            let remote = wins(&mut dough, (spread, Input::Remote), (spread, Input::Touch), 40_000);
            assert!((0.48..=0.52).contains(&remote), "remote wins {remote:.3} at {spread}");
            let keys = wins(&mut dough, (spread, Input::Keys), (spread, Input::Touch), 40_000);
            assert!((0.48..=0.52).contains(&keys), "keys win {keys:.3} at {spread}");
        }
    }
}
