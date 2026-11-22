//! What the front (`semmeltv/front`) says about SemmelTV while it is closed,
//! asked on its control port, so the tv shows the phones' bakery screen and
//! its estimate when it cannot reach the core.

use crate::bus::Panel;
use serde::Deserialize;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// The front's control port unless `SEMMELTV_FRONT_CONTROL` says otherwise.
pub const CONTROL: &str = "127.0.0.1:8094";
const ASK_TIME: Duration = Duration::from_millis(500);
const HEADING: &str = "Der Ofen wird neu angeheizt";
const SOON: &str = "Wieder da in wenigen Sekunden";

#[derive(Debug, PartialEq, Deserialize)]
struct Closed {
    headline: String,
    late: Option<String>,
    eta: String,
}

#[derive(Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Status {
    Open,
    Closed(Closed),
}

#[derive(Deserialize)]
struct Report {
    pages: Status,
}

fn closed(answer: &str) -> Option<Closed> {
    let (head, body) = answer.split_once("\r\n\r\n")?;
    if !head.starts_with("HTTP/1.1 200") {
        return None;
    }
    match serde_json::from_str::<Report>(body).ok()?.pages {
        Status::Closed(closed) => Some(closed),
        Status::Open => None,
    }
}

fn ask(control: &str) -> Option<Closed> {
    let address = control.to_socket_addrs().ok()?.next()?;
    let mut stream = TcpStream::connect_timeout(&address, ASK_TIME).ok()?;
    stream.set_read_timeout(Some(ASK_TIME)).ok()?;
    stream.set_write_timeout(Some(ASK_TIME)).ok()?;
    write!(
        stream,
        "GET /status HTTP/1.1\r\nHost: {control}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut answer = String::new();
    stream.read_to_string(&mut answer).ok()?;
    closed(&answer)
}

fn panel_of(closed: Option<Closed>, hold: Duration) -> Panel {
    match closed {
        Some(Closed { headline, late, eta }) => Panel {
            heading: headline,
            status: late.map_or_else(|| eta.clone(), |late| format!("{late} {eta}")),
            hold,
        },
        None => Panel {
            heading: HEADING.to_string(),
            status: SOON.to_string(),
            hold,
        },
    }
}

/// The panel the tv shows while the core is away: the front's headline and
/// estimate, or that SemmelTV is back in a few seconds when the front has
/// nothing to say.
pub fn panel(control: Option<&str>, hold: Duration) -> Panel {
    panel_of(control.and_then(ask), hold)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOLD: Duration = Duration::from_secs(4);

    fn answer(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    #[test]
    fn a_closed_bakery_shows_the_fronts_headline_and_estimate() {
        let said = answer(
            r#"{"pages":{"state":"closed","cause":"restarting","headline":"Der Ofen wird neu angeheizt","note":"…","late":null,"eta":"Wieder da in etwa 40 Sekunden","until":1},"api":{"state":"open"},"guess_seconds":{"core":40,"web":5}}"#,
        );
        assert_eq!(
            panel_of(closed(&said), HOLD),
            Panel {
                heading: "Der Ofen wird neu angeheizt".to_string(),
                status: "Wieder da in etwa 40 Sekunden".to_string(),
                hold: HOLD,
            }
        );
    }

    #[test]
    fn an_estimate_that_ran_out_says_so_before_the_new_one() {
        let said = answer(
            r#"{"pages":{"state":"closed","headline":"Die Backstube ist kurz zu","late":"Dauert länger als gedacht.","eta":"Wieder da in etwa 5 Minuten"}}"#,
        );
        assert_eq!(
            panel_of(closed(&said), HOLD).status,
            "Dauert länger als gedacht. Wieder da in etwa 5 Minuten"
        );
    }

    #[test]
    fn an_open_front_a_silent_one_or_an_error_shows_the_bakery_back_in_a_few_seconds() {
        let soon = Panel {
            heading: HEADING.to_string(),
            status: SOON.to_string(),
            hold: HOLD,
        };
        assert_eq!(panel_of(closed(&answer(r#"{"pages":{"state":"open"}}"#)), HOLD), soon);
        assert_eq!(panel_of(closed("HTTP/1.1 404 Not Found\r\n\r\n{}"), HOLD), soon);
        assert_eq!(panel(None, HOLD), soon);
    }

    #[test]
    fn the_front_is_asked_on_its_control_port() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let control = listener.local_addr().unwrap().to_string();
        let front = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut asked = Vec::new();
            let mut byte = [0; 1];
            while !asked.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap() == 1 {
                asked.push(byte[0]);
            }
            let body = r#"{"pages":{"state":"closed","headline":"Wir haben es versemmelt","late":null,"eta":"Wieder da in etwa 1 Minute"}}"#;
            stream.write_all(answer(body).as_bytes()).unwrap();
            String::from_utf8(asked).unwrap()
        });
        let shown = panel(Some(&control), HOLD);
        assert!(front.join().unwrap().starts_with("GET /status HTTP/1.1\r\n"));
        assert_eq!(shown.heading, "Wir haben es versemmelt");
        assert_eq!(shown.status, "Wieder da in etwa 1 Minute");
    }
}
