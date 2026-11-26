use crate::api::{Api, Span, Usage, UsageControl, UsagePerson, UsagePlay, UsageTitle};
use crate::layout::back;
use topcoat::Result;
use topcoat::context::Cx;
use topcoat::router::page;
use topcoat::router::request::uri;
use topcoat::view::{View, component, view};

const PERIODS: [(u64, &str); 4] = [(7, "Week"), (30, "Month"), (90, "3 months"), (365, "Year")];
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const HOUR: u64 = 3_600;

#[component]
pub async fn usage_icon() -> Result<impl View> {
    Ok(view! {
        <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M4.5 19.5h15"></path>
            <path d="M7 16v-5M12 16V6.5M17 16v-8"></path>
        </svg>
    })
}

fn period(query: Option<&str>) -> u64 {
    query
        .unwrap_or_default()
        .split('&')
        .find_map(|pair| pair.strip_prefix("days="))
        .and_then(|days| days.parse::<u64>().ok())
        .filter(|days| PERIODS.iter().any(|(known, _)| known == days))
        .unwrap_or(30)
}

pub fn watched(seconds: u64) -> String {
    let minutes = (seconds + 30) / 60;
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes} min"),
        (hours, 0) => format!("{hours} h"),
        (hours, minutes) => format!("{hours} h {minutes:02}"),
    }
}

fn percent(part: u64, whole: u64) -> String {
    if whole == 0 {
        return "0%".to_string();
    }
    format!("{:.1}%", (part as f64 / whole as f64 * 100.0).clamp(0.0, 100.0))
}

fn ceiling(seconds: u64) -> u64 {
    seconds.div_ceil(HOUR).max(1)
}

fn level(seconds: u64, most: u64) -> String {
    if seconds == 0 || most == 0 {
        return "0".to_string();
    }
    format!("{:.2}", (seconds as f64 / most as f64).sqrt().max(0.15))
}

fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

fn person_facts(person: &UsagePerson) -> String {
    let mut facts = vec![
        plural(person.plays, "play", "plays"),
        plural(person.days, "day", "days"),
    ];
    if let Some(favourite) = &person.favourite {
        facts.push(format!("mostly {favourite}"));
    }
    if person.controls > 0 {
        facts.push(format!(
            "controlled the TV {}",
            plural(person.controls, "time", "times")
        ));
    }
    facts.join(" · ")
}

fn person_where(person: &UsagePerson) -> String {
    let mut facts = vec![
        format!("TV {}", watched(person.tv)),
        format!("phone {}", watched(person.phone)),
    ];
    if let Some(seen) = &person.last_seen {
        facts.push(format!("last seen {seen}"));
    }
    if !person.devices.is_empty() {
        facts.push(person.devices.join(", "));
    }
    facts.join(" · ")
}

