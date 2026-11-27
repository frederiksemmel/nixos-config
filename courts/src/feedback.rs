//! The feedback framework's store for this app (feedback/README.md): every
//! batch sent from feedback mode is a report, and Claude's sessions on beast
//! read and settle them through `/api/feedback`.

use crate::auth;
use crate::db::{Db, Report};
use crate::time;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::router::content::{Form, Json};
use topcoat::router::error::{SeeOther, not_found, see_other};
use topcoat::router::{page, path_param, query_params, route};
use topcoat::view::{View, component, view};

path_param!(report_id);

const STATES: [&str; 4] = ["new", "reopened", "needs_review", "done"];
const LONGEST: usize = 4_000_000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Said {
    pub at: i64,
    pub by: String,
    pub text: String,
}

fn summary(report: &Report) -> Value {
    let batch = &report.batch.0;
    let items: Vec<Value> = batch["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| json!({"n": item["n"], "what": item["what"], "comment": item["comment"], "page": item["page"]}))
                .collect()
        })
        .unwrap_or_default();
    json!({
        "id": report.id.to_string(),
        "at": report.created_at,
        "page": report.page,
        "state": report.state,
        "outcome": report.outcome,
        "answer": report.answer,
        "commit": report.commit,
        "marked_at": report.marked_at,
        "note": batch["note"],
        "items": items,
        "photo": batch["photo"].is_string(),
        "comments": report.thread.0.len(),
    })
}

fn full(report: &Report) -> Value {
    let mut value = summary(report);
    value["batch"] = report.batch.0.clone();
    value["thread"] = serde_json::to_value(&report.thread.0).unwrap_or_default();
    value
}

fn answer(ok: bool, message: impl Into<String>) -> Json<Value> {
    Json(json!({"ok": ok, "message": message.into()}))
}

fn db(cx: &Cx) -> Db {
    app_context::<Db>(cx).clone()
}

#[route(POST "/api/feedback")]
async fn api_send(cx: &Cx, Json(batch): Json<Value>) -> Result<Json<Value>> {
    let items = batch["items"].as_array().map_or(0, Vec::len);
    let note = batch["note"].as_str().unwrap_or("").trim().to_string();
    if items == 0 && note.is_empty() {
        return Ok(answer(false, "Nothing to send: mark something and comment on it first"));
    }
    if batch.to_string().len() > LONGEST {
        return Ok(answer(
            false,
            "That is too much at once: leave out the photo, or send fewer comments",
        ));
    }
    let page = batch["page"]
        .as_str()
        .unwrap_or("/")
        .chars()
        .take(500)
        .collect::<String>();
    let report = db(cx)
        .add_report(&page, batch, time::now())
        .await
        .map_err(topcoat::Error::msg)?;
    Ok(Json(json!({
        "ok": true,
        "id": report.id.to_string(),
        "message": format!("Sent {}. Claude takes them up together in a few minutes; see Feedback for what it did.",
            if items == 1 { "1 comment".to_string() } else { format!("{items} comments") }),
    })))
}

#[query_params]
struct ListQuery {
    state: Option<String>,
}

#[route(GET "/api/feedback")]
async fn api_list(cx: &Cx) -> Result<Json<Value>> {
    let state = query_params::<ListQuery>(cx)
        .map_err(|error| topcoat::Error::msg(error.to_string()))?
        .state
        .clone();
    let reports: Vec<Value> = db(cx)
        .reports()
        .await
        .map_err(topcoat::Error::msg)?
        .iter()
        .filter(|report| state.as_deref().is_none_or(|state| report.state == state))
        .map(summary)
        .collect();
    Ok(Json(json!({"reports": reports})))
}

async fn find(cx: &Cx) -> Result<Report> {
    let id: i64 = path_param::<ReportId>(cx).parse().map_err(|_| not_found())?;
    db(cx)
        .report(id)
        .await
        .map_err(topcoat::Error::msg)?
        .ok_or_else(|| not_found().into())
}

#[route(GET "/api/feedback/{report_id}")]
async fn api_show(cx: &Cx) -> Result<Json<Value>> {
    Ok(Json(full(&find(cx).await?)))
}

#[derive(Deserialize)]
struct Mark {
    state: String,
    outcome: Option<String>,
    answer: Option<String>,
    commit: Option<String>,
}

#[route(POST "/api/feedback/{report_id}/status")]
async fn api_mark(cx: &Cx, Json(mark): Json<Mark>) -> Result<Json<Value>> {
    if !STATES.contains(&mark.state.as_str()) {
        return Ok(answer(false, format!("state must be one of {}", STATES.join(", "))));
    }
    if mark.state == "done" && !matches!(mark.outcome.as_deref(), Some("fixed" | "no_change")) {
        return Ok(answer(false, "a done report needs outcome fixed or no_change"));
    }
    let mut report = find(cx).await?;
    report.state = mark.state;
    report.outcome = mark.outcome;
    report.answer = mark.answer.map(|a| a.chars().take(4000).collect());
    report.commit = mark.commit;
    report.marked_at = time::now();
    db(cx).save_report(report).await.map_err(topcoat::Error::msg)?;
    Ok(answer(true, "marked"))
}

