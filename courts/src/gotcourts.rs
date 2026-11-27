//! The JSON GotCourts' own booking pages load. It has no documentation and
//! no key: `/api/public/clubs/{id}/reservations?date=` answers for one club
//! and one day with the club, its courts and what is taken.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

const BASE: &str = "https://apps.gotcourts.com";

/// The sports this app is for; beach fields and sports halls ("other") are left out.
pub const SPORTS: [&str; 3] = ["tennis", "padel", "squash"];

/// A court as GotCourts describes it for one day. Times are seconds after
/// midnight in Zurich.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Court {
    pub id: i64,
    #[serde(default = "tennis")]
    pub sport: String,
    pub label: String,
    pub surface: String,
    /// Indoor or outdoor as GotCourts states it; None when it doesn't.
    pub indoor: Option<bool>,
    /// GotCourts' own word for it: indoor, outdoor, balloon (an air hall), lights.
    #[serde(default)]
    pub cover: String,
    pub ball_machine: bool,
    pub order: i64,
    pub interval: i64,
    pub durations: Vec<i64>,
    pub opening: i64,
    pub closing: i64,
    /// The stretches the court is booked in, where they don't follow from
    /// `opening` and `interval` (Eversports lays out each day on its own).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cells: Vec<Cell>,
}

/// One bookable stretch of a court, and what it costs in Rappen when known.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    pub start: i64,
    pub end: i64,
    pub price: Option<i64>,
}

fn tennis() -> String {
    "tennis".into()
}

/// Whether a court is indoors, from GotCourts' court type; None when the
/// type is missing or one this app doesn't know.
pub fn covered(cover: &str) -> Option<bool> {
    match cover {
        "indoor" | "balloon" => Some(true),
        "outdoor" | "lights" => Some(false),
        _ => None,
    }
}

/// How a court's cover reads on the page.
pub fn cover_label(court: &Court) -> &'static str {
    match (court.cover.as_str(), court.indoor) {
        ("balloon", _) => "air hall",
        ("lights", _) => "outdoor, lit",
        (_, Some(true)) => "indoor",
        (_, Some(false)) => "outdoor",
        (_, None) => "indoor or outdoor not stated",
    }
}

/// A stretch of one court that can't be booked: someone's reservation, or a
/// rule of the club's, which `note` explains.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Busy {
    pub court: Option<i64>,
    pub start: i64,
    pub end: i64,
    pub note: Option<String>,
    /// A club rule that only keeps out who isn't a member ("Only club
    /// members can create reservations…"): free for the club's members.
    #[serde(default, skip_serializing_if = "is_false")]
    pub members: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

#[derive(Clone, Debug, PartialEq)]
pub struct Club {
    pub id: i64,
    pub source: crate::source::Source,
    pub name: String,
    pub canonical: String,
    pub address: String,
    pub zip: String,
    pub city: String,
    pub latitude: f64,
    pub longitude: f64,
    pub phone: String,
    pub web: String,
    pub kind: String,
}

/// A club on GotCourts' map. `books` is whether GotCourts takes bookings
/// for it (its "syc" flow) rather than only listing it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Listed {
    pub canonical: String,
    /// The app's own name for the club (`clb_…`), which its links take.
    #[serde(default)]
    pub app: String,
    pub name: String,
    pub city: String,
    pub latitude: f64,
    pub longitude: f64,
    pub books: bool,
}

pub fn listing(body: &Value) -> Result<Vec<Listed>, Error> {
    if body.get("status") != Some(&Value::Bool(true)) {
        let response = body.get("response").unwrap_or(body);
        let why = [text(&response["exception"]), text(&response["message"])]
            .into_iter()
            .find(|why| !why.is_empty())
            .unwrap_or_else(|| body.to_string());
        return Err(Error::Failed(why));
    }
    Ok(body["response"]["clubs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|club| {
            Some(Listed {
                canonical: Some(text(&club["canonical_name"])).filter(|c| !c.is_empty())?,
                app: text(&club["club_id"]),
                name: text(&club["club_name"]),
                city: text(&club["city"]),
                latitude: number(&club["latitude"])?,
                longitude: number(&club["longitude"])?,
                books: text(&club["flow"]) == "syc",
            })
        })
        .collect())
}

