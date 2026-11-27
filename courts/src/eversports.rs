//! What Eversports' own booking pages load, without an account. A venue's
//! booking page (`/sb/{slug}`) names its number and sports; its profile
//! (`/s/{slug}`) its address. For each sport, `/api/booking/calendar/update`
//! lays out a week: every court, the stretches it is booked in on each day
//! (cells, often of different lengths) and what each costs. `/api/slot`
//! lists what is taken in that week, one entry per taken cell, so what is
//! free is the cells nobody has.

use crate::gotcourts::{Busy, Cell, Club, Court, Day, SPORTS, covered};
use crate::source::Source;
use crate::time;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

const BASE: &str = "https://www.eversports.ch";
const BETWEEN_REQUESTS: Duration = Duration::from_millis(300);
/// How long a week's layout (courts, cells, prices) is used before it is read again.
const LAYOUT_FOR: i64 = 60 * 60;
/// How long a venue's page and address are used before they are read again.
const VENUE_FOR: i64 = 24 * 60 * 60;
/// The weeks after the first are asked about every this many minutes, unless watched.
const LATER_WEEKS_EVERY: usize = 5;

/// Courts by day, as a calendar lays them out.
pub type Layout = BTreeMap<String, Vec<Court>>;

pub fn booking_url(slug: &str) -> String {
    format!("{BASE}/sb/{slug}")
}

/// A sport a venue books, as its booking page names it.
#[derive(Clone, Debug, PartialEq)]
pub struct Sport {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub uuid: String,
}

/// A venue's booking page: Eversports' number for it, and the sports of this app it books.
#[derive(Clone, Debug, PartialEq)]
pub struct Facility {
    pub id: i64,
    pub uuid: String,
    pub slug: String,
    pub sports: Vec<Sport>,
}

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One start tag: its name and attributes, with entities decoded.
struct Tag<'a> {
    name: &'a str,
    attributes: Vec<(&'a str, String)>,
}

impl Tag<'_> {
    fn get(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

fn decode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';').filter(|end| *end <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|n| n.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The start tags of `html`, in order. Comments are skipped; text and end tags are not returned.
fn tags(html: &str) -> Vec<Tag<'_>> {
    let mut found = Vec::new();
    let bytes = html.as_bytes();
    let mut at = 0;
    while let Some(open) = html[at..].find('<').map(|i| at + i) {
        if html[open..].starts_with("<!--") {
            at = html[open..].find("-->").map_or(html.len(), |i| open + i + 3);
            continue;
        }
        let mut i = open + 1;
        if !bytes.get(i).is_some_and(u8::is_ascii_alphabetic) {
            at = i;
            continue;
        }
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-') {
            i += 1;
        }
        let name = &html[open + 1..i];
        let mut attributes = Vec::new();
        loop {
            while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
                i += 1;
            }
            if i >= bytes.len() || bytes[i] == b'>' {
                i += 1;
                break;
            }
            let start = i;
            while i < bytes.len() && !matches!(bytes[i], b'=' | b'>' | b'/') && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let key = &html[start..i];
            if bytes.get(i) != Some(&b'=') {
                attributes.push((key, String::new()));
                continue;
            }
            i += 1;
            let value = match bytes.get(i) {
                Some(&quote @ (b'"' | b'\'')) => {
                    let end = html[i + 1..].find(quote as char).map_or(html.len(), |e| i + 1 + e);
                    let value = &html[i + 1..end];
                    i = (end + 1).min(html.len());
                    value
                }
                _ => {
                    let start = i;
                    while i < bytes.len() && bytes[i] != b'>' && !bytes[i].is_ascii_whitespace() {
                        i += 1;
                    }
                    &html[start..i]
                }
            };
            attributes.push((key, decode(value)));
        }
        found.push(Tag { name, attributes });
        at = i.min(html.len());
    }
    found
}

/// "0815" as seconds after midnight; "0000" or "2400" at the end of a day is midnight after it.
fn hhmm(text: &str) -> Option<i64> {
    let number: i64 = text.trim().parse().ok()?;
    Some(number / 100 * 3600 + number % 100 * 60)
}

/// "22" or "22.5" francs as Rappen.
fn rappen(text: &str) -> Option<i64> {
    let francs: f64 = text.trim().parse().ok()?;
    Some((francs * 100.0).round() as i64)
}

