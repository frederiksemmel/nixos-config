use crate::api::{
    Api, CommentAuthor, DeployRun, DeployState, FeedbackComment, FeedbackStatus, FeedbackSummary, Question, Resolution,
};
use crate::auth::admin;
use crate::layout::back;
use crate::public::Public;
use std::time::{SystemTime, UNIX_EPOCH};
use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::router::page;
use topcoat::runtime::{Event, Signal, procedure, signal};
use topcoat::view::{View, component, view};

const CHOICES: [(&str, &str); 4] = [
    ("new", "New"),
    ("needs_review", "Needs review"),
    ("fixed", "Fixed"),
    ("no_change", "No change"),
];

fn age(at: u64, now: u64) -> String {
    let seconds = now.saturating_sub(at);
    match seconds {
        0..60 => "just now".to_string(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..86_400 => format!("{} h ago", seconds / 3600),
        86_400..172_800 => "yesterday".to_string(),
        _ => format!("{} days ago", seconds / 86_400),
    }
}

fn stage(status: &FeedbackStatus) -> &'static str {
    match status {
        FeedbackStatus::New | FeedbackStatus::Reopened { .. } => "new",
        FeedbackStatus::NeedsReview { .. } => "needs_review",
        FeedbackStatus::Done { .. } => "done",
    }
}

fn badge(status: &FeedbackStatus) -> String {
    match status {
        FeedbackStatus::New => "New".to_string(),
        FeedbackStatus::Reopened { .. } => "Reopened".to_string(),
        FeedbackStatus::NeedsReview { .. } => "Needs review".to_string(),
        FeedbackStatus::Done {
            resolution: Resolution::Fixed { commit, .. },
        } => format!("Fixed in {}", commit.chars().take(8).collect::<String>()),
        FeedbackStatus::Done {
            resolution: Resolution::NoChange { .. },
        } => "Done, no change".to_string(),
    }
}

fn explanation(status: &FeedbackStatus) -> String {
    match status {
        FeedbackStatus::New => String::new(),
        FeedbackStatus::Reopened { was } => match explanation(was) {
            before if before.is_empty() => format!("A comment reopened it. Before: {}.", badge(was)),
            before => format!("A comment reopened it. Before: {}.\n\n{before}", badge(was)),
        },
        FeedbackStatus::NeedsReview { why, decide, .. } => format!("{why}\n\nTo decide: {decide}"),
        FeedbackStatus::Done {
            resolution: Resolution::Fixed { what, .. },
        } => what.clone(),
        FeedbackStatus::Done {
            resolution: Resolution::NoChange { why },
        } => why.clone(),
    }
}

fn chosen(choice: &str, first: String, second: String) -> Option<FeedbackStatus> {
    Some(match choice {
        "new" => FeedbackStatus::New,
        "needs_review" => FeedbackStatus::NeedsReview {
            why: first,
            decide: second,
            questions: Vec::new(),
        },
        "fixed" => FeedbackStatus::Done {
            resolution: Resolution::Fixed {
                commit: first,
                what: second,
            },
        },
        "no_change" => FeedbackStatus::Done {
            resolution: Resolution::NoChange { why: first },
        },
        _ => return None,
    })
}

fn author(comment: &FeedbackComment) -> (&'static str, String) {
    match &comment.author {
        CommentAuthor::Person { name } => ("comment person", name.clone()),
        CommentAuthor::Claude => ("comment claude", "Claude".to_string()),
    }
}

fn thread_after(earlier: &str, text: &str) -> String {
    let text = text.trim();
    if earlier.is_empty() {
        text.to_string()
    } else {
        format!("{earlier}\n\n{text}")
    }
}

struct Choice {
    value: String,
    group: String,
    text: String,
    recommended: bool,
}

fn letter(option: usize) -> char {
    char::from(b'a' + (option % 26) as u8)
}

fn asked(status: &FeedbackStatus) -> &[Question] {
    match status {
        FeedbackStatus::NeedsReview { questions, .. } => questions,
        _ => &[],
    }
}

