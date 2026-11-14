use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const LINES_PER_MINUTE: u32 = 240;
const MOST_ENTRIES: usize = 50;
const LONGEST_FIELD: usize = 2000;

#[derive(Deserialize)]
struct Batch {
    device: Option<String>,
    agent: Option<String>,
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    level: String,
    message: String,
    detail: Option<String>,
    page: Option<String>,
    at: Option<f64>,
}

#[derive(Default)]
struct Budget {
    since: Option<Instant>,
    lines: u32,
    dropped: u32,
}

impl Budget {
    fn take(&mut self, now: Instant) -> bool {
        if self.since.is_none_or(|since| now.duration_since(since) >= Duration::from_secs(60)) {
            if self.dropped > 0 {
                eprintln!("client: dropped {} lines over the limit of {LINES_PER_MINUTE} a minute", self.dropped);
            }
            *self = Budget { since: Some(now), lines: 0, dropped: 0 };
        }
        if self.lines >= LINES_PER_MINUTE {
            self.dropped += 1;
            return false;
        }
        self.lines += 1;
        true
    }
}

fn clean(text: &str) -> String {
    let flat: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let flat = flat.trim();
    match flat.char_indices().nth(LONGEST_FIELD) {
        Some((cut, _)) => format!("{}…", &flat[..cut]),
        None => flat.to_string(),
    }
}

fn line(device: &str, agent: &str, entry: &Entry) -> String {
    let mut line = format!("client {device}: {} {}", clean(&entry.level), clean(&entry.message));
    if let Some(detail) = entry.detail.as_deref().map(clean).filter(|detail| !detail.is_empty()) {
        line.push_str(&format!(" | {detail}"));
    }
    if let Some(page) = entry.page.as_deref().map(clean).filter(|page| !page.is_empty()) {
        line.push_str(&format!(" | at {page}"));
    }
    if let Some(at) = entry.at.filter(|at| at.is_finite()) {
        line.push_str(&format!(" | t={:.1}s", at / 1000.0));
    }
    if !agent.is_empty() {
        line.push_str(&format!(" | {agent}"));
    }
    line
}

async fn log(State(budget): State<Arc<Mutex<Budget>>>, Json(batch): Json<Batch>) -> StatusCode {
    let device = batch.device.as_deref().map(clean).unwrap_or_else(|| "unknown".to_string());
    let agent = batch.agent.as_deref().map(clean).unwrap_or_default();
    let now = Instant::now();
    for entry in batch.entries.iter().take(MOST_ENTRIES) {
        if !budget.lock().unwrap().take(now) {
            break;
        }
        eprintln!("{}", line(&device, &agent, entry));
    }
    StatusCode::NO_CONTENT
}

pub fn routes<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/api/log", post(log))
        .with_state(Arc::new(Mutex::new(Budget::default())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_names_the_device_and_flattens_what_it_was_sent() {
        let entry = Entry {
            level: "error".to_string(),
            message: "flip refused\nby the TV".to_string(),
            detail: Some("422".to_string()),
            page: Some("/shows/dark".to_string()),
            at: Some(1500.0),
        };
        assert_eq!(
            line("iPhone 1a2b", "Safari", &entry),
            "client iPhone 1a2b: error flip refused by the TV | 422 | at /shows/dark | t=1.5s | Safari"
        );
    }

    #[test]
    fn the_budget_lets_through_a_minutes_worth_and_then_refuses() {
        let mut budget = Budget::default();
        let now = Instant::now();
        assert!((0..LINES_PER_MINUTE).all(|_| budget.take(now)));
        assert!(!budget.take(now));
        assert!(budget.take(now + Duration::from_secs(61)));
    }

    #[test]
    fn long_fields_are_cut() {
        let long = "x".repeat(LONGEST_FIELD + 10);
        assert_eq!(clean(&long).chars().count(), LONGEST_FIELD + 1);
    }
}
