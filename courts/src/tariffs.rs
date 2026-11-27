//! What courts cost, without asking GotCourts about every slot. GotCourts
//! answers the price of one booking at a time, and its prices change by
//! court, weekday and time of day. So once a day for each club this asks
//! about every court at a few times, takes courts that answer alike as one
//! tariff, and asks one court of each tariff about every hour of every
//! weekday. A slot's price then follows from the hours it takes. Eversports
//! tells its prices with the courts, so those need no asking.

use crate::db::{self, Db};
use crate::gotcourts::{Court, Error, GotCourts};
use crate::slots::{self, Slot};
use crate::source::Source;
use crate::time;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

/// A court's price for an hour, in Rappen, by weekday (Monday first) and
/// hour of the day; None where nobody knows (closed, or no price listed).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Week(pub Vec<Vec<Option<i64>>>);

impl Week {
    fn empty() -> Self {
        Self(vec![vec![None; 24]; 7])
    }

    fn rate(&self, weekday: usize, hour: i64) -> Option<i64> {
        *self.0.get(weekday)?.get(usize::try_from(hour).ok()?)?
    }

    /// What a booking from `start` to `end` (seconds after midnight) costs
    /// on `weekday`, in Rappen: each hour it reaches into for as long as it
    /// does. None when any of those hours has no known price.
    pub fn cost(&self, weekday: usize, start: i64, end: i64) -> Option<i64> {
        if end <= start {
            return None;
        }
        let mut sum = 0;
        let mut at = start;
        while at < end {
            let hour = at / 3600;
            let until = ((hour + 1) * 3600).min(end);
            sum += self.rate(weekday, hour)? * (until - at);
            at = until;
        }
        Some((sum + 1800) / 3600)
    }

    /// Whether any hour has a price.
    pub fn known(&self) -> bool {
        self.0.iter().flatten().any(Option::is_some)
    }
}

/// Monday 0 to Sunday 6.
pub fn weekday(date: &str) -> Option<usize> {
    time::parse(date).map(|day| usize::try_from(day.weekday().to_monday_zero_offset()).unwrap_or(0))
}

/// How long a booking of `court` to ask the price of is, in minutes: an
/// hour when it can be booked for one, else its shortest booking.
fn minutes(court: &Court) -> i64 {
    if court.durations.contains(&60) || court.durations.is_empty() {
        60
    } else {
        court.durations.iter().copied().filter(|m| *m > 0).min().unwrap_or(60)
    }
}

/// Where to ask about each hour of the day a court is open: at the hour,
/// or at opening when it opens within the hour.
fn starts(court: &Court) -> Vec<(i64, i64)> {
    let length = minutes(court) * 60;
    (0..24)
        .filter_map(|hour| {
            let at = (hour * 3600).max(court.opening);
            (at < (hour + 1) * 3600 && at + length <= court.closing).then_some((hour, at))
        })
        .collect()
}

/// The start nearest `hour` on a court that day, if it is open.
fn near(court: &Court, hour: i64) -> Option<i64> {
    starts(court)
        .into_iter()
        .min_by_key(|(h, _)| (h - hour).abs())
        .map(|(_, at)| at)
}

/// One day of the coming week to ask about, and the courts as they are then.
struct Probe {
    pub weekday: usize,
    pub date: String,
    pub courts: Vec<Court>,
}

/// Asks the price of booking a court on a date from a start for some minutes.
trait Ask {
    async fn ask(&mut self, court: i64, date: &str, start: i64, minutes: i64) -> Result<Option<i64>, Error>;
}

/// Gives up on a club after this many failed answers in a row.
const FAILURES: usize = 3;

struct Asker<'a, A> {
    asker: &'a mut A,
    failed: usize,
}

impl<A: Ask> Asker<'_, A> {
    /// The price for an hour, or None; Err once GotCourts keeps failing.
    async fn hourly(&mut self, court: &Court, date: &str, start: i64) -> Result<Option<i64>, Error> {
        let minutes = minutes(court);
        match self.asker.ask(court.id, date, start, minutes).await {
            Ok(price) => {
                self.failed = 0;
                Ok(price.map(|rappen| (rappen * 60 + minutes / 2) / minutes))
            }
            Err(error) => {
                self.failed += 1;
                if self.failed >= FAILURES { Err(error) } else { Ok(None) }
            }
        }
    }
}

/// What a court answered at a few times, to tell its tariff by.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Print {
    sport: String,
    minutes: i64,
    answers: Vec<(usize, i64, Option<i64>)>,
}

