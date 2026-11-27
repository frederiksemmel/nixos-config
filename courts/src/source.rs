//! Where a club's courts are booked: GotCourts or Eversports. Both number
//! their clubs and courts on their own, so this app keeps GotCourts' numbers
//! and moves Eversports' past every number GotCourts could reach.

use crate::discover::Clubs;
use crate::slots::clock;
use std::collections::BTreeMap;
use topcoat::context::{Cx, app_context};
use topcoat::router::request::headers;

const EVERSPORTS_FROM: i64 = 1_000_000_000;

/// GotCourts' own Branch key, as its Android app ships it. iOS hands a link
/// on got.link to the GotCourts app; without the app, Branch sends the
/// phone on to the App Store.
const GOTCOURTS_BRANCH: &str = "https://got.link/a/key_live_len2iS5oAbcZRntzaijvWdoprDjB1ZgR";

/// The app's names for the clubs (`clb_…`) when the page is read on an
/// iPhone, where Book opens the app; none elsewhere.
pub fn apps(cx: &Cx) -> BTreeMap<i64, String> {
    if on_iphone(cx) {
        app_context::<Clubs>(cx).apps()
    } else {
        BTreeMap::new()
    }
}

/// Whether the page is read on an iPhone, iPad or iPod, by its user agent.
pub fn on_iphone(cx: &Cx) -> bool {
    headers(cx)
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .is_some_and(iphone)
}

fn iphone(user_agent: &str) -> bool {
    ["iPhone", "iPad", "iPod"]
        .iter()
        .any(|device| user_agent.contains(device))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    #[default]
    GotCourts,
    Eversports,
}

impl Source {
    /// How the source is stored.
    pub fn key(self) -> &'static str {
        match self {
            Self::GotCourts => "gotcourts",
            Self::Eversports => "eversports",
        }
    }

    pub fn parse(key: &str) -> Self {
        match key {
            "eversports" => Self::Eversports,
            _ => Self::GotCourts,
        }
    }

    /// How the source reads on the page.
    pub fn name(self) -> &'static str {
        match self {
            Self::GotCourts => "GotCourts",
            Self::Eversports => "Eversports",
        }
    }

    /// This app's number for a club or court the source numbers `theirs`.
    pub fn id(self, theirs: i64) -> i64 {
        match self {
            Self::GotCourts => theirs,
            Self::Eversports => EVERSPORTS_FROM + theirs,
        }
    }

    /// Where someone books in the source's iPhone app at the club its app
    /// names `app` (GotCourts' map gives each club a `clb_…`; its website
    /// number is unknown to the app): GotCourts' app opens on the club's
    /// page for today. None for Eversports, whose site opens on the day and
    /// time, and for a club without that name.
    pub fn app_url(self, app: Option<&str>) -> Option<String> {
        match (self, app) {
            (Self::GotCourts, Some(app)) if !app.is_empty() => {
                let query = url::form_urlencoded::Serializer::new(String::new())
                    .append_pair("club_id", app)
                    .finish();
                Some(format!("{GOTCOURTS_BRANCH}?{query}"))
            }
            _ => None,
        }
    }

    /// Where someone books at the club `canonical` (GotCourts' map name or
    /// Eversports' slug): on Eversports at the day, sport and start when given;
    /// GotCourts' page always opens on today.
    pub fn booking_url(self, canonical: &str, date: Option<&str>, sport: Option<&str>, start: Option<i64>) -> String {
        match self {
            Self::GotCourts => crate::gotcourts::GotCourts::booking_url(canonical),
            Self::Eversports => {
                let mut query = url::form_urlencoded::Serializer::new(String::new());
                if let Some(sport) = sport {
                    query.append_pair("sport", sport);
                }
                if let Some(date) = date {
                    query.append_pair("date", date);
                }
                if let Some(start) = start {
                    query.append_pair("time", &clock(start).replace(':', ""));
                }
                let query = query.finish();
                let base = crate::eversports::booking_url(canonical);
                if query.is_empty() {
                    base
                } else {
                    format!("{base}?{query}")
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eversports_numbers_stay_clear_of_gotcourts_and_links_open_the_slot() {
        assert_eq!(Source::GotCourts.id(53610), 53610);
        assert_eq!(Source::Eversports.id(78306), 1_000_078_306);
        for source in [Source::GotCourts, Source::Eversports] {
            assert_eq!(Source::parse(source.key()), source);
        }
        assert_eq!(
            Source::Eversports.booking_url("sihlsports", Some("2026-12-05"), Some("squash"), Some(18 * 3600 + 900)),
            "https://www.eversports.ch/sb/sihlsports?sport=squash&date=2026-12-05&time=1815"
        );
        assert_eq!(
            Source::GotCourts.booking_url("tennis-lengg", Some("2026-12-05"), None, None),
            "https://apps.gotcourts.com/de/reservations/tennis-lengg"
        );
    }

    #[test]
    fn on_an_iphone_gotcourts_opens_its_app_at_the_club() {
        assert_eq!(
            Source::GotCourts
                .app_url(Some("clb_11ec1c674f3f7c5c9240024b6438f3f4"))
                .as_deref(),
            Some(
                "https://got.link/a/key_live_len2iS5oAbcZRntzaijvWdoprDjB1ZgR?club_id=clb_11ec1c674f3f7c5c9240024b6438f3f4"
            )
        );
        assert_eq!(Source::GotCourts.app_url(None), None);
        assert_eq!(Source::GotCourts.app_url(Some("")), None);
        assert_eq!(Source::Eversports.app_url(Some("clb_1")), None);
        assert!(iphone(
            "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1"
        ));
        assert!(!iphone(
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Safari/537.36"
        ));
        assert!(!iphone(
            "Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Mobile Safari/537.36"
        ));
    }
}
