//! What the day page shows: when, how long, which sport, where, for whom.
//! The query string carries it; the last one used is kept in a cookie, so
//! the page opens with Frederik's usual search (but always on today).

use crate::time;
use std::collections::{BTreeSet, HashMap};
use topcoat::context::Cx;
use topcoat::cookie::{Cookies, SameSite, cookie, cookies, time::Duration};
use topcoat::router::request::uri;

const COOKIE: &str = "courts_filters";
const DAYS: &str = "DAYS";
pub const LENGTHS: [(i64, &str); 3] = [(60, "1 h"), (90, "1½ h"), (120, "2 h")];
pub const WITHIN: [i64; 5] = [2, 3, 5, 10, 15];
/// The most an hour may cost, in francs; 0 is any price.
pub const PRICES: [i64; 4] = [30, 40, 50, 60];
pub const PRESETS: [(&str, i64, i64); 4] = [
    ("Morning", 7, 12),
    ("Midday", 11, 15),
    ("Afternoon", 14, 18),
    ("Evening", 17, 22),
];
pub const EARLIEST: i64 = 6;
pub const LATEST: i64 = 23;

#[derive(Clone, Debug, PartialEq)]
pub struct Filters {
    /// The days to search, in order; at least one.
    pub dates: Vec<String>,
    pub sport: String,
    pub place: String,
    pub surface: String,
    pub from: i64,
    pub until: i64,
    pub length: i64,
    pub within: i64,
    /// The most an hour may cost, in francs; 0 is any price. A slot
    /// without a known price shows under every limit.
    pub price: i64,
    /// The clubs the person is a member of, from the store, not the query.
    pub member_of: BTreeSet<i64>,
    pub near: Option<(f64, f64)>,
    pub sort: String,
    pub view: String,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            dates: vec![time::today()],
            sport: "tennis".into(),
            place: String::new(),
            surface: String::new(),
            from: EARLIEST,
            until: LATEST,
            length: 60,
            within: 0,
            price: 0,
            member_of: BTreeSet::new(),
            near: None,
            sort: "soon".into(),
            view: "list".into(),
        }
    }
}

fn pairs(query: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(query.as_bytes()).into_owned().collect()
}

impl Filters {
    /// From a query string over defaults, keeping only what makes sense.
    pub fn parse(query: &str, dates: &[String], sports: &[String]) -> Self {
        let given = pairs(query);
        let mut filters = Self::default();
        let get = |name: &str| given.get(name).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        let hour = |name: &str| get(name).and_then(|v| v.parse::<i64>().ok());
        let mut chosen: Vec<String> = given
            .get("date")
            .map(|list| {
                list.split(',')
                    .map(|d| d.trim().to_string())
                    .filter(|d| dates.contains(d))
                    .collect()
            })
            .unwrap_or_default();
        chosen.sort();
        chosen.dedup();
        if !chosen.is_empty() {
            filters.dates = chosen;
        }
        if let Some(sport) = get("sport").filter(|s| sports.contains(s)) {
            filters.sport = sport;
        } else if !sports.contains(&filters.sport) {
            filters.sport = sports.first().cloned().unwrap_or_default();
        }
        filters.place = get("place")
            .filter(|p| p == "indoor" || p == "outdoor" || p == "unknown")
            .unwrap_or_default();
        filters.surface = get("surface").unwrap_or_default();
        if let Some((from, until)) = get("when").and_then(|w| {
            let (a, b) = w.split_once('-')?;
            Some((a.parse().ok()?, b.parse().ok()?))
        }) {
            filters.from = from;
            filters.until = until;
        } else {
            filters.from = hour("from").unwrap_or(EARLIEST);
            filters.until = hour("until").unwrap_or(LATEST);
        }
        filters.from = filters.from.clamp(0, 23);
        filters.until = filters.until.clamp(filters.from + 1, 24);
        filters.length = hour("length")
            .filter(|l| LENGTHS.iter().any(|(m, _)| m == l))
            .unwrap_or(60);
        filters.within = hour("within").filter(|w| WITHIN.contains(w)).unwrap_or(0);
        filters.price = hour("price").filter(|p| PRICES.contains(p)).unwrap_or(0);
        filters.near = get("near").and_then(|n| {
            let (lat, lng) = n.split_once(',')?;
            let (lat, lng): (f64, f64) = (lat.parse().ok()?, lng.parse().ok()?);
            (45.0..48.5).contains(&lat).then_some(())?;
            (5.0..11.0).contains(&lng).then_some((lat, lng))
        });
        filters.sort = get("sort").filter(|s| s == "near").unwrap_or_else(|| "soon".into());
        filters.view = get("view").filter(|v| v == "map").unwrap_or_else(|| "list".into());
        filters
    }

    /// The request's filters: its query if the form sent it, else the
    /// remembered ones with the query's day and view.
    pub fn of(cx: &Cx, dates: &[String], sports: &[String]) -> Self {
        let query = uri(cx).query().unwrap_or("").to_string();
        let given = pairs(&query);
        let remembered = cookies(cx)
            .get(COOKIE)
            .map(|c| c.value().to_string())
            .unwrap_or_default();
        let base = if given.contains_key("f") {
            query.clone()
        } else {
            let mut merged = pairs(&remembered);
            for key in ["date", "view"] {
                if let Some(value) = given.get(key) {
                    merged.insert(key.into(), value.clone());
                }
            }
            url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(merged)
                .finish()
        };
        let filters = Self::parse(&base, dates, sports);
        let keep = filters.query(false);
        if keep != remembered {
            cookies(cx).add(cookie! {
                COOKIE = keep;
                Path = "/";
                Secure;
                HttpOnly;
                SameSite = SameSite::Lax;
                MaxAge = Duration::days(400)
            });
        }
        filters
    }