fn surface(name: &str) -> String {
    match name.trim().to_lowercase().as_str() {
        "hartplatz" | "hart" => "hard".into(),
        "sand" | "sandplatz" | "asche" => "clay".into(),
        "kunstrasen" => "artificial_grass".into(),
        "rasen" => "grass".into(),
        "teppich" => "carpet".into(),
        "parkett" => "parquet".into(),
        other => other.into(),
    }
}

/// The venue's number, uuid and sports from its booking page.
pub fn facility(html: &str, slug: &str) -> Result<Facility, Error> {
    let from = html
        .find("id=\"calendar-title\"")
        .ok_or_else(|| Error(format!("{slug}: no booking calendar on the page")))?;
    let start = html[..from].rfind('<').unwrap_or(from);
    let end = html[from..].find("</nav>").map_or(html.len(), |i| from + i);
    let found = tags(&html[start..end]);
    let title = found
        .iter()
        .find(|tag| tag.get("id") == Some("calendar-title"))
        .ok_or_else(|| Error(format!("{slug}: no booking calendar on the page")))?;
    let id = title
        .get("data-id")
        .and_then(|id| id.trim().parse().ok())
        .ok_or_else(|| Error(format!("{slug}: no facility number")))?;
    let sports = found
        .iter()
        .filter(|tag| tag.name == "li" && tag.get("class").is_some_and(|c| c.contains("booking-sport")))
        .filter_map(|tag| {
            Some(Sport {
                id: tag.get("data-id")?.trim().parse().ok()?,
                name: tag.get("data-name")?.to_string(),
                slug: tag.get("data-slug")?.to_string(),
                uuid: tag.get("data-uuid")?.to_string(),
            })
        })
        .filter(|sport| SPORTS.contains(&sport.slug.as_str()))
        .collect();
    Ok(Facility {
        id,
        uuid: title.get("data-facility-uuid").unwrap_or_default().to_string(),
        slug: slug.to_string(),
        sports,
    })
}

/// The venue's name, address and contact from its profile page, where the
/// page's own state holds them under `Venue:{uuid}`.
pub fn venue(html: &str, facility: &Facility) -> Result<Club, Error> {
    let key = format!("\"Venue:{}\":", facility.uuid);
    let at = html
        .find(&key)
        .ok_or_else(|| Error(format!("{}: no venue on the profile page", facility.slug)))?;
    let object = &html[at + key.len()..];
    let found: Value = serde_json::Deserializer::from_str(object)
        .into_iter::<Value>()
        .next()
        .and_then(Result::ok)
        .ok_or_else(|| Error(format!("{}: the venue on the profile page isn't JSON", facility.slug)))?;
    let text = |value: &Value| value.as_str().unwrap_or_default().trim().to_string();
    let location = &found["location"];
    let latitude = location["latitude"].as_f64();
    let longitude = location["longitude"].as_f64();
    let (Some(latitude), Some(longitude)) = (latitude, longitude) else {
        return Err(Error(format!("{}: no location on the profile page", facility.slug)));
    };
    let zip = text(&location["zip"]);
    let city = text(&location["city"]);
    let full = text(&found["address"]);
    let street = full
        .split(',')
        .next()
        .map(|street| street.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    Ok(Club {
        id: Source::Eversports.id(facility.id),
        source: Source::Eversports,
        name: Some(text(&found["name"]))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| facility.slug.clone()),
        canonical: facility.slug.clone(),
        address: street,
        zip,
        city,
        latitude,
        longitude,
        phone: text(&found["contact"]["telephone"]),
        web: text(&found["contact"]["website"]),
        kind: "public".into(),
    })
}