fn picked(questions: &[Question], picks: &str) -> Vec<usize> {
    let mut chosen: Vec<usize> = questions.iter().map(|question| question.recommended).collect();
    for pick in picks.split(';') {
        let Some((question, option)) = pick.split_once('=') else {
            continue;
        };
        let (Ok(question), Ok(option)) = (question.parse::<usize>(), option.parse::<usize>()) else {
            continue;
        };
        if questions
            .get(question)
            .is_some_and(|asked| option < asked.options.len())
        {
            chosen[question] = option;
        }
    }
    chosen
}

fn answered(questions: &[Question], chosen: &[usize], words: &str) -> String {
    let mut text = questions
        .iter()
        .zip(chosen)
        .map(|(question, &option)| format!("{}\n→ {}: {}", question.ask, letter(option), question.options[option]))
        .collect::<Vec<_>>()
        .join("\n\n");
    let words = words.trim();
    if !words.is_empty() {
        text.push_str("\n\n");
        text.push_str(words);
    }
    text
}

async fn commented(cx: &Cx, id: &str, text: &str, earlier: String) -> (bool, String, String, String, String) {
    match Api::of(cx).comment_feedback(id, text).await {
        Ok(status) => (
            true,
            stage(&status).to_string(),
            badge(&status),
            explanation(&status),
            thread_after(&earlier, text),
        ),
        Err(problem) => (false, problem, String::new(), String::new(), earlier),
    }
}

#[procedure]
pub async fn choose(
    cx: &Cx,
    id: String,
    picks: String,
    words: String,
    earlier: String,
) -> Result<(bool, String, String, String, String)> {
    let questions = match Api::of(cx).feedback().await {
        Ok(list) => list
            .reports
            .into_iter()
            .find(|report| report.id == id)
            .map(|report| asked(&report.status).to_vec())
            .unwrap_or_default(),
        Err(problem) => return Ok((false, problem, String::new(), String::new(), earlier)),
    };
    if questions.is_empty() {
        return Ok((
            false,
            "These questions are settled already. Reload the page to see why.".to_string(),
            String::new(),
            String::new(),
            earlier,
        ));
    }
    let text = answered(&questions, &picked(&questions, &picks), &words);
    Ok(commented(cx, &id, &text, earlier).await)
}

#[procedure]
pub async fn say(cx: &Cx, id: String, text: String, earlier: String) -> Result<(bool, String, String, String, String)> {
    Ok(commented(cx, &id, &text, earlier).await)
}

#[procedure]
pub async fn mark(
    cx: &Cx,
    id: String,
    choice: String,
    first: String,
    second: String,
) -> Result<(bool, String, String, String)> {
    let Some(status) = chosen(&choice, first, second) else {
        return Ok((
            false,
            format!("There is no status called {choice}"),
            String::new(),
            String::new(),
        ));
    };
    Ok(match Api::of(cx).mark_feedback(&id, &status).await {
        Ok(status) => (true, stage(&status).to_string(), badge(&status), explanation(&status)),
        Err(problem) => (false, problem, String::new(), String::new()),
    })
}

fn for_claude(report: &FeedbackSummary, dir: &str) -> String {
    let who = report.person.as_ref().map_or("someone", |person| person.name.as_str());
    let device = report.device.as_deref().unwrap_or("an unknown device");
    let page = report.page.as_deref().unwrap_or("an unknown page");
    let mut text = format!(
        "SemmelTV feedback {} from {who} on {device}, {}, at {page}:\n\n{}\n\nFull report with the phone's trail, the TV's state and its journal: ssh {} cat {dir}/{}.json\n",
        report.id, report.when, report.message, report.host, report.id
    );
    for picture in [&report.screenshot, &report.tv_screenshot].into_iter().flatten() {
        text.push_str(&format!("Screenshot: scp {}:{dir}/{picture} .\n", report.host));
    }
    text
}

