use crate::layout::back;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use topcoat::Result;
use topcoat::context::{Cx, app_context};

fn downloads(cx: &Cx) -> Result<&Downloads> {
    crate::auth::may_download(cx)?;
    Ok(app_context::<Downloads>(cx))
}

fn asked_by(cx: &Cx) -> Option<String> {
    crate::auth::me(cx).map(|me| me.person.name.clone())
}
use topcoat::router::page;
use topcoat::runtime::{Event, Signal, connected, link, procedure, shard, signal};
use topcoat::view::{View, attributes, component, emit, live, view};

const REFRESH: Duration = Duration::from_secs(2);
const MISSING_WAIT: Duration = Duration::from_secs(3);
const PCLOUD_WAIT: Duration = Duration::from_secs(30);
const LONGEST_REQUEST: usize = 200;
const SHORTEST_MATCHED: usize = 2;
const MATCH_PAUSE: Duration = Duration::from_millis(300);
const MATCH_WAIT: Duration = Duration::from_secs(5);
const POSTERS: &str = "https://image.tmdb.org/t/p/w154";

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
    #[serde(default)]
    pub season: Option<u32>,
}

impl Candidate {
    fn key(&self) -> String {
        format!("{}-{}-{}", self.kind, self.tmdb, self.season.unwrap_or(0))
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Suggestion {
    pub kind: String,
    pub tmdb: u64,
    pub title: String,
    pub year: Option<u16>,
    pub poster: Option<String>,
    pub season: Option<u32>,
    pub owned: bool,
    #[serde(default)]
    pub credit: Option<String>,
}

impl Suggestion {
    fn key(&self) -> String {
        format!("{}-{}", self.kind, self.tmdb)
    }

    fn facts(&self) -> String {
        let what = if self.kind == "show" { "Series" } else { "Film" };
        let mut facts = vec![what.to_string()];
        facts.extend(self.year.map(|year| year.to_string()));
        facts.extend(self.season.map(|season| format!("season {season}")));
        facts.extend(self.credit.clone());
        if self.owned {
            facts.push("in the library".to_string());
        }
        facts.join(" · ")
    }

    fn poster(&self) -> Option<String> {
        self.poster
            .as_deref()
            .filter(|path| path.starts_with('/') && !path.contains(".."))
            .map(|path| format!("{POSTERS}{path}"))
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Question {
    pub text: String,
    pub options: Vec<Candidate>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Choice {
    pub label: String,
    pub facts: String,
    pub why: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Searched {
    pub label: String,
    pub message: String,
    pub choices: Vec<Choice>,
    pub dropped: usize,
    pub problems: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum Answer {
    Refused { message: String },
    Question(Question),
    Searched(Searched),
}

#[derive(Clone, Debug, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub asked: String,
    pub from: String,
    #[serde(flatten)]
    pub answer: Answer,
}

impl Proposal {
    fn heading(&self) -> &str {
        match &self.answer {
            Answer::Searched(searched) => &searched.label,
            Answer::Refused { .. } | Answer::Question(_) => &self.asked,
        }
    }

    fn message(&self) -> &str {
        match &self.answer {
            Answer::Refused { message } | Answer::Searched(Searched { message, .. }) => message,
            Answer::Question(question) => &question.text,
        }
    }

    fn found(&self) -> bool {
        matches!(&self.answer, Answer::Searched(searched) if !searched.choices.is_empty())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PartPhase {
    New,
    Checking,
    Downloading,
    Done,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PartState {
    pub phase: PartPhase,
    pub progress: u64,
    pub total: u64,
    pub uploaded: u64,
    pub speed: f64,
    pub peers: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    Checking,
    Downloading,
    Uploading,
    Done,
    Failed,
    Stopped,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Self::Checking => "checking",
            Self::Downloading => "downloading",
            Self::Uploading => "uploading",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }

    fn stoppable(self) -> bool {
        matches!(self, Self::Checking | Self::Downloading)
    }

    fn active(self) -> bool {
        matches!(self, Self::Checking | Self::Downloading | Self::Uploading)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Job {
    pub id: String,
    pub label: String,
    pub stage: Stage,
    pub message: String,
    pub parts: Vec<PartState>,
    pub log: Vec<String>,
    #[serde(default)]
    pub sending: Option<Sending>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Sending {
    pub sent: u64,
    pub size: u64,
}

impl Sending {
    fn percent(self) -> f64 {
        if self.size == 0 {
            return 0.0;
        }
        (self.sent as f64 * 100.0 / self.size as f64).clamp(0.0, 100.0)
    }

    fn line(self) -> String {
        format!(
            "Kühlt auf dem Gitter ab: Upload zu pCloud · {} von {}",
            gb(self.sent),
            gb(self.size)
        )
    }
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

#[derive(Debug, Deserialize)]
struct Done {
    message: String,
    ok: bool,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Space {
    pub total: u64,
    pub used: u64,
    pub free: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Held {
    #[serde(flatten)]
    pub title: crate::api::Stored,
    pub name: String,
    pub videos: usize,
    pub bytes: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Cloud {
    pub space: Space,
    pub titles: Vec<Held>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct MissingSeason {
    pub number: u32,
    pub downloading: bool,
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

    pub async fn missing(&self, show: u64) -> std::result::Result<Vec<MissingSeason>, String> {
        let response = self
            .client
            .get(format!("{}/api/downloads/shows/{show}/missing", self.base))
            .timeout(MISSING_WAIT)
            .send()
            .await
            .map_err(|_| "The download service is not reachable".to_string())?;
        if !response.status().is_success() {
            return Err(format!("The download service answered {}", response.status()));
        }
        response.json().await.map_err(|error| error.to_string())
    }

    pub async fn pcloud(&self) -> std::result::Result<Cloud, String> {
        let response = self
            .client
            .get(format!("{}/api/downloads/pcloud", self.base))
            .timeout(PCLOUD_WAIT)
            .send()
            .await
            .map_err(|_| "The download service is not reachable".to_string())?;
        if !response.status().is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|problem| problem["error"].as_str().map(str::to_string))
                .unwrap_or(text));
        }
        response.json().await.map_err(|error| error.to_string())
    }

    pub async fn delete_title(&self, title: &crate::api::Stored) -> (bool, String) {
        match self.post::<Done>("/api/downloads/pcloud/delete", &json!(title)).await {
            Ok(done) => (done.ok, done.message),
            Err(error) => (false, error),
        }
    }

    async fn suggest(&self, text: &str) -> std::result::Result<Vec<Suggestion>, String> {
        let mut url =
            reqwest::Url::parse(&format!("{}/api/downloads/suggest", self.base)).map_err(|error| error.to_string())?;
        url.query_pairs_mut().append_pair("text", text);
        let response = self
            .client
            .get(url)
            .timeout(MATCH_WAIT)
            .send()
            .await
            .map_err(|_| "The download service is not reachable".to_string())?;
        if !response.status().is_success() {
            return Err(format!("TMDB did not answer ({})", response.status()));
        }
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
    let found = downloads(cx)?
        .search(json!({ "text": text, "from": "web", "by": asked_by(cx) }))
        .await;
    Ok(found.map_or_else(|error| format!("!{error}"), |proposal| proposal.id))
}

#[procedure]
pub async fn pick(cx: &Cx, kind: String, tmdb: u64, season: i64, episode: i64, asked: String) -> Result<String> {
    let number = |value: i64| u32::try_from(value).ok().filter(|value| *value > 0);
    let body = json!({
        "text": asked,
        "from": "web",
        "by": asked_by(cx),
        "pick": { "kind": kind, "tmdb": tmdb, "season": number(season), "episode": number(episode) },
    });
    let found = downloads(cx)?.search(body).await;
    Ok(found.map_or_else(|error| format!("!{error}"), |proposal| proposal.id))
}

#[procedure]
pub async fn confirm(cx: &Cx, proposal: String, choice: u32) -> Result<String> {
    Ok(downloads(cx)?
        .outcome(
            "/api/downloads/confirm",
            json!({ "proposal": proposal, "choice": choice, "by": asked_by(cx) }),
        )
        .await)
}

#[procedure]
pub async fn dismiss(cx: &Cx, proposal: String) -> Result<String> {
    Ok(downloads(cx)?
        .outcome("/api/downloads/dismiss", json!({ "proposal": proposal }))
        .await)
}

#[procedure]
pub async fn stop(cx: &Cx, job: String) -> Result<String> {
    crate::auth::admin(cx)?;
    Ok(downloads(cx)?
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
    match job.stage {
        Stage::Downloading if job.parts.iter().any(|part| part.phase == PartPhase::Checking) => {
            format!("{} downloaded before the restart · {uploaded} bytes uploaded", gb(done))
        }
        Stage::Downloading if total > 0 => format!(
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
    match job.stage {
        Stage::Uploading | Stage::Done => 100.0,
        _ if total == 0 => 0.0,
        _ => (done as f64 * 100.0 / total as f64).clamp(0.0, 100.0),
    }
}

#[component]
async fn job_row(cx: &Cx, job: &Job) -> Result<impl View> {
    let said = signal(cx, String::new);
    let id = job.id.clone();
    let stoppable = job.stage.stoppable() && crate::auth::admin(cx).is_ok();
    let last = job.log.last().cloned().unwrap_or_default();
    let sending = job.sending.filter(|_| job.stage.active());
    Ok(view! {
        <li class="download-job" data-stage=(job.stage.name())>
            <div class="download-job-line">
                <b>(job.label.as_str())</b>
                <span>(job.message.as_str())</span>
            </div>
            if job.stage != Stage::Done {
                <div class="download-bar"><i style=(format!("width:{:.1}%", percent(job)))></i></div>
                <p class="download-facts">(progress_line(job))</p>
                if let Some(sending) = sending {
                    <div class="download-bar"><i style=(format!("width:{:.1}%", sending.percent()))></i></div>
                    <p class="download-facts">(sending.line())</p>
                }
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
    let downloads = downloads(cx)?.clone();
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
                                <p class="setting-note">"Noch nichts gebacken. Such oben einen Film oder eine Staffel."</p>
                            } else {
                                <ul class="download-jobs">
                                    #[key(job.id.clone())]
                                    for job in &overview.jobs {
                                        job_row(job: job)
                                    }
                                </ul>
                            }
                            <p class="setting-note">(format!("Noch {} Platz im Regal für neue Downloads", gb(overview.room)))</p>
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
async fn choice_card(cx: &Cx, id: &str, index: usize, choice: &Choice) -> Result<impl View> {
    let said = signal(cx, String::new);
    let id = id.to_string();
    let number = index as u32;
    let class = if index == 0 {
        "download-choice best"
    } else {
        "download-choice"
    };
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
async fn question_buttons(asked: &str, question: &Question, shown: Signal<String>) -> Result<impl View> {
    let season = question.season.map_or(0, i64::from);
    let episode = question.episode.map_or(0, i64::from);
    let asked = asked.to_string();
    Ok(view! {
        <p class="setting-note">(question.text.as_str())</p>
        <div class="tracks">
            #[key(option.key())]
            for option in &question.options {
                let kind = option.kind.clone();
                let tmdb = option.tmdb;
                let season = option.season.map_or(season, i64::from);
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

#[component]
async fn searched_view(id: &str, searched: &Searched) -> Result<impl View> {
    Ok(view! {
        if searched.choices.is_empty() {
            <p class="setting-note">(searched.message.as_str())</p>
        } else {
            <p class="setting-note">(format!(
                "{} releases checked, {} dropped by the checks.",
                searched.dropped + searched.choices.len(),
                searched.dropped
            ))</p>
            <ul class="download-choices">
                #[key(index)]
                for (index, choice) in searched.choices.iter().enumerate() {
                    choice_card(id: id, index: index, choice: choice)
                }
            </ul>
        }
        for problem in &searched.problems {
            <p class="setting-note">(problem.as_str())</p>
        }
    })
}

#[shard]
pub async fn proposal_view(cx: &Cx, id: String, shown: Signal<String>) -> Result<impl View> {
    let found = match id.strip_prefix('!') {
        Some(error) => Err(error.to_string()),
        None if id.is_empty() => Ok(None),
        None => {
            let overview = downloads(cx)?.overview().await.unwrap_or_default();
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
                    <h2>(found.heading().to_string())</h2>
                    match &found.answer {
                        Answer::Refused { message } => <p class="setting-note">(message.as_str())</p>,
                        Answer::Question(question) => question_buttons(asked: found.asked.as_str(), question: question, shown: shown),
                        Answer::Searched(searched) => searched_view(id: found.id.as_str(), searched: searched),
                    }
                </section>
            },
        }
    })
}

#[shard]
pub async fn title_matches(cx: &Cx, text: String, shown: Signal<String>, busy: Signal<bool>) -> Result<impl View> {
    let text: String = text.trim().chars().take(LONGEST_REQUEST).collect();
    let found = if text.chars().count() < SHORTEST_MATCHED {
        Ok(Vec::new())
    } else {
        tokio::time::sleep(MATCH_PAUSE).await;
        downloads(cx)?.suggest(&text).await
    };
    Ok(view! {
        match found {
            Err(error) => <p class="setting-note">(error)</p>,
            Ok(found) => {
                if !found.is_empty() {
                    <ul class="download-matches">
                        #[key(found.key())]
                        for found in &found {
                            let kind = found.kind.clone();
                            let tmdb = found.tmdb;
                            let season = found.season.map_or(0, i64::from);
                            let whole_season: i64 = 0;
                            let asked = found.title.clone();
                            <li>
                                <button type="button" class="download-match" :disabled=$(busy.get()) @click=$(async |_e| {
                                    busy.set(true);
                                    let id = pick(kind.to_owned(), tmdb, season, whole_season, asked.to_owned()).await;
                                    shown.set(id);
                                    busy.set(false);
                                })>
                                    <span class="art poster" style=(crate::pages::tint(&found.title))>
                                        if let Some(poster) = found.poster() {
                                            <img src=(poster) alt="" loading="lazy" decoding="async" onerror="this.remove()">
                                        }
                                    </span>
                                    <span class="download-match-text">
                                        <b>(found.title.as_str())</b>
                                        <span>(found.facts())</span>
                                    </span>
                                </button>
                            </li>
                        }
                    </ul>
                }
            },
        }
    })
}

#[component]
pub async fn season_downloads(cx: &Cx, show: u64, title: &str) -> Result<impl View> {
    let missing = if crate::auth::downloads(cx) {
        app_context::<Downloads>(cx).missing(show).await.unwrap_or_default()
    } else {
        Vec::new()
    };
    let shown = signal(cx, String::new);
    let busy = signal(cx, || false);
    Ok(view! {
        if !missing.is_empty() {
            <section class="detail-section season-downloads">
                <h2>"More seasons"</h2>
                <div class="season-download-buttons">
                    #[key(season.number)]
                    for season in &missing {
                        let number = i64::from(season.number);
                        let whole_season: i64 = 0;
                        let asked = format!("{title} season {}", season.number);
                        if season.downloading {
                            <p class="setting-note">(format!("Staffel {} ist im Ofen", season.number))</p>
                        } else {
                            <button type="button" class="pill quiet" :disabled=$(busy.get()) @click=$(async |_e| {
                                busy.set(true);
                                let id = pick("show".to_owned(), show, number, whole_season, asked.to_owned()).await;
                                shown.set(id);
                                busy.set(false);
                            })>(format!("Download season {}", season.number))</button>
                        }
                    }
                </div>
                <p class="setting-note" :hidden=$(!busy.get())>"Der Teig wird geknetet: Suche bei den Indexern…"</p>
                proposal_view(id: $(shown.get()), shown: shown.clone())
            </section>
        }
    })
}

#[component]
pub async fn search_download(cx: &Cx, query: Signal<String>) -> Result<impl View> {
    let shown = signal(cx, String::new);
    let busy = signal(cx, || false);
    Ok(view! {
        if crate::auth::downloads(cx) {
            <section class="settings search-download" :hidden=$(query.get().trim().is_empty())>
                <h2>"Not in the library?"</h2>
                title_matches(text: $(query.get()), shown: shown.clone(), busy: busy.clone())
                <p class="setting-note" :hidden=$(!busy.get())>"Der Teig wird geknetet: Suche bei den Indexern…"</p>
                proposal_view(id: $(shown.get()), shown: shown.clone())
                <p class="setting-note">"Tap a match to download it, or type the full title, like “Dark season 2”, and tap Download."</p>
                <div class="search-download-actions">
                    <button type="button" class="pill" :disabled=$(busy.get()) @click=$(async |_e| {
                        busy.set(true);
                        let id = find(query.get()).await;
                        shown.set(id);
                        busy.set(false);
                    })>"Download “" $(query.get().trim().to_owned()) "”"</button>
                    link(href: "/downloads", attrs: attributes! { cx => class="pill quiet" }, "All downloads")
                </div>
            </section>
        }
    })
}

#[component]
async fn waiting(cx: &Cx, shown: Signal<String>) -> Result<impl View> {
    let overview = downloads(cx)?.overview().await.unwrap_or_default();
    let asked: Vec<Proposal> = overview
        .proposals
        .into_iter()
        .filter(|proposal| proposal.from == "tv" && proposal.found())
        .collect();
    Ok(view! {
        if !asked.is_empty() {
            <section class="settings">
                <h2>"Asked for on the TV"</h2>
                <div class="tracks">
                    #[key(found.id.clone())]
                    for found in &asked {
                        let id = found.id.clone();
                        <button type="button" class="track" @click=$(|_e| shown.set(id.to_owned()))>
                            <b>(found.heading().to_string())</b>
                            <span>(found.message().to_string())</span>
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
            title_matches(text: $(text.get()), shown: shown.clone(), busy: busy.clone())
            <p class="setting-note" :hidden=$(!busy.get())>"Der Teig wird geknetet: Suche bei TMDB und den Indexern…"</p>
            proposal_view(id: $(shown.get()), shown: shown.clone())
            waiting(shown: shown)
            job_list()
        </article>
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(stage: &str) -> serde_json::Result<Job> {
        serde_json::from_value(json!({
            "id": "1", "label": "Up", "stage": stage, "message": "", "log": [],
            "parts": [{ "phase": "downloading", "progress": 50, "total": 100, "uploaded": 0, "speed": 0.0, "peers": null }]
        }))
    }

    #[test]
    fn a_title_match_reads_as_kind_year_season_and_whether_the_library_has_it() {
        let read = |value: Value| serde_json::from_value::<Suggestion>(value).unwrap();
        let show = read(
            json!({ "kind": "show", "tmdb": 3, "title": "Would I Lie to You?", "year": 2007,
            "poster": "/uk.jpg", "season": 2, "owned": false }),
        );
        assert_eq!(show.facts(), "Series · 2007 · season 2");
        assert_eq!(show.poster().as_deref(), Some("https://image.tmdb.org/t/p/w154/uk.jpg"));
        let film = read(json!({ "kind": "movie", "tmdb": 4, "title": "Dune", "year": null,
            "poster": "/../x.jpg", "season": null, "owned": true }));
        assert_eq!(film.facts(), "Film · in the library");
        assert_eq!(film.poster(), None);
        assert_ne!(show.key(), film.key());
        let directed = read(
            json!({ "kind": "movie", "tmdb": 27205, "title": "Inception", "year": 2010,
            "poster": null, "season": null, "owned": true, "credit": "directed by Christopher Nolan" }),
        );
        assert_eq!(
            directed.facts(),
            "Film · 2010 · directed by Christopher Nolan · in the library"
        );
    }

    #[test]
    fn every_stage_the_tv_sends_reads_and_an_unknown_one_does_not() {
        for stage in ["checking", "downloading", "uploading", "done", "failed", "stopped"] {
            assert_eq!(job(stage).unwrap().stage.name(), stage);
        }
        assert!(job("finished").is_err());
    }

    #[test]
    fn only_a_running_download_can_be_stopped_and_an_upload_counts_as_complete() {
        let downloading = job("downloading").unwrap();
        assert!(downloading.stage.stoppable());
        assert_eq!(percent(&downloading), 50.0);
        let uploading = job("uploading").unwrap();
        assert!(!uploading.stage.stoppable());
        assert_eq!(percent(&uploading), 100.0);
    }

    #[test]
    fn an_upload_in_progress_reads_as_bytes_sent_of_the_file_size() {
        assert!(
            job("uploading").unwrap().sending.is_none(),
            "a TV without upload progress still reads"
        );
        let mut sent = job("uploading").unwrap();
        sent.sending = serde_json::from_value(json!({ "sent": 1_200_000_000u64, "size": 4_000_000_000u64 })).unwrap();
        let sending = sent.sending.unwrap();
        assert_eq!(sending.percent(), 30.0);
        assert_eq!(
            sending.line(),
            "Kühlt auf dem Gitter ab: Upload zu pCloud · 1.2 GB von 4.0 GB"
        );
        assert_eq!(Sending { sent: 5, size: 0 }.percent(), 0.0);
    }

    #[test]
    fn a_proposal_reads_as_refused_a_question_with_its_season_or_a_search() {
        let read = |answer: Value| {
            let mut proposal = json!({ "id": "p", "asked": "dark season 2", "from": "tv", "created": 1 });
            proposal
                .as_object_mut()
                .unwrap()
                .extend(answer.as_object().unwrap().clone());
            serde_json::from_value::<Proposal>(proposal)
        };
        let question =
            read(json!({ "outcome": "question", "text": "Which one?", "options": [], "season": 2, "episode": null }))
                .unwrap();
        assert!(matches!(
            &question.answer,
            Answer::Question(Question { season: Some(2), .. })
        ));
        assert!(!question.found());
        let seasons = read(
            json!({ "outcome": "question", "text": "Which season of Dark?", "season": null, "episode": null,
            "options": [{ "kind": "show", "tmdb": 7, "label": "Season 1", "season": 1 },
                        { "kind": "show", "tmdb": 7, "label": "Season 3", "season": 3 },
                        { "kind": "movie", "tmdb": 7, "label": "Dark (2005, film)" }] }),
        )
        .unwrap();
        let Answer::Question(Question { options, .. }) = &seasons.answer else {
            panic!("no question");
        };
        assert_eq!(
            options.iter().map(Candidate::key).collect::<Vec<_>>(),
            vec!["show-7-1", "show-7-3", "movie-7-0"]
        );
        let empty = read(
            json!({ "outcome": "searched", "label": "Dark S2", "message": "No releases of Dark S2 found",
            "target": {}, "choices": [], "dropped": 0, "problems": [] }),
        )
        .unwrap();
        assert_eq!((empty.heading(), empty.found()), ("Dark S2", false));
        let refused = read(json!({ "outcome": "refused", "message": "Nothing called \"x\" on TMDB" })).unwrap();
        assert_eq!(
            (refused.heading(), refused.message()),
            ("dark season 2", "Nothing called \"x\" on TMDB")
        );
        assert!(read(json!({ "outcome": "searched", "message": "x" })).is_err());
        assert!(read(json!({ "ok": true, "message": "x" })).is_err());
    }
}
