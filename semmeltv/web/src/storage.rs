use crate::api::{Api, Download, Storage, TitleKey};
use crate::layout::back;
use std::time::{SystemTime, UNIX_EPOCH};
use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::router::page;
use topcoat::runtime::{procedure, signal};
use topcoat::view::{View, component, view};

const DAY: u64 = 86_400;

#[procedure]
pub async fn remove(cx: &Cx, kind: String, key: String) -> Result<(bool, String)> {
    Ok(app_context::<Api>(cx).remove_download(&TitleKey { kind, key }).await)
}

#[component]
pub async fn storage_icon() -> Result<impl View> {
    Ok(view! {
        <svg viewBox="0 0 24 24" aria-hidden="true">
            <rect x="3.5" y="5" width="17" height="14" rx="2.5"></rect>
            <path d="M3.5 13.5h17"></path>
            <path d="M16.5 16.25h.01"></path>
        </svg>
    })
}

fn gb(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb < 10.0 {
        format!("{gb:.1} GB")
    } else {
        format!("{gb:.0} GB")
    }
}

fn ago(now: u64, then: u64) -> String {
    match now.saturating_sub(then) / DAY {
        0 => "today".to_string(),
        1 => "yesterday".to_string(),
        days @ 2..=13 => format!("{days} days ago"),
        days @ 14..=59 => format!("{} weeks ago", days / 7),
        days => format!("{} months ago", days / 30),
    }
}

fn keep_label(keep: Option<&str>) -> Option<&'static str> {
    match keep? {
        "playing" => Some("Playing"),
        "watching" => Some("Currently watching"),
        "up-next" => Some("Up next"),
        "started" => Some("Started"),
        _ => None,
    }
}

fn removable(download: &Download) -> bool {
    !matches!(download.keep.as_deref(), Some("playing" | "up-next"))
}

fn detail(download: &Download, now: u64) -> String {
    let amount = match (download.title.kind.as_str(), download.total) {
        ("other", _) => "Not in the library".to_string(),
        ("series", total) => format!("{} of {total} episodes complete", download.complete),
        (_, total) if download.complete >= total && total > 0 => "Complete".to_string(),
        _ => "Partly downloaded".to_string(),
    };
    let when = match download.last_watched {
        Some(at) => format!("watched {}", ago(now, at)),
        None => format!("downloaded {}, not watched", ago(now, download.cached_at)),
    };
    format!("{amount} · {when}")
}

fn ranks(titles: &[Download], next: &[TitleKey]) -> Vec<(usize, usize)> {
    let mut by_size: Vec<usize> = (0..titles.len()).collect();
    by_size.sort_by_key(|&index| std::cmp::Reverse(titles[index].bytes));
    let mut by_age: Vec<usize> = (0..titles.len()).collect();
    by_age.sort_by_key(|&index| {
        let title = &titles[index];
        let place = next.iter().position(|key| *key == title.title).unwrap_or(next.len());
        (place, title.last_watched.unwrap_or(title.cached_at))
    });
    (0..titles.len())
        .map(|index| {
            let age = by_age.iter().position(|&at| at == index).unwrap_or(0);
            let size = by_size.iter().position(|&at| at == index).unwrap_or(0);
            (age, size)
        })
        .collect()
}

fn percent(part: u64, whole: u64) -> String {
    let share = if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64 * 100.0
    };
    format!("{:.1}%", share.clamp(0.0, 100.0))
}

