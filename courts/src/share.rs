//! A link to one slot that anybody can open without signing in. The slot
//! is in the link itself, signed with the sign-in key, so a link can't be
//! made up for another slot and the key itself never leaves the server.
//! The page says what the slot is, what it costs, and whether it is still
//! free as of the last time its source (GotCourts or Eversports) was asked.

use crate::db::{self, Db};
use crate::gotcourts::{self, Court, GotCourts};
use crate::slots::clock;
use crate::source::Source;
use crate::time;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::router::content::Html;
use topcoat::router::{StatusCode, path_param, route};
use topcoat::view::{ViewExt, view};

/// One court on one day, from `start` to `end` (seconds after midnight).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shared {
    pub club: i64,
    pub court: i64,
    pub date: String,
    pub start: i64,
    pub end: i64,
}

impl Shared {
    fn minutes(&self) -> i64 {
        (self.end - self.start) / 60
    }

    fn payload(&self) -> Option<String> {
        let date = time::parse(&self.date)?;
        let fits = self.start >= 0 && self.start % 60 == 0 && self.end > self.start && self.end <= 24 * 3600;
        fits.then(|| {
            format!(
                "{}-{}-{}-{}-{}",
                self.club,
                self.court,
                date.strftime("%Y%m%d"),
                clock(self.start).replace(':', ""),
                self.minutes()
            )
        })
    }
}

fn signature(key: &str, payload: &str) -> String {
    use base64::Engine;
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key.as_bytes());
    let tag = ring::hmac::sign(&key, format!("share {payload}").as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&tag.as_ref()[..12])
}

/// The path of the page for `shared`: `/s/CLUB-COURT-YYYYMMDD-HHMM-MINUTES-SIGNATURE`.
pub fn path(key: &str, shared: &Shared) -> String {
    let payload = shared.payload().unwrap_or_default();
    format!("/s/{payload}-{}", signature(key, &payload))
}

/// The slot a link's code names, if the signature is right.
pub fn read(key: &str, code: &str) -> Option<Shared> {
    let mut parts = code.splitn(6, '-');
    let mut number = || -> Option<i64> {
        let part = parts.next()?;
        part.bytes().all(|b| b.is_ascii_digit()).then(|| part.parse().ok())?
    };
    let (club, court, day, hhmm, minutes) = (number()?, number()?, number()?, number()?, number()?);
    let given = parts.next()?;
    let date = jiff::civil::Date::new(
        i16::try_from(day / 10000).ok()?,
        i8::try_from(day / 100 % 100).ok()?,
        i8::try_from(day % 100).ok()?,
    )
    .ok()?;
    let start = hhmm / 100 * 3600 + hhmm % 100 * 60;
    let shared = Shared {
        club,
        court,
        date: date.to_string(),
        start,
        end: start + minutes * 60,
    };
    let payload = shared.payload()?;
    let wanted = signature(key, &payload);
    let same =
        wanted.len() == given.len() && wanted.bytes().zip(given.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0;
    (same && code.len() == payload.len() + 1 + given.len()).then_some(shared)
}

/// "Tennis, Sat 29 Nov 18:00–19:00, TC Riesbach": the link preview's title.
pub fn title(sport: &str, shared: &Shared, club: &str) -> String {
    let day = time::parse(&shared.date).map_or(shared.date.clone(), |d| d.strftime("%a %-d %b").to_string());
    format!(
        "{}, {day} {}–{}, {club}",
        crate::pages::sport_name(sport),
        clock(shared.start),
        clock(shared.end)
    )
}

/// "Tennis at Belvoir Indoors", or just the club when its name says the sport.
pub fn headline(sport: &str, club: &str) -> String {
    if club.to_lowercase().contains(&sport.to_lowercase()) {
        club.to_string()
    } else {
        format!("{sport} at {club}")
    }
}

/// "CHF 75.00".
pub fn francs(rappen: i64) -> String {
    format!("CHF {}.{:02}", rappen / 100, rappen % 100)
}

/// "1 hour", "1½ hours", "45 minutes".
pub fn length(minutes: i64) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} minutes"),
        (1, 0) => "1 hour".into(),
        (h, 0) => format!("{h} hours"),
        (h, 30) => format!("{h}½ hours"),
        _ => format!("{minutes} minutes"),
    }
}

/// What a price lookup found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Price {
    Rappen(i64),
    NotListed,
    /// GotCourts didn't answer this time.
    Unknown,
}

/// A court, date, start and end.
type Asked = (i64, String, i64, i64);

/// Prices asked of GotCourts once per slot, kept while the app runs.
/// Eversports tells its prices with the courts, so those need no asking.
#[derive(Clone, Default)]
pub struct Prices {
    known: Arc<Mutex<HashMap<Asked, Price>>>,
    gotcourts: GotCourts,
}

