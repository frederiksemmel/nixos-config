//! Challenges for a place to watch: when every place is taken, someone can
//! challenge one of the people watching to a bake-off, and the winner keeps
//! or takes the place. The rules and the edge cases are in docs/streaming.md.
//! Times come in as arguments, so the whole arena is a plain state machine.

pub mod bake;

use crate::profile::ProfileId;
use bake::{Baked, Calibration, Input, Oven, SEMMELS};
use serde::Serialize;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long the person challenged has to accept before they lose by default.
pub const ANSWER_WITHIN: Duration = Duration::from_secs(60);
/// The countdown on both screens before the first Semmel goes in.
pub const COUNTDOWN: Duration = Duration::from_secs(3);
/// How long after the oven's end a player's presses may still arrive.
pub const GRACE: Duration = Duration::from_secs(5);
/// How long a loser waits before they may challenge again.
pub const RESTING: Duration = Duration::from_secs(10 * 60);
/// How long the place a challenger won waits for them to start watching.
pub const KEPT_FOR: Duration = Duration::from_secs(90);
/// How long a finished bake-off stays readable for both players.
const REMEMBERED: Duration = Duration::from_secs(120);

/// A place someone watches from.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "lowercase")]
pub enum Seat {
    Stream(String),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Watcher {
    pub seat: Seat,
    pub profile: ProfileId,
    pub person: String,
    pub device: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Challenger {
    pub profile: ProfileId,
    pub person: String,
    pub device: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Challenger,
    Defender,
}

impl Side {
    fn index(self) -> usize {
        match self {
            Self::Challenger => 0,
            Self::Defender => 1,
        }
    }
}

/// How a bake-off ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Ending {
    Baked,
    /// The person challenged stopped watching instead of baking.
    DroppedOff,
    /// The person challenged did not answer in time.
    NoAnswer,
    /// The challenger went away before the bake.
    Withdrawn,
    /// The person challenged stopped watching some other way.
    SeatGone,
}

#[derive(Clone, Debug)]
enum Stage {
    Asked {
        since: Instant,
    },
    Baking {
        starts: Instant,
        oven: Oven,
        seed: u64,
    },
    Done {
        winner: Option<Side>,
        ending: Ending,
        at: Instant,
    },
}

#[derive(Clone, Debug)]
struct Entry {
    challenger: Challenger,
    defender: Watcher,
    stage: Stage,
    presses: [Option<(Input, [Option<f64>; SEMMELS])>; 2],
    baked: [Option<Baked>; 2],
}

/// What the arena asks the streams to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// The defender lost: their stream ends.
    Stop(Seat),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    Resting(Duration),
    Busy(String),
    AlreadyChallenging,
    NoSuchSeat,
    NoSuchChallenge,
    NotNow,
}

impl Refusal {
    pub fn message(&self) -> String {
        match self {
            Self::Resting(left) => format!(
                "Your dough is still resting: you can challenge again in {}",
                minutes(*left)
            ),
            Self::Busy(person) => format!("{person} is already in a bake-off"),
            Self::AlreadyChallenging => "You are already in a bake-off".to_string(),
            Self::NoSuchSeat => "They stopped watching; try opening the video again".to_string(),
            Self::NoSuchChallenge => "That bake-off is over".to_string(),
            Self::NotNow => "That doesn't fit this moment of the bake-off".to_string(),
        }
    }
}

pub fn minutes(left: Duration) -> String {
    let minutes = left.as_secs().div_ceil(60).max(1);
    if minutes == 1 {
        "1 minute".to_string()
    } else {
        format!("{minutes} minutes")
    }
}

/// What a player's screen shows of a challenge.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "stage", rename_all = "lowercase")]
pub enum View {
    Asked {
        challenger: String,
        defender: String,
        title: String,
        answer_within_ms: u64,
    },
    Baking {
        challenger: String,
        defender: String,
        starts_in_ms: i64,
        golden_ms: [u32; SEMMELS],
        length_ms: u32,
        /// How late each input presses, which the oven takes off.
        bias_ms: HashMap<Input, f64>,
        pressed: bool,
    },
    Done {
        challenger: String,
        defender: String,
        winner: Option<Side>,
        ending: Ending,
        baked: [Option<Baked>; 2],
    },
}

#[derive(Default)]
pub struct Arena {
    challenges: HashMap<String, Entry>,
    resting: HashMap<ProfileId, Instant>,
    kept: HashMap<ProfileId, Instant>,
    pub calibration: Calibration,
}