/// Every court's week at a club, from the days of the coming week. Asks
/// each court twice to tell its tariff, then for one court of each tariff
/// every hour of the first weekday and the first weekend day; any other
/// day that answers alike at two hours is taken to be the same.
async fn survey(days: &[Probe], asker: &mut impl Ask) -> Result<BTreeMap<i64, Week>, Error> {
    let mut asker = Asker { asker, failed: 0 };
    let weekday = days.iter().find(|day| day.weekday < 5);
    let weekend = days.iter().find(|day| day.weekday >= 5);
    let mut ids: Vec<i64> = days.iter().flat_map(|day| day.courts.iter().map(|c| c.id)).collect();
    ids.sort();
    ids.dedup();
    // Courts of a sport that answer alike at the same times share a tariff.
    let mut tariffs: BTreeMap<Print, Vec<i64>> = BTreeMap::new();
    for id in &ids {
        let Some(any) = days.iter().flat_map(|day| &day.courts).find(|c| c.id == *id) else {
            continue;
        };
        let mut print = Print {
            sport: any.sport.clone(),
            minutes: minutes(any),
            answers: Vec::new(),
        };
        for (day, hour) in [(weekday, 19), (weekend, 11)] {
            let Some(day) = day else { continue };
            let Some(court) = day.courts.iter().find(|c| c.id == *id) else {
                continue;
            };
            if let Some(start) = near(court, hour) {
                let price = asker.hourly(court, &day.date, start).await?;
                print.answers.push((day.weekday, start, price));
            }
        }
        tariffs.entry(print).or_default().push(*id);
    }
    // The first weekday and weekend day first, as the others are told by them.
    let first_days: Vec<&Probe> = weekday.into_iter().chain(weekend).collect();
    let order: Vec<&Probe> = first_days
        .iter()
        .copied()
        .chain(
            days.iter()
                .filter(|day| !first_days.iter().any(|o| o.weekday == day.weekday)),
        )
        .collect();
    let mut weeks = BTreeMap::new();
    for (print, courts) in tariffs {
        let first = courts[0];
        let mut week = Week::empty();
        if print.answers.iter().any(|(_, _, price)| price.is_some()) {
            for day in &order {
                let Some(court) = day.courts.iter().find(|c| c.id == first) else {
                    continue;
                };
                let like = [weekday, weekend][usize::from(day.weekday >= 5)]
                    .filter(|like| like.weekday != day.weekday)
                    .map(|like| week.0[like.weekday].clone());
                let open = starts(court);
                week.0[day.weekday] = match like {
                    Some(like) if same(&mut asker, court, day, &like).await? => (0..24)
                        .map(|hour| like[hour as usize].filter(|_| open.iter().any(|(h, _)| *h == hour)))
                        .collect(),
                    _ => {
                        let mut row = vec![None; 24];
                        for (hour, start) in open {
                            row[hour as usize] = asker.hourly(court, &day.date, start).await?;
                        }
                        row
                    }
                };
            }
        }
        for id in courts {
            weeks.insert(id, week.clone());
        }
    }
    Ok(weeks)
}

/// Whether `court` costs on `day` what `like` says at a morning and an evening hour.
async fn same<A: Ask>(
    asker: &mut Asker<'_, A>,
    court: &Court,
    day: &Probe,
    like: &[Option<i64>],
) -> Result<bool, Error> {
    let open = starts(court);
    for wanted in [10, 19] {
        let Some(&(hour, start)) = open.iter().min_by_key(|(h, _)| (h - wanted).abs()) else {
            return Ok(false);
        };
        if asker.hourly(court, &day.date, start).await? != like[hour as usize] {
            return Ok(false);
        }
    }
    Ok(true)
}

/// The prices every page uses: GotCourts' from the stored weeks, and
/// Eversports' from its cells.
#[derive(Default)]
pub struct Rates {
    weeks: HashMap<i64, Week>,
}

impl Rates {
    pub fn new(weeks: HashMap<i64, Week>) -> Self {
        Self { weeks }
    }

    pub async fn load(db: &Db) -> Self {
        Self::new(db.tariffs().await.unwrap_or_else(|error| {
            eprintln!("tariffs: {error}");
            HashMap::new()
        }))
    }

    /// What booking `slot` on `date` costs, in Rappen, as far as booking it reaches.
    pub fn cost(&self, source: Source, court: &Court, date: &str, slot: &Slot) -> Option<i64> {
        let booked = Slot {
            end: slots::booked_end(court, slot),
            ..*slot
        };
        match source {
            Source::Eversports => slots::price(court, &booked),
            Source::GotCourts => self
                .weeks
                .get(&court.id)?
                .cost(weekday(date)?, booked.start, booked.end),
        }
    }