impl Prices {
    pub async fn of(&self, shared: &Shared, club: &db::Club, court: &Court) -> Price {
        if Source::parse(&club.source) == Source::Eversports {
            let slot = crate::slots::Slot {
                court: court.id,
                start: shared.start,
                end: shared.end,
            };
            return crate::slots::price(court, &slot).map_or(Price::NotListed, Price::Rappen);
        }
        let key = (shared.court, shared.date.clone(), shared.start, shared.end);
        if let Some(price) = self.known.lock().ok().and_then(|known| known.get(&key).copied()) {
            return price;
        }
        let asked = self
            .gotcourts
            .price(shared.court, &shared.date, shared.start, shared.minutes());
        let price = match tokio::time::timeout(Duration::from_secs(6), asked).await {
            Ok(Ok(Some(rappen))) => Price::Rappen(rappen),
            Ok(Ok(None)) => Price::NotListed,
            Ok(Err(error)) => {
                eprintln!("price of court {} on {}: {error}", shared.court, shared.date);
                return Price::Unknown;
            }
            Err(_) => return Price::Unknown,
        };
        if let Ok(mut known) = self.known.lock() {
            if known.len() > 2000 {
                known.clear();
            }
            known.insert(key, price);
        }
        price
    }
}

/// Whether the slot is still free, from what GotCourts said last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Now {
    Free { checked: i64 },
    Taken { checked: i64 },
    Over,
    Unknown,
}

pub fn now(day: Option<&db::Day>, shared: &Shared, today: &str, seconds_today: i64) -> Now {
    if shared.date.as_str() < today || (shared.date == today && shared.start <= seconds_today) {
        return Now::Over;
    }
    let Some(day) = day else { return Now::Unknown };
    if !day.courts.0.iter().any(|court| court.id == shared.court) {
        return Now::Unknown;
    }
    let taken = day.busy.0.iter().any(|busy| {
        busy.court.is_none_or(|id| id == shared.court) && busy.start < shared.end && busy.end > shared.start
    });
    if taken {
        Now::Taken {
            checked: day.fetched_at,
        }
    } else {
        Now::Free {
            checked: day.fetched_at,
        }
    }
}

fn maps(club: &db::Club) -> String {
    let place = format!("{}, {}, {} {}", club.name, club.address, club.zip, club.city);
    let query: String = url::form_urlencoded::byte_serialize(place.as_bytes()).collect();
    format!("https://www.google.com/maps/search/?api=1&query={query}")
}

path_param!(code);

type Found = (Shared, db::Club, Court, Option<db::Day>);

/// The slot, club, court and stored day a link's code leads to.
async fn find(db: &Db, key: &str, code: &str) -> Result<Option<Found>> {
    let Some(shared) = read(key, code) else { return Ok(None) };
    let Some(club) = db.club(shared.club).await.map_err(topcoat::Error::msg)? else {
        return Ok(None);
    };
    let day = db.day(shared.club, &shared.date).await.map_err(topcoat::Error::msg)?;
    let court = day
        .as_ref()
        .and_then(|day| day.courts.0.iter().find(|c| c.id == shared.court).cloned())
        .or_else(|| club.courts.0.iter().find(|c| c.id == shared.court).cloned());
    Ok(court.map(|court| (shared, club, court, day)))
}