#[derive(Deserialize)]
struct Comment {
    text: String,
    #[serde(default)]
    claude: bool,
}

async fn comment(cx: &Cx, mut report: Report, text: &str, claude: bool) -> Result<()> {
    report.thread.0.push(Said {
        at: time::now(),
        by: if claude { "claude" } else { "you" }.into(),
        text: text.chars().take(4000).collect(),
    });
    if !claude && (report.state == "done" || report.state == "needs_review") {
        report.state = "reopened".into();
        report.marked_at = time::now();
    }
    db(cx).save_report(report).await.map_err(topcoat::Error::msg)
}

#[route(POST "/api/feedback/{report_id}/comments")]
async fn api_comment(cx: &Cx, Json(said): Json<Comment>) -> Result<Json<Value>> {
    if said.text.trim().is_empty() {
        return Ok(answer(false, "say something"));
    }
    let report = find(cx).await?;
    comment(cx, report, said.text.trim(), said.claude && auth::bearer(cx)).await?;
    Ok(answer(true, "added"))
}

#[derive(Deserialize)]
struct Reply {
    text: String,
}

#[route(POST "/feedback/{report_id}/reply")]
async fn reply_form(cx: &Cx, Form(reply): Form<Reply>) -> Result<SeeOther> {
    let report = find(cx).await?;
    let id = report.id;
    if !reply.text.trim().is_empty() {
        comment(cx, report, reply.text.trim(), false).await?;
    }
    Ok(see_other(format!("/feedback#report-{id}")))
}

fn state_label(report: &Report) -> &'static str {
    match (report.state.as_str(), report.outcome.as_deref()) {
        ("new", _) => "Waiting for Claude",
        ("reopened", _) => "Reopened",
        ("needs_review", _) => "Needs your answer",
        ("done", Some("fixed")) => "Fixed",
        ("done", _) => "Closed, no change",
        _ => "Unknown",
    }
}

struct Item {
    n: String,
    what: String,
    comment: String,
}

fn items(report: &Report) -> Vec<Item> {
    report.batch.0["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| Item {
                    n: item["n"].to_string(),
                    what: item["what"].as_str().unwrap_or("").to_string(),
                    comment: item["comment"].as_str().unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[page("/feedback")]
async fn feedback_page(cx: &Cx) -> Result<impl View> {
    let reports = db(cx).reports().await.map_err(topcoat::Error::msg)?;
    Ok(view! {
        crate::pages::nav(current: "feedback")
        <article class="page feedback-page">
            <h1>"Feedback"</h1>
            <p class="lede">"Tap Feedback at the bottom of any page, mark what you mean and comment on it. Everything you send together goes to one Claude session on beast, which changes the app, deploys it and answers here."</p>
            if reports.is_empty() {
                <p class="empty">"Nothing sent yet."</p>
            }
            <ol class="reports">
                for report in &reports {
                    report_card(report: report)
                }
            </ol>
        </article>
    })
}

#[component]
async fn report_card(report: &Report) -> Result<impl View> {
    let list = items(report);
    let note = report.batch.0["note"].as_str().unwrap_or("").to_string();
    let state = report.state.clone();
    Ok(view! {
        <li class="report" id=(format!("report-{}", report.id))>
            <header>
                <span class=(format!("state state-{state}"))>(state_label(report))</span>
                <span class="quiet">(format!("{} · {}", time::ago(report.created_at), report.page))</span>
            </header>
            if !list.is_empty() {
                <ol class="report-items">
                    for item in &list {
                        <li>
                            <span class="n">(item.n.as_str())</span>
                            <span>
                                <span class="quiet">(item.what.as_str())</span>
                                <br>
                                (item.comment.as_str())
                            </span>
                        </li>
                    }
                </ol>
            }
            if !note.is_empty() {
                <p class="report-note">(note.as_str())</p>
            }
            if let Some(answer) = &report.answer {
                <div class="claude">
                    <p class="who">
                        "Claude"
                        if let Some(commit) = &report.commit {
                            <span class="quiet">(format!(" · {}", commit.chars().take(7).collect::<String>()))</span>
                        }
                    </p>
                    <p>(answer.as_str())</p>
                </div>
            }
            for said in &report.thread.0 {
                <div class=(if said.by == "claude" { "said claude" } else { "said" })>
                    <p class="who">(if said.by == "claude" { "Claude" } else { "You" }) <span class="quiet">(format!(" · {}", time::ago(said.at)))</span></p>
                    <p>(said.text.as_str())</p>
                </div>
            }
            <form class="reply" method="post" action=(format!("/feedback/{}/reply", report.id))>
                <textarea name="text" rows="2" maxlength="4000" placeholder=(if state == "done" || state == "needs_review" { "Not right yet? Say what, and it goes back to Claude." } else { "Add to it" }) aria-label="Reply"></textarea>
                <button class="pill quiet" type="submit">"Reply"</button>
            </form>
        </li>
    })
}
