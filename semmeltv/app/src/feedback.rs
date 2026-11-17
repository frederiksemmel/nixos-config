use crate::audio;
use crate::auth::Person;
use crate::auth::gate::Caller;
use crate::bus::{Bus, NowPlaying};
use crate::manifest::Date;
use crate::screen;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::Mutex;

const KEEP: usize = 300;
const LISTED: usize = 100;
const BODY_LIMIT: usize = 6 * 1024 * 1024;
const LONGEST_MESSAGE: usize = 20_000;
const LARGEST_CLIENT: usize = 400 * 1024;
const LARGEST_PICTURE: usize = 4 * 1024 * 1024;
const COMMAND_TIME: Duration = Duration::from_secs(4);
const LONGEST_NOTE: usize = 4000;
const MOST_QUESTIONS: usize = 6;
const MOST_OPTIONS: usize = 8;
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    New,
    Reopened {
        was: Box<Status>,
    },
    NeedsReview {
        why: String,
        decide: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        questions: Vec<Question>,
    },
    Done {
        resolution: Resolution,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Question {
    ask: String,
    options: Vec<String>,
    recommended: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Resolution {
    Fixed { commit: String, what: String },
    NoChange { why: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Stage {
    New,
    Reopened,
    NeedsReview,
    Done,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Author {
    Person { name: String },
    Claude,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    at: u64,
    author: Author,
    text: String,
}

#[derive(Deserialize)]
struct Said {
    text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Marked {
    status: Status,
    at: u64,
    by: String,
}

#[derive(Serialize)]
struct Listed {
    #[serde(flatten)]
    summary: Summary,
    status: Status,
    marked_at: Option<u64>,
    marked_by: Option<String>,
    comments: Vec<Comment>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

fn note(text: &str, what: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(format!("Say {what}"));
    }
    Ok(cut(text, LONGEST_NOTE))
}

impl Status {
    fn stage(&self) -> Stage {
        match self {
            Status::New => Stage::New,
            Status::Reopened { .. } => Stage::Reopened,
            Status::NeedsReview { .. } => Stage::NeedsReview,
            Status::Done { .. } => Stage::Done,
        }
    }

    fn checked(self) -> Result<Status, String> {
        Ok(match self {
            Status::New => Status::New,
            Status::Reopened { .. } => return Err("A report reopens when a person comments on it".to_string()),
            Status::NeedsReview { why, decide, questions } => {
                if questions.len() > MOST_QUESTIONS {
                    return Err(format!("Ask at most {MOST_QUESTIONS} questions"));
                }
                Status::NeedsReview {
                    why: note(&why, "why it needs a review")?,
                    decide: note(&decide, "what a person has to decide")?,
                    questions: questions.into_iter().map(Question::checked).collect::<Result<_, _>>()?,
                }
            }
            Status::Done {
                resolution: Resolution::Fixed { commit, what },
            } => {
                let commit = commit.trim().to_ascii_lowercase();
                if !(7..=40).contains(&commit.len()) || !commit.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err(format!("{commit:?} is not a commit hash"));
                }
                Status::Done {
                    resolution: Resolution::Fixed {
                        commit,
                        what: note(&what, "what the fix changed")?,
                    },
                }
            }
            Status::Done {
                resolution: Resolution::NoChange { why },
            } => Status::Done {
                resolution: Resolution::NoChange {
                    why: note(&why, "why nothing changes")?,
                },
            },
        })
    }

    fn reopened_by_a_comment(self) -> Status {
        match self {
            settled @ (Status::NeedsReview { .. } | Status::Done { .. }) => Status::Reopened { was: Box::new(settled) },
            waiting @ (Status::New | Status::Reopened { .. }) => waiting,
        }
    }
}

impl Question {
    fn checked(self) -> Result<Question, String> {
        let ask = note(&self.ask, "what each question asks")?;
        if !(2..=MOST_OPTIONS).contains(&self.options.len()) {
            return Err(format!("{ask:?} needs 2 to {MOST_OPTIONS} options"));
        }
        if self.recommended >= self.options.len() {
            return Err(format!("{ask:?} recommends an option it doesn't have"));
        }
        let options = self
            .options
            .iter()
            .map(|option| note(option, "each option"))
            .collect::<Result<_, _>>()?;
        Ok(Question {
            ask,
            options,
            recommended: self.recommended,
        })
    }
}

#[derive(Deserialize)]
struct Filter {
    state: Option<Stage>,
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
    let compact = format!(
        "{:04}{:02}{:02}-{hour:02}{minute:02}{second:02}",
        date.year(),
        date.month(),
        date.day()
    );
    let spoken = format!(
        "{:04}-{:02}-{:02} {hour:02}:{minute:02}:{second:02} UTC",
        date.year(),
        date.month(),
        date.day()
    );
    (compact, spoken)
}

fn new_id(seconds: u64) -> String {
    let mut bytes = [0u8; 2];
    rand::fill(&mut bytes);
    format!("{}-{:02x}{:02x}", civil(seconds).0, bytes[0], bytes[1])
}

#[derive(Debug)]
enum Refused {
    Unknown,
    Invalid(String),
    Failed(String),
}

fn status_file(id: &str) -> String {
    format!("{id}.status.json")
}

fn comments_file(id: &str) -> String {
    format!("{id}.comments.jsonl")
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
        Ok(out) => String::from_utf8_lossy(&out)
            .lines()
            .map(|line| cut(line, 1000))
            .collect(),
        Err(problem) => {
            problems.push(problem);
            Vec::new()
        }
    }
}

async fn read(path: &str) -> Option<String> {
    tokio::fs::read_to_string(path)
        .await
        .ok()
        .map(|text| text.trim().to_string())
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
        let services_journal = journal(
            &["-u", "semmeltv-web", "-u", "semmeltv-mount", "-u", "nginx", "-n", "80"],
            &mut problems,
        )
        .await;
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
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
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
            device: client
                .get("device")
                .and_then(Value::as_str)
                .map(|device| cut(device, 80)),
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

    async fn marked(&self, id: &str) -> Option<Marked> {
        let text = tokio::fs::read(self.dir.join(status_file(id))).await.ok()?;
        serde_json::from_slice(&text).ok()
    }

    async fn listed(&self, summary: Summary) -> Listed {
        let marked = self.marked(&summary.id).await;
        let summary_id = summary.id.clone();
        Listed {
            summary,
            status: marked.as_ref().map_or(Status::New, |marked| marked.status.clone()),
            marked_at: marked.as_ref().map(|marked| marked.at),
            marked_by: marked.map(|marked| marked.by),
            comments: self.comments(&summary_id).await,
        }
    }

    async fn comments(&self, id: &str) -> Vec<Comment> {
        let text = tokio::fs::read_to_string(self.dir.join(comments_file(id)))
            .await
            .unwrap_or_default();
        text.lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    async fn known(&self, id: &str) -> bool {
        valid_id(id)
            && tokio::fs::try_exists(self.dir.join(format!("{id}.json")))
                .await
                .unwrap_or(false)
    }

    async fn mark(&self, id: &str, status: Status, by: String) -> Result<Marked, Refused> {
        let status = status.checked().map_err(Refused::Invalid)?;
        let _writing = self.writing.lock().await;
        if !self.known(id).await {
            return Err(Refused::Unknown);
        }
        self.write_marked(id, Marked { status, at: now(), by }).await
    }

    async fn comment(&self, id: &str, author: Author, text: &str) -> Result<(Comment, Marked), Refused> {
        let text = note(text, "something first").map_err(Refused::Invalid)?;
        let _writing = self.writing.lock().await;
        if !self.known(id).await {
            return Err(Refused::Unknown);
        }
        let comment = Comment {
            at: now(),
            author,
            text,
        };
        let mut line = serde_json::to_string(&comment).map_err(|error| Refused::Failed(error.to_string()))?;
        line.push('\n');
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join(comments_file(id)))
            .await
            .map_err(|error| Refused::Failed(format!("saving the comment: {error}")))?;
        file.write_all(line.as_bytes())
            .await
            .map_err(|error| Refused::Failed(format!("saving the comment: {error}")))?;
        let current = self.marked(id).await;
        let Author::Person { name } = &comment.author else {
            return Ok((
                comment,
                current.unwrap_or(Marked {
                    status: Status::New,
                    at: 0,
                    by: String::new(),
                }),
            ));
        };
        let status = current.as_ref().map_or(Status::New, |marked| marked.status.clone());
        let reopened = status.clone().reopened_by_a_comment();
        let marked = match current {
            Some(current) if reopened == status => current,
            None => Marked {
                status: Status::New,
                at: 0,
                by: String::new(),
            },
            Some(_) => {
                self.write_marked(
                    id,
                    Marked {
                        status: reopened,
                        at: comment.at,
                        by: name.clone(),
                    },
                )
                .await?
            }
        };
        Ok((comment, marked))
    }

    async fn write_marked(&self, id: &str, marked: Marked) -> Result<Marked, Refused> {
        let body = serde_json::to_vec_pretty(&marked).map_err(|error| Refused::Failed(error.to_string()))?;
        let temporary = self.dir.join(format!("{}.new", status_file(id)));
        let written = tokio::fs::write(&temporary, body).await;
        written
            .and(tokio::fs::rename(&temporary, self.dir.join(status_file(id))).await)
            .map_err(|error| Refused::Failed(format!("saving the status: {error}")))?;
        Ok(marked)
    }

    async fn summaries(&self) -> Vec<Summary> {
        let text = tokio::fs::read_to_string(self.dir.join(INDEX))
            .await
            .unwrap_or_default();
        text.lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    async fn prune(&self, mut index: Vec<Summary>) {
        let excess = index.len().saturating_sub(KEEP);
        for old in index.drain(..excess) {
            let files = [
                Some(format!("{}.json", old.id)),
                Some(status_file(&old.id)),
                Some(comments_file(&old.id)),
                old.screenshot,
                old.tv_screenshot,
            ];
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

async fn send(State(feedback): State<Feedback>, person: Option<Extension<Person>>, Json(sent): Json<Sent>) -> Response {
    let person = person.map(|Extension(person)| person);
    let who = person
        .as_ref()
        .map_or("someone".to_string(), |person| person.name.clone());
    match feedback.store(person, sent).await {
        Ok(summary) => {
            eprintln!(
                "feedback: {} from {who}: {}",
                summary.id,
                cut(&summary.message, 200).replace('\n', " ")
            );
            (StatusCode::CREATED, Json(json!({ "id": summary.id }))).into_response()
        }
        Err(problem) => {
            eprintln!("feedback from {who} not stored: {problem}");
            (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": problem }))).into_response()
        }
    }
}

async fn list(State(feedback): State<Feedback>, Query(filter): Query<Filter>) -> Json<Value> {
    let mut reports = Vec::new();
    for summary in feedback.summaries().await.into_iter().rev() {
        if reports.len() == LISTED {
            break;
        }
        let listed = feedback.listed(summary).await;
        if filter.state.is_none_or(|state| listed.status.stage() == state) {
            reports.push(listed);
        }
    }
    Json(json!({ "dir": feedback.dir, "reports": reports }))
}

async fn set_status(
    State(feedback): State<Feedback>,
    caller: Option<Extension<Caller>>,
    Path(id): Path<String>,
    Json(status): Json<Status>,
) -> Response {
    let by = match caller {
        Some(Extension(Caller::Person(person))) => person.name,
        Some(Extension(Caller::Service)) => "the service token".to_string(),
        None => "someone".to_string(),
    };
    match feedback.mark(&id, status, by).await {
        Ok(marked) => {
            eprintln!(
                "feedback: {id} is now {:?}, set by {}",
                marked.status.stage(),
                marked.by
            );
            Json(json!({ "id": id, "status": marked.status, "marked_at": marked.at, "marked_by": marked.by }))
                .into_response()
        }
        Err(Refused::Unknown) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "There is no such report" })),
        )
            .into_response(),
        Err(Refused::Invalid(problem)) => {
            (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": problem }))).into_response()
        }
        Err(Refused::Failed(problem)) => {
            eprintln!("feedback: marking {id}: {problem}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": problem }))).into_response()
        }
    }
}

fn refused(id: &str, refusal: Refused) -> Response {
    match refusal {
        Refused::Unknown => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "There is no such report" })),
        )
            .into_response(),
        Refused::Invalid(problem) => {
            (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": problem }))).into_response()
        }
        Refused::Failed(problem) => {
            eprintln!("feedback: {id}: {problem}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": problem }))).into_response()
        }
    }
}

