/// Who, other than the viewer, holds the TV's player paused or is stopping it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Hold {
    #[default]
    Free,
    WakingTv,
    HandedOff {
        resume: bool,
    },
    Stopping,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Nothing,
    Pause,
    Resume,
}

impl Hold {
    pub fn tv_waking(self, playing: bool) -> (Self, Act) {
        match self {
            Self::Free if playing => (Self::WakingTv, Act::Pause),
            held => (held, Act::Nothing),
        }
    }

    pub fn tv_showing(self) -> (Self, Act) {
        match self {
            Self::WakingTv => (Self::Free, Act::Resume),
            held => (held, Act::Nothing),
        }
    }

    pub fn hand_off(self, playing: bool) -> (Self, Act) {
        match self {
            Self::Free => (Self::HandedOff { resume: playing }, Act::Pause),
            Self::WakingTv => (Self::HandedOff { resume: true }, Act::Pause),
            held @ (Self::HandedOff { .. } | Self::Stopping) => (held, Act::Pause),
        }
    }

    pub fn hand_back(self) -> (Self, Act) {
        match self {
            Self::HandedOff { resume: true } => (Self::Free, Act::Resume),
            Self::HandedOff { resume: false } => (Self::Free, Act::Nothing),
            held => (held, Act::Nothing),
        }
    }

    pub fn viewer_chose(self) -> Self {
        match self {
            Self::Stopping => Self::Stopping,
            _ => Self::Free,
        }
    }

    pub fn stopping(self) -> bool {
        self == Self::Stopping
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phone_taking_over_while_the_tv_wakes_keeps_the_tv_paused_until_it_is_handed_back() {
        let (hold, act) = Hold::Free.tv_waking(true);
        assert_eq!((hold, act), (Hold::WakingTv, Act::Pause));
        let (hold, _) = hold.hand_off(false);
        assert_eq!(hold, Hold::HandedOff { resume: true });
        assert_eq!(hold.tv_showing(), (hold, Act::Nothing));
        assert_eq!(hold.hand_back(), (Hold::Free, Act::Resume));
    }

    #[test]
    fn a_film_paused_by_the_viewer_stays_paused_after_a_round_trip_to_a_phone() {
        let (hold, act) = Hold::Free.hand_off(false);
        assert_eq!(act, Act::Pause);
        let (hold, _) = hold.hand_off(false);
        assert_eq!(hold.hand_back(), (Hold::Free, Act::Nothing));
        let (hold, _) = Hold::Free.hand_off(true);
        let (hold, _) = hold.hand_off(false);
        assert_eq!(hold.hand_back(), (Hold::Free, Act::Resume));
    }

    #[test]
    fn the_viewer_pausing_or_playing_drops_every_hold_but_a_stop() {
        for held in [Hold::WakingTv, Hold::HandedOff { resume: true }] {
            let hold = held.viewer_chose();
            assert_eq!(hold, Hold::Free);
            assert_eq!(hold.hand_back().1, Act::Nothing);
            assert_eq!(hold.tv_showing().1, Act::Nothing);
        }
        assert_eq!(Hold::Stopping.viewer_chose(), Hold::Stopping);
    }

    #[test]
    fn nothing_resumes_a_player_that_is_stopping() {
        let hold = Hold::Stopping;
        assert_eq!(hold.tv_showing(), (Hold::Stopping, Act::Nothing));
        assert_eq!(hold.hand_back(), (Hold::Stopping, Act::Nothing));
        assert_eq!(hold.tv_waking(true), (Hold::Stopping, Act::Nothing));
        assert!(hold.hand_off(true).0.stopping());
    }

    #[test]
    fn a_paused_player_is_not_held_for_the_tv_and_so_is_not_resumed_by_it() {
        let (hold, act) = Hold::Free.tv_waking(false);
        assert_eq!((hold, act), (Hold::Free, Act::Nothing));
        assert_eq!(hold.tv_showing().1, Act::Nothing);
    }
}