    /// What an hour of `slot` costs, in Rappen.
    pub fn hourly(&self, source: Source, court: &Court, date: &str, slot: &Slot) -> Option<i64> {
        let end = slots::booked_end(court, slot);
        let cost = self.cost(source, court, date, slot)?;
        (end > slot.start).then(|| (cost * 3600 + (end - slot.start) / 2) / (end - slot.start))
    }
}

/// Asks GotCourts, one question every `gap`.
struct Live<'a> {
    gotcourts: &'a GotCourts,
    gap: Duration,
    asked: usize,
}

impl Ask for Live<'_> {
    async fn ask(&mut self, court: i64, date: &str, start: i64, minutes: i64) -> Result<Option<i64>, Error> {
        tokio::time::sleep(self.gap).await;
        self.asked += 1;
        self.gotcourts.price(court, date, start, minutes).await
    }
}

/// How often each club's tariffs are asked again.
const EVERY: i64 = 24 * 60 * 60;
/// How long a club whose prices failed waits before it is asked again.
const RETRY: i64 = 6 * 60 * 60;

/// Keeps every GotCourts club's tariffs a day fresh, a question at a time.
pub struct Prober {
    pub db: Db,
    pub gotcourts: GotCourts,
    pub clubs: crate::discover::Clubs,
    pub gap: Duration,
}

impl Prober {
    pub async fn run(self) {
        let mut failed: HashMap<i64, i64> = HashMap::new();
        loop {
            let now = time::now();
            let Some(club) = self.due(now, &failed).await else {
                tokio::time::sleep(Duration::from_secs(60)).await;
                continue;
            };
            match self.club(&club).await {
                Ok((weeks, asked)) => {
                    failed.remove(&club.id);
                    let priced = weeks.values().filter(|week| week.known()).count();
                    if let Err(error) = self.db.save_tariffs(club.id, &weeks, time::now()).await {
                        eprintln!("tariffs: saving {}: {error}", club.name);
                    } else {
                        eprintln!(
                            "tariffs: {}: {priced} of {} courts priced, {asked} questions",
                            club.name,
                            weeks.len()
                        );
                    }
                }
                Err(error) => {
                    eprintln!("tariffs: {}: {error}", club.name);
                    failed.insert(club.id, now);
                }
            }
        }
    }

    /// The GotCourts club whose tariffs are oldest, if a day old or never
    /// asked; of those never asked, the one with the most free starts today
    /// and tomorrow.
    async fn due(&self, now: i64, failed: &HashMap<i64, i64>) -> Option<db::Club> {
        let clubs = self.db.clubs(&self.clubs.gotcourts()).await.ok()?;
        let checked = self.db.tariffs_checked().await.ok()?;
        let mut free: HashMap<i64, usize> = HashMap::new();
        for (n, date) in time::dates(2).iter().enumerate() {
            let after = if n == 0 { time::seconds_today() } else { 0 };
            for day in self.db.days_on(date).await.unwrap_or_default() {
                *free.entry(day.club_id).or_default() += slots::free(&day.courts.0, &day.busy.0, after).len();
            }
        }
        clubs
            .into_iter()
            .filter(|club| Source::parse(&club.source) == Source::GotCourts)
            .filter(|club| failed.get(&club.id).is_none_or(|at| now - at > RETRY))
            .map(|club| (checked.get(&club.id).copied().unwrap_or(0), club))
            .filter(|(at, _)| now - at > EVERY)
            .min_by_key(|(at, club)| {
                (
                    *at,
                    std::cmp::Reverse(free.get(&club.id).copied().unwrap_or(0)),
                    club.id,
                )
            })
            .map(|(_, club)| club)
    }