#[component]
async fn report_row(
    cx: &Cx,
    report: &FeedbackSummary,
    dir: &str,
    api: &str,
    now: u64,
    shown: Signal<String>,
    fresh: Signal<usize>,
    review: Signal<usize>,
    done: Signal<usize>,
) -> Result<impl View> {
    let who = report
        .person
        .as_ref()
        .map_or("Someone".to_string(), |person| person.name.clone());
    let copy = format!("copy-{}", report.id);
    let pictures: Vec<(String, &'static str)> = [
        report.screenshot.as_ref().map(|name| (name, "Their screenshot")),
        report.tv_screenshot.as_ref().map(|name| (name, "The TV's screen")),
    ]
    .into_iter()
    .flatten()
    .map(|(name, label)| (format!("{api}/api/feedback/files/{name}"), label))
    .collect();
    let marked = match (&report.marked_by, report.marked_at) {
        (Some(by), Some(at)) => format!("Set by {by}, {}", age(at, now)),
        _ => String::new(),
    };
    let id = report.id.clone();
    let status = signal(cx, || stage(&report.status).to_string());
    let label = signal(cx, || badge(&report.status));
    let note = signal(cx, || explanation(&report.status));
    let by = signal(cx, || marked.clone());
    let editing = signal(cx, || false);
    let choice = signal(cx, || "needs_review".to_string());
    let first = signal(cx, String::new);
    let second = signal(cx, String::new);
    let said = signal(cx, String::new);
    let words = signal(cx, String::new);
    let mine = signal(cx, String::new);
    let answer = signal(cx, String::new);
    let sent = signal(cx, || 0usize);
    let saved = signal(cx, || 0usize);
    let picks = signal(cx, String::new);
    let extra = signal(cx, String::new);
    let refused = signal(cx, String::new);
    let answers = signal(cx, || 0usize);
    let questions: Vec<(&str, Vec<Choice>)> = asked(&report.status)
        .iter()
        .enumerate()
        .map(|(index, question)| {
            let options = question
                .options
                .iter()
                .enumerate()
                .map(|(option, text)| Choice {
                    value: format!("{index}={option};"),
                    group: format!("{}-question-{index}", report.id),
                    text: format!("{}: {text}", letter(option)),
                    recommended: option == question.recommended,
                })
                .collect();
            (question.ask.as_str(), options)
        })
        .collect();
    let answerer = id.clone();
    let thread: Vec<(&'static str, String, String, &str)> = report
        .comments
        .iter()
        .map(|comment| {
            let (class, name) = author(comment);
            (class, name, age(comment.at, now), comment.text.as_str())
        })
        .collect();
    let speaker = id.clone();
    Ok(view! {
        <li class="report" id=(report.id.as_str()) :hidden=$(if shown.get() == "all" { false } else { shown.get() != status.get() })>
            <p class="report-who">
                <b>(who)</b>
                <span>(report.device.clone().unwrap_or_default())</span>
                <time title=(report.when.as_str())>(age(report.at, now))</time>
            </p>
            <p class="report-message">(report.message.as_str())</p>
            if let Some(page) = &report.page {
                <p class="report-page">"On " (page.as_str())</p>
            }
            if !pictures.is_empty() {
                <div class="report-pictures">
                    for (src, label) in &pictures {
                        <a href=(src.as_str()) target="_blank" rel="noopener"><img src=(src.as_str()) alt=(*label) loading="lazy"></a>
                    }
                </div>
            }
            <div class="report-status">
                <b :class=$(status.get())>$(label.get())</b>
                <span :hidden=$(by.get().is_empty())>$(by.get())</span>
            </div>
            <p class="report-note" :hidden=$(note.get().is_empty())>$(note.get())</p>
            <ol class="report-thread">
                #[key(index)]
                for (index, (class, name, when, text)) in thread.iter().enumerate() {
                    <li class=(*class) data-index=(index.to_string())>
                        <p class="comment-who"><b>(name.as_str())</b> <time>(when.as_str())</time></p>
                        <p class="comment-text">(*text)</p>
                    </li>
                }
                <li class="comment person" :hidden=$(mine.get().is_empty())>
                    <p class="comment-who"><b>"You"</b> <time>"just now"</time></p>
                    <p class="comment-text">$(mine.get())</p>
                </li>
            </ol>
            if !questions.is_empty() {
                <form class="report-answer" :hidden=$(status.get() != "needs_review") :data-sent=$(answers.get()) @submit=$(async |e: Event| {
                    e.prevent_default();
                    let reply = choose(answerer.to_owned(), picks.get(), extra.get(), mine.get()).await;
                    if reply.0 {
                        if status.get() == "new" { fresh.decrement(); } else if status.get() == "needs_review" { review.decrement(); } else { done.decrement(); }
                        status.set(reply.1);
                        if status.get() == "new" { fresh.increment(); } else if status.get() == "needs_review" { review.increment(); } else { done.increment(); }
                        label.set(reply.2);
                        note.set(reply.3);
                        mine.set(reply.4);
                        extra.set("".to_owned());
                        refused.set("".to_owned());
                        answers.set(answers.get() + 1);
                    } else {
                        refused.set(reply.1);
                    }
                })>
                    #[key(index)]
                    for (index, (ask, options)) in questions.iter().enumerate() {
                        <fieldset class="report-question" data-index=(index.to_string())>
                            <legend>(*ask)</legend>
                            #[key(choice.value.clone())]
                            for choice in options.iter() {
                                <label class="report-option">
                                    <input type="radio" name=(choice.group.as_str()) value=(choice.value.as_str()) checked=(choice.recommended)
                                        @change=$(|e: Event| picks.push_str(e.target.value))>
                                    <span>(choice.text.as_str())</span>
                                    if choice.recommended {
                                        <small>"Recommended"</small>
                                    }
                                </label>
                            }
                        </fieldset>
                    }
                    <textarea rows="2" maxlength="4000" placeholder="Anything to add (optional)" aria-label="Anything to add"
                        data-draft=(format!("{}:answer", report.id))
                        :value=$(extra.get())
                        @input=$(|e: Event| extra.set(e.target.value))></textarea>
                    <p class="controls-said" aria-live="polite" :hidden=$(refused.get().is_empty())>$(refused.get())</p>
                    <button type="submit" class="pill">"Answer"</button>
                </form>
            }
            <form class="report-say" :data-sent=$(sent.get()) @submit=$(async |e: Event| {
                e.prevent_default();
                let reply = say(speaker.to_owned(), words.get(), mine.get()).await;
                if reply.0 {
                    if status.get() == "new" { fresh.decrement(); } else if status.get() == "needs_review" { review.decrement(); } else { done.decrement(); }
                    status.set(reply.1);
                    if status.get() == "new" { fresh.increment(); } else if status.get() == "needs_review" { review.increment(); } else { done.increment(); }
                    label.set(reply.2);
                    note.set(reply.3);
                    mine.set(reply.4);
                    words.set("".to_owned());
                    answer.set("".to_owned());
                    sent.set(sent.get() + 1);
                } else {
                    answer.set(reply.1);
                }
            })>
                <textarea rows="2" maxlength="4000" placeholder="Reply, or answer the question" aria-label="Comment"
                    data-draft=(format!("{}:comment", report.id))
                    :value=$(words.get())
                    @input=$(|e: Event| words.set(e.target.value))></textarea>
                <p class="controls-said" aria-live="polite" :hidden=$(answer.get().is_empty())>$(answer.get())</p>
                <button type="submit" class="pill" :disabled=$(words.get().trim().is_empty())>"Comment"</button>
            </form>
            <pre class="report-copy" id=(copy.as_str()) hidden=(true)>(for_claude(report, dir))</pre>
            <div class="report-actions" :hidden=$(editing.get())>
                <button type="button" class="pill quiet" data-copy=(copy.as_str())>"Copy for Claude"</button>
                <button type="button" class="pill quiet" @click=$(|_e| editing.set(true))>"Set status"</button>
            </div>
            <form class="report-edit" :hidden=$(!editing.get()) :data-sent=$(saved.get()) @submit=$(async |e: Event| {
                e.prevent_default();
                let answer = mark(id.to_owned(), choice.get(), first.get(), second.get()).await;
                if answer.0 {
                    if status.get() == "new" { fresh.decrement(); } else if status.get() == "needs_review" { review.decrement(); } else { done.decrement(); }
                    status.set(answer.1);
                    if status.get() == "new" { fresh.increment(); } else if status.get() == "needs_review" { review.increment(); } else { done.increment(); }
                    label.set(answer.2);
                    note.set(answer.3);
                    by.set("Set just now".to_owned());
                    said.set("".to_owned());
                    first.set("".to_owned());
                    second.set("".to_owned());
                    editing.set(false);
                    saved.set(saved.get() + 1);
                } else {
                    said.set(answer.1);
                }
            })>
                <div class="report-choices" role="group" aria-label="Status">
                    for (value, text) in CHOICES {
                        <button type="button" :class=$(if choice.get() == value { "track on" } else { "track" })
                            @click=$(|_e| choice.set(value.to_owned()))>
                            <b>(text)</b>
                        </button>
                    }
                </div>
                <textarea rows="2" maxlength="4000" data-draft=(format!("{}:first", report.id))
                    :hidden=$(choice.get() == "new")
                    :placeholder=$(if choice.get() == "fixed" { "Commit" } else if choice.get() == "no_change" { "Why nothing changes" } else { "Why it needs a person" })
                    :value=$(first.get())
                    @input=$(|e: Event| first.set(e.target.value))></textarea>
                <textarea rows="2" maxlength="4000" data-draft=(format!("{}:second", report.id))
                    :hidden=$(if choice.get() == "new" { true } else { choice.get() == "no_change" })
                    :placeholder=$(if choice.get() == "fixed" { "What the fix changed" } else { "What they have to decide" })
                    :value=$(second.get())
                    @input=$(|e: Event| second.set(e.target.value))></textarea>
                <p class="controls-said" aria-live="polite" :hidden=$(said.get().is_empty())>$(said.get())</p>
                <div class="report-actions">
                    <button type="button" class="pill quiet" @click=$(|_e| editing.set(false))>"Cancel"</button>
                    <button type="submit" class="pill">"Save"</button>
                </div>
            </form>
        </li>
    })
}

fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

#[derive(Debug, PartialEq)]
struct Deployment {
    summary: String,
    status: String,
    offer: bool,
}

fn deployment(state: &DeployState, now: u64) -> Deployment {
    let main = state
        .main
        .as_deref()
        .map_or("main".to_string(), |main| format!("main ({})", short(main)));
    let (mut summary, mut offer) = match (&state.pending, &state.revision) {
        (Some(pending), Some(_)) if pending.is_empty() && state.others == 0 => {
            (format!("This TV runs {main}; nothing waits to deploy."), false)
        }
        (Some(pending), Some(revision)) if pending.is_empty() => (
            format!(
                "This TV runs {}. Main has {} since, only for other machines.",
                short(revision),
                plural(state.others, "commit", "commits")
            ),
            false,
        ),
        (Some(pending), _) => {
            let mut text = format!(
                "{} on main {} built but not on this TV yet.",
                plural(pending.len(), "change", "changes"),
                if pending.len() == 1 { "is" } else { "are" }
            );
            if state.others > 0 {
                text.push_str(&format!(
                    " Main also has {} for other machines.",
                    plural(state.others, "commit", "commits")
                ));
            }
            (text, true)
        }
        (None, Some(revision)) => (
            format!(
                "This TV runs {}, which isn't among main's newest commits. Deploy now brings it to {main}.",
                short(revision)
            ),
            true,
        ),
        (None, None) => (
            format!("This TV doesn't know which commit it runs. Deploy now brings it to {main}."),
            true,
        ),
    };
    if let Some(checked) = state.checked_at {
        summary.push_str(&format!(" Beast checked main {}.", age(checked, now)));
    }
    let mut status = String::new();
    if let Some(asked) = &state.asked {
        status = format!(
            "{} asked for a deploy {}. Beast starts it within half a minute.",
            asked.by,
            age(asked.at, now)
        );
        offer = false;
    } else {
        match &state.run {
            Some(DeployRun::Running { at, by }) => {
                status = format!(
                    "Deploying since {}, asked by {by}. The TV app restarts when it switches; reload then.",
                    age(*at, now)
                );
                offer = false;
            }
            _ if state.waiting.is_some() => {
                let held = state.waiting.as_ref().map_or(0, |waiting| waiting.at);
                status = format!(
                    "Held {} because someone was watching. Beast deploys once nobody watches; Deploy now deploys at once.",
                    age(held, now)
                );
            }
            Some(DeployRun::Failed { at, by, message }) => {
                status = format!("The deploy {by} asked for failed {}: {message}", age(*at, now));
            }
            Some(DeployRun::Finished { at, by }) => {
                status = format!("Deployed {}, asked by {by}.", age(*at, now));
            }
            None => {}
        }
    }
    Deployment { summary, status, offer }
}

#[procedure]
pub async fn deploy_now(cx: &Cx) -> Result<(bool, String)> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    Ok(match Api::of(cx).ask_deploy().await {
        Ok(state) => (true, deployment(&state, now).status),
        Err(problem) => (false, problem),
    })
}

