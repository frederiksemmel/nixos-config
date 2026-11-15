use crate::api::{Api, FeedbackSummary};
use crate::auth::admin;
use crate::layout::back;
use crate::public::Public;
use std::time::{SystemTime, UNIX_EPOCH};
use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::router::page;
use topcoat::view::{View, component, view};

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
async fn report_row(report: &FeedbackSummary, dir: &str, api: &str, now: u64) -> Result<impl View> {
    let who = report.person.as_ref().map_or("Someone".to_string(), |person| person.name.clone());
    let copy = format!("copy-{}", report.id);
    let pictures: Vec<(String, &'static str)> = [
        report.screenshot.as_ref().map(|name| (name, "Their screenshot")),
        report.tv_screenshot.as_ref().map(|name| (name, "The TV's screen")),
    ]
    .into_iter()
    .flatten()
    .map(|(name, label)| (format!("{api}/api/feedback/files/{name}"), label))
    .collect();
    Ok(view! {
        <li class="report">
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
            <pre class="report-copy" id=(copy.as_str()) hidden=(true)>(for_claude(report, dir))</pre>
            <button type="button" class="pill quiet" data-copy=(copy.as_str())>"Copy for Claude"</button>
        </li>
    })
}

#[page("/feedback")]
async fn feedback_page(cx: &Cx) -> Result<impl View> {
    admin(cx)?;
    let list = Api::of(cx).feedback().await;
    let empty = list.as_ref().is_ok_and(|list| list.reports.is_empty());
    let api = app_context::<Public>(cx).api.clone();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
    Ok(view! {
        <article class="page feedback-page">
            back(href: "/settings")
            <h1>"Feedback"</h1>
            match list {
                Ok(_) if empty => {
                    <p class="setting-note">"Nothing sent yet. The speech bubble at the top of every page sends feedback from wherever you are."</p>
                },
                Ok(list) => {
                    <p class="setting-note">"Newest first. Each report keeps what the phone and the TV did just before it was sent."</p>
                    <ul class="reports">
                        for report in &list.reports {
                            report_row(report: report, dir: list.dir.as_str(), api: api.as_str(), now: now)
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
            person: Some(Reporter { name: "Frederik".to_string() }),
            device: Some("iPhone 1a2b".to_string()),
            page: Some("/shows/dark".to_string()),
            message: "The episode list jumped".to_string(),
            screenshot: None,
            tv_screenshot: Some("20261115-131211-ab12-tv.jpg".to_string()),
        };
        let text = for_claude(&report, "/var/lib/semmeltv/feedback");
        assert!(text.starts_with("SemmelTV feedback 20261115-131211-ab12 from Frederik on iPhone 1a2b"));
        assert!(text.contains("ssh semmeltv5 cat /var/lib/semmeltv/feedback/20261115-131211-ab12.json"));
        assert!(text.contains("scp semmeltv5:/var/lib/semmeltv/feedback/20261115-131211-ab12-tv.jpg ."));
    }
}
