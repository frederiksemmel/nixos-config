use crate::api::{Api, FeedbackStatus, FeedbackSummary, Resolution};
use crate::auth::admin;
use crate::layout::back;
use crate::public::Public;
use std::time::{SystemTime, UNIX_EPOCH};
use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::router::page;
use topcoat::runtime::{Event, Signal, procedure, signal};
use topcoat::view::{View, component, view};

const FILTERS: [(&str, &str); 4] = [
    ("all", "All"),
    ("new", "New"),
    ("needs_review", "Needs review"),
    ("done", "Done"),
];

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
        FeedbackStatus::New => "new",
        FeedbackStatus::NeedsReview { .. } => "needs_review",
        FeedbackStatus::Done { .. } => "done",
    }
}

fn badge(status: &FeedbackStatus) -> String {
    match status {
        FeedbackStatus::New => "New".to_string(),
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
        FeedbackStatus::NeedsReview { why, decide } => format!("{why}\n\nTo decide: {decide}"),
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
    Ok(view! {
        <li class="report" :hidden=$(if shown.get() == "all" { false } else { shown.get() != status.get() })>
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
            <pre class="report-copy" id=(copy.as_str()) hidden=(true)>(for_claude(report, dir))</pre>
            <div class="report-actions" :hidden=$(editing.get())>
                <button type="button" class="pill quiet" data-copy=(copy.as_str())>"Copy for Claude"</button>
                <button type="button" class="pill quiet" @click=$(|_e| editing.set(true))>"Set status"</button>
            </div>
            <form class="report-edit" :hidden=$(!editing.get()) @submit=$(async |e: Event| {
                e.prevent_default();
                let answer = mark(id.to_owned(), choice.get(), first.get(), second.get()).await;
                if answer.0 {
                    status.set(answer.1);
                    label.set(answer.2);
                    note.set(answer.3);
                    by.set("Set just now".to_owned());
                    said.set("".to_owned());
                    first.set("".to_owned());
                    second.set("".to_owned());
                    editing.set(false);
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
                <textarea rows="2" maxlength="4000"
                    :hidden=$(choice.get() == "new")
                    :placeholder=$(if choice.get() == "fixed" { "Commit" } else if choice.get() == "no_change" { "Why nothing changes" } else { "Why it needs a person" })
                    :value=$(first.get())
                    @input=$(|e: Event| first.set(e.target.value))></textarea>
                <textarea rows="2" maxlength="4000"
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

#[page("/feedback")]
async fn feedback_page(cx: &Cx) -> Result<impl View> {
    admin(cx)?;
    let list = Api::of(cx).feedback().await;
    let empty = list.as_ref().is_ok_and(|list| list.reports.is_empty());
    let shown = signal(cx, || "all".to_string());
    let counts: Vec<(&'static str, &'static str, String)> = FILTERS
        .iter()
        .map(|&(value, text)| {
            let count = list.as_ref().map_or(0, |list| {
                list.reports
                    .iter()
                    .filter(|report| value == "all" || stage(&report.status) == value)
                    .count()
            });
            (value, text, count.to_string())
        })
        .collect();
    let api = app_context::<Public>(cx).api.clone();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    Ok(view! {
        <article class="page feedback-page">
            back(href: "/settings")
            <h1>"Feedback"</h1>
            match list {
                Ok(_) if empty => {
                    <p class="setting-note">"Nothing sent yet. The speech bubble at the top of every page sends feedback from wherever you are."</p>
                },
                Ok(list) => {
                    <p class="setting-note">"Newest first. Each report keeps what the phone and the TV did just before it was sent. Beast triages new reports: easy fixes land and are marked done, the rest wait here as needing review."</p>
                    <div class="report-filters" role="group" aria-label="Show">
                        #[key(value.to_string())]
                        for (value, text, count) in counts.clone() {
                            <button type="button" :class=$(if shown.get() == value { "track on" } else { "track" })
                                @click=$(|_e| shown.set(value.to_owned()))>
                                <b>(text)</b>
                                <span>(count.as_str())</span>
                            </button>
                        }
                    </div>
                    <ul class="reports">
                        #[key(report.id.clone())]
                        for report in &list.reports {
                            report_row(report: report, dir: list.dir.as_str(), api: api.as_str(), now: now, shown: shown.clone())
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
}
