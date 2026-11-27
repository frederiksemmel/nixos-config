use crate::db::{self, Db};
use crate::gotcourts::{self, Court, GotCourts};
use crate::poll::Poller;
use crate::slots::{self, Slot, clock};
use crate::time;
use std::collections::{BTreeMap, BTreeSet};
use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::router::content::Form;
use topcoat::router::error::{SeeOther, not_found, see_other};
use topcoat::router::{page, path_param, query_params, route};
use topcoat::view::{View, component, view};

path_param!(id);

/// Where distances count from, and how far ahead to look.
#[derive(Clone)]
pub struct Site {
    pub home: (f64, f64),
    pub days: i64,
}

const HOURS: std::ops::Range<i64> = 6..23;
const FROM_HOURS: [i64; 4] = [12, 17, 18, 19];

pub fn club_info(club: &db::Club) -> gotcourts::Club {
    gotcourts::Club {
        id: club.id,
        name: club.name.clone(),
        canonical: club.canonical.clone(),
        address: club.address.clone(),
        zip: club.zip.clone(),
        city: club.city.clone(),
        latitude: club.latitude,
        longitude: club.longitude,
        phone: club.phone.clone(),
        web: club.web.clone(),
        kind: club.kind.clone(),
    }
}

fn km(site: &Site, club: &db::Club) -> f64 {
    let (lat, lng) = site.home;
    let dy = (club.latitude - lat) * 111.2;
    let dx = (club.longitude - lng) * 111.2 * lat.to_radians().cos();
    (dx * dx + dy * dy).sqrt()
}

fn surfaces(courts: &[Court]) -> String {
    let set: BTreeSet<&str> = courts.iter().map(|c| surface(&c.surface)).collect();
    set.into_iter().collect::<Vec<_>>().join(", ")
}

fn surface(name: &str) -> &str {
    match name {
        "clay" => "clay",
        "hard" => "hard",
        "rebound" => "Rebound Ace",
        "carpet" => "carpet",
        "grass" => "grass",
        "artificial_grass" | "turf" => "artificial grass",
        "" => "?",
        other => other,
    }
}

pub fn sport_name(sport: &str) -> String {
    let mut chars = sport.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Tennis".into(),
    }
}

fn sports(courts: &[Court]) -> String {
    let set: BTreeSet<String> = courts.iter().map(|c| sport_name(&c.sport)).collect();
    set.into_iter().collect::<Vec<_>>().join(", ")
}

fn kind(club: &db::Club) -> &'static str {
    match club.kind.as_str() {
        "public" => "public",
        "mixed" => "members and guests",
        "private" => "members only",
        _ => "",
    }
}

/// What the day page shows: a day, a sport, indoor or outdoor, and from when.
#[derive(Clone, Default)]
struct Filters {
    date: String,
    sport: String,
    only: String,
    from: i64,
}

impl Filters {
    fn keeps(&self, court: &Court) -> bool {
        (self.sport.is_empty() || court.sport == self.sport)
            && match self.only.as_str() {
                "indoor" => court.indoor,
                "outdoor" => !court.indoor,
                _ => true,
            }
    }

    fn link(&self) -> String {
        let mut link = format!("/?date={}", self.date);
        if !self.sport.is_empty() {
            link.push_str(&format!("&sport={}", self.sport));
        }
        if !self.only.is_empty() {
            link.push_str(&format!("&only={}", self.only));
        }
        if self.from > 0 {
            link.push_str(&format!("&from={}", self.from));
        }
        link
    }

    fn date(&self, date: &str) -> String {
        Filters {
            date: date.into(),
            ..self.clone()
        }
        .link()
    }

    fn sport(&self, sport: &str) -> String {
        Filters {
            sport: sport.into(),
            ..self.clone()
        }
        .link()
    }

    fn only(&self, only: &str) -> String {
        Filters {
            only: only.into(),
            ..self.clone()
        }
        .link()
    }

