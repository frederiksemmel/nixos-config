use crate::auth::gate::Caller;
use crate::feedback::{Brief, Feedback};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

const MOST_COMMITS: usize = 200;
const LONGEST_SUBJECT: usize = 200;
const LONGEST_MESSAGE: usize = 2000;
const MOST_REPORTS: usize = 10;
const SHORTEST_COMMIT: usize = 7;

#[derive(Clone)]
pub struct Deploy {
    feedback: Feedback,
    revision: PathBuf,
    state: Arc<Mutex<Known>>,
}

impl Deploy {
    pub fn new(feedback: Feedback, revision: PathBuf) -> Self {
        Self {
            feedback,
            revision,
            state: Arc::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Commit {
    sha: String,
    at: u64,
    subject: String,
    #[serde(default)]
    reports: Vec<String>,
    #[serde(default)]
    tv: bool,
}

#[derive(Deserialize)]
struct Main {
    commits: Vec<Commit>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct Asked {
    at: u64,
    by: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Run {
    Running { at: u64, by: String },
    Failed { at: u64, by: String, message: String },
    Finished { at: u64, by: String },
}

#[derive(Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Progress {
    Running,
    Failed { message: String },
    Finished,
}

#[derive(Default)]
struct Known {
    main: Option<(u64, Vec<Commit>)>,
    asked: Option<Asked>,
    /// A deploy beast holds because someone watches; it runs once nobody does.
    waiting: Option<Asked>,
    run: Option<Run>,
}

#[derive(Debug, PartialEq, Serialize)]
struct Pending {
    sha: String,
    at: u64,
    subject: String,
    reports: Vec<Brief>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

fn cut(text: &str, most: usize) -> String {
    match text.char_indices().nth(most) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

fn checked(commits: Vec<Commit>) -> Result<Vec<Commit>, String> {
    if commits.len() > MOST_COMMITS {
        return Err(format!("At most {MOST_COMMITS} commits"));
    }
    commits
        .into_iter()
        .map(|mut commit| {
            if commit.sha.len() < SHORTEST_COMMIT || !commit.sha.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(format!("{:?} is not a commit", commit.sha));
            }
            commit.sha.make_ascii_lowercase();
            commit.subject = cut(commit.subject.trim(), LONGEST_SUBJECT);
            commit.reports.truncate(MOST_REPORTS);
            Ok(commit)
        })
        .collect()
}

fn fixes(commit: &Commit, brief: &Brief) -> bool {
    commit.reports.contains(&brief.id)
        || brief.fixed_in.as_deref().is_some_and(|fixed| {
            let fixed = fixed.trim().to_ascii_lowercase();
            fixed.len() >= SHORTEST_COMMIT && commit.sha.starts_with(&fixed)
        })
}

/// The commits on main after `revision`, newest first, split into those that
/// change the TVs and how many change only other machines. None when the
/// revision is not among main's commits, so nobody can tell what is new.
fn pending(revision: &str, commits: &[Commit], briefs: &[Brief]) -> Option<(Vec<Pending>, usize)> {
    let revision = revision.trim().to_ascii_lowercase();
    let at = commits
        .iter()
        .position(|commit| !revision.is_empty() && commit.sha == revision)?;
    let newer = &commits[..at];
    let shown = newer
        .iter()
        .filter(|commit| commit.tv)
        .map(|commit| Pending {
            sha: commit.sha.clone(),
            at: commit.at,
            subject: commit.subject.clone(),
            reports: briefs.iter().filter(|brief| fixes(commit, brief)).cloned().collect(),
        })
        .collect::<Vec<_>>();
    let others = newer.len() - shown.len();
    Some((shown, others))
}

fn caller_name(caller: Option<Extension<Caller>>) -> String {
    match caller {
        Some(Extension(Caller::Person(person))) => person.name,
        Some(Extension(Caller::Service)) => "beast".to_string(),
        None => "someone".to_string(),
    }
}

impl Deploy {
    async fn view(&self) -> serde_json::Value {
        let revision = tokio::fs::read_to_string(&self.revision)
            .await
            .ok()
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty());
        let (checked_at, main, asked, waiting, run, commits) = {
            let known = self.state.lock().await;
            (
                known.main.as_ref().map(|(at, _)| *at),
                known
                    .main
                    .as_ref()
                    .and_then(|(_, commits)| commits.first().map(|commit| commit.sha.clone())),
                known.asked.clone(),
                known.waiting.clone(),
                known.run.clone(),
                known.main.as_ref().map(|(_, commits)| commits.clone()),
            )
        };
        let briefs = if commits.is_some() && main.is_some() && revision != main {
            self.feedback.briefs().await
        } else {
            Vec::new()
        };
        let found = commits
            .as_deref()
            .zip(revision.as_deref())
            .and_then(|(commits, revision)| pending(revision, commits, &briefs));
        json!({
            "revision": revision,
            "checked_at": checked_at,
            "main": main,
            "pending": found.as_ref().map(|(shown, _)| shown),
            "others": found.as_ref().map_or(0, |(_, others)| *others),
            "asked": asked,
            "waiting": waiting,
            "run": run,
        })
    }
}

async fn show(State(deploy): State<Deploy>) -> Json<serde_json::Value> {
    Json(deploy.view().await)
}

async fn ask(State(deploy): State<Deploy>, caller: Option<Extension<Caller>>) -> Response {
    let by = caller_name(caller);
    {
        let mut known = deploy.state.lock().await;
        if known.main.is_none() {
            return (
                StatusCode::CONFLICT,
                Json(json!({ "error": "Beast hasn't said what's on main yet, so nothing can deploy from here" })),
            )
                .into_response();
        }
        if matches!(known.run, Some(Run::Running { .. })) {
            return (
                StatusCode::CONFLICT,
                Json(json!({ "error": "A deploy is running already" })),
            )
                .into_response();
        }
        known.asked = Some(Asked {
            at: now(),
            by: by.clone(),
        });
    }
    eprintln!("deploy: {by} asked for a deploy");
    Json(deploy.view().await).into_response()
}

async fn tell_main(State(deploy): State<Deploy>, Json(main): Json<Main>) -> Response {
    let commits = match checked(main.commits) {
        Ok(commits) => commits,
        Err(problem) => return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": problem }))).into_response(),
    };
    deploy.state.lock().await.main = Some((now(), commits));
    Json(deploy.view().await).into_response()
}

async fn progress(State(deploy): State<Deploy>, Json(progress): Json<Progress>) -> Response {
    {
        let mut known = deploy.state.lock().await;
        let by = known
            .asked
            .as_ref()
            .or(known.waiting.as_ref())
            .map(|asked| asked.by.clone())
            .or_else(|| match &known.run {
                Some(Run::Running { by, .. }) => Some(by.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "beast".to_string());
        let at = now();
        known.run = Some(match progress {
            Progress::Running => {
                known.asked = None;
                known.waiting = None;
                Run::Running { at, by }
            }
            Progress::Failed { message } => Run::Failed {
                at,
                by,
                message: cut(message.trim(), LONGEST_MESSAGE),
            },
            Progress::Finished => Run::Finished { at, by },
        });
        eprintln!("deploy: {:?}", known.run);
    }
    Json(deploy.view().await).into_response()
}

async fn wait(State(deploy): State<Deploy>, caller: Option<Extension<Caller>>) -> Response {
    let by = caller_name(caller);
    deploy.state.lock().await.waiting = Some(Asked {
        at: now(),
        by: by.clone(),
    });
    eprintln!("deploy: {by} holds a deploy until nobody watches");
    Json(deploy.view().await).into_response()
}

async fn stop_waiting(State(deploy): State<Deploy>) -> Response {
    deploy.state.lock().await.waiting = None;
    Json(deploy.view().await).into_response()
}

pub fn routes<S: Clone + Send + Sync + 'static>(deploy: Deploy) -> Router<S> {
    Router::new()
        .route("/api/deploy", get(show).post(ask))
        .route("/api/deploy/main", post(tell_main))
        .route("/api/deploy/progress", post(progress))
        .route("/api/deploy/wait", post(wait).delete(stop_waiting))
        .with_state(deploy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(sha: &str, tv: bool, reports: &[&str]) -> Commit {
        Commit {
            sha: sha.to_string(),
            at: 0,
            subject: format!("subject of {sha}"),
            reports: reports.iter().map(|id| id.to_string()).collect(),
            tv,
        }
    }

    fn brief(id: &str, fixed_in: Option<&str>) -> Brief {
        Brief {
            id: id.to_string(),
            person: Some("Frederik".to_string()),
            message: "a wish".to_string(),
            fixed_in: fixed_in.map(str::to_string),
        }
    }

    #[test]
    fn what_is_on_main_but_not_on_the_tv_lists_its_tv_commits_and_the_reports_they_fixed() {
        let commits = vec![
            commit("cccccccc", true, &["20261119-131455-d5b1"]),
            commit("bbbbbbbb", false, &[]),
            commit("aaaaaaaa", true, &[]),
            commit("99999999", true, &[]),
        ];
        let briefs = vec![
            brief("20261119-131455-d5b1", None),
            brief("20261119-120000-0001", Some("aaaaaaa")),
            brief("20261119-120000-0002", Some("999")),
        ];
        let (shown, others) = pending("99999999\n", &commits, &briefs).unwrap();
        assert_eq!(others, 1);
        assert_eq!(
            shown.iter().map(|pending| pending.sha.as_str()).collect::<Vec<_>>(),
            ["cccccccc", "aaaaaaaa"]
        );
        assert_eq!(shown[0].reports[0].id, "20261119-131455-d5b1");
        assert_eq!(shown[1].reports[0].id, "20261119-120000-0001");
        assert_eq!(shown[1].reports.len(), 1, "a 3-letter commit matches nothing");

        let (shown, others) = pending("CCCCCCCC", &commits, &briefs).unwrap();
        assert!(shown.is_empty());
        assert_eq!(others, 0);
        assert_eq!(pending("dddddddd", &commits, &briefs), None);
        assert_eq!(pending("", &commits, &briefs), None);
    }

    #[test]
    fn main_from_beast_is_checked_and_trimmed() {
        let long = Commit {
            subject: "x".repeat(500),
            ..commit("ABCDEF12", true, &[])
        };
        let checked = checked(vec![long]).unwrap();
        assert_eq!(checked[0].sha, "abcdef12");
        assert_eq!(checked[0].subject.chars().count(), LONGEST_SUBJECT + 1);
        assert!(super::checked(vec![commit("main", true, &[])]).is_err());
        assert!(super::checked(vec![commit("abc", true, &[])]).is_err());
    }
}