impl Arena {
    /// How long `profile` must still wait before challenging again.
    pub fn resting(&self, profile: ProfileId, now: Instant) -> Option<Duration> {
        self.resting
            .get(&profile)
            .map(|until| until.saturating_duration_since(now))
            .filter(|left| !left.is_zero())
    }

    /// Whether `profile` won a place it hasn't started watching from yet.
    pub fn keeps_a_place_for(&self, profile: ProfileId, now: Instant) -> bool {
        self.kept.get(&profile).is_some_and(|until| *until > now)
    }

    /// Places won but not taken yet, which count as taken.
    pub fn places_kept(&self, now: Instant) -> usize {
        self.kept.values().filter(|until| **until > now).count()
    }

    /// The challenger started watching from the place they won.
    pub fn took_place(&mut self, profile: ProfileId) {
        self.kept.remove(&profile);
    }

    /// Whether `seat` is in a bake-off that hasn't ended.
    pub fn busy(&self, seat: &Seat) -> bool {
        self.challenges
            .values()
            .any(|entry| &entry.defender.seat == seat && !matches!(entry.stage, Stage::Done { .. }))
    }

    pub fn challenge(&mut self, id: String, by: Challenger, defender: Watcher, now: Instant) -> Result<(), Refusal> {
        if let Some(left) = self.resting(by.profile, now) {
            return Err(Refusal::Resting(left));
        }
        let open = |entry: &Entry| !matches!(entry.stage, Stage::Done { .. });
        if self
            .challenges
            .values()
            .any(|entry| open(entry) && entry.challenger.profile == by.profile)
        {
            return Err(Refusal::AlreadyChallenging);
        }
        if self.busy(&defender.seat) {
            return Err(Refusal::Busy(defender.person));
        }
        self.challenges.insert(
            id,
            Entry {
                challenger: by,
                defender,
                stage: Stage::Asked { since: now },
                presses: [None, None],
                baked: [None, None],
            },
        );
        Ok(())
    }

    /// The challenge waiting for `seat`, if any.
    pub fn for_seat(&self, seat: &Seat) -> Option<&str> {
        self.challenges
            .iter()
            .filter(|(_, entry)| &entry.defender.seat == seat)
            .max_by_key(|(_, entry)| match entry.stage {
                Stage::Done { at, .. } => at,
                Stage::Asked { since } => since,
                Stage::Baking { starts, .. } => starts,
            })
            .map(|(id, _)| id.as_str())
    }

    pub fn challenger_of(&self, id: &str) -> Option<ProfileId> {
        self.challenges.get(id).map(|entry| entry.challenger.profile)
    }

    pub fn accept(&mut self, id: &str, seed: u64, now: Instant) -> Result<(), Refusal> {
        let entry = self.challenges.get_mut(id).ok_or(Refusal::NoSuchChallenge)?;
        match entry.stage {
            Stage::Asked { .. } => {
                entry.stage = Stage::Baking {
                    starts: now + COUNTDOWN,
                    oven: Oven::from_seed(seed),
                    seed,
                };
                Ok(())
            }
            Stage::Baking { .. } => Ok(()),
            Stage::Done { .. } => Err(Refusal::NotNow),
        }
    }

    /// The person challenged stops watching rather than bake.
    pub fn drop_off(&mut self, id: &str, now: Instant) -> Result<Vec<Effect>, Refusal> {
        let entry = self.challenges.get(id).ok_or(Refusal::NoSuchChallenge)?;
        if !matches!(entry.stage, Stage::Asked { .. }) {
            return Err(Refusal::NotNow);
        }
        Ok(self.end(id, Some(Side::Challenger), Ending::DroppedOff, now))
    }

    /// The challenger goes away. Before the bake nothing happens to anyone;
    /// during it, it counts as losing.
    pub fn withdraw(&mut self, id: &str, now: Instant) -> Vec<Effect> {
        match self.challenges.get(id).map(|entry| &entry.stage) {
            Some(Stage::Asked { .. }) => self.end(id, None, Ending::Withdrawn, now),
            Some(Stage::Baking { .. }) => self.end(id, Some(Side::Defender), Ending::Withdrawn, now),
            _ => Vec::new(),
        }
    }

    /// The person challenged stopped watching on their own.
    pub fn seat_gone(&mut self, seat: &Seat, now: Instant) -> Vec<Effect> {
        let open: Vec<String> = self
            .challenges
            .iter()
            .filter(|(_, entry)| &entry.defender.seat == seat && !matches!(entry.stage, Stage::Done { .. }))
            .map(|(id, _)| id.clone())
            .collect();
        open.iter()
            .flat_map(|id| self.end(id, Some(Side::Challenger), Ending::SeatGone, now))
            .collect()
    }

