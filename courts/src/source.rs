//! Where a club's courts are booked: GotCourts or Eversports. Both number
//! their clubs and courts on their own, so this app keeps GotCourts' numbers
//! and moves Eversports' past every number GotCourts could reach.

use crate::slots::clock;

const EVERSPORTS_FROM: i64 = 1_000_000_000;

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
}