    /// Every court's week at `club`, and how many questions that took.
    async fn club(&self, club: &db::Club) -> Result<(BTreeMap<i64, Week>, usize), Error> {
        let mut days = Vec::new();
        for date in time::dates(8).into_iter().skip(1) {
            let stored = self.db.day(club.id, &date).await.ok().flatten();
            let courts = stored.map_or_else(|| club.courts.0.clone(), |day| day.courts.0);
            if let Some(weekday) = weekday(&date) {
                days.push(Probe { weekday, date, courts });
            }
        }
        let mut live = Live {
            gotcourts: &self.gotcourts,
            gap: self.gap,
            asked: 0,
        };
        let weeks = survey(&days, &mut live).await?;
        Ok((weeks, live.asked))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn court(id: i64) -> Court {
        Court {
            id,
            sport: "tennis".into(),
            label: format!("Platz {id}"),
            surface: "clay".into(),
            indoor: Some(true),
            cover: "indoor".into(),
            ball_machine: false,
            order: id,
            interval: 60,
            durations: vec![60],
            opening: 7 * 3600,
            closing: 22 * 3600,
            cells: Vec::new(),
        }
    }

    /// Lengg's prices: 49 before 8 on weekdays, 63 on weekend mornings,
    /// 75 otherwise; court 3 costs a franc more.
    struct Lengg {
        asked: usize,
        broken: bool,
        /// Friday evenings from 18 cost 85.
        friday: bool,
    }

    impl Ask for Lengg {
        async fn ask(&mut self, court: i64, date: &str, start: i64, minutes: i64) -> Result<Option<i64>, Error> {
            self.asked += 1;
            if self.broken {
                return Err(Error::Failed("down".into()));
            }
            let weekday = weekday(date).unwrap();
            let hourly = match (weekday >= 5, start / 3600) {
                (false, h) if self.friday && weekday == 4 && h >= 18 => 8500,
                (false, 7) => 4900,
                (true, h) if h < 12 => 6300,
                _ => 7500,
            } + if court == 3 { 100 } else { 0 };
            Ok(Some(hourly * minutes / 60))
        }
    }

    fn week() -> Vec<Probe> {
        // 2026-11-30 is a Monday.
        (0..7)
            .map(|n| Probe {
                weekday: n,
                date: format!(
                    "2026-{}",
                    ["11-30", "12-01", "12-02", "12-03", "12-04", "12-05", "12-06"][n]
                ),
                courts: vec![court(1), court(2), court(3)],
            })
            .collect()
    }

    #[tokio::test]
    async fn courts_that_answer_alike_are_asked_about_once() {
        let mut lengg = Lengg {
            asked: 0,
            broken: false,
            friday: false,
        };
        let weeks = survey(&week(), &mut lengg).await.unwrap();
        // Three courts at two times, then for two tariffs every hour of a
        // weekday and a weekend day, and two hours of each other day.
        assert_eq!(lengg.asked, 6 + 2 * (2 * 15 + 5 * 2));
        assert_eq!(weeks[&1].rate(3, 7), Some(4900));
        assert_eq!(weeks[&1].rate(6, 21), Some(7500));
        assert_eq!(weeks[&1], weeks[&2]);
        assert_eq!(weeks[&1].rate(0, 7), Some(4900));
        assert_eq!(weeks[&1].rate(0, 8), Some(7500));
        assert_eq!(weeks[&1].rate(5, 7), Some(6300));
        assert_eq!(weeks[&3].rate(5, 12), Some(7600));
        assert_eq!(weeks[&1].rate(0, 22), None);
        assert_eq!(weeks[&1].rate(0, 6), None);
    }

    #[tokio::test]
    async fn a_day_that_answers_otherwise_is_asked_about_in_full() {
        let mut lengg = Lengg {
            asked: 0,
            broken: false,
            friday: true,
        };
        let weeks = survey(&week(), &mut lengg).await.unwrap();
        assert_eq!(lengg.asked, 6 + 2 * (2 * 15 + 5 * 2 + 15));
        assert_eq!(weeks[&1].rate(4, 17), Some(7500));
        assert_eq!(weeks[&1].rate(4, 18), Some(8500));
        assert_eq!(weeks[&1].rate(3, 18), Some(7500));
    }

    #[tokio::test]
    async fn a_club_that_keeps_failing_is_given_up() {
        let mut broken = Lengg {
            asked: 0,
            broken: true,
            friday: false,
        };
        assert!(survey(&week(), &mut broken).await.is_err());
        assert_eq!(broken.asked, FAILURES);
    }

    #[test]
    fn a_slot_costs_each_hour_it_takes() {
        let mut week = Week::empty();
        week.0[0][7] = Some(4900);
        week.0[0][8] = Some(7500);
        assert_eq!(week.cost(0, 7 * 3600, 8 * 3600), Some(4900));
        assert_eq!(week.cost(0, 7 * 3600 + 1800, 8 * 3600 + 1800), Some(6200));
        assert_eq!(week.cost(0, 8 * 3600, 9 * 3600 + 1800), None);
        assert_eq!(week.cost(1, 7 * 3600, 8 * 3600), None);
        assert_eq!(weekday("2026-11-30"), Some(0));
        assert_eq!(weekday("2026-12-06"), Some(6));
    }

    #[test]
    fn a_court_that_books_in_45_minutes_is_asked_for_45() {
        let squash = Court {
            durations: vec![45, 90],
            opening: 7 * 3600 + 1800,
            closing: 9 * 3600,
            ..court(9)
        };
        assert_eq!(minutes(&squash), 45);
        assert_eq!(starts(&squash), [(7, 7 * 3600 + 1800), (8, 8 * 3600)]);
    }
}