    /// The query string, with or without the day and view.
    pub fn query(&self, all: bool) -> String {
        let mut out = url::form_urlencoded::Serializer::new(String::new());
        out.append_pair("f", "1");
        if all {
            out.append_pair("date", &self.dates.join(","));
            if self.view == "map" {
                out.append_pair("view", "map");
            }
        }
        out.append_pair("sport", &self.sport);
        if !self.place.is_empty() {
            out.append_pair("place", &self.place);
        }
        if !self.surface.is_empty() {
            out.append_pair("surface", &self.surface);
        }
        out.append_pair("from", &self.from.to_string());
        out.append_pair("until", &self.until.to_string());
        out.append_pair("length", &self.length.to_string());
        if self.within > 0 {
            out.append_pair("within", &self.within.to_string());
        }
        if self.price > 0 {
            out.append_pair("price", &self.price.to_string());
        }
        if let Some((lat, lng)) = self.near {
            out.append_pair("near", &format!("{lat:.4},{lng:.4}"));
        }
        if self.sort == "near" {
            out.append_pair("sort", "near");
        }
        out.finish()
    }

    pub fn link(&self) -> String {
        format!("/?{}", self.query(true))
    }

    /// The link a day's chip goes to: that day alone.
    pub fn with_date(&self, date: &str) -> String {
        self.with_dates(vec![date.to_string()])
    }

    pub fn with_dates(&self, dates: Vec<String>) -> String {
        Self { dates, ..self.clone() }.link()
    }

    /// The link to any days, with `DAYS` where they go, comma-separated:
    /// for picking several with Shift or Ctrl.
    pub fn days_template(&self) -> String {
        self.with_dates(vec![DAYS.to_string()])
    }

    pub fn first_date(&self) -> &str {
        &self.dates[0]
    }

    pub fn with_view(&self, view: &str) -> String {
        Self {
            view: view.into(),
            ..self.clone()
        }
        .link()
    }

    /// One line saying what is filtered, for the folded filter bar.
    pub fn summary(&self, sport: &str) -> String {
        let mut parts = vec![sport.to_string()];
        if self.from != EARLIEST || self.until != LATEST {
            parts.push(format!("{}–{}", self.from, self.until));
        }
        parts.push(
            LENGTHS
                .iter()
                .find(|(m, _)| *m == self.length)
                .map_or("1 h", |(_, l)| l)
                .to_string(),
        );
        if !self.place.is_empty() {
            parts.push(
                match self.place.as_str() {
                    "indoor" => "indoor",
                    "outdoor" => "outdoor",
                    _ => "indoor/outdoor not stated",
                }
                .into(),
            );
        }
        if !self.surface.is_empty() {
            parts.push(self.surface.clone());
        }
        if self.within > 0 {
            parts.push(format!("≤ {} km", self.within));
        }
        if self.price > 0 {
            parts.push(format!("≤ CHF {}/h", self.price));
        }
        parts.join(" · ")
    }

    pub fn origin(&self, home: (f64, f64)) -> (f64, f64) {
        self.near.unwrap_or(home)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sports() -> Vec<String> {
        vec!["tennis".into(), "squash".into()]
    }

    #[test]
    fn a_query_round_trips_and_nonsense_falls_back() {
        let dates = time::dates(3);
        let filters = Filters::parse(
            &format!(
                "f=1&date={}&sport=squash&place=indoor&from=17&until=22&length=90&within=5&price=40&near=47.37,8.54&sort=near",
                dates[1]
            ),
            &dates,
            &sports(),
        );
        assert_eq!((filters.sport.as_str(), filters.place.as_str()), ("squash", "indoor"));
        assert_eq!(
            (filters.from, filters.until, filters.length, filters.within),
            (17, 22, 90, 5)
        );
        assert_eq!(filters.price, 40);
        assert_eq!(filters.near, Some((47.37, 8.54)));
        assert_eq!(Filters::parse(&filters.query(true), &dates, &sports()), filters);
        let odd = Filters::parse(
            "sport=golf&length=45&within=7&price=35&from=30&until=2&near=1,2",
            &dates,
            &sports(),
        );
        assert_eq!(
            (odd.sport.as_str(), odd.length, odd.within, odd.near),
            ("tennis", 60, 0, None)
        );
        assert_eq!((odd.from, odd.until), (23, 24));
        assert_eq!(odd.price, 0);
    }

    #[test]
    fn a_day_is_chosen_alone_and_the_template_takes_any_days() {
        let dates = time::dates(4);
        let several = Filters::parse(&format!("date={},{},nonsense", dates[2], dates[0]), &dates, &sports());
        assert_eq!(several.dates, vec![dates[0].clone(), dates[2].clone()]);
        let alone = Filters::parse(&several.with_date(&dates[1])[2..], &dates, &sports());
        assert_eq!(alone.dates, vec![dates[1].clone()]);
        let range = several.days_template().replace("DAYS", &dates[1..].join(","));
        let range = Filters::parse(&range[2..], &dates, &sports());
        assert_eq!(range.dates, dates[1..].to_vec());
        assert_eq!(
            Filters {
                dates: several.dates.clone(),
                ..range
            },
            several
        );
    }

    #[test]
    fn a_preset_sets_the_hours() {
        let filters = Filters::parse("when=17-22&from=8", &time::dates(1), &sports());
        assert_eq!(filters.dates, time::dates(1));
        assert_eq!((filters.from, filters.until), (17, 22));
        assert_eq!(filters.summary("Tennis"), "Tennis · 17–22 · 1 h");
    }
}