#[component]
async fn download_row(
    cx: &Cx,
    download: &Download,
    age: usize,
    size: usize,
    next: bool,
    now: u64,
) -> Result<impl View> {
    let asking = signal(cx, || false);
    let gone = signal(cx, || false);
    let said = signal(cx, String::new);
    let kind = download.title.kind.clone();
    let key = download.title.key.clone();
    let keep = keep_label(download.keep.as_deref());
    Ok(view! {
        <li :class=$(if gone.get() { "storage-title gone" } else { "storage-title" })
            style=(format!("--by-age:{age};--by-size:{size}"))>
            <div class="storage-title-line">
                <div class="storage-title-text">
                    <b>(download.name.clone())</b>
                    <span>(detail(download, now))</span>
                </div>
                <span class="storage-size">(gb(download.bytes))</span>
            </div>
            <div class="storage-title-line">
                if let Some(label) = keep {
                    <em class="storage-tag kept">"Kept: " (label)</em>
                } else if next {
                    <em class="storage-tag next">"Removed next"</em>
                } else {
                    <span></span>
                }
                if removable(download) {
                    <span class="storage-actions" :hidden=$(if gone.get() { true } else { asking.get() })>
                        <button type="button" class="storage-button" @click=$(|_e| asking.set(true))>"Remove"</button>
                    </span>
                    <span class="storage-actions" :hidden=$(if asking.get() { false } else { true })>
                        <button type="button" class="storage-button" @click=$(|_e| asking.set(false))>"Keep"</button>
                        <button type="button" class="storage-button danger"
                            @click=$(async |_e| {
                                let answer = remove(kind.to_owned(), key.to_owned()).await;
                                asking.set(false);
                                gone.set(answer.0);
                                said.set(answer.1);
                            })>
                            "Remove download"
                        </button>
                    </span>
                }
            </div>
            <p class="controls-said" aria-live="polite" :hidden=$(said.get().is_empty())>$(said.get())</p>
        </li>
    })
}

#[component]
async fn download_list(cx: &Cx, storage: &Storage, now: u64) -> Result<impl View> {
    let by_size = signal(cx, || false);
    let ranks = ranks(&storage.titles, &storage.next);
    let mut shown: Vec<usize> = (0..storage.titles.len()).collect();
    shown.sort_by_key(|&index| ranks[index].0);
    let next = storage.next.first();
    Ok(view! {
        <section class="settings">
            <h2>"Downloaded titles"</h2>
            if storage.titles.is_empty() {
                <p class="setting-note">"Nothing is downloaded yet."</p>
            } else {
                <div class="storage-sort" role="group" aria-label="Order">
                    <button type="button" :class=$(if by_size.get() { "track" } else { "track on" }) @click=$(|_e| by_size.set(false))>
                        <b>"Removed first"</b>
                    </button>
                    <button type="button" :class=$(if by_size.get() { "track on" } else { "track" }) @click=$(|_e| by_size.set(true))>
                        <b>"Biggest"</b>
                    </button>
                </div>
                <ul :class=$(if by_size.get() { "storage-titles by-size" } else { "storage-titles" })>
                    #[key(storage.titles[index].title.key.clone())]
                    for &index in &shown {
                        let download = &storage.titles[index];
                        let (age, size) = ranks[index];
                        let is_next = next == Some(&download.title);
                        download_row(download: download, age: age, size: size, next: is_next, now: now)
                    }
                </ul>
            }
        </section>
    })
}