    fn from(&self, from: i64) -> String {
        Filters { from, ..self.clone() }.link()
    }
}

/// How many courts are free starting in each hour.
fn by_hour(free: &[Slot]) -> BTreeMap<i64, usize> {
    let mut hours: BTreeMap<i64, BTreeSet<i64>> = BTreeMap::new();
    for slot in free {
        hours.entry(slot.start / 3600).or_default().insert(slot.court);
    }
    hours.into_iter().map(|(hour, courts)| (hour, courts.len())).collect()
}

fn heat(count: usize) -> &'static str {
    match count {
        0 => "cell none",
        1 => "cell one",
        2 => "cell two",
        _ => "cell many",
    }
}

fn chip(on: bool) -> &'static str {
    if on { "chip on" } else { "chip" }
}

fn current(on: bool) -> &'static str {
    if on { "page" } else { "false" }
}

struct Row {
    club: db::Club,
    km: f64,
    courts: Vec<Court>,
    free: Vec<Slot>,
    hours: BTreeMap<i64, usize>,
    fetched_at: i64,
}

#[query_params]
struct DayQuery {
    date: Option<String>,
    sport: Option<String>,
    only: Option<String>,
    from: Option<i64>,
}

/// Every sport some court of these clubs has, tennis first.
async fn known_sports(db: &Db) -> Vec<String> {
    let mut set: BTreeSet<String> = db
        .clubs()
        .await
        .unwrap_or_default()
        .iter()
        .flat_map(|club| club.courts.0.iter().map(|c| c.sport.clone()).collect::<Vec<_>>())
        .collect();
    let mut list = Vec::new();
    if set.remove("tennis") {
        list.push("tennis".to_string());
    }
    list.extend(set);
    list
}