/// One sport's courts on each day of the week the calendar lays out.
pub fn calendar(html: &str, sport: &str) -> Layout {
    struct Row {
        theirs: i64,
        label: String,
        area: String,
        surface: String,
        cells: Vec<Cell>,
    }
    let mut days = Layout::new();
    let mut date: Option<String> = None;
    let mut row: Option<Row> = None;
    let mut flush = |date: &Option<String>, row: &mut Option<Row>| {
        let (Some(date), Some(row)) = (date, row.take()) else {
            return;
        };
        if row.theirs == 0 {
            return;
        }
        let courts = days.entry(date.clone()).or_default();
        let lengths: BTreeSet<i64> = row.cells.iter().map(|c| (c.end - c.start) / 60).collect();
        let mut counts: BTreeMap<i64, usize> = BTreeMap::new();
        for cell in &row.cells {
            *counts.entry((cell.end - cell.start) / 60).or_default() += 1;
        }
        let interval = counts.iter().max_by_key(|(_, n)| **n).map_or(60, |(m, _)| *m);
        courts.push(Court {
            id: Source::Eversports.id(row.theirs),
            sport: sport.to_string(),
            label: row.label.split_whitespace().collect::<Vec<_>>().join(" "),
            surface: surface(&row.surface),
            indoor: covered(&row.area),
            cover: row.area,
            ball_machine: false,
            order: courts.len() as i64,
            interval,
            durations: lengths.into_iter().collect(),
            opening: row.cells.first().map_or(0, |c| c.start),
            closing: row.cells.last().map_or(0, |c| c.end),
            cells: row.cells,
        });
    };
    for tag in tags(html) {
        match tag.name {
            "tbody" => {
                flush(&date, &mut row);
                date = tag.get("data-date").map(str::to_string);
            }
            "tr" => {
                flush(&date, &mut row);
                if tag
                    .get("class")
                    .is_some_and(|c| c.split_whitespace().any(|c| c == "court"))
                {
                    row = Some(Row {
                        theirs: 0,
                        label: String::new(),
                        area: tag.get("data-area").unwrap_or_default().to_lowercase(),
                        surface: tag.get("data-surface").unwrap_or_default().to_string(),
                        cells: Vec::new(),
                    });
                }
            }
            "td" => {
                let Some(row) = row.as_mut() else { continue };
                let court: Option<i64> = tag.get("data-court").and_then(|c| c.trim().parse().ok());
                if tag.get("data-firststart").is_some() {
                    row.theirs = court.unwrap_or(0);
                    continue;
                }
                if tag.get("data-state") != Some("free")
                    || court != Some(row.theirs)
                    || tag.get("data-date") != date.as_deref()
                {
                    continue;
                }
                let (Some(start), Some(end)) =
                    (tag.get("data-start").and_then(hhmm), tag.get("data-end").and_then(hhmm))
                else {
                    continue;
                };
                let end = if end <= start { end + 24 * 3600 } else { end };
                row.cells.push(Cell {
                    start,
                    end,
                    price: tag.get("data-price").and_then(rappen),
                });
            }
            "span" => {
                if let (Some(row), Some(title)) = (row.as_mut(), tag.get("data-title")) {
                    row.label = title.to_string();
                }
            }
            _ => {}
        }
    }
    flush(&date, &mut row);
    days
}

/// What `/api/slot` says is taken, by day: each entry one taken cell of the
/// courts `days` lays out (an hour where the cell isn't known).
pub fn taken(body: &Value, days: &Layout) -> BTreeMap<String, Vec<Busy>> {
    let mut busy: BTreeMap<String, Vec<Busy>> = BTreeMap::new();
    for slot in body["slots"].as_array().into_iter().flatten() {
        let Some(date) = slot["date"].as_str() else { continue };
        let Some(start) = slot["start"].as_str().and_then(hhmm) else {
            continue;
        };
        let Some(theirs) = slot["court"].as_i64() else { continue };
        let id = Source::Eversports.id(theirs);
        let court = days.get(date).and_then(|courts| courts.iter().find(|c| c.id == id));
        let end = court
            .and_then(|court| court.cells.iter().find(|cell| cell.start == start).map(|cell| cell.end))
            .unwrap_or_else(|| start + court.map_or(60, |c| c.interval) * 60);
        busy.entry(date.to_string()).or_default().push(Busy {
            court: Some(id),
            start,
            end,
            note: slot["title"]
                .as_str()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_string),
            members: false,
        });
    }
    busy
}

#[derive(Clone)]
pub struct Eversports {
    agent: ureq::Agent,
}

impl Default for Eversports {
    fn default() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .user_agent("courts (a personal court finder)")
            .build()
            .new_agent();
        Self { agent }
    }
}

