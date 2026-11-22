use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub const USAGE: &str = "usage: semmeltv-maintenance on DURATION [REASON]
       semmeltv-maintenance off
       semmeltv-maintenance status
       semmeltv-maintenance restarting

on closes SemmelTV for planned work: phones and the web app show the
bakery's closed screen with REASON and a countdown from DURATION (20m, 90s,
1h30m; a bare number is minutes). off opens it again. restarting announces
that the core is about to restart, as its unit's ExecStop does.
SEMMELTV_FRONT_CONTROL is the front's control address (default 127.0.0.1:8094).";

pub fn seconds(duration: &str) -> Result<u64, String> {
    let wrong = || format!("{duration:?} is not a duration like 20m, 90s or 1h30m");
    let mut total = 0;
    let mut number: Option<u64> = None;
    for character in duration.trim().chars() {
        if let Some(digit) = character.to_digit(10) {
            number = Some(number.unwrap_or(0) * 10 + u64::from(digit));
            continue;
        }
        let unit = match character {
            's' => 1,
            'm' => 60,
            'h' => 3600,
            _ => return Err(wrong()),
        };
        total += number.take().ok_or_else(wrong)? * unit;
    }
    total += number.unwrap_or(0) * 60;
    if total == 0 { Err(wrong()) } else { Ok(total) }
}

fn ask(control: &str, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
    let fail = |error: std::io::Error| format!("the front at {control} does not answer: {error}");
    let address = control.parse().map_err(|error| format!("{control}: {error}"))?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).map_err(fail)?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).map_err(fail)?;
    let body = body.map(|body| body.to_string()).unwrap_or_default();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {control}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .map_err(fail)?;
    let mut answer = String::new();
    stream.read_to_string(&mut answer).map_err(fail)?;
    let (head, body) = answer.split_once("\r\n\r\n").ok_or("the front's answer has no body")?;
    let report: Value = serde_json::from_str(body).map_err(|error| format!("the front's answer: {error}"))?;
    if head.starts_with("HTTP/1.1 200") {
        Ok(report)
    } else {
        Err(report["error"].as_str().unwrap_or(head).to_string())
    }
}

pub fn describe(report: &Value) -> String {
    let pages = &report["pages"];
    let guess = report["guess_seconds"]["core"].as_u64().unwrap_or(0);
    if pages["state"] == "open" {
        return format!("Open. A restart of the core is guessed to take about {guess} s.");
    }
    let line = |key: &str| pages[key].as_str().map(str::to_string);
    [
        Some(format!("Closed ({}):", pages["cause"].as_str().unwrap_or("?"))),
        line("headline").map(|headline| format!("{headline}.")),
        line("note"),
        line("late"),
        line("eta").map(|eta| format!("{eta}.")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ")
}

pub fn run(arguments: &[String]) -> Result<String, String> {
    let control = std::env::var("SEMMELTV_FRONT_CONTROL").unwrap_or_else(|_| "127.0.0.1:8094".to_string());
    let report = match arguments {
        [on, duration, reason @ ..] if on == "on" => {
            let reason = (!reason.is_empty()).then(|| reason.join(" "));
            ask(
                &control,
                "PUT",
                "/planned",
                Some(json!({ "seconds": seconds(duration)?, "reason": reason })),
            )?
        }
        [off] if off == "off" => ask(&control, "DELETE", "/planned", None)?,
        [status] if status == "status" => ask(&control, "GET", "/status", None)?,
        [restarting] if restarting == "restarting" => ask(&control, "POST", "/restarting", None)?,
        _ => return Err(USAGE.to_string()),
    };
    Ok(describe(&report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_in_seconds_minutes_and_hours() {
        assert_eq!(seconds("90s"), Ok(90));
        assert_eq!(seconds("20m"), Ok(1200));
        assert_eq!(seconds("1h30m"), Ok(5400));
        assert_eq!(seconds("45"), Ok(2700));
    }

    #[test]
    fn a_duration_of_nothing_or_nonsense_is_refused() {
        assert!(seconds("0m").is_err());
        assert!(seconds("m").is_err());
        assert!(seconds("soon").is_err());
        assert!(seconds("").is_err());
    }

    #[test]
    fn an_open_bakery_is_described_with_its_guess() {
        let report = json!({ "pages": { "state": "open" }, "guess_seconds": { "core": 45 } });
        assert_eq!(
            describe(&report),
            "Open. A restart of the core is guessed to take about 45 s."
        );
    }
}