struct Waiting {
    subject: String,
    meta: String,
    reports: Vec<(String, String)>,
}

#[component]
async fn deploy_box(cx: &Cx, state: &DeployState, now: u64) -> Result<impl View> {
    let shown = deployment(state, now);
    let offer = signal(cx, || shown.offer);
    let said = signal(cx, || shown.status.clone());
    let waiting: Vec<Waiting> = state
        .pending
        .iter()
        .flatten()
        .map(|commit| Waiting {
            subject: commit.subject.clone(),
            meta: format!("{} · {}", short(&commit.sha), age(commit.at, now)),
            reports: commit
                .reports
                .iter()
                .map(|report| {
                    let who = report.person.as_deref().unwrap_or("Someone");
                    (format!("#{}", report.id), format!("{who}: {}", report.message))
                })
                .collect(),
        })
        .collect();
    Ok(view! {
        <section class="deploy" aria-label="Deploy">
            <h2>"Deploy"</h2>
            <p class="setting-note">(shown.summary.as_str())</p>
            if !waiting.is_empty() {
                <ol class="deploy-commits">
                    for commit in &waiting {
                        <li>
                            <b>(commit.subject.as_str())</b>
                            <span>(commit.meta.as_str())</span>
                            for (href, text) in &commit.reports {
                                <a href=(href.as_str())>(text.as_str())</a>
                            }
                        </li>
                    }
                </ol>
            }
            <p class="deploy-said" aria-live="polite" :hidden=$(said.get().is_empty())>$(said.get())</p>
            <div class="deploy-actions" :hidden=$(!offer.get())>
                <button type="button" class="pill" @click=$(async |_e: Event| {
                    let reply = deploy_now().await;
                    said.set(reply.1);
                    if reply.0 { offer.set(false); }
                })>"Deploy now"</button>
                <small>"Deploys at once, even mid-film: the TV app restarts."</small>
            </div>
        </section>
    })
}

