//! The JSON GotCourts' own booking pages load. It has no documentation and
//! no key: `/api/public/clubs/{id}/reservations?date=` answers for one club
//! and one day with the club, its courts and what is taken.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

const BASE: &str = "https://apps.gotcourts.com";

/// A court as GotCourts describes it for one day. Times are seconds after
/// midnight in Zurich.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Court {
    pub id: i64,
    pub label: String,
    pub surface: String,
    pub indoor: bool,
    pub ball_machine: bool,
    pub order: i64,
    pub interval: i64,
    pub durations: Vec<i64>,
    pub opening: i64,
    pub closing: i64,
}

/// A stretch of one court that can't be booked: someone's reservation, or a
/// rule of the club's, which `note` explains.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Busy {
    pub court: Option<i64>,
    pub start: i64,
    pub end: i64,
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Club {
    pub id: i64,
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
    /// The club, its courts and everything taken on `date` (YYYY-MM-DD).
    pub async fn day(&self, club: i64, date: &str) -> Result<Day, Error> {
        let agent = self.agent.clone();
        let url = format!("{BASE}/en/api/public/clubs/{club}/reservations?date={date}");
        let body = tokio::task::spawn_blocking(move || -> Result<Value, Error> {
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
        .map_err(|error| Error::Failed(error.to_string()))??;
        parse(&body)
    }

    /// Where someone books at `club` on `date`, on GotCourts' own page.
    pub fn booking_url(canonical: &str, date: &str) -> String {
        match crate::time::midnight(date) {
            Some(midnight) => format!("{BASE}/de/reservations/{canonical}?date={midnight}"),
            None => format!("{BASE}/de/reservations/{canonical}"),
        }
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
                label: text(&court["label"]).split_whitespace().collect::<Vec<_>>().join(" "),
                surface: text(&court["surfaceType"]),
                indoor: text(&court["type"]) != "outdoor",
                ball_machine: court["hasBallMachine"].as_bool().unwrap_or(false),
                order: int(&court["orderNumber"]).unwrap_or(0),
                interval: int(&court["interval"]).filter(|minutes| *minutes > 0).unwrap_or(60),
                durations: court["durations"]
                    .as_array()
                    .map(|list| list.iter().filter_map(int).collect())
                    .unwrap_or_default(),
                opening: int(&court["openingTime"]).unwrap_or(0),
                closing: int(&court["closingTime"]).unwrap_or(0),
            })
        })
        .collect();
    courts.sort_by_key(|court| (court.order, court.id));
    let reservations = response["reservations"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|taken| (taken, None));
    let blockings = response["blockings"].as_array().into_iter().flatten().map(|taken| {
        let note = text(&taken["text"]).replace("<br>", " ");
        (taken, Some(note).filter(|note| !note.is_empty()))
    });
    let busy = reservations
        .chain(blockings)
        .filter_map(|(taken, note)| {
            Some(Busy {
                court: int(&taken["courtId"]),
                start: int(&taken["startTime"])?,
                end: int(&taken["endTime"])?,
                note,
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
                "blockings": [{"id": 1, "courtId": 115, "type": "blocking", "startTime": 54000, "endTime": 57600,
                    "text": "No booking from 8:00 to 21:00.<br>Only club members can book."}],
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
        assert!(day.courts[1].indoor && !day.courts[0].indoor);
        assert_eq!(day.courts[0].durations, [60, 90]);
        assert_eq!(day.busy.len(), 2);
        assert_eq!(
            day.busy[1].note.as_deref(),
            Some("No booking from 8:00 to 21:00. Only club members can book.")
        );
    }

    #[test]
    fn a_club_gotcourts_does_not_book_for_is_unknown() {
        let body = serde_json::json!({"status": false, "response": {"error": 515, "message": "Unknown club"}});
        assert!(matches!(parse(&body), Err(Error::Unknown)));
        let body = serde_json::json!({"status": false, "response": {"error": 500, "message": "Fatal error"}});
        assert!(matches!(parse(&body), Err(Error::Failed(why)) if why == "Fatal error"));
    }
}