#[page("/")]
async fn day_page(cx: &Cx) -> Result<impl View> {
    let site = app_context::<Site>(cx).clone();
    let db = app_context::<Db>(cx).clone();
    let query = query_params::<DayQuery>(cx).map_err(|error| topcoat::Error::msg(error.to_string()))?;
    let dates = time::dates(site.days);
    let today = time::today();
    let all_sports = known_sports(&db).await;
    let filters = Filters {
        date: query
            .date
            .clone()
            .filter(|d| dates.contains(d))
            .unwrap_or_else(|| today.clone()),
        sport: query
            .sport
            .clone()
            .filter(|s| all_sports.contains(s))
            .unwrap_or_default(),
        only: query
            .only
            .clone()
            .filter(|o| o == "indoor" || o == "outdoor")
            .unwrap_or_default(),
        from: query.from.unwrap_or(0).clamp(0, 23),
    };
    let date = filters.date.clone();
    let after = (filters.from * 3600).max(if date == today { time::seconds_today() } else { 0 });
    let clubs = db.clubs().await.map_err(topcoat::Error::msg)?;
    let days: BTreeMap<i64, db::Day> = db
        .days_on(&date)
        .await
        .map_err(topcoat::Error::msg)?
        .into_iter()
        .map(|d| (d.club_id, d))
        .collect();
    let mut rows: Vec<Row> = clubs
        .into_iter()
        .filter_map(|club| {
            let stored = days.get(&club.id)?;
            let courts: Vec<Court> = stored.courts.0.iter().filter(|c| filters.keeps(c)).cloned().collect();
            if courts.is_empty() {
                return None;
            }
            let free = slots::free(&courts, &stored.busy.0, after);
            let hours = by_hour(&free);
            Some(Row {
                km: km(&site, &club),
                club,
                courts,
                free,
                hours,
                fetched_at: stored.fetched_at,
            })
        })
        .collect();
    rows.sort_by(|a, b| {
        (a.free.is_empty(), a.km)
            .partial_cmp(&(b.free.is_empty(), b.km))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let oldest = rows.iter().map(|r| r.fetched_at).min();
    let (open, full): (Vec<Row>, Vec<Row>) = rows.into_iter().partition(|r| !r.free.is_empty());
    let free_total: usize = open.iter().map(|r| r.free.len()).sum();
    let several_sports = all_sports.len() > 1;
    let summary = format!(
        "{} free {} at {} {}",
        free_total,
        if free_total == 1 { "slot" } else { "slots" },
        open.len(),
        if open.len() == 1 { "club" } else { "clubs" }
    );
    Ok(view! {
        nav(current: "courts")
        <section class="filters" aria-label="Filters">
            <nav class="chips dates" aria-label="Day">
                for d in &dates {
                    <a class=(chip(*d == date)) aria-current=(current(*d == date)) href=(filters.date(d))>(time::label(d))</a>
                }
            </nav>
            <div class="chips">
                if several_sports {
                    <a class=(chip(filters.sport.is_empty())) href=(filters.sport(""))>"All sports"</a>
                    for sport in &all_sports {
                        <a class=(chip(filters.sport == *sport)) href=(filters.sport(sport))>(sport_name(sport))</a>
                    }
                    <span class="gap" aria-hidden="true"></span>
                }
                <a class=(chip(filters.only.is_empty())) href=(filters.only(""))>"Indoor and outdoor"</a>
                <a class=(chip(filters.only == "indoor")) href=(filters.only("indoor"))>"Indoor"</a>
                <a class=(chip(filters.only == "outdoor")) href=(filters.only("outdoor"))>"Outdoor"</a>
            </div>
            <div class="chips">
                <a class=(chip(filters.from == 0)) href=(filters.from(0))>"Any time"</a>
                for hour in FROM_HOURS {
                    <a class=(chip(filters.from == hour)) href=(filters.from(hour))>(format!("From {hour}:00"))</a>
                }
            </div>
        </section>
        <p class="summary">
            <strong>(time::label(&date))</strong>
            (format!(" · {summary}"))
            if let Some(at) = oldest {
                <span class="quiet">(format!(" · updated {}", time::ago(at)))</span>
            }
        </p>
        if open.is_empty() && full.is_empty() {
            <p class="empty">"Nothing from GotCourts for this day yet. After a restart the first round takes a few minutes."</p>
        }
        if !open.is_empty() {
            <section class="overview" aria-label="Free courts by hour">
                <div class="grid-wrap">
                    <table class="grid">
                        <thead>
                            <tr>
                                <th scope="col"><span class="sr">"Club"</span></th>
                                for hour in HOURS {
                                    <th scope="col">(format!("{hour}"))</th>
                                }
                            </tr>
                        </thead>
                        <tbody>
                            for row in &open {
                                <tr>
                                    <th scope="row"><a href=(format!("#club-{}", row.club.id))>(row.club.name.as_str())</a></th>
                                    for hour in HOURS {
                                        <td class=(heat(row.hours.get(&hour).copied().unwrap_or(0)))>
                                            if let Some(count) = row.hours.get(&hour) {
                                                (count.to_string())
                                            }
                                        </td>
                                    }
                                </tr>
                            }
                        </tbody>
                    </table>
                </div>
                <p class="legend quiet">"Courts free per hour. Tap a club for its times."</p>
            </section>
        }
        <div class="cards">
            for row in &open {
                club_card(row: row, date: &date, several_sports: several_sports)
            }
        </div>
        if !full.is_empty() {
            <section class="full">
                <h2>"Nothing free"</h2>
                <ul>
                    for row in &full {
                        <li>
                            <a href=(format!("/clubs/{}", row.club.id))>(row.club.name.as_str())</a>
                            <span class="quiet">(format!(" · {:.1} km", row.km))</span>
                        </li>
                    }
                </ul>
            </section>
        }
    })
}

#[component]
pub async fn nav(current: &'static str) -> Result<impl View> {
    Ok(view! {
        <header class="top">
            <a class="brand" href="/">"Courts"</a>
            <nav class="tabs" aria-label="Sections">
                <a href="/" aria-current=(if current == "courts" { "page" } else { "false" })>"Free courts"</a>
                <a href="/watches" aria-current=(if current == "watches" { "page" } else { "false" })>"Watches"</a>
                <a href="/feedback" aria-current=(if current == "feedback" { "page" } else { "false" })>"Feedback"</a>
            </nav>
        </header>
    })
}

#[component]
async fn club_card(row: &Row, date: &str, several_sports: bool) -> Result<impl View> {
    let club = &row.club;
    let courts: Vec<(&Court, Vec<&Slot>)> = row
        .courts
        .iter()
        .map(|court| {
            (
                court,
                row.free.iter().filter(|s| s.court == court.id).collect::<Vec<_>>(),
            )
        })
        .filter(|(_, free)| !free.is_empty())
        .collect();
    let book = GotCourts::booking_url(&club.canonical, date);
    let what = if several_sports {
        format!("{} · {}", sports(&row.courts), surfaces(&row.courts))
    } else {
        surfaces(&row.courts)
    };
    Ok(view! {
        <section class="club" id=(format!("club-{}", club.id))>
            <header>
                <div>
                    <h2><a href=(format!("/clubs/{}", club.id))>(club.name.as_str())</a></h2>
                    <p class="meta">
                        (format!("{:.1} km · {} · {}", row.km, what, club.city))
                        if !kind(club).is_empty() {
                            (format!(" · {}", kind(club)))
                        }
                    </p>
                </div>
                <a class="pill" href=(book.as_str()) target="_blank" rel="noopener">"Book"</a>
            </header>
            <ul class="courts">
                for (court, free) in &courts {
                    <li>
                        <span class="court">
                            (court.label.as_str())
                            <small>(format!("{}{}{}", if several_sports { format!("{}, ", sport_name(&court.sport)) } else { String::new() }, surface(&court.surface), if court.indoor { ", indoor" } else { "" }))</small>
                        </span>
                        <span class="times">
                            for slot in free {
                                <a class="time" href=(book.as_str()) target="_blank" rel="noopener">(clock(slot.start))</a>
                            }
                        </span>
                    </li>
                }
            </ul>
        </section>
    })
}

#[page("/clubs/{id}")]
async fn club_page(cx: &Cx) -> Result<impl View> {
    let site = app_context::<Site>(cx).clone();
    let db = app_context::<Db>(cx).clone();
    let id: i64 = path_param::<Id>(cx).parse().map_err(|_| not_found())?;
    let club = db.club(id).await.map_err(topcoat::Error::msg)?.ok_or_else(not_found)?;
    let today = time::today();
    let mut days = db.days_of(id).await.map_err(topcoat::Error::msg)?;
    days.sort_by(|a, b| a.date.cmp(&b.date));
    let weeks: Vec<(String, BTreeMap<i64, usize>)> = days
        .iter()
        .map(|day| {
            let after = if day.date == today { time::seconds_today() } else { 0 };
            (
                day.date.clone(),
                by_hour(&slots::free(&day.courts.0, &day.busy.0, after)),
            )
        })
        .collect();
    let courts = club.courts.0.clone();
    let map = format!(
        "https://www.openstreetmap.org/?mlat={0}&mlon={1}#map=17/{0}/{1}",
        club.latitude, club.longitude
    );
    let web = if club.web.is_empty() || club.web.starts_with("http") {
        club.web.clone()
    } else {
        format!("https://{}", club.web)
    };
    let phone: String = club.phone.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect();
    Ok(view! {
        nav(current: "courts")
        <article class="page club-page">
            <h1>(club.name.as_str())</h1>
            <p class="meta">
                (format!("{:.1} km · {} · {}", km(&site, &club), sports(&courts), surfaces(&courts)))
                if !kind(&club).is_empty() {
                    (format!(" · {}", kind(&club)))
                }
            </p>
            <dl class="facts">
                <dt>"Address"</dt>
                <dd><a href=(map.as_str()) target="_blank" rel="noopener">(format!("{}, {} {}", club.address, club.zip, club.city))</a></dd>
                if !phone.is_empty() {
                    <dt>"Phone"</dt>
                    <dd><a href=(format!("tel:{phone}"))>(club.phone.as_str())</a></dd>
                }
                if !web.is_empty() {
                    <dt>"Web"</dt>
                    <dd><a href=(web.as_str()) target="_blank" rel="noopener">(club.web.as_str())</a></dd>
                }
                <dt>"GotCourts"</dt>
                <dd><a href=(GotCourts::booking_url(&club.canonical, &today)) target="_blank" rel="noopener">"Book there"</a></dd>
            </dl>
            <h2>"Courts"</h2>
            <table class="list">
                <thead><tr><th>"Court"</th><th>"Sport"</th><th>"Surface"</th><th>"Hours"</th><th>"Booked in"</th></tr></thead>
                <tbody>
                    for court in &courts {
                        <tr>
                            <td>
                                (court.label.as_str())
                                if court.ball_machine {
                                    <small>" · ball machine"</small>
                                }
                            </td>
                            <td>(sport_name(&court.sport))</td>
                            <td>(format!("{}{}", surface(&court.surface), if court.indoor { ", indoor" } else { ", outdoor" }))</td>
                            <td>(format!("{}–{}", clock(court.opening), clock(court.closing)))</td>
                            <td>(court.durations.iter().map(|m| format!("{m} min")).collect::<Vec<_>>().join(", "))</td>
                        </tr>
                    }
                </tbody>
            </table>
            <h2>"Free courts by hour"</h2>
            <div class="grid-wrap">
                <table class="grid">
                    <thead>
                        <tr>
                            <th></th>
                            for hour in HOURS {
                                <th>(format!("{hour}"))</th>
                            }
                        </tr>
                    </thead>
                    <tbody>
                        for (date, hours) in &weeks {
                            <tr>
                                <th><a href=(format!("/?date={date}#club-{}", club.id))>(time::label(date))</a></th>
                                for hour in HOURS {
                                    <td class=(heat(hours.get(&hour).copied().unwrap_or(0)))>
                                        if let Some(count) = hours.get(&hour) {
                                            (count.to_string())
                                        }
                                    </td>
                                }
                            </tr>
                        }
                    </tbody>
                </table>
            </div>
        </article>
    })
}

struct Listed {
    watch: db::Watch,
    club: String,
    free: usize,
}

#[page("/watches")]
async fn watches_page(cx: &Cx) -> Result<impl View> {
    let site = app_context::<Site>(cx).clone();
    let db = app_context::<Db>(cx).clone();
    let clubs = db.clubs().await.map_err(topcoat::Error::msg)?;
    let mut listed = Vec::new();
    for watch in db.watches().await.map_err(topcoat::Error::msg)? {
        let mut free = 0;
        for stored in db.days_on(&watch.date).await.map_err(topcoat::Error::msg)? {
            if watch.club_id.is_some_and(|c| c != stored.club_id) {
                continue;
            }
            let Some(club) = clubs.iter().find(|c| c.id == stored.club_id) else {
                continue;
            };
            let day = gotcourts::Day {
                club: club_info(club),
                courts: stored.courts.0,
                busy: stored.busy.0,
            };
            free += crate::poll::matching(&watch, &day).len();
        }
        let club = match watch.club_id {
            Some(id) => clubs
                .iter()
                .find(|c| c.id == id)
                .map_or_else(|| format!("club {id}"), |c| c.name.clone()),
            None => "any club".to_string(),
        };
        listed.push(Listed { watch, club, free });
    }
    let dates = time::dates(site.days);
    let all_sports = known_sports(&db).await;
    Ok(view! {
        nav(current: "watches")
        <article class="page watches">
            <h1>"Watches"</h1>
            <p>"A watch sends a notification when a court frees up that fits it: when someone cancels, or a day opens for booking."</p>
            <section class="notifications" data-notify="section">
                <p data-notify="said">"Notifications on this device: checking…"</p>
                <button type="button" class="pill" data-notify="toggle" hidden=(true)>"Turn on"</button>
            </section>
            if !listed.is_empty() {
                <ul class="watch-list">
                    for item in &listed {
                        <li>
                            <span>
                                <strong>(format!("{} {}–{}", time::label(&item.watch.date), clock(item.watch.earliest), clock(item.watch.latest)))</strong>
                                (format!(" · {}{}{}", item.watch.sport.as_deref().map(|s| format!("{}, ", sport_name(s))).unwrap_or_default(), item.club, if item.watch.indoor { ", indoor only" } else { "" }))
                                <br>
                                <a class="quiet" href=(format!("/?date={}", item.watch.date))>(format!("{} slots fit now", item.free))</a>
                            </span>
                            <form method="post" action=(format!("/watches/{}/delete", item.watch.id))>
                                <button class="pill quiet" type="submit">"Remove"</button>
                            </form>
                        </li>
                    }
                </ul>
            }
            <h2>"New watch"</h2>
            <form class="new-watch" method="post" action="/watches/add">
                <label>"Day"
                    <select name="date">
                        for d in &dates {
                            <option value=(d.as_str())>(time::label(d))</option>
                        }
                    </select>
                </label>
                <label>"From"
                    <select name="earliest">
                        for hour in HOURS {
                            <option value=(hour.to_string()) selected=(hour == 17)>(format!("{hour}:00"))</option>
                        }
                    </select>
                </label>
                <label>"Until"
                    <select name="latest">
                        for hour in 7..24 {
                            <option value=(hour.to_string()) selected=(hour == 21)>(format!("{hour}:00"))</option>
                        }
                    </select>
                </label>
                <label>"Club"
                    <select name="club">
                        <option value="">"Any club"</option>
                        for club in &clubs {
                            <option value=(club.id.to_string())>(club.name.as_str())</option>
                        }
                    </select>
                </label>
                if all_sports.len() > 1 {
                    <label>"Sport"
                        <select name="sport">
                            <option value="">"Any sport"</option>
                            for sport in &all_sports {
                                <option value=(sport.as_str())>(sport_name(sport))</option>
                            }
                        </select>
                    </label>
                }
                <label class="check"><input type="checkbox" name="indoor" value="1"> "Indoor only"</label>
                <button class="pill" type="submit">"Watch"</button>
            </form>
        </article>
    })
}

#[derive(serde::Deserialize)]
struct NewWatch {
    date: String,
    earliest: i64,
    latest: i64,
    club: Option<String>,
    indoor: Option<String>,
    sport: Option<String>,
}

#[route(POST "/watches/add")]
async fn add_watch(cx: &Cx, Form(new): Form<NewWatch>) -> Result<SeeOther> {
    let db = app_context::<Db>(cx).clone();
    let site = app_context::<Site>(cx);
    if time::dates(site.days).contains(&new.date) && new.earliest < new.latest {
        let club = new.club.as_deref().and_then(|c| c.parse().ok());
        let watch = db
            .add_watch(
                club,
                &new.date,
                new.earliest.clamp(0, 24) * 3600,
                new.latest.clamp(0, 24) * 3600,
                new.indoor.is_some(),
                new.sport.clone().filter(|s| !s.is_empty()),
                time::now(),
            )
            .await
            .map_err(topcoat::Error::msg)?;
        app_context::<Poller>(cx).note(&watch).await;
    }
    Ok(see_other("/watches"))
}

#[route(POST "/watches/{id}/delete")]
async fn delete_watch(cx: &Cx) -> Result<SeeOther> {
    let db = app_context::<Db>(cx).clone();
    if let Ok(id) = path_param::<Id>(cx).parse::<i64>() {
        db.forget_watch(id).await.map_err(topcoat::Error::msg)?;
    }
    Ok(see_other("/watches"))
}