#[derive(Clone, Debug, PartialEq)]
pub struct Day {
    pub club: Club,
    pub courts: Vec<Court>,
    pub busy: Vec<Busy>,
}

#[derive(Debug)]
pub enum Error {
    /// GotCourts doesn't take bookings for this club.
    Unknown,
    Failed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown => f.write_str("unknown club"),
            Self::Failed(why) => f.write_str(why),
        }
    }
}

#[derive(Clone)]
pub struct GotCourts {
    agent: ureq::Agent,
}

impl Default for GotCourts {
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

impl GotCourts {
    async fn get(&self, url: String) -> Result<Value, Error> {
        let agent = self.agent.clone();
        tokio::task::spawn_blocking(move || -> Result<Value, Error> {
            let mut response = agent
                .get(&url)
                .header("Accept", "application/json")
                .header("X-Requested-With", "XMLHttpRequest")
                .call()
                .map_err(|error| Error::Failed(error.to_string()))?;
            response
                .body_mut()
                .read_json::<Value>()
                .map_err(|error| Error::Failed(format!("not JSON: {error}")))
        })
        .await
        .map_err(|error| Error::Failed(error.to_string()))?
    }

    /// The club, its courts and everything taken on `date` (YYYY-MM-DD).
    pub async fn day(&self, club: i64, date: &str) -> Result<Day, Error> {
        parse(
            &self
                .get(format!("{BASE}/en/api/public/clubs/{club}/reservations?date={date}"))
                .await?,
        )
    }

    /// Every club GotCourts' own map shows within `km` of a point.
    pub async fn listed(&self, latitude: f64, longitude: f64, km: f64) -> Result<Vec<Listed>, Error> {
        let url = format!(
            "{BASE}/en/api2/public/clubs/for/map?location%5Blatitude%5D={latitude:.5}&location%5Blongitude%5D={longitude:.5}&location%5BtimeZone%5D=Europe%2FZurich&location%5Brange%5D={km:.2}"
        );
        listing(&self.get(url).await?)
    }

    /// The number a club's reservations are asked for by, from its map name.
    pub async fn id_of(&self, canonical: &str) -> Result<i64, Error> {
        let body = self
            .get(format!("{BASE}/en/api/public/club/{canonical}/profile"))
            .await?;
        let club = &body.get("response").unwrap_or(&body)["club"];
        int(&club["id"]).ok_or_else(|| Error::Failed(format!("no id for {canonical}")))
    }

    /// What booking `court` on `date` from `start` (seconds after midnight)
    /// for `minutes` costs, in Rappen; None when the club lists no price.
    pub async fn price(&self, court: i64, date: &str, start: i64, minutes: i64) -> Result<Option<i64>, Error> {
        price(
            &self
                .get(format!(
                    "{BASE}/de/api2/public/reservation/price?newFormat=true&courtId={court}&date={date}&time={start}&duration={minutes}&ballMachine=false&withGuest=false"
                ))
                .await?,
        )
    }

    /// Where someone books at a club, on GotCourts' own page.
    pub fn booking_url(canonical: &str) -> String {
        format!("{BASE}/de/reservations/{canonical}")
    }
}

fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.trim().to_string(),
        Value::Number(number) => number.to_string(),
        _ => String::new(),
    }
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn int(value: &Value) -> Option<i64> {
    number(value).map(|number| number as i64)
}

/// The total of a price answer, in Rappen. A total of nothing is a club
/// that set no tariff on GotCourts (ASVZ's courts answer so), not a free
/// court, so it counts as no price.
pub fn price(body: &Value) -> Result<Option<i64>, Error> {
    let response = body.get("response").unwrap_or(body);
    if body.get("status") != Some(&Value::Bool(true)) {
        let message = text(&response["message"]);
        return Err(Error::Failed(if message.is_empty() {
            body.to_string()
        } else {
            message
        }));
    }
    let currency = text(&response["currency"]);
    if !currency.is_empty() && !currency.eq_ignore_ascii_case("CHF") {
        return Ok(None);
    }
    Ok(number(&response["total"])
        .map(|francs| (francs * 100.0).round() as i64)
        .filter(|rappen| *rappen > 0))
}