    pub fn press(
        &mut self,
        id: &str,
        side: Side,
        input: Input,
        presses: [Option<f64>; SEMMELS],
        now: Instant,
    ) -> Result<Vec<Effect>, Refusal> {
        let entry = self.challenges.get_mut(id).ok_or(Refusal::NoSuchChallenge)?;
        if !matches!(entry.stage, Stage::Baking { .. }) {
            return Err(Refusal::NotNow);
        }
        if entry.presses[side.index()].is_none() {
            entry.presses[side.index()] = Some((input, presses));
        }
        if entry.presses.iter().all(Option::is_some) {
            return Ok(self.finish_bake(id, now));
        }
        Ok(Vec::new())
    }

    /// Ends what has run out of time, and forgets what ended long ago.
    pub fn settle(&mut self, now: Instant) -> Vec<Effect> {
        let mut effects = Vec::new();
        let due: Vec<(String, bool)> = self
            .challenges
            .iter()
            .filter_map(|(id, entry)| match &entry.stage {
                Stage::Asked { since } if now >= *since + ANSWER_WITHIN => Some((id.clone(), false)),
                Stage::Baking { starts, oven, .. }
                    if now >= *starts + Duration::from_millis(u64::from(oven.length_ms())) + GRACE =>
                {
                    Some((id.clone(), true))
                }
                _ => None,
            })
            .collect();
        for (id, baking) in due {
            if baking {
                effects.extend(self.finish_bake(&id, now));
            } else {
                effects.extend(self.end(&id, Some(Side::Challenger), Ending::NoAnswer, now));
            }
        }
        self.challenges
            .retain(|_, entry| !matches!(entry.stage, Stage::Done { at, .. } if now >= at + REMEMBERED));
        self.resting.retain(|_, until| *until > now);
        self.kept.retain(|_, until| *until > now);
        effects
    }

    fn finish_bake(&mut self, id: &str, now: Instant) -> Vec<Effect> {
        let Some(entry) = self.challenges.get_mut(id) else {
            return Vec::new();
        };
        let Stage::Baking { seed, .. } = entry.stage else {
            return Vec::new();
        };
        for side in [Side::Challenger, Side::Defender] {
            let (input, presses) = entry.presses[side.index()].unwrap_or((Input::Touch, [None; SEMMELS]));
            let rise = bake::rises(seed.wrapping_add(side.index() as u64 + 1));
            let bias = self.calibration.bias(input);
            entry.baked[side.index()] = Some(bake::bake(&presses, input, bias, rise));
            if entry.presses[side.index()].is_some() {
                self.calibration.learn(input, &presses);
            }
        }
        let total = |side: Side| entry.baked[side.index()].as_ref().map_or(0.0, |baked| baked.total);
        let winner = if total(Side::Challenger) > total(Side::Defender) {
            Side::Challenger
        } else {
            Side::Defender
        };
        self.end(id, Some(winner), Ending::Baked, now)
    }

    fn end(&mut self, id: &str, winner: Option<Side>, ending: Ending, now: Instant) -> Vec<Effect> {
        let Some(entry) = self.challenges.get_mut(id) else {
            return Vec::new();
        };
        entry.stage = Stage::Done {
            winner,
            ending,
            at: now,
        };
        let mut effects = Vec::new();
        match winner {
            Some(Side::Challenger) => {
                self.kept.insert(entry.challenger.profile, now + KEPT_FOR);
                if ending != Ending::SeatGone {
                    self.resting.insert(entry.defender.profile, now + RESTING);
                    effects.push(Effect::Stop(entry.defender.seat.clone()));
                }
            }
            Some(Side::Defender) => {
                self.resting.insert(entry.challenger.profile, now + RESTING);
            }
            None => {}
        }
        effects
    }

    pub fn view(&self, id: &str, side: Side, now: Instant) -> Option<View> {
        let entry = self.challenges.get(id)?;
        let challenger = entry.challenger.person.clone();
        let defender = entry.defender.person.clone();
        Some(match &entry.stage {
            Stage::Asked { since } => View::Asked {
                challenger,
                defender,
                title: entry.defender.title.clone(),
                answer_within_ms: (*since + ANSWER_WITHIN).saturating_duration_since(now).as_millis() as u64,
            },
            Stage::Baking { starts, oven, .. } => View::Baking {
                challenger,
                defender,
                starts_in_ms: signed_ms(*starts, now),
                golden_ms: oven.golden_ms,
                length_ms: oven.length_ms(),
                bias_ms: [Input::Touch, Input::Mouse, Input::Keys, Input::Remote]
                    .into_iter()
                    .map(|input| (input, (self.calibration.bias(input) * 1000.0).round()))
                    .collect(),
                pressed: entry.presses[side.index()].is_some(),
            },
            Stage::Done { winner, ending, .. } => View::Done {
                challenger,
                defender,
                winner: *winner,
                ending: *ending,
                baked: entry.baked.clone(),
            },
        })
    }
}

