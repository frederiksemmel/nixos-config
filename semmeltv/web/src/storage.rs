use crate::api::{Api, Download, Storage, Stored};
use crate::downloads::{Cloud, Downloads, Held};
use crate::layout::back;
use std::time::{SystemTime, UNIX_EPOCH};
use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::router::page;
use topcoat::runtime::{procedure, signal};
use topcoat::view::{View, component, view};

const DAY: u64 = 86_400;
const SHOWN_FIRST: usize = 20;

#[procedure]
pub async fn remove(cx: &Cx, kind: String, key: String) -> Result<(bool, String)> {
    match Stored::parse(&kind, key) {
        Some(title) => Ok(Api::of(cx).remove_download(&title).await),
        None => Ok((false, format!("There is no kind of download called {kind}"))),
    }
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
    } else if gb < 1000.0 {
        format!("{gb:.0} GB")
    } else {
        format!("{:.1} TB", gb / 1000.0)
    }
}

fn held_facts(held: &Held) -> String {
    match (&held.title, held.videos) {
        (Stored::Series(_), 1) => "Series · 1 episode".to_string(),
        (Stored::Series(_), videos) => format!("Series · {videos} episodes"),
        _ => "Film".to_string(),
    }
}

fn library_bytes(cloud: &Cloud) -> u64 {
    cloud.titles.iter().map(|held| held.bytes).sum()
}

fn deletable(kind: &str, key: String) -> Option<Stored> {
    Stored::parse(kind, key).filter(|title| !matches!(title, Stored::Other(_)))
}