fn title_facts(title: &UsageTitle) -> String {
    let by = title
        .people
        .iter()
        .map(|share| format!("{} {}", share.name, watched(share.seconds)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{by} · {}", plural(title.plays, "play", "plays"))
}

fn play_facts(play: &UsagePlay) -> String {
    let device = if play.tv { "TV" } else { play.device.as_str() };
    format!("{} · {device} · {} · {}", play.name, watched(play.seconds), play.when)
}

fn control_action(control: &UsageControl) -> String {
    match control.times {
        1 => control.action.clone(),
        times => format!("{} ×{times}", control.action),
    }
}

fn control_facts(control: &UsageControl) -> String {
    format!("{} · {} · {}", control.name, control.device, control.when)
}

fn marked(index: usize, count: usize) -> bool {
    let step = count.div_ceil(7).max(1);
    index.is_multiple_of(step)
}

fn hour_cell(day: usize, hour: usize, seconds: u64) -> String {
    format!(
        "{} {hour:02}:00–{:02}:00: {}",
        WEEKDAYS[day],
        (hour + 1) % 24,
        watched(seconds)
    )
}

#[component]
async fn split_key() -> Result<impl View> {
    Ok(view! {
        <p class="usage-key"><span class="usage-swatch tv"></span>"TV" <span class="usage-swatch phone"></span>"Phone or laptop"</p>
    })
}

#[component]
async fn hours_chart(usage: &Usage) -> Result<impl View> {
    let most = usage
        .buckets
        .iter()
        .map(|bucket| bucket.tv + bucket.phone)
        .max()
        .unwrap_or(0);
    let top = ceiling(most) * HOUR;
    let count = usage.buckets.len();
    let per = match usage.span {
        Span::Day => "per day",
        Span::Week => "per week",
    };
    Ok(view! {
        <section class="settings">
            <h2>"Hours watched " <small>(per)</small></h2>
            split_key()
            <div class="usage-chart">
                <span class="usage-top">(watched(top))</span>
                <div class="usage-columns" style=(format!("--count:{count}"))>
                    for bucket in &usage.buckets {
                        <div class="usage-column" title=(format!("{}: TV {}, phone {}", bucket.label, watched(bucket.tv), watched(bucket.phone)))>
                            <i class="phone" style=(format!("height:{}", percent(bucket.phone, top)))></i>
                            <i class="tv" style=(format!("height:{}", percent(bucket.tv, top)))></i>
                        </div>
                    }
                </div>
                <div class="usage-marks" style=(format!("--count:{count}"))>
                    for (index, bucket) in usage.buckets.iter().enumerate() {
                        <span>(if marked(index, count) { bucket.mark.as_str() } else { "" })</span>
                    }
                </div>
            </div>
        </section>
    })
}

#[component]
async fn heatmap(usage: &Usage) -> Result<impl View> {
    let most = usage.hours.iter().flatten().copied().max().unwrap_or(0);
    Ok(view! {
        <section class="settings">
            <h2>"When"</h2>
            <p class="setting-note">"Each square is an hour of the week; the brighter, the more was watched then."</p>
            <div class="usage-heatmap">
                for (day, hours) in usage.hours.iter().enumerate() {
                    <span class="usage-day">(WEEKDAYS[day])</span>
                    for (hour, seconds) in hours.iter().enumerate() {
                        <i style=(format!("--v:{}", level(*seconds, most))) title=(hour_cell(day, hour, *seconds))></i>
                    }
                }
                <span></span>
                for hour in 0..24 {
                    <span class="usage-hour">(if hour % 6 == 0 { format!("{hour}") } else { String::new() })</span>
                }
            </div>
        </section>
    })
}

#[component]
async fn people_list(usage: &Usage) -> Result<impl View> {
    let most = usage
        .people
        .iter()
        .map(|person| person.tv + person.phone)
        .max()
        .unwrap_or(0);
    Ok(view! {
        <section class="settings">
            <h2>"People"</h2>
            <ul class="usage-list">
                for person in &usage.people {
                    <li class="usage-row usage-person">
                        <div class="usage-line"><b>(person.name.as_str())</b><span class="usage-amount">(watched(person.tv + person.phone))</span></div>
                        <div class="usage-bar" style=(format!("width:{}", percent(person.tv + person.phone, most)))>
                            <i class="tv" style=(format!("flex-grow:{}", person.tv))></i>
                            <i class="phone" style=(format!("flex-grow:{}", person.phone))></i>
                        </div>
                        <span class="usage-facts">(person_facts(person))</span>
                        <span class="usage-facts">(person_where(person))</span>
                    </li>
                }
            </ul>
        </section>
    })
}

#[component]
async fn titles_list(usage: &Usage) -> Result<impl View> {
    let most = usage.titles.first().map_or(0, |title| title.seconds);
    Ok(view! {
        if !usage.titles.is_empty() {
            <section class="settings">
                <h2>"Most watched"</h2>
                <ol class="usage-list">
                    for title in &usage.titles {
                        <li class="usage-row usage-title">
                            <div class="usage-line"><b>(title.title.as_str())</b><span class="usage-amount">(watched(title.seconds))</span></div>
                            <div class="usage-bar single" style=(format!("width:{}", percent(title.seconds, most)))><i></i></div>
                            <span class="usage-facts">(title_facts(title))</span>
                        </li>
                    }
                </ol>
            </section>
        }
    })
}

#[component]
async fn recent_plays(usage: &Usage) -> Result<impl View> {
    Ok(view! {
        if !usage.plays.is_empty() {
            <section class="settings">
                <h2>"Latest plays"</h2>
                <ul class="usage-list">
                    for play in &usage.plays {
                        <li class="usage-row usage-play">
                            <b>(play.title.as_str())</b>
                            <span class="usage-facts">(play_facts(play))</span>
                        </li>
                    }
                </ul>
            </section>
        }
    })
}

#[component]
async fn tv_controls(usage: &Usage) -> Result<impl View> {
    Ok(view! {
        <section class="settings" id="tv-control">
            <h2>"Who controlled the TV"</h2>
            if usage.controls.is_empty() {
                <p class="setting-note">"Nobody controlled the TV from the app in these " (usage.days) " days."</p>
            } else {
                <ul class="usage-list">
                    for control in &usage.controls {
                        <li class="usage-row usage-control">
                            <b>(control_action(control))</b>
                            <span class="usage-facts">(control_facts(control))</span>
                        </li>
                    }
                </ul>
            }
        </section>
    })
}

#[component]
pub async fn usage_report(usage: &Usage) -> Result<impl View> {
    let totals = &usage.totals;
    Ok(view! {
        <div class="usage-tiles">
            <div class="usage-tile"><b>(watched(totals.tv + totals.phone))</b><span>"watched"</span></div>
            <div class="usage-tile"><b>(totals.plays)</b><span>"plays"</span></div>
            <div class="usage-tile"><b>(totals.people)</b><span>"people"</span></div>
            <div class="usage-tile"><b>(totals.titles)</b><span>"titles"</span></div>
        </div>
        if totals.plays == 0 {
            <p class="setting-note usage-empty">"Nobody watched anything in these " (usage.days) " days."</p>
        } else {
            <section class="settings">
                <div class="usage-split" role="img" aria-label=(format!("TV {}, phone {}", watched(totals.tv), watched(totals.phone)))>
                    <i class="tv" style=(format!("flex-grow:{}", totals.tv))></i>
                    <i class="phone" style=(format!("flex-grow:{}", totals.phone))></i>
                </div>
                <p class="storage-numbers">"On the TV " <b>(watched(totals.tv))</b> " · on a phone or laptop " <b>(watched(totals.phone))</b></p>
            </section>
            hours_chart(usage: usage)
            heatmap(usage: usage)
        }
        if !usage.people.is_empty() {
            people_list(usage: usage)
        }
        tv_controls(usage: usage)
        titles_list(usage: usage)
        recent_plays(usage: usage)
    })
}

#[page("/usage")]
async fn usage_page(cx: &Cx) -> Result<impl View> {
    let days = period(uri(cx).query());
    let usage = Api::of(cx).usage(days).await;
    Ok(view! {
        <article class="usage">
            back(href: "/settings")
            <h1>"Usage"</h1>
            <nav class="usage-periods" aria-label="Period">
                for (period, label) in PERIODS {
                    <a href=(format!("/usage?days={period}")) class=(if period == days { "track on" } else { "track" })><b>(label)</b></a>
                }
            </nav>
            match &usage {
                Ok(usage) => usage_report(usage: usage),
                Err(error) => <p class="setting-note">(error.clone())</p>,
            }
        </article>
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{UsageBucket, UsageShare, UsageTotals};
    use topcoat::view::ViewExt;

    fn sample() -> Usage {
        let mut hours = [[0; 24]; 7];
        hours[1][20] = 3_600;
        hours[1][21] = 900;
        Usage {
            days: 7,
            totals: UsageTotals {
                tv: 3_600,
                phone: 900,
                plays: 2,
                people: 2,
                titles: 2,
            },
            span: Span::Day,
            buckets: (0..7)
                .map(|day| UsageBucket {
                    label: format!("{} {}", WEEKDAYS[day], 13 + day),
                    mark: WEEKDAYS[day].to_string(),
                    tv: if day == 1 { 3_600 } else { 0 },
                    phone: if day == 1 { 900 } else { 0 },
                })
                .collect(),
            hours,
            people: vec![
                UsagePerson {
                    name: "Frederik".to_string(),
                    tv: 3_600,
                    plays: 1,
                    days: 1,
                    favourite: Some("Arrival".to_string()),
                    last_seen: Some("Tuesday 21:00".to_string()),
                    devices: vec!["TV".to_string()],
                    ..UsagePerson::default()
                },
                UsagePerson {
                    name: "Mami".to_string(),
                    phone: 900,
                    plays: 1,
                    days: 1,
                    favourite: Some("Dark".to_string()),
                    devices: vec!["iPad".to_string()],
                    controls: 5,
                    ..UsagePerson::default()
                },
            ],
            titles: vec![
                UsageTitle {
                    title: "Arrival".to_string(),
                    seconds: 3_600,
                    plays: 1,
                    people: vec![UsageShare {
                        name: "Frederik".to_string(),
                        seconds: 3_600,
                    }],
                },
                UsageTitle {
                    title: "Dark".to_string(),
                    seconds: 900,
                    plays: 1,
                    people: vec![UsageShare {
                        name: "Mami".to_string(),
                        seconds: 900,
                    }],
                },
            ],
            plays: vec![UsagePlay {
                at: 1,
                when: "Tuesday 21:00".to_string(),
                name: "Mami".to_string(),
                device: "iPad".to_string(),
                tv: false,
                title: "Dark · S01E01".to_string(),
                seconds: 900,
            }],
            controls: vec![
                UsageControl {
                    at: 3,
                    when: "today 19:31".to_string(),
                    name: "Mami".to_string(),
                    device: "iPad · Safari".to_string(),
                    action: "Volume up".to_string(),
                    times: 4,
                },
                UsageControl {
                    at: 2,
                    when: "today 19:30".to_string(),
                    name: "Mami".to_string(),
                    device: "iPad · Safari".to_string(),
                    action: "Played Dark · S01E02".to_string(),
                    times: 1,
                },
            ],
        }
    }

    async fn render(usage: &Usage) -> String {
        let cx = &Cx::default();
        view! { cx => usage_report(usage: usage) }
            .single()
            .await
            .unwrap()
            .render(cx)
    }

    #[tokio::test]
    async fn the_report_draws_the_charts_and_lists_people_titles_and_plays() {
        let html = render(&sample()).await;
        assert_eq!(html.matches("class=\"usage-column\"").count(), 7);
        assert_eq!(html.matches("--v:").count(), 7 * 24);
        assert!(html.contains("--v:1.00"), "the busiest hour is the brightest");
        assert!(html.contains("--v:0.50"), "a quarter of it shows at half");
        assert!(html.contains("title=\"Tue 20:00–21:00: 1 h\""));
        assert!(html.contains("title=\"Tue 14: TV 1 h, phone 15 min\""));
        assert!(
            html.contains("height:50.0%"),
            "the column's TV part fills 1 h of a 2 h top"
        );
        assert!(html.contains("1 h 15"), "the total watched");
        let frederik = html.find("Frederik").unwrap();
        let mami = html.find("Mami").unwrap();
        assert!(frederik < mami);
        assert!(html.contains("1 play · 1 day · mostly Arrival"));
        assert!(html.contains("TV 1 h · phone 0 min · last seen Tuesday 21:00 · TV"));
        assert!(html.contains("Mami 15 min · 1 play"));
        assert!(html.contains("Mami · iPad · 15 min · Tuesday 21:00"));
    }

    #[tokio::test]
    async fn the_report_lists_who_controlled_the_tv_newest_first() {
        let html = render(&sample()).await;
        assert!(html.contains("Who controlled the TV"));
        assert!(html.contains("1 play · 1 day · mostly Dark · controlled the TV 5 times"));
        let volume = html.find("Volume up ×4").unwrap();
        let played = html.find("Played Dark · S01E02").unwrap();
        assert!(volume < played, "the newest comes first");
        assert!(html.contains("Mami · iPad · Safari · today 19:31"));
        let quiet = render(&Usage {
            days: 7,
            ..Usage::default()
        })
        .await;
        assert!(quiet.contains("Nobody controlled the TV from the app in these 7 days."));
    }

    #[tokio::test]
    async fn an_empty_period_says_nobody_watched() {
        let usage = Usage {
            days: 7,
            ..Usage::default()
        };
        let html = render(&usage).await;
        assert!(html.contains("Nobody watched anything in these 7 days."));
        assert!(!html.contains("usage-heatmap") && !html.contains("Most watched"));
    }

    #[test]
    fn the_period_comes_from_the_query_and_falls_back_to_a_month() {
        assert_eq!(period(Some("days=7")), 7);
        assert_eq!(period(Some("x=1&days=365")), 365);
        assert_eq!(period(Some("days=12")), 30);
        assert_eq!(period(None), 30);
    }

    #[test]
    fn times_and_marks_read_naturally() {
        assert_eq!(watched(0), "0 min");
        assert_eq!(watched(3_000), "50 min");
        assert_eq!(watched(7_200), "2 h");
        assert_eq!(watched(10_200), "2 h 50");
        assert_eq!(ceiling(0), 1);
        assert_eq!(ceiling(3_601), 2);
        let shown: Vec<usize> = (0..30).filter(|index| marked(*index, 30)).collect();
        assert_eq!(shown, [0, 5, 10, 15, 20, 25]);
        assert!((0..7).all(|index| marked(index, 7)));
    }
}