async fn add_comment(
    State(feedback): State<Feedback>,
    caller: Option<Extension<Caller>>,
    Path(id): Path<String>,
    Json(said): Json<Said>,
) -> Response {
    let author = match caller {
        Some(Extension(Caller::Person(person))) => Author::Person { name: person.name },
        Some(Extension(Caller::Service)) => Author::Claude,
        None => Author::Person {
            name: "someone".to_string(),
        },
    };
    match feedback.comment(&id, author, &said.text).await {
        Ok((comment, marked)) => {
            eprintln!(
                "feedback: {id} has a comment from {:?}, now {:?}",
                comment.author,
                marked.status.stage()
            );
            (
                StatusCode::CREATED,
                Json(json!({ "id": id, "comment": comment, "status": marked.status, "marked_at": (marked.at > 0).then_some(marked.at), "marked_by": (!marked.by.is_empty()).then_some(marked.by) })),
            )
                .into_response()
        }
        Err(refusal) => refused(&id, refusal),
    }
}

async fn one(State(feedback): State<Feedback>, Path(id): Path<String>) -> Response {
    if !valid_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(body) = tokio::fs::read(feedback.dir.join(format!("{id}.json"))).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(mut report) = serde_json::from_slice::<Value>(&body) else {
        return ([(header::CONTENT_TYPE, "application/json")], body).into_response();
    };
    let marked = feedback.marked(&id).await;
    report["status"] = json!(marked.as_ref().map_or(Status::New, |marked| marked.status.clone()));
    report["marked_at"] = json!(marked.as_ref().map(|marked| marked.at));
    report["marked_by"] = json!(marked.map(|marked| marked.by));
    report["comments"] = json!(feedback.comments(&id).await);
    Json(report).into_response()
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
        Ok(body) => (
            [
                (header::CONTENT_TYPE, kind),
                (header::CACHE_CONTROL, "private, max-age=86400"),
            ],
            body,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

pub fn routes<S: Clone + Send + Sync + 'static>(feedback: Feedback) -> Router<S> {
    Router::new()
        .route("/api/feedback", get(list).post(send))
        .route("/api/feedback/{id}", get(one))
        .route("/api/feedback/{id}/status", post(set_status))
        .route("/api/feedback/{id}/comments", post(add_comment))
        .route("/api/feedback/files/{name}", get(file))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(feedback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_sort_by_time_and_say_it() {
        assert_eq!(
            civil(1_794_748_331),
            ("20261115-131211".to_string(), "2026-11-15 13:12:11 UTC".to_string())
        );
        let id = new_id(1_794_748_331);
        assert!(id.starts_with("20261115-131211-") && valid_id(&id));
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
    fn a_phone_screenshot_is_kept_next_to_its_report() {
        let dir = std::env::temp_dir().join(format!("semmeltv-feedback-shot-{}", std::process::id()));
        let feedback = Feedback::new(dir.clone(), Bus::default(), None);
        let sent = Sent {
            message: "Look at the status bar".to_string(),
            page: None,
            client: json!({}),
            screenshot: Some("data:image/jpeg;base64,/9j/4AAQ".to_string()),
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let summary = runtime.block_on(feedback.store(None, sent)).unwrap();
        let name = summary.screenshot.unwrap();
        assert_eq!(name, format!("{}-phone.jpg", summary.id));
        assert_eq!(
            std::fs::read(dir.join(&name)).unwrap(),
            [0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_status_says_its_state_and_what_a_person_or_a_commit_settled() {
        let fixed: Status = serde_json::from_value(json!({
            "state": "done",
            "resolution": { "kind": "fixed", "commit": " ABC1234 ", "what": "The sheet stays shut" }
        }))
        .unwrap();
        assert_eq!(
            fixed.checked().unwrap(),
            Status::Done {
                resolution: Resolution::Fixed {
                    commit: "abc1234".to_string(),
                    what: "The sheet stays shut".to_string()
                }
            }
        );
        let review = Status::NeedsReview {
            why: "It is a design question".to_string(),
            decide: "Keep or drop the button".to_string(),
            questions: Vec::new(),
        };
        assert_eq!(
            serde_json::to_value(&review).unwrap(),
            json!({ "state": "needs_review", "why": "It is a design question", "decide": "Keep or drop the button" })
        );
        assert_eq!(serde_json::to_value(Status::New).unwrap(), json!({ "state": "new" }));
        let no_commit = Status::Done {
            resolution: Resolution::Fixed {
                commit: "main".to_string(),
                what: "x".to_string(),
            },
        };
        assert!(no_commit.checked().is_err());
        let silent = Status::Done {
            resolution: Resolution::NoChange { why: "  ".to_string() },
        };
        assert!(silent.checked().is_err());
        assert!(serde_json::from_value::<Status>(json!({ "state": "needs_review", "why": "x" })).is_err());
    }

    #[test]
    fn only_a_person_reopens_a_settled_report_and_claude_answers_in_the_thread() {
        let dir = std::env::temp_dir().join(format!("semmeltv-feedback-thread-{}", std::process::id()));
        let feedback = Feedback::new(dir.clone(), Bus::default(), None);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let sent = Sent {
            message: "The sheet reopens".to_string(),
            page: None,
            client: json!({}),
            screenshot: None,
        };
        let id = runtime.block_on(feedback.store(None, sent)).unwrap().id;
        let frederik = Author::Person {
            name: "Frederik".to_string(),
        };

        let (_, marked) = runtime
            .block_on(feedback.comment(&id, frederik.clone(), "Still there"))
            .unwrap();
        assert_eq!(marked.status, Status::New);

        let done = Status::Done {
            resolution: Resolution::NoChange {
                why: "Works as meant".to_string(),
            },
        };
        runtime
            .block_on(feedback.mark(&id, done.clone(), "the service token".to_string()))
            .unwrap();
        let (_, marked) = runtime
            .block_on(feedback.comment(&id, Author::Claude, "Nothing to do"))
            .unwrap();
        assert_eq!(marked.status, done);

        let (_, marked) = runtime.block_on(feedback.comment(&id, frederik, "It is not")).unwrap();
        assert_eq!(marked.status, Status::Reopened { was: Box::new(done) });
        assert_eq!(marked.by, "Frederik");
        let thread: Vec<(Author, String)> = runtime
            .block_on(feedback.comments(&id))
            .into_iter()
            .map(|comment| (comment.author, comment.text))
            .collect();
        assert_eq!(thread.len(), 3);
        assert_eq!(thread[1], (Author::Claude, "Nothing to do".to_string()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_review_asks_questions_whose_options_a_person_picks_from() {
        let review: Status = serde_json::from_value(json!({
            "state": "needs_review",
            "why": "A new feature",
            "decide": "How the options get there",
            "questions": [
                { "ask": " Where do options come from? ", "options": ["Structured", " Parsed "], "recommended": 0 },
                { "ask": "Radios or checkboxes?", "options": ["Radios", "Checkboxes"], "recommended": 1 }
            ]
        }))
        .unwrap();
        let checked = review.checked().unwrap();
        let Status::NeedsReview { questions, .. } = &checked else {
            panic!("{checked:?}");
        };
        assert_eq!(questions[0].ask, "Where do options come from?");
        assert_eq!(questions[0].options, ["Structured", "Parsed"]);
        assert_eq!(questions[1].recommended, 1);

        let plain: Status =
            serde_json::from_value(json!({ "state": "needs_review", "why": "x", "decide": "y" })).unwrap();
        assert_eq!(
            serde_json::to_value(plain.checked().unwrap()).unwrap(),
            json!({ "state": "needs_review", "why": "x", "decide": "y" })
        );

        let asking = |questions: Vec<(Vec<&str>, usize)>| Status::NeedsReview {
            why: "x".to_string(),
            decide: "y".to_string(),
            questions: questions
                .into_iter()
                .map(|(options, recommended)| Question {
                    ask: "Which?".to_string(),
                    options: options.into_iter().map(str::to_string).collect(),
                    recommended,
                })
                .collect(),
        };
        assert!(asking(vec![(vec!["only"], 0)]).checked().is_err());
        assert!(asking(vec![(vec!["a", "b"], 2)]).checked().is_err());
        assert!(asking(vec![(vec!["a", " "], 0)]).checked().is_err());
        assert!(asking(vec![(vec!["a", "b"], 1)]).checked().is_ok());
        assert!(asking(vec![(vec!["a", "b"], 0); MOST_QUESTIONS + 1]).checked().is_err());
    }
}