#[procedure]
pub async fn delete(cx: &Cx, kind: String, key: String) -> Result<(bool, String)> {
    crate::auth::admin(cx)?;
    let Some(title) = deletable(&kind, key) else {
        return Ok((false, "Only a whole film or series can be deleted".to_string()));
    };
    let (deleted, message) = app_context::<Downloads>(cx).delete_title(&title).await;
    if deleted {
        let _ = Api::of(cx).remove_download(&title).await;
    }
    Ok((deleted, message))
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

fn removable(download: &Download) -> bool {
    download.keep.is_none_or(|keep| keep.removable())
}

fn detail(download: &Download, now: u64) -> String {
    let amount = match (&download.title, download.total) {
        (Stored::Other(_), _) => "Not in the library".to_string(),
        (Stored::Series(_), total) => format!("{} of {total} episodes complete", download.complete),
        (Stored::Movie(_), total) if download.complete >= total && total > 0 => "Complete".to_string(),
        _ => "Partly downloaded".to_string(),
    };
    let when = match download.last_watched {
        Some(at) => format!("watched {}", ago(now, at)),
        None => format!("downloaded {}, not watched", ago(now, download.cached_at)),
    };
    format!("{amount} · {when}")
}

fn ranks(titles: &[Download], next: &[Stored]) -> Vec<(usize, usize)> {
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
    let kind = download.title.kind().to_string();
    let key = download.title.key().to_string();
    let keep = download.keep.map(|keep| keep.label());
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
                    #[key(storage.titles[index].title.key().to_string())]
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

#[component]
async fn held_row(cx: &Cx, held: &Held, more: bool) -> Result<impl View> {
    let asking = signal(cx, || false);
    let gone = signal(cx, || false);
    let said = signal(cx, String::new);
    let kind = held.title.kind().to_string();
    let key = held.title.key().to_string();
    Ok(view! {
        <li :class=$(if gone.get() { "storage-title gone" } else { "storage-title" }) data-more=(more.to_string())>
            <div class="storage-title-line">
                <div class="storage-title-text">
                    <b>(held.name.clone())</b>
                    <span>(held_facts(held))</span>
                </div>
                <span class="storage-size">(gb(held.bytes))</span>
            </div>
            <div class="storage-title-line">
                <span class="storage-actions" :hidden=$(if gone.get() { true } else { asking.get() })>
                    <button type="button" class="storage-button" @click=$(|_e| asking.set(true))>"Delete"</button>
                </span>
                <span class="storage-actions" :hidden=$(if asking.get() { false } else { true })>
                    <button type="button" class="storage-button" @click=$(|_e| asking.set(false))>"Keep"</button>
                    <button type="button" class="storage-button danger"
                        @click=$(async |_e| {
                            asking.set(false);
                            said.set("Deleting…".to_owned());
                            let answer = delete(kind.to_owned(), key.to_owned()).await;
                            gone.set(answer.0);
                            said.set(answer.1);
                        })>
                        "Delete from pCloud"
                    </button>
                </span>
            </div>
            <p class="controls-said" aria-live="polite" :hidden=$(said.get().is_empty())>$(said.get())</p>
        </li>
    })
}

#[component]
async fn pcloud_space(cloud: &Cloud) -> Result<impl View> {
    let space = cloud.space;
    let library = library_bytes(cloud).min(space.used);
    Ok(view! {
        <section class="settings pcloud">
            <h2>"pCloud"</h2>
            <div class="storage-meter" role="img" aria-label=(format!("{} of {} used", gb(space.used), gb(space.total)))>
                <i style=(format!("width:{}", percent(space.used - library, space.total)))></i>
                <i class="downloads" style=(format!("width:{}", percent(library, space.total)))></i>
            </div>
            <p class="storage-numbers"><b>(gb(space.free)) " free"</b> " of " (gb(space.total))</p>
            <p class="setting-note">"The library takes " (gb(library)) ". Other files take the rest of the " (gb(space.used)) " used."</p>
        </section>
    })
}

#[component]
async fn pcloud_titles(cx: &Cx, cloud: &Cloud) -> Result<impl View> {
    let all = signal(cx, || false);
    let count = cloud.titles.len();
    Ok(view! {
        <section class="settings">
            <h2>"Library on pCloud"</h2>
            <p class="setting-note">"Deleting a film or a whole series takes it out of the library on both TVs and moves its files to pCloud's trash."</p>
            if cloud.titles.is_empty() {
                <p class="setting-note">"The library is empty."</p>
            } else {
                <ul :class=$(if all.get() { "storage-titles pcloud-titles all" } else { "storage-titles pcloud-titles" })>
                    #[key(format!("{}-{}", cloud.titles[index].title.kind(), cloud.titles[index].title.key()))]
                    for index in 0..count {
                        held_row(held: &cloud.titles[index], more: index >= SHOWN_FIRST)
                    }
                </ul>
                if count > SHOWN_FIRST {
                    <button type="button" class="storage-button pcloud-more" :hidden=$(all.get()) @click=$(|_e| all.set(true))>
                        "Show all " (count) " titles"
                    </button>
                }
            }
        </section>
    })
}

#[component]
async fn disk_section(storage: &Storage) -> Result<impl View> {
    let disk = storage.disk;
    let used = disk.total.saturating_sub(disk.free);
    Ok(view! {
        <section class="settings">
            <h2>"Disk"</h2>
            <div class="storage-meter" role="img" aria-label=(format!("{} of {} used", gb(used), gb(disk.total)))>
                <i style=(format!("width:{}", percent(used - storage.used.min(used), disk.total)))></i>
                <i class="downloads" style=(format!("width:{}", percent(storage.used.min(used), disk.total)))></i>
            </div>
            <p class="storage-numbers"><b>(gb(disk.free)) " free"</b> " of " (gb(disk.total))</p>
            <p class="setting-note">"Downloads take " (gb(storage.used)) ". The system and its apps take the rest of the " (gb(used)) " used."</p>
        </section>
    })
}

#[page("/storage")]
async fn storage_page(cx: &Cx) -> Result<impl View> {
    let api = Api::of(cx);
    let (storage, cloud) = tokio::join!(api.storage(), app_context::<Downloads>(cx).pcloud());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    Ok(view! {
        <article class="storage">
            back(href: "/settings")
            <h1>"Storage"</h1>
            match &storage {
                Ok(storage) => disk_section(storage: storage),
                Err(error) => <section class="settings"><h2>"Disk"</h2><p class="setting-note">(error.clone())</p></section>,
            }
            match &cloud {
                Ok(cloud) => pcloud_space(cloud: cloud),
                Err(error) => <section class="settings pcloud"><h2>"pCloud"</h2><p class="setting-note">"pCloud can't be read: " (error.clone())</p></section>,
            }
            if let Ok(storage) = &storage {
                downloads_section(storage: storage)
                download_list(storage: storage, now: now)
            }
            if let Ok(cloud) = &cloud {
                pcloud_titles(cloud: cloud)
            }
        </article>
    })
}

#[component]
async fn downloads_section(storage: &Storage) -> Result<impl View> {
    let next = storage
        .next
        .first()
        .and_then(|key| storage.titles.iter().find(|title| title.title == *key))
        .map(|title| (title.name.clone(), title.bytes));
    Ok(view! {
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Keep;

    fn download(kind: &str, key: &str, bytes: u64, last_watched: Option<u64>, cached_at: u64) -> Download {
        Download {
            title: Stored::parse(kind, key.to_string()).unwrap(),
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
    fn pcloud_sizes_read_in_terabytes() {
        assert_eq!(gb(2_199_023_255_552), "2.2 TB");
        assert_eq!(gb(999_000_000_000), "999 GB");
    }

    #[test]
    fn pcloud_titles_say_what_they_are_and_the_library_adds_up() {
        let sent = serde_json::json!({
            "space": { "total": 2_000_000_000_000_u64, "used": 800_000_000_000_u64, "free": 1_200_000_000_000_u64 },
            "titles": [
                { "kind": "series", "key": "dark", "name": "Dark", "videos": 26, "bytes": 30_000_000_000_u64 },
                { "kind": "series", "key": "pilot", "name": "Pilot", "videos": 1, "bytes": 1_000_000_000 },
                { "kind": "movie", "key": "up-2009", "name": "Up (2009)", "videos": 1, "bytes": 9_000_000_000_u64 },
            ],
        });
        let cloud: Cloud = serde_json::from_value(sent).unwrap();
        let facts: Vec<String> = cloud.titles.iter().map(held_facts).collect();
        assert_eq!(facts, ["Series · 26 episodes", "Series · 1 episode", "Film"]);
        assert_eq!(cloud.titles[2].title, Stored::Movie("up-2009".to_string()));
        assert_eq!(library_bytes(&cloud), 40_000_000_000);
        assert_eq!(
            deletable("movie", "up-2009".to_string()),
            Some(Stored::Movie("up-2009".to_string()))
        );
        assert_eq!(deletable("other", "Movies/Up".to_string()), None);
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
        title.keep = Some(Keep::Watching);
        assert!(removable(&title));
        title.keep = Some(Keep::UpNext);
        assert!(!removable(&title));
        title.keep = Some(Keep::Playing);
        assert!(!removable(&title));
    }

    #[test]
    fn storage_titles_are_the_json_the_tv_speaks() {
        let report = serde_json::json!({
            "kind": "series", "key": "dark", "name": "Dark", "bytes": 1, "complete": 1, "total": 2,
            "last_watched": null, "cached_at": 0, "keep": "up-next"
        });
        let rows = [
            (
                "series",
                "up-next",
                Some((Stored::Series("dark".to_string()), Some(Keep::UpNext))),
            ),
            ("movie", "", Some((Stored::Movie("dark".to_string()), None))),
            ("series", "up_next", None),
            ("show", "up-next", None),
        ];
        for (kind, keep, want) in rows {
            let mut sent = report.clone();
            sent["kind"] = serde_json::json!(kind);
            sent["keep"] = if keep.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::json!(keep)
            };
            let read = serde_json::from_value::<Download>(sent)
                .ok()
                .map(|download| (download.title, download.keep));
            assert_eq!(read, want, "{kind} {keep}");
            if let Some((title, _)) = want {
                let removal = serde_json::json!({ "kind": kind, "key": "dark" });
                assert_eq!(serde_json::to_value(title).unwrap(), removal, "{kind} {keep}");
            }
        }
        assert_eq!(Stored::parse("show", "dark".to_string()), None);
    }
}