pub fn parse(body: &Value) -> Result<Day, Error> {
    let response = body.get("response").unwrap_or(body);
    if body.get("status") == Some(&Value::Bool(false)) {
        let message = text(&response["message"]);
        return Err(if message == "Unknown club" {
            Error::Unknown
        } else {
            Error::Failed(if message.is_empty() { body.to_string() } else { message })
        });
    }
    let club = &response["club"];
    let id = int(&club["id"]).ok_or_else(|| Error::Failed("no club in the answer".into()))?;
    let address = [text(&club["address1"]), text(&club["address2"])]
        .into_iter()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    let club_info = Club {
        id,
        source: crate::source::Source::GotCourts,
        name: text(&club["name"]),
        canonical: text(&club["canonicalName"]),
        address,
        zip: text(&club["zipCode"]),
        city: text(&club["city"]),
        latitude: number(&club["latitude"]).unwrap_or(0.0),
        longitude: number(&club["longitude"]).unwrap_or(0.0),
        phone: text(&club["phone"]),
        web: text(&club["web"]),
        kind: text(&club["clubType"]),
    };
    let mut courts: Vec<Court> = club["courts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|court| {
            Some(Court {
                id: int(&court["id"])?,
                sport: Some(text(&court["sport"]).to_lowercase())
                    .filter(|sport| !sport.is_empty())
                    .unwrap_or_else(tennis),
                label: text(&court["label"]).split_whitespace().collect::<Vec<_>>().join(" "),
                surface: text(&court["surfaceType"]),
                indoor: covered(&text(&court["type"])),
                cover: text(&court["type"]),
                ball_machine: court["hasBallMachine"].as_bool().unwrap_or(false),
                order: int(&court["orderNumber"]).unwrap_or(0),
                interval: int(&court["interval"]).filter(|minutes| *minutes > 0).unwrap_or(60),
                durations: court["durations"]
                    .as_array()
                    .map(|list| list.iter().filter_map(int).collect())
                    .unwrap_or_default(),
                opening: int(&court["openingTime"]).unwrap_or(0),
                closing: int(&court["closingTime"]).unwrap_or(0),
                cells: Vec::new(),
            })
        })
        .collect();
    courts.retain(|court| SPORTS.contains(&court.sport.as_str()));
    courts.sort_by_key(|court| (court.order, court.id));
    let reservations = response["reservations"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|taken| (taken, None, false));
    let blockings = response["blockings"].as_array().into_iter().flatten().map(|taken| {
        let note = text(&taken["text"]).replace("<br>", " ");
        let rule = text(&taken["blockingType"]) == "rule";
        (taken, Some(note).filter(|note| !note.is_empty()), rule)
    });
    let busy = reservations
        .chain(blockings)
        .filter_map(|(taken, note, members)| {
            Some(Busy {
                court: int(&taken["courtId"]),
                start: int(&taken["startTime"])?,
                end: int(&taken["endTime"])?,
                note,
                members,
            })
        })
        .collect();
    Ok(Day {
        club: club_info,
        courts,
        busy,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_day_reads_the_club_its_courts_and_what_is_taken() {
        let body = serde_json::json!({
            "status": true,
            "response": {
                "blockings": [{"id": 1, "courtId": 115, "type": "blocking", "blockingType": "rule", "startTime": 54000, "endTime": 57600,
                    "text": "No booking from 8:00 to 21:00.<br>Only club members can book."},
                    {"courtId": 111, "startTime": 28800, "endTime": 79200, "shortDesc": "Winterpause", "type": "other"}],
                "reservations": [{"id": 9, "courtId": 111, "startTime": 32400, "endTime": 36000, "type": "club"}],
                "club": {"id": 20, "name": "Tennis Lengg", "canonicalName": "tennis-lengg", "address1": "Lengg 1",
                    "address2": "", "zipCode": "8008", "city": "Zürich", "latitude": "47.35", "longitude": "8.57",
                    "phone": " 044 000 ", "web": "", "clubType": "public",
                    "courts": [
                        {"id": 115, "label": "Halle  Platz 6", "surfaceType": "rebound", "hasBallMachine": false,
                         "orderNumber": 2, "type": "indoor", "interval": 60, "durations": [60],
                         "openingTime": 25200, "closingTime": 79200},
                        {"id": 111, "label": "Platz 5", "surfaceType": "clay", "hasBallMachine": true,
                         "orderNumber": 1, "type": "outdoor", "interval": 30, "durations": [60, 90],
                         "openingTime": 28800, "closingTime": 75600}
                    ]}
            }
        });
        let day = parse(&body).unwrap();
        assert_eq!(day.club.name, "Tennis Lengg");
        assert_eq!(day.club.phone, "044 000");
        assert_eq!(day.club.latitude, 47.35);
        assert_eq!(day.courts.iter().map(|c| c.id).collect::<Vec<_>>(), [111, 115]);
        assert_eq!(day.courts[1].label, "Halle Platz 6");
        assert_eq!((day.courts[1].indoor, day.courts[0].indoor), (Some(true), Some(false)));
        assert_eq!(covered("balloon"), Some(true));
        assert_eq!(covered(""), None);
        assert_eq!(covered("hall"), None);
        assert_eq!(day.courts[0].durations, [60, 90]);
        assert_eq!(day.busy.len(), 3);
        assert_eq!(
            day.busy[1].note.as_deref(),
            Some("No booking from 8:00 to 21:00. Only club members can book.")
        );
        assert_eq!(
            day.busy.iter().map(|busy| busy.members).collect::<Vec<_>>(),
            [false, true, false],
            "a rule keeps out only who isn't a member; a reservation or a closure keeps out everybody"
        );
    }

    #[test]
    fn the_map_lists_clubs_and_whether_gotcourts_books_them() {
        let body = serde_json::json!({"status": true, "response": {"clubs": [
            {"club_id": "clb_1", "club_name": "TA Hardhof", "city": "Zürich", "canonical_name": "hardhof-sportamt",
             "flow": "syc", "latitude": "47.39", "longitude": "8.49"},
            {"club_id": "clb_2", "club_name": "Heidi", "city": "Zürich", "canonical_name": "heidi",
             "flow": "ncc", "latitude": "47.37", "longitude": "8.50"}]}});
        let listed = listing(&body).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed[0].books && !listed[1].books);
        assert_eq!(listed[0].canonical, "hardhof-sportamt");
        assert_eq!(listed[0].app, "clb_1");
        let broken = serde_json::json!({"status": false, "response": {"error": 500, "message": "",
            "exception": "Doctrine\\ORM\\NonUniqueResultException"}});
        assert!(matches!(listing(&broken), Err(Error::Failed(why)) if why.contains("NonUnique")));
    }

    #[test]
    fn a_price_is_the_total_in_rappen_and_nothing_is_no_price() {
        let answer = |total: &str| {
            serde_json::json!({"status": true, "response": {"currency": "CHF", "total": total, "additional": [],
                "params": {"cancelDateTime": false, "transactionFee": 0.05}}})
        };
        assert_eq!(price(&answer("75.00")).unwrap(), Some(7500));
        assert_eq!(price(&answer("37.50")).unwrap(), Some(3750));
        assert_eq!(price(&answer("0.00")).unwrap(), None);
        let euros = serde_json::json!({"status": true, "response": {"currency": "EUR", "total": "20.00"}});
        assert_eq!(price(&euros).unwrap(), None);
        let broken = serde_json::json!({"status": false, "response": {"error": 500,
            "message": "Fatal error: Call to a member function getClub() on null"}});
        assert!(matches!(price(&broken), Err(Error::Failed(why)) if why.contains("getClub")));
    }

    #[test]
    fn a_club_gotcourts_does_not_book_for_is_unknown() {
        let body = serde_json::json!({"status": false, "response": {"error": 515, "message": "Unknown club"}});
        assert!(matches!(parse(&body), Err(Error::Unknown)));
        let body = serde_json::json!({"status": false, "response": {"error": 500, "message": "Fatal error"}});
        assert!(matches!(parse(&body), Err(Error::Failed(why)) if why == "Fatal error"));
    }
}