#[page("/storage")]
async fn storage_page(cx: &Cx) -> Result<impl View> {
    let storage = app_context::<Api>(cx).storage().await.map_err(topcoat::Error::msg)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let disk = storage.disk;
    let used = disk.total.saturating_sub(disk.free);
    let next = storage
        .next
        .first()
        .and_then(|key| storage.titles.iter().find(|title| title.title == *key))
        .map(|title| (title.name.clone(), title.bytes));
    Ok(view! {
        <article class="storage">
            back(href: "/settings")
            <h1>"Storage"</h1>
            <section class="settings">
                <h2>"Disk"</h2>
                <div class="storage-meter" role="img" aria-label=(format!("{} of {} used", gb(used), gb(disk.total)))>
                    <i style=(format!("width:{}", percent(used - storage.used.min(used), disk.total)))></i>
                    <i class="downloads" style=(format!("width:{}", percent(storage.used.min(used), disk.total)))></i>
                </div>
                <p class="storage-numbers"><b>(gb(disk.free)) " free"</b> " of " (gb(disk.total))</p>
                <p class="setting-note">"Downloads take " (gb(storage.used)) ". The system and its apps take the rest of the " (gb(used)) " used."</p>
            </section>
            if let Some(marks) = storage.marks {
                <section class="settings">
                    <h2>"Downloads"</h2>
                    <div class="storage-meter" role="img" aria-label=(format!("{} of {}", gb(storage.used), gb(marks.high)))>
                        <i class="downloads" style=(format!("width:{}", percent(storage.used, marks.high)))></i>
                        <b class="storage-mark" style=(format!("left:{}", percent(marks.low, marks.high)))></b>
                    </div>
                    <p class="storage-numbers"><b>(gb(storage.used))</b> " of " (gb(marks.high))</p>
                    <p class="setting-note">
                        "Past " (gb(marks.high)) ", or with less than " (gb(marks.floor)) " free on the disk, the TV removes whole titles, least recently watched first, until downloads are under " (gb(marks.low)) ". "
                        "It keeps what is playing, up next, started or in Currently watching."
                    </p>
                    match next {
                        Some((name, bytes)) => <p class="storage-numbers">"Removed next: " <b>(name)</b> " · " (gb(bytes))</p>,
                        None => <p class="setting-note">"Every downloaded title is kept for now."</p>,
                    }
                </section>
            }
            download_list(storage: &storage, now: now)
        </article>
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn download(kind: &str, key: &str, bytes: u64, last_watched: Option<u64>, cached_at: u64) -> Download {
        Download {
            title: TitleKey {
                kind: kind.to_string(),
                key: key.to_string(),
            },
            name: key.to_string(),
            bytes,
            complete: 1,
            total: 1,
            last_watched,
            cached_at,
            keep: None,
        }
    }

    #[test]
    fn sizes_and_ages_read_naturally() {
        assert_eq!(gb(2_340_000_000), "2.3 GB");
        assert_eq!(gb(85_899_345_920), "86 GB");
        assert_eq!(ago(10 * DAY, 10 * DAY), "today");
        assert_eq!(ago(10 * DAY, 9 * DAY), "yesterday");
        assert_eq!(ago(10 * DAY, 5 * DAY), "5 days ago");
        assert_eq!(ago(100 * DAY, 70 * DAY), "4 weeks ago");
        assert_eq!(ago(200 * DAY, 10 * DAY), "6 months ago");
    }

    #[test]
    fn a_row_says_how_complete_it_is_and_when_it_was_watched() {
        let mut dark = download("series", "dark", 1, Some(9 * DAY), 0);
        dark.complete = 3;
        dark.total = 10;
        assert_eq!(detail(&dark, 10 * DAY), "3 of 10 episodes complete · watched yesterday");
        let up = download("movie", "up", 1, None, 10 * DAY);
        assert_eq!(detail(&up, 10 * DAY), "Complete · downloaded today, not watched");
        let stray = download("other", "Movies/Gone", 1, None, 0);
        assert!(detail(&stray, 0).starts_with("Not in the library"));
    }

    #[test]
    fn rows_order_by_removal_or_by_size() {
        let titles = [
            download("movie", "big", 9, Some(50), 0),
            download("movie", "old", 1, Some(10), 0),
            download("movie", "kept", 5, Some(1), 0),
        ];
        let next = [titles[1].title.clone(), titles[0].title.clone()];
        assert_eq!(ranks(&titles, &next), [(1, 0), (0, 2), (2, 1)]);
    }

    #[test]
    fn playing_and_up_next_titles_have_no_remove_button() {
        let mut title = download("movie", "up", 1, None, 0);
        assert!(removable(&title));
        title.keep = Some("watching".to_string());
        assert!(removable(&title));
        title.keep = Some("up-next".to_string());
        assert!(!removable(&title));
        assert_eq!(keep_label(Some("started")), Some("Started"));
    }
}