impl Eversports {
    async fn call<T: Send + 'static>(
        &self,
        what: String,
        send: impl FnOnce(&ureq::Agent) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> + Send + 'static,
        read: impl FnOnce(&mut ureq::Body) -> Result<T, ureq::Error> + Send + 'static,
    ) -> Result<T, Error> {
        let agent = self.agent.clone();
        tokio::task::spawn_blocking(move || {
            let mut response = send(&agent).map_err(|error| Error(format!("{what}: {error}")))?;
            if !response.status().is_success() {
                return Err(Error(format!("{what}: HTTP {}", response.status())));
            }
            read(response.body_mut()).map_err(|error| Error(format!("{what}: {error}")))
        })
        .await
        .map_err(|error| Error(error.to_string()))?
    }

    async fn page(&self, url: String) -> Result<String, Error> {
        let what = url.clone();
        self.call(
            what,
            move |agent| agent.get(&url).call(),
            |body| body.with_config().limit(8 << 20).read_to_string(),
        )
        .await
    }

    pub async fn facility(&self, slug: &str) -> Result<Facility, Error> {
        facility(&self.page(booking_url(slug)).await?, slug)
    }

    pub async fn venue(&self, facility: &Facility) -> Result<Club, Error> {
        venue(&self.page(format!("{BASE}/s/{}", facility.slug)).await?, facility)
    }

    /// One sport's layout of the week from `date` on.
    pub async fn calendar(&self, facility: &Facility, sport: &Sport, date: &str) -> Result<Layout, Error> {
        let form = [
            ("facilityId", facility.id.to_string()),
            ("facilitySlug", facility.slug.clone()),
            ("sport[id]", sport.id.to_string()),
            ("sport[name]", sport.name.clone()),
            ("sport[slug]", sport.slug.clone()),
            ("sport[uuid]", sport.uuid.clone()),
            ("date", date.to_string()),
            ("type", "user".to_string()),
        ];
        let html = self
            .call(
                format!("{} {} calendar from {date}", facility.slug, sport.slug),
                move |agent| {
                    agent
                        .post(format!("{BASE}/api/booking/calendar/update"))
                        .send_form(form.iter().map(|(k, v)| (*k, v.as_str())))
                },
                |body| body.with_config().limit(8 << 20).read_to_string(),
            )
            .await?;
        Ok(calendar(&html, &sport.slug))
    }

    /// What is taken in the week from `date` on, on Eversports' courts `courts`.
    pub async fn taken(&self, facility: &Facility, date: &str, courts: &[i64]) -> Result<Value, Error> {
        let query = {
            let mut query = url::form_urlencoded::Serializer::new(String::new());
            query.append_pair("facilityId", &facility.id.to_string());
            query.append_pair("startDate", date);
            for court in courts {
                query.append_pair("courts[]", &court.to_string());
            }
            query.finish()
        };
        let url = format!("{BASE}/api/slot?{query}");
        self.call(
            format!("{} taken from {date}", facility.slug),
            move |agent| agent.get(&url).header("Accept", "application/json").call(),
            |body| body.read_json::<Value>(),
        )
        .await
    }
}

/// What is known about one venue, and its layouts by (sport, first day of the week).
struct Known {
    facility: Facility,
    club: Club,
    at: i64,
    layouts: BTreeMap<(String, String), (i64, Layout)>,
}

/// The venues to ask Eversports about, by slug.
pub struct Venues {
    pub eversports: Eversports,
    pub slugs: Vec<String>,
    known: Mutex<BTreeMap<String, Known>>,
}

impl Venues {
    pub fn new(slugs: Vec<String>) -> Arc<Self> {
        Arc::new(Self {
            eversports: Eversports::default(),
            slugs,
            known: Mutex::new(BTreeMap::new()),
        })
    }

    async fn look_up(&self, slug: &str) -> Result<(Facility, Club), Error> {
        let facility = self.eversports.facility(slug).await?;
        tokio::time::sleep(BETWEEN_REQUESTS).await;
        let club = self.eversports.venue(&facility).await?;
        Ok((facility, club))
    }

    /// Every venue's days among `dates`, asked about in weeks: the first week
    /// every time, later ones every few `turn`s or when a day in them is watched.
    pub async fn poll(&self, dates: &[String], watched: &BTreeSet<String>, turn: usize) -> Vec<(String, Day)> {
        let now = time::now();
        let mut found = Vec::new();
        let mut known = self.known.lock().await;
        for slug in &self.slugs {
            if known.get(slug).is_none_or(|k| now - k.at >= VENUE_FOR) {
                match self.look_up(slug).await {
                    Ok((facility, club)) => {
                        let layouts = known.remove(slug).map(|k| k.layouts).unwrap_or_default();
                        known.insert(
                            slug.clone(),
                            Known {
                                facility,
                                club,
                                at: now,
                                layouts,
                            },
                        );
                    }
                    Err(error) => {
                        eprintln!("eversports: {error}");
                        if let Some(stale) = known.get_mut(slug) {
                            stale.at = now - VENUE_FOR + LAYOUT_FOR;
                        }
                    }
                }
                tokio::time::sleep(BETWEEN_REQUESTS).await;
            }
            let Some(venue) = known.get_mut(slug) else { continue };
            for (n, week) in dates.chunks(7).enumerate() {
                if n > 0 && !turn.is_multiple_of(LATER_WEEKS_EVERY) && !week.iter().any(|d| watched.contains(d)) {
                    continue;
                }
                match self.week(venue, week, now).await {
                    Ok(days) => found.extend(days),
                    Err(error) => eprintln!("eversports: {error}"),
                }
            }
        }
        found
    }

