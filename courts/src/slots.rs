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
    free_for(courts, busy, after, 0)
}

/// Where a court is free for `seconds` in a row (at least its shortest
/// booking), starting from `after` on, at every step it can be booked at.
pub fn free_for(courts: &[Court], busy: &[Busy], after: i64, seconds: i64) -> Vec<Slot> {
    let mut slots = Vec::new();
    for court in courts {
        if !court.cells.is_empty() {
            slots.extend(in_cells(court, busy, after, seconds));
            continue;
        }
        let shortest = court.durations.iter().copied().filter(|m| *m > 0).min().unwrap_or(60) * 60;
        let length = shortest.max(seconds);
        let step = court.interval.max(15) * 60;
        let mut start = court.opening;
        while start + length <= court.closing {
            let end = start + length;
            if start >= after && !taken(court, busy, start, end) {
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

fn taken(court: &Court, busy: &[Busy], start: i64, end: i64) -> bool {
    busy.iter()
        .any(|busy| busy.court.is_none_or(|id| id == court.id) && busy.start < end && busy.end > start)
}

/// Free slots of a court laid out in cells: each starts where a cell does
/// and lasts `seconds` (at least that cell), over cells that follow each
/// other without a gap and that nobody has.
fn in_cells(court: &Court, busy: &[Busy], after: i64, seconds: i64) -> Vec<Slot> {
    let mut slots = Vec::new();
    for (i, cell) in court.cells.iter().enumerate() {
        if cell.start < after {
            continue;
        }
        let end = cell.start + (cell.end - cell.start).max(seconds);
        let mut reach = cell.start;
        for next in &court.cells[i..] {
            if next.start != reach || reach >= end {
                break;
            }
            reach = next.end;
        }
        if reach >= end && !taken(court, busy, cell.start, end) {
            slots.push(Slot {
                court: court.id,
                start: cell.start,
                end,
            });
        }
    }
    slots
}

/// Where booking `slot` ends: on a court laid out in cells, at the end of
/// the last cell it reaches into.
pub fn booked_end(court: &Court, slot: &Slot) -> i64 {
    court
        .cells
        .iter()
        .filter(|cell| cell.start < slot.end && cell.end > slot.start)
        .map(|cell| cell.end)
        .max()
        .unwrap_or(slot.end)
}

/// What booking `slot` costs, in Rappen: every cell it reaches into, when
/// the source tells the prices.
pub fn price(court: &Court, slot: &Slot) -> Option<i64> {
    let cells: Vec<_> = court
        .cells
        .iter()
        .filter(|cell| cell.start < slot.end && cell.end > slot.start)
        .collect();
    if cells.is_empty() {
        return None;
    }
    cells.iter().map(|cell| cell.price).sum()
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
            indoor: Some(false),
            cover: "outdoor".into(),
            ball_machine: false,
            order: id,
            interval,
            durations: vec![60],
            opening: 8 * 3600,
            closing: 12 * 3600,
            cells: Vec::new(),
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
    fn two_hours_need_two_free_hours_in_a_row() {
        let slots = free_for(&[court(1, 60)], &[busy(Some(1), 10.0, 11.0)], 0, 2 * 3600);
        assert_eq!(starts(&slots, 1), ["08:00"]);
        assert_eq!(slots[0].end, 10 * 3600);
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
