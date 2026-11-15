use crate::audio;
use crate::auth::Person;
use crate::bus::{Bus, NowPlaying};
use crate::manifest::Date;
use crate::screen;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::process::Command;
use tokio::sync::Mutex;

const KEEP: usize = 300;
const LISTED: usize = 100;
const BODY_LIMIT: usize = 6 * 1024 * 1024;
const LONGEST_MESSAGE: usize = 20_000;
const LARGEST_CLIENT: usize = 400 * 1024;
const LARGEST_PICTURE: usize = 4 * 1024 * 1024;
const COMMAND_TIME: Duration = Duration::from_secs(4);
const INDEX: &str = "index.jsonl";
const REVISION: &str = "/etc/semmeltv/revision";

#[derive(Clone)]
pub struct Feedback {
    dir: PathBuf,
    bus: Bus,
    started: Instant,
    grim: Option<String>,
    writing: Arc<Mutex<()>>,
}

impl Feedback {
    pub fn new(dir: PathBuf, bus: Bus, grim: Option<String>) -> Self {
        Self {
            dir,
            bus,
            started: Instant::now(),
            grim,
            writing: Arc::default(),
        }
    }
}

#[derive(Deserialize)]
struct Sent {
    message: String,
    page: Option<String>,
    #[serde(default)]
    client: Value,
    screenshot: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Reporter {
    id: i64,
    name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    id: String,
    at: u64,
    when: String,
    host: String,
    person: Option<Reporter>,
    device: Option<String>,
    page: Option<String>,
    message: String,
    screenshot: Option<String>,
    tv_screenshot: Option<String>,
}

#[derive(Serialize)]
struct Snapshot {
    host: String,
    revision: Option<String>,
    system: Option<String>,
    app_up_s: u64,
    up_s: Option<f64>,
    load: Option<String>,
    memory_available_mb: Option<u64>,
    temperature_c: Option<f64>,
    throttled: Option<String>,
    now_playing: Option<NowPlaying>,
    screen: screen::View,
    audio: Option<audio::View>,
    app_journal: Vec<String>,
    services_journal: Vec<String>,
    kernel_journal: Vec<String>,
    screenshot: Option<String>,
    problems: Vec<String>,
}

#[derive(Serialize)]
struct Report<'a> {
    #[serde(flatten)]
    summary: &'a Summary,
    client: Value,
    tv: Snapshot,
}

fn civil(seconds: u64) -> (String, String) {
    let date = Date::from_unix(seconds);
    let of_day = seconds % 86_400;
    let (hour, minute, second) = (of_day / 3600, of_day / 60 % 60, of_day % 60);
    let compact = format!("{:04}{:02}{:02}-{hour:02}{minute:02}{second:02}", date.year(), date.month(), date.day());
    let spoken = format!("{:04}-{:02}-{:02} {hour:02}:{minute:02}:{second:02} UTC", date.year(), date.month(), date.day());
    (compact, spoken)
}

fn new_id(seconds: u64) -> String {
    let mut bytes = [0u8; 2];
    rand::fill(&mut bytes);
    format!("{}-{:02x}{:02x}", civil(seconds).0, bytes[0], bytes[1])
}

