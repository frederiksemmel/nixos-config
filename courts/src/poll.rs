//! Asks GotCourts every minute: today at every club, every watched day, and
//! a slice of the coming two weeks, so every club and day comes round about
//! every quarter of an hour. Then Eversports, a week at a time: the first
//! week every minute, the second every few minutes or when watched.

use crate::db::{Db, Watch};
use crate::eversports::Venues;
use crate::gotcourts::{self, GotCourts};
use crate::push::{Notice, Push};
use crate::slots::{self, Slot};
use crate::time;
use std::collections::{BTreeSet, HashSet};
use std::time::Duration;

const CLOSED_FOR: i64 = 24 * 60 * 60;
const BETWEEN_REQUESTS: Duration = Duration::from_millis(300);

#[derive(Clone)]
pub struct Poller {
    pub db: Db,
    pub gotcourts: GotCourts,
    pub push: Push,
    pub clubs: crate::discover::Clubs,
    pub finder: std::sync::Arc<crate::discover::Finder>,
    pub venues: std::sync::Arc<Venues>,
    pub days: i64,
    pub per_minute: usize,
}

impl Poller {
    pub async fn run(self) {
        let mut turn = 0usize;
        let mut today = String::new();
        self.known_venues().await;
        self.finder.ready().await;
        self.first_pass().await;
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            self.finder.refresh().await;
            let now = time::today();
            if now != today {
                if let Err(error) = self.db.forget_before(&now).await {
                    eprintln!("poll: forgetting past days: {error}");
                }
                today = now;
            }
            let watches = self.db.watches().await.unwrap_or_else(|error| {
                eprintln!("poll: watches: {error}");
                Vec::new()
            });
            let wanted = self.wanted(&today, &watches, turn);
            turn = turn.wrapping_add(1);
            for (club, date) in wanted {
                self.fetch(club, &date, &watches).await;
                tokio::time::sleep(BETWEEN_REQUESTS).await;
            }
            self.eversports(&watches, turn).await;
        }
    }

    /// Shows the Eversports venues stored before right away, while they are looked up again.
    async fn known_venues(&self) {
        for club in self
            .db
            .clubs_from(crate::source::Source::Eversports)
            .await
            .unwrap_or_default()
        {
            if self.venues.slugs.contains(&club.canonical) {
                self.clubs.add_eversports(club.id);
            }
        }
    }

    async fn eversports(&self, watches: &[Watch], turn: usize) {
        if self.venues.slugs.is_empty() {
            return;
        }
        let watched: BTreeSet<String> = watches.iter().map(|w| w.date.clone()).collect();
        for (date, day) in self.venues.poll(&time::dates(self.days), &watched, turn).await {
            self.clubs.add_eversports(day.club.id);
            self.take(&day, &date, watches).await;
        }
    }

    /// Today and the next few days at every club, before the minute-by-minute round starts.
    async fn first_pass(&self) {
        let watches = self.db.watches().await.unwrap_or_default();
        self.eversports(&watches, 0).await;
        for date in time::dates(self.days.min(7)) {
            for club in self.clubs.gotcourts() {
                self.fetch(club, &date, &watches).await;
                tokio::time::sleep(BETWEEN_REQUESTS).await;
            }
        }
    }

    /// What to ask about this minute: today and the watched days at every
    /// club, then the `turn`th slice of the days after.
    fn wanted(&self, today: &str, watches: &[Watch], turn: usize) -> Vec<(i64, String)> {
        let dates = time::dates(self.days);
        let clubs = self.clubs.gotcourts();
        let mut wanted: BTreeSet<(i64, String)> = clubs.iter().map(|&club| (club, today.to_string())).collect();
        for watch in watches.iter().filter(|w| dates.contains(&w.date)) {
            match watch.club_id {
                Some(club) => {
                    wanted.insert((club, watch.date.clone()));
                }
                None => wanted.extend(clubs.iter().map(|&club| (club, watch.date.clone()))),
            }
        }
        let rest: Vec<(i64, String)> = dates
            .iter()
            .skip(1)
            .flat_map(|date| clubs.iter().map(move |&club| (club, date.clone())))
            .filter(|key| !wanted.contains(key))
            .collect();
        let mut list: Vec<(i64, String)> = wanted.into_iter().collect();
        if !rest.is_empty() {
            let start = turn * self.per_minute % rest.len();
            list.extend(
                rest.iter()
                    .cycle()
                    .skip(start)
                    .take(self.per_minute.min(rest.len()))
                    .cloned(),
            );
        }
        list
    }

    async fn fetch(&self, club: i64, date: &str, watches: &[Watch]) {
        let now = time::now();
        match self.db.closed_at(club).await {
            Ok(Some(at)) if now - at < CLOSED_FOR => return,
            _ => {}
        }
        match self.gotcourts.day(club, date).await {
            Ok(day) => self.take(&day, date, watches).await,
            Err(gotcourts::Error::Unknown) => {
                eprintln!("poll: GotCourts does not book for club {club}; asking again tomorrow");
                if let Err(error) = self.db.close(club, "unknown club", now).await {
                    eprintln!("poll: {error}");
                }
            }
            Err(gotcourts::Error::Failed(why)) => eprintln!("poll: club {club} on {date}: {why}"),
        }
    }

    /// Keeps what a source said about a club on `date`, and tells the watches on it.
    async fn take(&self, day: &gotcourts::Day, date: &str, watches: &[Watch]) {
        let club = day.club.id;
        if let Err(error) = self.db.save_day(day, date, time::now()).await {
            eprintln!("poll: saving club {club} on {date}: {error}");
            return;
        }
        for watch in watches
            .iter()
            .filter(|w| w.date == date && w.club_id.is_none_or(|c| c == club))
        {
            self.check(watch, day).await;
        }
    }

    /// Tells about slots of `day` that `watch` wants and didn't tell about yet.
    async fn check(&self, watch: &Watch, day: &gotcourts::Day) {
        let free = matching(watch, day);
        let ours: HashSet<i64> = day.courts.iter().map(|c| c.id).collect();
        let told: HashSet<(i64, i64)> = match self.db.told(watch.id).await {
            Ok(told) => told.into_iter().filter(|(court, _)| ours.contains(court)).collect(),
            Err(error) => {
                eprintln!("poll: watch {}: {error}", watch.id);
                return;
            }
        };
        let now: HashSet<(i64, i64)> = free.iter().map(|s| (s.court, s.start)).collect();
        for &(court, start) in told.difference(&now) {
            let _ = self.db.untell(watch.id, court, start).await;
        }
        let fresh: Vec<&Slot> = free.iter().filter(|s| !told.contains(&(s.court, s.start))).collect();
        if fresh.is_empty() {
            return;
        }
        let notice = notice(watch, day, &fresh);
        let sent = self.push.notify(&notice).await;
        eprintln!("poll: watch {}: {} ({sent} browsers)", watch.id, notice.body);
        for slot in fresh {
            if let Err(error) = self.db.tell(watch.id, &watch.date, slot.court, slot.start).await {
                eprintln!("poll: {error}");
            }
        }
    }

    /// Takes note of what is free for a new watch now, so it tells only
    /// about what frees up from here on.
    pub async fn note(&self, watch: &Watch) {
        let Ok(days) = self.db.days_on(&watch.date).await else {
            return;
        };
        for stored in days
            .into_iter()
            .filter(|d| watch.club_id.is_none_or(|c| c == d.club_id))
        {
            let Ok(Some(club)) = self.db.club(stored.club_id).await else {
                continue;
            };
            let day = gotcourts::Day {
                club: crate::pages::club_info(&club),
                courts: stored.courts.0,
                busy: stored.busy.0,
            };
            for slot in matching(watch, &day) {
                let _ = self.db.tell(watch.id, &watch.date, slot.court, slot.start).await;
            }
        }
    }
}