#[route(GET "/s/{code}")]
async fn shared_slot(cx: &Cx) -> Result<(StatusCode, Html<String>)> {
    let code = path_param::<Code>(cx).to_string();
    let key = app_context::<crate::auth::Key>(cx).0.clone();
    let db = app_context::<Db>(cx).clone();
    let Some((shared, club, court, day)) = find(&db, &key, &code).await? else {
        let html = view! {
            <!DOCTYPE html>
            <html lang="en">
                <head>
                    <meta charset="utf-8">
                    <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
                    <meta name="color-scheme" content="dark">
                    <meta name="robots" content="noindex">
                    <title>"Courts"</title>
                    <link rel="icon" href=(crate::layout::ICON_URL) type="image/svg+xml">
                    <link rel="stylesheet" href=(crate::layout::STYLE)>
                </head>
                <body>
                    <main class="shared">
                        <article class="slot">
                            <h1>"This link doesn't lead to a court"</h1>
                            <p class="quiet">"It may have been cut off when it was copied. Ask whoever sent it for the whole link."</p>
                        </article>
                    </main>
                </body>
            </html>
        }
        .single()
        .await?
        .render(cx);
        return Ok((StatusCode::NOT_FOUND, Html(html)));
    };
    let state = now(day.as_ref(), &shared, &time::today(), time::seconds_today());
    let price = app_context::<Prices>(cx).of(&shared, &club, &court).await;
    let origin = app_context::<crate::auth::Origin>(cx)
        .0
        .trim_end_matches('/')
        .to_string();
    let here = format!("{origin}{}", path(&key, &shared));
    let heading = title(&court.sport, &shared, &club.name);
    let sport = crate::pages::sport_name(&court.sport);
    let date = time::parse(&shared.date).map_or(shared.date.clone(), |d| d.strftime("%A %-d %B").to_string());
    let hours = format!("{}–{}", clock(shared.start), clock(shared.end));
    let where_ = format!("{}, {} {}", club.address, club.zip, club.city);
    let place = gotcourts::cover_label(&court);
    let surface = crate::pages::surface(&court.surface);
    let court_text = format!("{}, {surface}, {place}", court.label);
    let cost = match price {
        Price::Rappen(rappen) => format!("{} for {}", francs(rappen), length(shared.minutes())),
        Price::NotListed => "Price not listed".into(),
        Price::Unknown => "Price couldn't be checked just now".into(),
    };
    let short_cost = match price {
        Price::Rappen(rappen) => francs(rappen),
        _ => "price not listed".into(),
    };
    let description = format!("{court_text} · {short_cost} · {where_}");
    let (status_class, status) = match state {
        Now::Free { checked } => ("status free", format!("Still free · checked {}", time::ago(checked))),
        Now::Taken { checked } => (
            "status taken",
            format!("No longer free · checked {}", time::ago(checked)),
        ),
        Now::Over => ("status over", "This slot is over".to_string()),
        Now::Unknown => ("status unknown", "Courts can't see this day right now".to_string()),
    };
    let free = matches!(state, Now::Free { .. });
    let when_class = match state {
        Now::Free { .. } => "when",
        Now::Taken { .. } | Now::Over => "when gone",
        Now::Unknown => "when unsure",
    };
    let source = Source::parse(&club.source);
    let book = source.booking_url(
        &club.canonical,
        Some(&shared.date),
        Some(&court.sport),
        Some(shared.start),
    );
    let book_hint = match source {
        Source::GotCourts => format!(
            "On GotCourts, pick {date}, then {} on {}.",
            clock(shared.start),
            court.label
        ),
        Source::Eversports => format!(
            "Eversports opens on {date}: pick {} on {}.",
            clock(shared.start),
            court.label
        ),
    };
    let book_free = format!("Book on {}", source.name());
    let book_other = format!("Other times on {}", source.name());
    let credit = format!(
        "Shared from Courts, which finds free courts on GotCourts and Eversports. Times and prices are {}'.",
        source.name()
    );
    let map = maps(&club);
    let signed_in = crate::auth::signed_in(cx);
    let html = view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
                <meta name="theme-color" content="#10241a">
                <meta name="color-scheme" content="dark">
                <meta name="robots" content="noindex">
                <title>(heading.as_str())</title>
                <meta name="description" content=(description.as_str())>
                <meta property="og:type" content="website">
                <meta property="og:site_name" content="Courts">
                <meta property="og:title" content=(heading.as_str())>
                <meta property="og:description" content=(description.as_str())>
                <meta property="og:url" content=(here.as_str())>
                <meta name="twitter:card" content="summary">
                <link rel="icon" href=(crate::layout::ICON_URL) type="image/svg+xml">
                <link rel="stylesheet" href=(crate::layout::STYLE)>
            </head>
            <body>
                <main class="shared">
                    <article class="slot">
                        <h1>(headline(&sport, &club.name))</h1>
                        <p class=(when_class)>
                            <span class="date">(date.as_str())</span>
                            <span class="hours">(hours.as_str())</span>
                        </p>
                        <p class=(status_class)><span class="dot" aria-hidden="true"></span>(status.as_str())</p>
                        <dl class="facts">
                            <dt>"Court"</dt>
                            <dd>(court_text.as_str())</dd>
                            <dt>"Cost"</dt>
                            <dd>(cost.as_str())</dd>
                            <dt>"Where"</dt>
                            <dd><a href=(map.as_str()) target="_blank" rel="noopener">(where_.as_str())</a></dd>
                        </dl>
                        <div class="slot-actions">
                            if free {
                                <a class="pill" href=(book.as_str()) target="_blank" rel="noopener">(book_free.as_str())</a>
                            } else {
                                <a class="pill quiet" href=(book.as_str()) target="_blank" rel="noopener">(book_other.as_str())</a>
                            }
                            <a class="pill quiet" href=(map.as_str()) target="_blank" rel="noopener">"Open in Google Maps"</a>
                        </div>
                        if free {
                            <p class="quiet hint">(book_hint.as_str())</p>
                        }
                    </article>
                    <footer class="shared-foot quiet">
                        (credit.as_str())
                        if signed_in {
                            " " <a href="/">"Back to Courts"</a>
                        }
                    </footer>
                </main>
            </body>
        </html>
    }
    .single()
    .await?
    .render(cx);
    Ok((StatusCode::OK, Html(html)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gotcourts::Busy;
    use toasty::Json;

    fn slot() -> Shared {
        Shared {
            club: 20,
            court: 111,
            date: "2026-11-29".into(),
            start: 18 * 3600,
            end: 19 * 3600,
        }
    }

    #[test]
    fn a_link_names_its_slot_and_only_the_key_can_make_one() {
        let link = path("secret key", &slot());
        assert!(link.starts_with("/s/20-111-20261129-1800-60-"), "{link}");
        let code = link.trim_start_matches("/s/");
        assert_eq!(read("secret key", code), Some(slot()));
        assert_eq!(read("other key", code), None);
        let moved = code.replacen("1800", "1700", 1);
        assert_eq!(read("secret key", &moved), None);
        assert_eq!(read("secret key", &code[..code.len() - 1]), None);
        assert_eq!(read("secret key", "20-111-20261129-1800-60"), None);
        assert_eq!(read("secret key", "x"), None);
        let half = Shared {
            end: 18 * 3600 + 90 * 60,
            ..slot()
        };
        let code = path("k", &half);
        assert_eq!(read("k", code.trim_start_matches("/s/")), Some(half));
    }

    #[test]
    fn a_title_reads_well_in_a_preview() {
        assert_eq!(
            title("tennis", &slot(), "TC Riesbach"),
            "Tennis, Sun 29 Nov 18:00–19:00, TC Riesbach"
        );
        assert_eq!(headline("Tennis", "Tennis Lengg"), "Tennis Lengg");
        assert_eq!(
            headline("Padel", "Tennis- und Padelclub Unterengstringen"),
            "Tennis- und Padelclub Unterengstringen"
        );
        assert_eq!(headline("Tennis", "Belvoir Indoors"), "Tennis at Belvoir Indoors");
        assert_eq!(francs(7500), "CHF 75.00");
        assert_eq!(francs(3750), "CHF 37.50");
        assert_eq!(length(60), "1 hour");
        assert_eq!(length(90), "1½ hours");
        assert_eq!(length(120), "2 hours");
        assert_eq!(length(45), "45 minutes");
    }

    fn day(busy: Vec<Busy>) -> db::Day {
        let court = Court {
            id: 111,
            sport: "tennis".into(),
            label: "Platz 5".into(),
            surface: "clay".into(),
            indoor: Some(false),
            cover: "outdoor".into(),
            ball_machine: false,
            order: 1,
            interval: 60,
            durations: vec![60],
            opening: 8 * 3600,
            closing: 22 * 3600,
            cells: Vec::new(),
        };
        db::Day {
            club_id: 20,
            date: "2026-11-29".into(),
            fetched_at: 7,
            courts: Json(vec![court]),
            busy: Json(busy),
        }
    }

    fn busy(court: Option<i64>, from: i64, to: i64) -> Busy {
        Busy {
            court,
            start: from * 3600,
            end: to * 3600,
            note: None,
        }
    }

    #[test]
    fn the_page_says_whether_the_slot_is_still_free() {
        let free = day(vec![busy(Some(111), 17, 18), busy(Some(112), 18, 19)]);
        assert_eq!(now(Some(&free), &slot(), "2026-11-28", 0), Now::Free { checked: 7 });
        let taken = day(vec![busy(Some(111), 18, 19)]);
        assert_eq!(now(Some(&taken), &slot(), "2026-11-28", 0), Now::Taken { checked: 7 });
        let closed = day(vec![busy(None, 0, 24)]);
        assert_eq!(now(Some(&closed), &slot(), "2026-11-28", 0), Now::Taken { checked: 7 });
        assert_eq!(now(None, &slot(), "2026-11-28", 0), Now::Unknown);
        assert_eq!(now(Some(&free), &slot(), "2026-11-29", 18 * 3600), Now::Over);
        assert_eq!(
            now(Some(&free), &slot(), "2026-11-29", 17 * 3600),
            Now::Free { checked: 7 }
        );
        assert_eq!(now(Some(&free), &slot(), "2026-11-30", 0), Now::Over);
        let other = Shared { court: 999, ..slot() };
        assert_eq!(now(Some(&free), &other, "2026-11-28", 0), Now::Unknown);
    }
}
