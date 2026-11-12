use crate::layout::back;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::router::page;
use topcoat::runtime::{Event, Signal, connected, procedure, shard, signal};
use topcoat::view::{View, component, emit, live, view};

const REFRESH: Duration = Duration::from_secs(2);
const LONGEST_REQUEST: usize = 200;

#[derive(Clone)]
pub struct Downloads {
    base: String,
    client: reqwest::Client,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Candidate {
    pub kind: String,
    pub tmdb: u64,
    pub label: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Question {
    pub options: Vec<Candidate>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Choice {
    pub label: String,
    pub facts: String,
    pub why: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Target {
    pub season: Option<u32>,
    pub episode: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub asked: String,
    pub from: String,
    pub ok: bool,
    pub message: String,
    pub label: Option<String>,
    pub target: Option<Target>,
    pub question: Option<Question>,
    pub choices: Vec<Choice>,
    pub dropped: usize,
    pub problems: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PartState {
    pub progress: u64,
    pub total: u64,
    pub uploaded: u64,
    pub speed: f64,
    pub peers: Option<u64>,
    #[serde(default)]
    pub checking: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Job {
    pub id: String,
    pub label: String,
    pub stage: String,
    pub message: String,
    pub parts: Vec<PartState>,
    pub log: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Overview {
    pub proposals: Vec<Proposal>,
    pub jobs: Vec<Job>,
    pub room: u64,
}

#[derive(Debug, Deserialize)]
struct Outcome {
    message: String,
}

impl Downloads {
    pub fn new(base: String) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }

    async fn post<T: serde::de::DeserializeOwned>(&self, path: &str, body: &Value) -> std::result::Result<T, String> {
        let response = self
            .client
            .post(format!("{}{path}", self.base))
            .json(body)
            .send()
            .await
            .map_err(|_| "The download service is not reachable".to_string())?;
        response.json().await.map_err(|error| error.to_string())
    }

    pub async fn overview(&self) -> std::result::Result<Overview, String> {
        let response = self
            .client
            .get(format!("{}/api/downloads", self.base))
            .send()
            .await
            .map_err(|_| "The download service is not reachable".to_string())?;
        response.json().await.map_err(|error| error.to_string())
    }

    async fn search(&self, body: Value) -> std::result::Result<Proposal, String> {
        self.post("/api/downloads/search", &body).await
    }

    async fn outcome(&self, path: &str, body: Value) -> String {
        match self.post::<Outcome>(path, &body).await {
            Ok(outcome) => outcome.message,
            Err(error) => error,
        }
    }
}

#[procedure]
pub async fn find(cx: &Cx, text: String) -> Result<String> {
    let text: String = text.trim().chars().take(LONGEST_REQUEST).collect();
    if text.is_empty() {
        return Ok(String::new());
    }
    let found = app_context::<Downloads>(cx)
        .search(json!({ "text": text, "from": "web" }))
        .await;
    Ok(found.map_or_else(|error| format!("!{error}"), |proposal| proposal.id))
}

#[procedure]
pub async fn pick(cx: &Cx, kind: String, tmdb: u64, season: i64, episode: i64, asked: String) -> Result<String> {
    let number = |value: i64| u32::try_from(value).ok().filter(|value| *value > 0);
    let body = json!({
        "text": asked,
        "from": "web",
        "pick": { "kind": kind, "tmdb": tmdb, "season": number(season), "episode": number(episode) },
    });
    let found = app_context::<Downloads>(cx).search(body).await;
    Ok(found.map_or_else(|error| format!("!{error}"), |proposal| proposal.id))
}

#[procedure]
pub async fn confirm(cx: &Cx, proposal: String, choice: u32) -> Result<String> {
    Ok(app_context::<Downloads>(cx)
        .outcome("/api/downloads/confirm", json!({ "proposal": proposal, "choice": choice }))
        .await)
}

#[procedure]
pub async fn dismiss(cx: &Cx, proposal: String) -> Result<String> {
    Ok(app_context::<Downloads>(cx)
        .outcome("/api/downloads/dismiss", json!({ "proposal": proposal }))
        .await)
}

#[procedure]
pub async fn stop(cx: &Cx, job: String) -> Result<String> {
    Ok(app_context::<Downloads>(cx)
        .outcome(&format!("/api/downloads/jobs/{job}/stop"), json!({}))
        .await)
}

fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

fn progress_line(job: &Job) -> String {
    let done: u64 = job.parts.iter().map(|part| part.progress).sum();
    let total: u64 = job.parts.iter().map(|part| part.total).sum();
    let uploaded: u64 = job.parts.iter().map(|part| part.uploaded).sum();
    let speed: f64 = job.parts.iter().map(|part| part.speed).sum();
    let peers: u64 = job.parts.iter().filter_map(|part| part.peers).sum();
    match job.stage.as_str() {
        "downloading" if job.parts.iter().any(|part| part.checking) => {
            format!("{} downloaded before the restart · {uploaded} bytes uploaded", gb(done))
        }
        "downloading" if total > 0 => format!(
            "{} of {} · {speed:.1} MiB/s · {peers} peers · {uploaded} bytes uploaded",
            gb(done),
            gb(total)
        ),
        _ => format!("{uploaded} bytes uploaded to peers"),
    }
}

fn percent(job: &Job) -> f64 {
    let done: u64 = job.parts.iter().map(|part| part.progress).sum();
    let total: u64 = job.parts.iter().map(|part| part.total).sum();
    match job.stage.as_str() {
        "uploading" | "done" => 100.0,
        _ if total == 0 => 0.0,
        _ => (done as f64 * 100.0 / total as f64).clamp(0.0, 100.0),
    }
}

#[component]
async fn job_row(cx: &Cx, job: &Job) -> Result<impl View> {
    let said = signal(cx, String::new);
    let id = job.id.clone();
    let stoppable = matches!(job.stage.as_str(), "checking" | "downloading");
    let last = job.log.last().cloned().unwrap_or_default();
    Ok(view! {
        <li class="download-job" data-stage=(job.stage.as_str())>
            <div class="download-job-line">
                <b>(job.label.as_str())</b>
                <span>(job.message.as_str())</span>
            </div>
            if job.stage != "done" {
                <div class="download-bar"><i style=(format!("width:{:.1}%", percent(job)))></i></div>
                <p class="download-facts">(progress_line(job))</p>
                <p class="download-log">(last)</p>
            }
            if stoppable {
                <button type="button" class="storage-button danger" @click=$(async |_e| {
                    let answer = stop(id.to_owned()).await;
                    said.set(answer);
                })>"Stop and delete"</button>
            }
            <p class="controls-said" aria-live="polite" :hidden=$(said.get().is_empty())>$(said.get())</p>
        </li>
    })
}

#[component]
async fn job_list(cx: &Cx) -> Result<impl View> {
    let downloads = app_context::<Downloads>(cx).clone();
    Ok(live! {
        loop {
            let overview = downloads.overview().await;
            let token = emit! {
                <section class="settings">
                    <h2>"Recent"</h2>
                    match &overview {
                        Err(error) => <p class="setting-note">(error.as_str())</p>,
                        Ok(overview) => {
                            if overview.jobs.is_empty() {
                                <p class="setting-note">"Nothing downloaded yet."</p>
                            } else {
                                <ul class="download-jobs">
                                    #[key(job.id.clone())]
                                    for job in &overview.jobs {
                                        job_row(job: job)
                                    }
                                </ul>
                            }
                            <p class="setting-note">(format!("{} free for new downloads", gb(overview.room)))</p>
                        },
                    }
                </section>
            }?;
            if !connected(cx) {
                break Ok(token);
            }
            tokio::time::sleep(REFRESH).await;
        }
    })
}

#[component]
async fn choice_card(cx: &Cx, proposal: &Proposal, index: usize, choice: &Choice) -> Result<impl View> {
    let said = signal(cx, String::new);
    let id = proposal.id.clone();
    let number = index as u32;
    let class = if index == 0 { "download-choice best" } else { "download-choice" };
    Ok(view! {
        <li class=(class)>
            <b>(choice.facts.as_str())</b>
            <span class="download-name">(choice.label.as_str())</span>
            <ul class="download-why">
                for reason in &choice.why {
                    <li>(reason.as_str())</li>
                }
            </ul>
            <button type="button" class="storage-button" @click=$(async |_e| {
                let answer = confirm(id.to_owned(), number).await;
                said.set(answer);
            })>(if index == 0 { "Download" } else { "Download this one" })</button>
            <p class="controls-said" aria-live="polite" :hidden=$(said.get().is_empty())>$(said.get())</p>
        </li>
    })
}

#[component]
async fn question_buttons(proposal: &Proposal, shown: Signal<String>) -> Result<impl View> {
    let options = proposal.question.as_ref().map(|question| question.options.clone()).unwrap_or_default();
    let season = proposal.target.as_ref().and_then(|target| target.season).map_or(0, i64::from);
    let episode = proposal.target.as_ref().and_then(|target| target.episode).map_or(0, i64::from);
    let asked = proposal.asked.clone();
    Ok(view! {
        <div class="tracks">
            #[key(option.tmdb)]
            for option in &options {
                let kind = option.kind.clone();
                let tmdb = option.tmdb;
                let asked = asked.clone();
                <button type="button" class="track" @click=$(async |_e| {
                    let id = pick(kind.to_owned(), tmdb, season, episode, asked.to_owned()).await;
                    shown.set(id);
                })>
                    <b>(option.label.as_str())</b>
                </button>
            }
        </div>
    })
}

#[shard]
async fn proposal_view(cx: &Cx, id: String, shown: Signal<String>) -> Result<impl View> {
    let found = match id.strip_prefix('!') {
        Some(error) => Err(error.to_string()),
        None if id.is_empty() => Ok(None),
        None => {
            let overview = app_context::<Downloads>(cx).overview().await.unwrap_or_default();
            match overview.proposals.into_iter().find(|proposal| proposal.id == id) {
                Some(found) => Ok(Some(found)),
                None => Err("That search has expired. Search again.".to_string()),
            }
        }
    };
    Ok(view! {
        match found {
            Err(error) => <p class="setting-note">(error)</p>,
            Ok(None) => <span></span>,
            Ok(Some(found)) => {
                <section class="settings">
                    <h2>(found.label.clone().unwrap_or_else(|| found.asked.clone()))</h2>
                    if found.ok {
                        <p class="setting-note">(format!(
                            "{} releases checked, {} dropped by the checks.",
                            found.dropped + found.choices.len(),
                            found.dropped
                        ))</p>
                        <ul class="download-choices">
                            #[key(index)]
                            for (index, choice) in found.choices.iter().enumerate() {
                                choice_card(proposal: &found, index: index, choice: choice)
                            }
                        </ul>
                    } else {
                        <p class="setting-note">(found.message.as_str())</p>
                        if found.question.is_some() {
                            question_buttons(proposal: &found, shown: shown)
                        }
                    }
                    for problem in &found.problems {
                        <p class="setting-note">(problem.as_str())</p>
                    }
                </section>
            },
        }
    })
}

#[component]
async fn waiting(cx: &Cx, shown: Signal<String>) -> Result<impl View> {
    let overview = app_context::<Downloads>(cx).overview().await.unwrap_or_default();
    let voice: Vec<Proposal> = overview
        .proposals
        .into_iter()
        .filter(|proposal| proposal.from == "voice" && proposal.ok)
        .collect();
    Ok(view! {
        if !voice.is_empty() {
            <section class="settings">
                <h2>"Asked for by voice"</h2>
                <div class="tracks">
                    #[key(found.id.clone())]
                    for found in &voice {
                        let id = found.id.clone();
                        <button type="button" class="track" @click=$(|_e| shown.set(id.to_owned()))>
                            <b>(found.label.clone().unwrap_or_else(|| found.asked.clone()))</b>
                            <span>(found.message.as_str())</span>
                        </button>
                    }
                </div>
            </section>
        }
    })
}

#[page("/downloads")]
async fn downloads_page(cx: &Cx) -> Result<impl View> {
    let text = signal(cx, String::new);
    let shown = signal(cx, String::new);
    let busy = signal(cx, || false);
    Ok(view! {
        <article class="downloads">
            back(href: "/settings")
            <h1>"Downloads"</h1>
            <form class="search download-search" @submit=$(async |e: Event| {
                e.prevent_default();
                busy.set(true);
                let id = find(text.get()).await;
                shown.set(id);
                busy.set(false);
            })>
                <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="11" cy="11" r="6.5"></circle><path d="m16 16 4.5 4.5"></path></svg>
                <input
                    type="search"
                    placeholder="A film or a season"
                    autocomplete="off"
                    autocapitalize="off"
                    spellcheck="false"
                    enterkeyhint="search"
                    aria-label="What to download"
                    :value=$(text.get())
                    @input=$(|e: Event| text.set(e.target.value))
                >
                <button type="submit" class="storage-button" :disabled=$(busy.get())>"Find"</button>
            </form>
            <p class="setting-note" :hidden=$(!busy.get())>"Searching TMDB and the indexers…"</p>
            proposal_view(id: $(shown.get()), shown: shown.clone())
            waiting(shown: shown)
            job_list()
        </article>
    })
}