pub fn matching(watch: &Watch, day: &gotcourts::Day) -> Vec<Slot> {
    let courts: Vec<_> = day
        .courts
        .iter()
        .filter(|court| !watch.indoor || court.indoor == Some(true))
        .filter(|court| watch.sport.as_deref().is_none_or(|sport| court.sport == sport))
        .cloned()
        .collect();
    let after = if watch.date == time::today() {
        time::seconds_today()
    } else {
        0
    };
    slots::free(&courts, &day.busy, after)
        .into_iter()
        .filter(|slot| slot.start >= watch.earliest && slot.end <= watch.latest)
        .collect()
}

fn notice(watch: &Watch, day: &gotcourts::Day, fresh: &[&Slot]) -> Notice {
    let mut starts: Vec<i64> = fresh.iter().map(|s| s.start).collect();
    starts.sort();
    starts.dedup();
    let times: Vec<String> = starts.iter().take(6).map(|&s| slots::clock(s)).collect();
    let courts: BTreeSet<&str> = fresh
        .iter()
        .filter_map(|s| day.courts.iter().find(|c| c.id == s.court))
        .map(|c| c.label.as_str())
        .collect();
    let what = if fresh.len() == 1 {
        format!("{} is free at {}", courts.iter().next().unwrap_or(&"A court"), times[0])
    } else {
        format!("{} slots free: {}", fresh.len(), times.join(", "))
    };
    Notice {
        title: format!("{} · {}", day.club.name, time::label(&watch.date)),
        body: what,
        url: format!("/?date={}#club-{}", watch.date, day.club.id),
        tag: format!("w{}-c{}", watch.id, day.club.id),
    }
}
