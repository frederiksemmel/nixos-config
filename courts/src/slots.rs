use crate::gotcourts::{Busy, Court};

/// An hour, or whatever the court's shortest booking is, that nobody has.
/// Times are seconds after midnight in Zurich.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Slot {
    pub court: i64,
    pub start: i64,
    pub end: i64,
}

/// The free slots of every court, the ones starting from `after` on.
pub fn free(courts: &[Court], busy: &[Busy], after: i64) -> Vec<Slot> {
    let mut slots = Vec::new();
    for court in courts {
        let length = court.durations.iter().copied().filter(|m| *m > 0).min().unwrap_or(60) * 60;
        let step = court.interval.max(15) * 60;
        let mut start = court.opening;
        while start + length <= court.closing {
            let end = start + length;
            let taken = busy
                .iter()
                .any(|busy| busy.court.is_none_or(|id| id == court.id) && busy.start < end && busy.end > start);
            if start >= after && !taken {
                slots.push(Slot {
                    court: court.id,
                    start,
                    end,
                });
            }
            start += step;
        }
    }
    slots
}

pub fn clock(seconds: i64) -> String {
    format!("{:02}:{:02}", seconds / 3600, seconds % 3600 / 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn court(id: i64, interval: i64) -> Court {
        Court {
            id,
            sport: "tennis".into(),
            label: format!("Platz {id}"),
            surface: "clay".into(),
            indoor: false,
            ball_machine: false,
            order: id,
            interval,
            durations: vec![60],
            opening: 8 * 3600,
            closing: 12 * 3600,
        }
    }

    fn busy(court: Option<i64>, from: f64, to: f64) -> Busy {
        Busy {
            court,
            start: (from * 3600.0) as i64,
            end: (to * 3600.0) as i64,
            note: None,
        }
    }

    fn starts(slots: &[Slot], court: i64) -> Vec<String> {
        slots
            .iter()
            .filter(|s| s.court == court)
            .map(|s| clock(s.start))
            .collect()
    }

    #[test]
    fn an_hour_is_free_where_no_booking_overlaps_it() {
        let slots = free(&[court(1, 60)], &[busy(Some(1), 9.0, 10.0)], 0);
        assert_eq!(starts(&slots, 1), ["08:00", "10:00", "11:00"]);
    }

    #[test]
    fn half_hour_steps_skip_starts_a_booking_cuts_into() {
        let slots = free(&[court(1, 30)], &[busy(Some(1), 9.5, 10.0)], 0);
        assert_eq!(starts(&slots, 1), ["08:00", "08:30", "10:00", "10:30", "11:00"]);
    }

    #[test]
    fn a_rule_for_the_whole_club_blocks_every_court_and_the_past_is_gone() {
        let slots = free(&[court(1, 60), court(2, 60)], &[busy(None, 8.0, 9.0)], 10 * 3600);
        assert_eq!(starts(&slots, 1), ["10:00", "11:00"]);
        assert_eq!(starts(&slots, 2), ["10:00", "11:00"]);
    }
}