fn signed_ms(at: Instant, now: Instant) -> i64 {
    if at >= now {
        at.duration_since(now).as_millis() as i64
    } else {
        -(now.duration_since(at).as_millis() as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watcher(stream: &str, profile: i64, person: &str) -> Watcher {
        Watcher {
            seat: Seat::Stream(stream.to_string()),
            profile: ProfileId(profile),
            person: person.to_string(),
            device: "iPhone".to_string(),
            title: "Dune".to_string(),
        }
    }

    fn challenger(profile: i64, person: &str) -> Challenger {
        Challenger {
            profile: ProfileId(profile),
            person: person.to_string(),
            device: "iPad".to_string(),
        }
    }

    fn baked(arena: &mut Arena, id: &str, challenger: f64, defender: f64, now: Instant) -> Vec<Effect> {
        arena.accept(id, 9, now).unwrap();
        arena
            .press(id, Side::Challenger, Input::Touch, [Some(0.05 + challenger); 3], now)
            .unwrap();
        arena
            .press(id, Side::Defender, Input::Touch, [Some(0.05 + defender); 3], now)
            .unwrap()
    }

    fn winner(arena: &Arena, id: &str, now: Instant) -> (Option<Side>, Ending) {
        match arena.view(id, Side::Challenger, now) {
            Some(View::Done { winner, ending, .. }) => (winner, ending),
            other => panic!("not done: {other:?}"),
        }
    }

    #[test]
    fn a_challenger_who_bakes_better_takes_the_place_and_the_loser_rests() {
        let now = Instant::now();
        let mut arena = Arena::default();
        arena
            .challenge("c".into(), challenger(2, "Mami"), watcher("s1", 3, "Ramu"), now)
            .unwrap();
        assert!(arena.busy(&Seat::Stream("s1".into())));
        let effects = baked(&mut arena, "c", 0.0, 0.24, now);
        assert_eq!(winner(&arena, "c", now), (Some(Side::Challenger), Ending::Baked));
        assert_eq!(effects, vec![Effect::Stop(Seat::Stream("s1".into()))]);
        assert!(arena.keeps_a_place_for(ProfileId(2), now));
        assert_eq!(arena.places_kept(now), 1);
        assert!(arena.resting(ProfileId(3), now + Duration::from_secs(599)).is_some());
        assert!(arena.resting(ProfileId(3), now + Duration::from_secs(600)).is_none());
        assert!(arena.resting(ProfileId(2), now).is_none());
    }

    #[test]
    fn a_defender_who_wins_keeps_watching_and_the_challenger_rests_ten_minutes() {
        let now = Instant::now();
        let mut arena = Arena::default();
        arena
            .challenge("c".into(), challenger(2, "Mami"), watcher("s1", 3, "Ramu"), now)
            .unwrap();
        let effects = baked(&mut arena, "c", 0.3, 0.0, now);
        assert_eq!(effects, vec![]);
        assert_eq!(winner(&arena, "c", now).0, Some(Side::Defender));
        assert_eq!(
            arena.challenge("d".into(), challenger(2, "Mami"), watcher("s2", 4, "Chris"), now),
            Err(Refusal::Resting(RESTING))
        );
        assert_eq!(
            Refusal::Resting(Duration::from_secs(61)).message(),
            "Your dough is still resting: you can challenge again in 2 minutes"
        );
        assert!(!arena.keeps_a_place_for(ProfileId(2), now));
    }

    #[test]
    fn someone_who_drops_off_or_does_not_answer_loses_without_baking() {
        let now = Instant::now();
        let mut arena = Arena::default();
        arena
            .challenge("c".into(), challenger(2, "Mami"), watcher("s1", 3, "Ramu"), now)
            .unwrap();
        assert_eq!(
            arena.drop_off("c", now).unwrap(),
            vec![Effect::Stop(Seat::Stream("s1".into()))]
        );
        assert_eq!(winner(&arena, "c", now), (Some(Side::Challenger), Ending::DroppedOff));
        arena
            .challenge("d".into(), challenger(5, "Chris"), watcher("s2", 4, "Paul"), now)
            .unwrap();
        assert_eq!(arena.settle(now + ANSWER_WITHIN - Duration::from_millis(1)), vec![]);
        assert_eq!(
            arena.settle(now + ANSWER_WITHIN),
            vec![Effect::Stop(Seat::Stream("s2".into()))]
        );
        assert_eq!(winner(&arena, "d", now), (Some(Side::Challenger), Ending::NoAnswer));
        assert!(arena.resting(ProfileId(4), now + ANSWER_WITHIN).is_some());
    }

    #[test]
    fn a_challenger_who_leaves_before_the_bake_changes_nothing_but_during_it_loses() {
        let now = Instant::now();
        let mut arena = Arena::default();
        arena
            .challenge("c".into(), challenger(2, "Mami"), watcher("s1", 3, "Ramu"), now)
            .unwrap();
        assert_eq!(arena.withdraw("c", now), vec![]);
        assert_eq!(winner(&arena, "c", now), (None, Ending::Withdrawn));
        assert!(arena.resting(ProfileId(2), now).is_none());
        assert!(!arena.busy(&Seat::Stream("s1".into())));
        arena
            .challenge("d".into(), challenger(2, "Mami"), watcher("s1", 3, "Ramu"), now)
            .unwrap();
        arena.accept("d", 1, now).unwrap();
        assert_eq!(arena.withdraw("d", now), vec![]);
        assert_eq!(winner(&arena, "d", now), (Some(Side::Defender), Ending::Withdrawn));
        assert!(arena.resting(ProfileId(2), now).is_some());
    }

    #[test]
    fn nobody_can_be_challenged_twice_at_once_nor_challenge_twice() {
        let now = Instant::now();
        let mut arena = Arena::default();
        arena
            .challenge("c".into(), challenger(2, "Mami"), watcher("s1", 3, "Ramu"), now)
            .unwrap();
        assert_eq!(
            arena.challenge("d".into(), challenger(5, "Chris"), watcher("s1", 3, "Ramu"), now),
            Err(Refusal::Busy("Ramu".into()))
        );
        assert_eq!(
            arena.challenge("e".into(), challenger(2, "Mami"), watcher("s2", 4, "Paul"), now),
            Err(Refusal::AlreadyChallenging)
        );
    }

    #[test]
    fn a_bake_off_with_presses_missing_scores_them_as_burnt_once_the_oven_is_done() {
        let now = Instant::now();
        let mut arena = Arena::default();
        arena
            .challenge("c".into(), challenger(2, "Mami"), watcher("s1", 3, "Ramu"), now)
            .unwrap();
        arena.accept("c", 4, now).unwrap();
        let oven = Oven::from_seed(4);
        let over = now + COUNTDOWN + Duration::from_millis(u64::from(oven.length_ms())) + GRACE;
        arena
            .press("c", Side::Challenger, Input::Touch, [Some(0.05); 3], now + COUNTDOWN)
            .unwrap();
        assert_eq!(arena.settle(over - Duration::from_millis(1)), vec![]);
        assert_eq!(arena.settle(over), vec![Effect::Stop(Seat::Stream("s1".into()))]);
        let Some(View::Done { baked, .. }) = arena.view("c", Side::Defender, over) else {
            panic!("not done");
        };
        assert_eq!(baked[1].as_ref().unwrap().crust, [0.0; 3]);
    }

    #[test]
    fn a_defender_who_stops_watching_on_their_own_hands_over_the_place_without_resting() {
        let now = Instant::now();
        let mut arena = Arena::default();
        arena
            .challenge("c".into(), challenger(2, "Mami"), watcher("s1", 3, "Ramu"), now)
            .unwrap();
        assert_eq!(arena.seat_gone(&Seat::Stream("s1".into()), now), vec![]);
        assert_eq!(winner(&arena, "c", now), (Some(Side::Challenger), Ending::SeatGone));
        assert!(arena.keeps_a_place_for(ProfileId(2), now));
        assert!(arena.resting(ProfileId(3), now).is_none());
    }

    #[test]
    fn a_won_place_is_kept_for_a_while_and_then_freed() {
        let now = Instant::now();
        let mut arena = Arena::default();
        arena
            .challenge("c".into(), challenger(2, "Mami"), watcher("s1", 3, "Ramu"), now)
            .unwrap();
        arena.drop_off("c", now).unwrap();
        assert_eq!(arena.places_kept(now + KEPT_FOR - Duration::from_secs(1)), 1);
        assert_eq!(arena.places_kept(now + KEPT_FOR), 0);
        arena.took_place(ProfileId(2));
        assert_eq!(arena.places_kept(now), 0);
    }
}