fn valid_id(id: &str) -> bool {
    id.len() <= 32 && !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn cut(text: &str, most: usize) -> String {
    match text.char_indices().nth(most) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

fn trim_client(mut client: Value) -> Value {
    let size = |value: &Value| serde_json::to_vec(value).map_or(0, |bytes| bytes.len());
    while size(&client) > LARGEST_CLIENT {
        match client.get_mut("trail").and_then(Value::as_array_mut) {
            Some(trail) if !trail.is_empty() => {
                let drop = trail.len().div_ceil(4);
                trail.drain(..drop);
            }
            _ => return json!({ "dropped": "the client's diagnostics were too large" }),
        }
    }
    client
}

fn picture(data_url: &str) -> Result<(&'static str, Vec<u8>), String> {
    let (head, data) = data_url.split_once(',').ok_or("the screenshot is not a data URL")?;
    let extension = match head {
        "data:image/jpeg;base64" => "jpg",
        "data:image/png;base64" => "png",
        _ => return Err(format!("the screenshot is {head}, not a JPEG or PNG")),
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|error| format!("the screenshot is not base64: {error}"))?;
    if bytes.len() > LARGEST_PICTURE {
        return Err("the screenshot is larger than 4 MB".to_string());
    }
    Ok((extension, bytes))
}

async fn run(program: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(COMMAND_TIME, output).await {
        Err(_) => Err(format!("{program} took longer than {} s", COMMAND_TIME.as_secs())),
        Ok(Err(error)) => Err(format!("{program}: {error}")),
        Ok(Ok(output)) if !output.status.success() => Err(format!(
            "{program} {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Ok(Ok(output)) => Ok(output.stdout),
    }
}

async fn journal(args: &[&str], problems: &mut Vec<String>) -> Vec<String> {
    let mut all = vec!["--no-pager", "-o", "short-iso", "--since", "-20min"];
    all.extend_from_slice(args);
    match run("journalctl", &all).await {
        Ok(out) => String::from_utf8_lossy(&out).lines().map(|line| cut(line, 1000)).collect(),
        Err(problem) => {
            problems.push(problem);
            Vec::new()
        }
    }
}

async fn read(path: &str) -> Option<String> {
    tokio::fs::read_to_string(path).await.ok().map(|text| text.trim().to_string())
}

fn meminfo_available(meminfo: &str) -> Option<u64> {
    let line = meminfo.lines().find(|line| line.starts_with("MemAvailable:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

impl Feedback {
    async fn snapshot(&self, id: &str) -> Snapshot {
        let mut problems = Vec::new();
        let app_journal = journal(&["-t", "cage", "-n", "250"], &mut problems).await;
        let services_journal = journal(&["-u", "semmeltv-web", "-u", "semmeltv-mount", "-u", "nginx", "-n", "80"], &mut problems).await;
        let kernel_journal = journal(&["-k", "-p", "warning", "-n", "30"], &mut problems).await;
        let screenshot = match &self.grim {
            Some(grim) => {
                let name = format!("{id}-tv.jpg");
                let path = self.dir.join(&name);
                let path = path.to_string_lossy();
                match run(grim, &["-s", "0.5", "-t", "jpeg", "-q", "70", &path]).await {
                    Ok(_) => Some(name),
                    Err(problem) => {
                        problems.push(problem);
                        None
                    }
                }
            }
            None => None,
        };
        Snapshot {
            host: read("/proc/sys/kernel/hostname").await.unwrap_or_default(),
            revision: read(REVISION).await,
            system: tokio::fs::read_link("/run/current-system")
                .await
                .ok()
                .map(|path| path.display().to_string()),
            app_up_s: self.started.elapsed().as_secs(),
            up_s: read("/proc/uptime")
                .await
                .and_then(|text| text.split_whitespace().next()?.parse().ok()),
            load: read("/proc/loadavg").await,
            memory_available_mb: read("/proc/meminfo").await.as_deref().and_then(meminfo_available),
            temperature_c: read("/sys/class/thermal/thermal_zone0/temp")
                .await
                .and_then(|text| text.parse::<f64>().ok())
                .map(|milli| milli / 1000.0),
            throttled: read("/sys/devices/platform/soc/soc:firmware/get_throttled").await,
            now_playing: self.bus.now_playing(),
            screen: self.bus.screen_state(),
            audio: self.bus.audio_state(),
            app_journal,
            services_journal,
            kernel_journal,
            screenshot,
            problems,
        }
    }

    async fn store(&self, person: Option<Person>, sent: Sent) -> Result<Summary, String> {
        let message = cut(sent.message.trim(), LONGEST_MESSAGE);
        if message.is_empty() && sent.screenshot.is_none() {
            return Err("Write what happened first".to_string());
        }
        tokio::fs::create_dir_all(&self.dir)
            .await
            .map_err(|error| format!("{}: {error}", self.dir.display()))?;
        let at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
        let id = new_id(at);
        let mut problems = Vec::new();
        let screenshot = match sent.screenshot.as_deref().map(picture) {
            Some(Ok((extension, bytes))) => {
                let name = format!("{id}-phone.{extension}");
                match tokio::fs::write(self.dir.join(&name), bytes).await {
                    Ok(()) => Some(name),
                    Err(error) => {
                        problems.push(format!("saving the screenshot: {error}"));
                        None
                    }
                }
            }
            Some(Err(problem)) => {
                problems.push(problem);
                None
            }
            None => None,
        };
        let client = trim_client(sent.client);
        let mut tv = self.snapshot(&id).await;
        tv.problems.extend(problems);
        let summary = Summary {
            id: id.clone(),
            at,
            when: civil(at).1,
            host: tv.host.clone(),
            person: person.map(|person| Reporter {
                id: person.id.0,
                name: person.name,
            }),
            device: client.get("device").and_then(Value::as_str).map(|device| cut(device, 80)),
            page: sent.page.map(|page| cut(&page, 300)),
            message,
            screenshot,
            tv_screenshot: tv.screenshot.clone(),
        };
        let report = Report {
            summary: &summary,
            client,
            tv,
        };
        let body = serde_json::to_vec_pretty(&report).map_err(|error| error.to_string())?;
        let _writing = self.writing.lock().await;
        tokio::fs::write(self.dir.join(format!("{id}.json")), body)
            .await
            .map_err(|error| format!("saving the report: {error}"))?;
        let mut index = self.summaries().await;
        index.push(summary.clone());
        self.prune(index).await;
        Ok(summary)
    }

    async fn summaries(&self) -> Vec<Summary> {
        let text = tokio::fs::read_to_string(self.dir.join(INDEX)).await.unwrap_or_default();
        text.lines().filter_map(|line| serde_json::from_str(line).ok()).collect()
    }

    async fn prune(&self, mut index: Vec<Summary>) {
        let excess = index.len().saturating_sub(KEEP);
        for old in index.drain(..excess) {
            let files = [Some(format!("{}.json", old.id)), old.screenshot, old.tv_screenshot];
            for file in files.into_iter().flatten() {
                let _ = tokio::fs::remove_file(self.dir.join(file)).await;
            }
        }
        let lines: String = index
            .iter()
            .filter_map(|summary| serde_json::to_string(summary).ok())
            .map(|line| line + "\n")
            .collect();
        let temporary = self.dir.join(format!("{INDEX}.new"));
        let written = tokio::fs::write(&temporary, lines).await;
        if let Err(error) = written.and(tokio::fs::rename(&temporary, self.dir.join(INDEX)).await) {
            eprintln!("feedback: writing the index: {error}");
        }
    }
}

async fn send(
    State(feedback): State<Feedback>,
    person: Option<Extension<Person>>,
    Json(sent): Json<Sent>,
) -> Response {
    let person = person.map(|Extension(person)| person);
    let who = person.as_ref().map_or("someone".to_string(), |person| person.name.clone());
    match feedback.store(person, sent).await {
        Ok(summary) => {
            eprintln!("feedback: {} from {who}: {}", summary.id, cut(&summary.message, 200).replace('\n', " "));
            (StatusCode::CREATED, Json(json!({ "id": summary.id }))).into_response()
        }
        Err(problem) => {
            eprintln!("feedback from {who} not stored: {problem}");
            (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": problem }))).into_response()
        }
    }
}

async fn list(State(feedback): State<Feedback>) -> Json<Value> {
    let mut summaries = feedback.summaries().await;
    summaries.reverse();
    summaries.truncate(LISTED);
    Json(json!({ "dir": feedback.dir, "reports": summaries }))
}

async fn one(State(feedback): State<Feedback>, Path(id): Path<String>) -> Response {
    if !valid_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match tokio::fs::read(feedback.dir.join(format!("{id}.json"))).await {
        Ok(body) => ([(header::CONTENT_TYPE, "application/json")], body).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn file(State(feedback): State<Feedback>, Path(name): Path<String>) -> Response {
    let (stem, extension) = name.rsplit_once('.').unwrap_or((&name, ""));
    let kind = match extension {
        "jpg" => "image/jpeg",
        "png" => "image/png",
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    if !valid_id(stem) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match tokio::fs::read(feedback.dir.join(&name)).await {
        Ok(body) => ([(header::CONTENT_TYPE, kind), (header::CACHE_CONTROL, "private, max-age=86400")], body).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

pub fn routes<S: Clone + Send + Sync + 'static>(feedback: Feedback) -> Router<S> {
    Router::new()
        .route("/api/feedback", get(list).post(send))
        .route("/api/feedback/{id}", get(one))
        .route("/api/feedback/files/{name}", get(file))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(feedback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_sort_by_time_and_say_it() {
        assert_eq!(civil(1_794_748_331), ("20261115-131211".to_string(), "2026-11-15 13:12:11 UTC".to_string()));
        let id = new_id(1_794_748_331);
        assert!(id.starts_with("20261115-131211-") && valid_id(&id));
    }

    #[test]
    fn only_plain_ids_name_files() {
        assert!(!valid_id("../semmeltv"));
        assert!(!valid_id("a/b"));
        assert!(!valid_id(""));
    }

    #[test]
    fn screenshots_are_jpeg_or_png_data_urls() {
        assert_eq!(picture("data:image/png;base64,aGk=").unwrap(), ("png", b"hi".to_vec()));
        assert!(picture("data:image/gif;base64,aGk=").is_err());
        assert!(picture("hello").is_err());
    }

    #[test]
    fn an_oversized_trail_loses_its_oldest_entries() {
        let entry = "x".repeat(1000);
        let trail: Vec<Value> = (0..1000).map(|index| json!({ "i": index, "m": entry })).collect();
        let client = trim_client(json!({ "device": "iPhone", "trail": trail }));
        let kept = client["trail"].as_array().unwrap();
        assert!(serde_json::to_vec(&client).unwrap().len() <= LARGEST_CLIENT);
        assert_eq!(kept.last().unwrap()["i"], 999);
        assert_eq!(client["device"], "iPhone");
    }

    #[test]
    fn available_memory_is_read_in_megabytes() {
        assert_eq!(meminfo_available("MemTotal: 100 kB\nMemAvailable:    2048000 kB\n"), Some(2000));
    }
}