    async fn week(&self, venue: &mut Known, week: &[String], now: i64) -> Result<Vec<(String, Day)>, Error> {
        let first = &week[0];
        let mut courts = Layout::new();
        for sport in &venue.facility.sports {
            let key = (sport.slug.clone(), first.clone());
            let fresh = venue.layouts.get(&key).filter(|(at, _)| now - at < LAYOUT_FOR);
            let layout = match fresh {
                Some((_, layout)) => layout.clone(),
                None => {
                    let layout = self.eversports.calendar(&venue.facility, sport, first).await?;
                    tokio::time::sleep(BETWEEN_REQUESTS).await;
                    venue.layouts.insert(key, (now, layout.clone()));
                    layout
                }
            };
            for (date, list) in layout {
                courts.entry(date).or_default().extend(list);
            }
        }
        let today = time::today();
        venue.layouts.retain(|(_, from), _| *from >= today);
        let theirs: BTreeSet<i64> = courts
            .values()
            .flatten()
            .map(|court| court.id - Source::Eversports.id(0))
            .collect();
        if theirs.is_empty() {
            return Ok(Vec::new());
        }
        let body = self
            .eversports
            .taken(&venue.facility, first, &theirs.into_iter().collect::<Vec<_>>())
            .await?;
        tokio::time::sleep(BETWEEN_REQUESTS).await;
        let mut busy = taken(&body, &courts);
        Ok(week
            .iter()
            .map(|date| {
                let day = Day {
                    club: venue.club.clone(),
                    courts: courts.get(date).cloned().unwrap_or_default(),
                    busy: busy.remove(date).unwrap_or_default(),
                };
                (date.clone(), day)
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slots::{self, clock};

    /// Squash Arena Uster on Monday 30 November 2026, two of its courts, as
    /// Eversports answered on 27 November: 45-minute cells after a first hour.
    const CALENDAR: &str = include_str!("testdata/eversports-calendar.html");
    const TAKEN: &str = include_str!("testdata/eversports-slots.json");
    const MONDAY: &str = "2026-11-30";

    fn starts(free: &[slots::Slot], court: i64) -> Vec<String> {
        free.iter()
            .filter(|s| s.court == court)
            .map(|s| clock(s.start))
            .collect()
    }

    #[test]
    fn the_calendar_lays_out_each_court_in_its_cells_with_prices() {
        let days = calendar(CALENDAR, "squash");
        let courts = &days[MONDAY];
        assert_eq!(courts.len(), 2);
        let court = &courts[0];
        assert_eq!(court.id, Source::Eversports.id(99112));
        assert_eq!((court.label.as_str(), court.sport.as_str()), ("Court 1", "squash"));
        assert_eq!((court.indoor, court.surface.as_str()), (Some(true), "parquet"));
        assert_eq!(
            (clock(court.opening), clock(court.closing)),
            ("10:15".into(), "22:15".into())
        );
        assert_eq!(court.interval, 45);
        assert_eq!(court.durations, [30, 45, 60]);
        assert_eq!(court.cells.len(), 16);
        assert_eq!(
            court.cells[0],
            Cell {
                start: 10 * 3600 + 900,
                end: 11 * 3600 + 900,
                price: Some(2200)
            }
        );
        assert_eq!(court.cells[9].price, Some(3000));
    }

    #[test]
    fn free_time_is_the_cells_nobody_has() {
        let days = calendar(CALENDAR, "squash");
        let body: Value = serde_json::from_str(TAKEN).unwrap();
        let busy = taken(&body, &days);
        let one = Source::Eversports.id(99112);
        let mine: Vec<(String, String)> = busy[MONDAY]
            .iter()
            .filter(|b| b.court == Some(one))
            .map(|b| (clock(b.start), clock(b.end)))
            .collect();
        assert_eq!(mine[0], ("10:15".into(), "11:15".into()));
        assert_eq!(mine[1], ("11:15".into(), "12:00".into()));
        assert_eq!(mine.len(), 6);
        assert!(
            busy.contains_key("2026-12-01"),
            "days the calendar doesn't lay out still count"
        );
        let courts = &days[MONDAY];
        let cells = slots::free(courts, &busy[MONDAY], 0);
        assert_eq!(
            starts(&cells, one),
            [
                "12:00", "12:45", "13:30", "14:15", "15:00", "15:45", "16:30", "17:15", "19:30", "21:45"
            ]
        );
        let hour = slots::free_for(courts, &busy[MONDAY], 0, 3600);
        assert_eq!(
            starts(&hour, one),
            ["12:00", "12:45", "13:30", "14:15", "15:00", "15:45", "16:30"],
            "an hour from 17:15 runs into the booking at 18:00, and 21:45 closes at 22:15"
        );
        let late = hour
            .iter()
            .find(|s| s.court == one && s.start == 16 * 3600 + 1800)
            .unwrap();
        assert_eq!(slots::price(&courts[0], late), Some(2200 + 3000));
        assert_eq!(slots::booked_end(&courts[0], late), 17 * 3600 + 3600);
        let after_four = slots::free_for(courts, &busy[MONDAY], 16 * 3600, 3600);
        assert_eq!(starts(&after_four, one), ["16:30"]);
    }

    #[test]
    fn the_booking_page_names_the_facility_and_its_sports() {
        let html = r#"<script>if (a<b) x = "<li data-id='1'>";</script><div class="container profile-calendar"><div id="calendar-title" data-facility="sihlsports" data-facility-shortid="kjex7h" data-facility-uuid="043cb080-6240-4358-a345-8487247b4537" data-id='78306' data-sportid='433' data-sportslug="tennis"></div><nav title="Sportart auswählen" class="tab-menu"><ul class="nav-tabs"><li data-id='433' data-name="Tennis" data-slug="tennis" data-uuid="b38729e9-69de-11e8-bdc6-02bd505aa7b2" tabindex="0" class="booking-sport">Tennis</li><li data-id='435' data-name="Badminton" data-slug="badminton" data-uuid="b3872f37" class="booking-sport">Badminton</li><li data-id='978' data-name="Padel" data-slug="padel" data-uuid="b388f5e3-69de-11e8-bdc6-02bd505aa7b2" class="booking-sport">Padel</li></ul></nav>"#;
        let found = facility(html, "sihlsports").unwrap();
        assert_eq!(
            (found.id, found.uuid.as_str()),
            (78306, "043cb080-6240-4358-a345-8487247b4537")
        );
        let sports: Vec<&str> = found.sports.iter().map(|s| s.slug.as_str()).collect();
        assert_eq!(sports, ["tennis", "padel"]);
        assert_eq!(found.sports[1].id, 978);
        assert!(facility("<html></html>", "gone").is_err());
        let profile = r#"{"ROOT_QUERY":{},"Venue:043cb080-6240-4358-a345-8487247b4537":{"__typename":"Venue","name":"SIHLSPORTS","slug":"sihlsports","location":{"__typename":"Location","city":"Langnau am Albis","zip":"8135","country":"CH","latitude":47.2923583984,"longitude":8.5416994095},"address":"Sihltalstrasse 63 , 8135 Langnau am Albis","contact":{"__typename":"Contact","website":"https://sihlsports.ch/de/home.html","telephone":"+41 43 377 70 00"}},"Venue:other":{}}"#;
        let club = venue(profile, &found).unwrap();
        assert_eq!(club.id, Source::Eversports.id(78306));
        assert_eq!(
            (club.name.as_str(), club.address.as_str()),
            ("SIHLSPORTS", "Sihltalstrasse 63")
        );
        assert_eq!((club.zip.as_str(), club.city.as_str()), ("8135", "Langnau am Albis"));
        assert_eq!(club.web, "https://sihlsports.ch/de/home.html");
        assert_eq!((club.latitude, club.source), (47.2923583984, Source::Eversports));
    }

    #[test]
    fn tags_read_both_quotes_and_entities() {
        let found = tags(
            r#"<!-- <td data-x="no"> --><td data-a='1' data-b="x &amp; y" hidden data-c=plain><span data-title="Court 7/ TT">"#,
        );
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].get("data-a"), Some("1"));
        assert_eq!(found[0].get("data-b"), Some("x & y"));
        assert_eq!(found[0].get("hidden"), Some(""));
        assert_eq!(found[0].get("data-c"), Some("plain"));
        assert_eq!(found[1].get("data-title"), Some("Court 7/ TT"));
        assert_eq!(decode("&#39;a&#x27;&unknown;"), "'a'&unknown;");
    }
}