#[page("/feedback")]
async fn feedback_page(cx: &Cx) -> Result<impl View> {
    admin(cx)?;
    let list = Api::of(cx).feedback().await;
    let deploy = Api::of(cx)
        .deploy_state()
        .await
        .ok()
        .flatten()
        .filter(|state| state.checked_at.is_some());
    let empty = list.as_ref().is_ok_and(|list| list.reports.is_empty());
    let shown = signal(cx, || "all".to_string());
    let tally = |value: &str| {
        list.as_ref().map_or(0, |list| {
            list.reports
                .iter()
                .filter(|report| value == "all" || stage(&report.status) == value)
                .count()
        })
    };
    let all = signal(cx, || tally("all"));
    let fresh = signal(cx, || tally("new"));
    let review = signal(cx, || tally("needs_review"));
    let done = signal(cx, || tally("done"));
    let filters = [
        ("all", "All", all),
        ("new", "New", fresh.clone()),
        ("needs_review", "Needs review", review.clone()),
        ("done", "Done", done.clone()),
    ];
    let api = app_context::<Public>(cx).api.clone();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    Ok(view! {
        <article class="page feedback-page">
            back(href: "/settings")
            <h1>"Feedback"</h1>
            if let Some(state) = &deploy {
                deploy_box(state: state, now: now)
            }
            match list {
                Ok(_) if empty => {
                    <p class="setting-note">"Nothing sent yet. The speech bubble at the top of every page sends feedback from wherever you are."</p>
                },
                Ok(list) => {
                    <p class="setting-note">"Newest first. Each report keeps what the phone and the TV did just before it was sent. Beast triages new reports: easy fixes land and are marked done, the rest wait here as needing review."</p>
                    <div class="report-filters" role="group" aria-label="Show">
                        #[key(value.to_string())]
                        for (value, text, count) in filters.clone() {
                            <button type="button" :class=$(if shown.get() == value { "track on" } else { "track" })
                                @click=$(|_e| shown.set(value.to_owned()))>
                                <b>(text)</b>
                                <span>$(count.get())</span>
                            </button>
                        }
                    </div>
                    <ul class="reports">
                        #[key(report.id.clone())]
                        for report in &list.reports {
                            report_row(report: report, dir: list.dir.as_str(), api: api.as_str(), now: now, shown: shown.clone(), fresh: fresh.clone(), review: review.clone(), done: done.clone())
                        }
                    </ul>
                },
                Err(error) => <p class="setting-note">(error)</p>,
            }
        </article>
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Reporter;

    #[test]
    fn the_deploy_box_says_what_waits_and_offers_a_deploy_only_when_something_does() {
        use crate::api::{DeployAsked, PendingCommit};
        let commit = |sha: &str| PendingCommit {
            sha: sha.to_string(),
            at: 0,
            subject: "semmeltv: a deploy button".to_string(),
            reports: Vec::new(),
        };
        let mut state = DeployState {
            revision: Some("aaaaaaa1111".to_string()),
            checked_at: Some(990),
            main: Some("ccccccc3333".to_string()),
            pending: Some(vec![commit("ccccccc3333"), commit("bbbbbbb2222")]),
            others: 1,
            asked: None,
            waiting: None,
            run: None,
        };
        assert_eq!(
            deployment(&state, 1000),
            Deployment {
                summary: "2 changes on main are built but not on this TV yet. Main also has 1 commit for other machines. Beast checked main just now.".to_string(),
                status: String::new(),
                offer: true,
            }
        );
        state.run = Some(DeployRun::Failed {
            at: 400,
            by: "Frederik".to_string(),
            message: "the build failed".to_string(),
        });
        let failed = deployment(&state, 1000);
        assert_eq!(
            failed.status,
            "The deploy Frederik asked for failed 10 min ago: the build failed"
        );
        assert!(failed.offer);

        state.asked = Some(DeployAsked {
            at: 1000,
            by: "Frederik".to_string(),
        });
        let asked = deployment(&state, 1000);
        assert_eq!(
            asked.status,
            "Frederik asked for a deploy just now. Beast starts it within half a minute."
        );
        assert!(!asked.offer);
        state.asked = None;
        state.run = Some(DeployRun::Running {
            at: 1000,
            by: "Frederik".to_string(),
        });
        assert!(!deployment(&state, 1000).offer);

        state.run = None;
        state.waiting = Some(DeployAsked {
            at: 400,
            by: "beast".to_string(),
        });
        let held = deployment(&state, 1000);
        assert_eq!(
            held.status,
            "Held 10 min ago because someone was watching. Beast deploys once nobody watches; Deploy now deploys at once."
        );
        assert!(held.offer);
        state.waiting = None;

        state.pending = Some(Vec::new());
        state.others = 0;
        let current = deployment(&state, 1000);
        assert_eq!(
            current.summary,
            "This TV runs main (ccccccc); nothing waits to deploy. Beast checked main just now."
        );
        assert!(!current.offer);
        state.others = 2;
        assert_eq!(
            deployment(&state, 1000).summary,
            "This TV runs aaaaaaa. Main has 2 commits since, only for other machines. Beast checked main just now."
        );
        state.pending = None;
        let lost = deployment(&state, 1000);
        assert_eq!(
            lost.summary,
            "This TV runs aaaaaaa, which isn't among main's newest commits. Deploy now brings it to main (ccccccc). Beast checked main just now."
        );
        assert!(lost.offer);
    }

    #[test]
    fn ages_read_the_way_people_say_them() {
        assert_eq!(age(100, 130), "just now");
        assert_eq!(age(0, 600), "10 min ago");
        assert_eq!(age(0, 7200), "2 h ago");
        assert_eq!(age(0, 90_000), "yesterday");
        assert_eq!(age(0, 3 * 86_400), "3 days ago");
    }

    #[test]
    fn the_text_for_claude_says_where_the_full_report_is() {
        let report = FeedbackSummary {
            id: "20261115-131211-ab12".to_string(),
            at: 0,
            when: "2026-11-15 13:12:11 UTC".to_string(),
            host: "semmeltv5".to_string(),
            person: Some(Reporter {
                name: "Frederik".to_string(),
            }),
            device: Some("iPhone 1a2b".to_string()),
            page: Some("/shows/dark".to_string()),
            message: "The episode list jumped".to_string(),
            screenshot: None,
            tv_screenshot: Some("20261115-131211-ab12-tv.jpg".to_string()),
            status: FeedbackStatus::New,
            marked_at: None,
            marked_by: None,
            comments: Vec::new(),
        };
        let text = for_claude(&report, "/var/lib/semmeltv/feedback");
        assert!(text.starts_with("SemmelTV feedback 20261115-131211-ab12 from Frederik on iPhone 1a2b"));
        assert!(text.contains("ssh semmeltv5 cat /var/lib/semmeltv/feedback/20261115-131211-ab12.json"));
        assert!(text.contains("scp semmeltv5:/var/lib/semmeltv/feedback/20261115-131211-ab12-tv.jpg ."));
    }

    #[test]
    fn the_page_offers_each_status_and_says_what_settled_it() {
        let fixed = chosen("fixed", "abc1234def".to_string(), "The sheet stays shut".to_string()).unwrap();
        assert_eq!(stage(&fixed), "done");
        assert_eq!(badge(&fixed), "Fixed in abc1234d");
        assert_eq!(explanation(&fixed), "The sheet stays shut");
        let review = chosen("needs_review", "Two ways".to_string(), "Which one".to_string()).unwrap();
        assert_eq!(stage(&review), "needs_review");
        assert_eq!(explanation(&review), "Two ways\n\nTo decide: Which one");
        assert_eq!(chosen("new", String::new(), String::new()), Some(FeedbackStatus::New));
        assert_eq!(
            badge(&chosen("no_change", "Works as meant".to_string(), String::new()).unwrap()),
            "Done, no change"
        );
        assert_eq!(chosen("later", String::new(), String::new()), None);
    }

    #[test]
    fn a_reopened_report_shows_what_it_was_and_a_thread_grows_from_the_page() {
        let was = chosen("needs_review", "Two ways".to_string(), "a or b".to_string()).unwrap();
        let reopened = FeedbackStatus::Reopened { was: Box::new(was) };
        assert_eq!(stage(&reopened), "new");
        assert_eq!(badge(&reopened), "Reopened");
        assert_eq!(
            explanation(&reopened),
            "A comment reopened it. Before: Needs review.\n\nTwo ways\n\nTo decide: a or b"
        );
        assert_eq!(thread_after("", " 1a "), "1a");
        assert_eq!(thread_after("1a", "and hurry"), "1a\n\nand hurry");
        let claude = FeedbackComment {
            at: 0,
            author: CommentAuthor::Claude,
            text: "Built option a".to_string(),
        };
        assert_eq!(author(&claude), ("comment claude", "Claude".to_string()));
    }

    #[test]
    fn picked_options_answer_a_review_and_untouched_questions_keep_the_recommendation() {
        let question = |ask: &str, options: &[&str], recommended: usize| Question {
            ask: ask.to_string(),
            options: options.iter().map(|option| option.to_string()).collect(),
            recommended,
        };
        let questions = vec![
            question("Where do options come from?", &["Structured", "Parsed", "Buttons"], 0),
            question("Radios or checkboxes?", &["Radios", "Checkboxes"], 0),
        ];
        let review = FeedbackStatus::NeedsReview {
            why: "A feature".to_string(),
            decide: "How".to_string(),
            questions: questions.clone(),
        };
        assert_eq!(asked(&review), questions.as_slice());
        assert!(asked(&FeedbackStatus::New).is_empty());

        assert_eq!(picked(&questions, ""), [0, 0]);
        assert_eq!(picked(&questions, "0=2;0=1;"), [1, 0]);
        assert_eq!(picked(&questions, "1=1;5=0;0=9;x=1;"), [0, 1]);
        assert_eq!(
            answered(&questions, &[1, 0], "  and quickly "),
            "Where do options come from?\n→ b: Parsed\n\nRadios or checkboxes?\n→ a: Radios\n\nand quickly"
        );
        assert_eq!(
            answered(&questions, &[0, 1], ""),
            "Where do options come from?\n→ a: Structured\n\nRadios or checkboxes?\n→ b: Checkboxes"
        );
    }
}
