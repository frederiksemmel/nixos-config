use crate::db::{self, Db};
use crate::filters::{EARLIEST, Filters, LATEST, LENGTHS, PRESETS, PRICES, WITHIN};
use crate::gotcourts::{self, Court};
use crate::poll::Poller;
use crate::slots::{self, Slot, clock};
use crate::source::Source;
use crate::tariffs::Rates;
use crate::time;
use std::collections::{BTreeMap, BTreeSet};
use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::router::content::Form;
use topcoat::router::error::{SeeOther, not_found, see_other};
use topcoat::router::{page, path_param, route};
use topcoat::view::{View, component, view};

path_param!(id);

/// Where distances count from, and how far ahead to look.
#[derive(Clone)]
pub struct Site {
    pub home: (f64, f64),
    pub days: i64,
}

const HOURS: std::ops::Range<i64> = 6..23;

pub fn club_info(club: &db::Club) -> gotcourts::Club {
    gotcourts::Club {
        id: club.id,
        source: Source::parse(&club.source),
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

/// Where to book at `club`: in GotCourts' app when `iphone`, on today; on
/// Eversports at the day, sport and start when given; else GotCourts' site
/// on today.
fn book(club: &db::Club, date: &str, sport: &str, start: Option<i64>, iphone: bool) -> String {
    let source = Source::parse(&club.source);
    source
        .app_url(club.id)
        .filter(|_| iphone)
        .unwrap_or_else(|| source.booking_url(&club.canonical, Some(date), Some(sport), start))
}

/// Whether `book` opens the club in an app rather than on a site.
fn in_app(club: &db::Club, iphone: bool) -> bool {
    iphone && Source::parse(&club.source).app_url(club.id).is_some()
}

/// The words on the button `book` makes: the app opens on today, so it
/// names the app rather than the day.
fn book_label(club: &db::Club, date: &str, iphone: bool) -> String {
    if in_app(club, iphone) {
        format!("Book in {}", Source::parse(&club.source).name())
    } else {
        format!("Book {}", time::weekday(date))
    }
}

fn km(site: &Site, club: &db::Club) -> f64 {
    distance(site.home, club)
}

fn distance((lat, lng): (f64, f64), club: &db::Club) -> f64 {
    let dy = (club.latitude - lat) * 111.2;
    let dx = (club.longitude - lng) * 111.2 * lat.to_radians().cos();
    (dx * dx + dy * dy).sqrt()
}

fn surfaces(courts: &[Court]) -> String {
    let set: BTreeSet<&str> = courts.iter().map(|c| surface(&c.surface)).collect();
    set.into_iter().collect::<Vec<_>>().join(", ")
}

pub fn surface(name: &str) -> &str {
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

/// "45", or "37.50" when it isn't whole francs.
fn chf(rappen: i64) -> String {
    if rappen % 100 == 0 {
        (rappen / 100).to_string()
    } else {
        format!("{}.{:02}", rappen / 100, rappen % 100)
    }
}

/// How cheap an hour is, for a grid cell's colour.
fn price_heat(hourly: Option<i64>) -> &'static str {
    match hourly {
        None => "cell unpriced",
        Some(rappen) if rappen <= 3000 => "cell cheap",
        Some(rappen) if rappen <= 5000 => "cell fair",
        Some(_) => "cell dear",
    }
}

fn chip(on: bool) -> &'static str {
    if on { "chip on" } else { "chip" }
}

fn current(on: bool) -> &'static str {
    if on { "page" } else { "false" }
}

/// One club's free courts on one of the chosen days, each free slot with
/// what booking it costs and what an hour of it costs (Rappen) when known.
struct DayFree {
    date: String,
    courts: Vec<Court>,
    free: Vec<Slot>,
    costs: Vec<Option<(i64, i64)>>,
}

impl DayFree {
    fn cost(&self, slot: &Slot) -> Option<(i64, i64)> {
        let at = self.free.iter().position(|s| s == slot)?;
        self.costs[at]
    }

    /// The cheapest free slot among those `keep` takes, the earliest of
    /// the cheapest; one with a known price before any without.
    fn cheapest(&self, keep: impl Fn(&Slot) -> bool) -> Option<usize> {
        (0..self.free.len()).filter(|&i| keep(&self.free[i])).min_by_key(|&i| {
            let total = self.costs[i].map(|(total, _)| total);
            (total.is_none(), total, self.free[i].start, self.free[i].court)
        })
    }
}

struct Row {
    club: db::Club,
    km: f64,
    days: Vec<DayFree>,
    fetched_at: i64,
}

impl Row {
    fn total(&self) -> usize {
        self.days.iter().map(|day| day.free.len()).sum()
    }

    /// The first free start: its day and time.
    fn soonest(&self) -> Option<(&str, i64)> {
        self.days.iter().find_map(|day| {
            day.free
                .iter()
                .map(|slot| slot.start)
                .min()
                .map(|start| (day.date.as_str(), start))
        })
    }

    fn courts(&self) -> Vec<Court> {
        let mut seen = BTreeSet::new();
        self.days
            .iter()
            .flat_map(|day| day.courts.iter())
            .filter(|court| seen.insert(court.id))
            .cloned()
            .collect()
    }
}

/// Every sport some court of these clubs has, tennis first.
async fn known_sports(db: &Db, clubs: &[i64]) -> Vec<String> {
    let set: BTreeSet<String> = db
        .clubs(clubs)
        .await
        .unwrap_or_default()
        .iter()
        .flat_map(|club| club.courts.0.iter().map(|c| c.sport.clone()).collect::<Vec<_>>())
        .collect();
    gotcourts::SPORTS
        .iter()
        .filter(|sport| set.contains(**sport))
        .map(|sport| sport.to_string())
        .collect()
}

fn guests(club: &db::Club) -> bool {
    club.kind != "private"
}

fn keeps(filters: &Filters, court: &Court) -> bool {
    court.sport == filters.sport
        && match filters.place.as_str() {
            "indoor" => court.indoor == Some(true),
            "outdoor" => court.indoor == Some(false),
            "unknown" => court.indoor.is_none(),
            _ => true,
        }
        && (filters.surface.is_empty() || surface(&court.surface) == filters.surface)
}

#[derive(serde::Serialize)]
struct Pin {
    id: i64,
    name: String,
    lat: f64,
    lng: f64,
    km: String,
    free: usize,
    times: Vec<PinTime>,
    book: String,
    book_label: String,
    about: String,
}

#[derive(serde::Serialize)]
struct PinTime {
    label: String,
    book: String,
}

/// What one day's stored answers hold, to search with different filters.
struct Found<'a> {
    clubs: &'a [db::Club],
    days: &'a BTreeMap<(i64, String), db::Day>,
    home: (f64, f64),
    today: &'a str,
    rates: &'a Rates,
}

/// How many free starts each other choice in the filter panel would give,
/// keyed "group:value", and the total without the when, length, court and
/// distance limits.
pub struct Facets {
    pub options: BTreeMap<String, usize>,
    pub current: usize,
    pub cleared: usize,
    /// Free starts at any price whose price isn't known.
    pub unpriced: usize,
}

impl Facets {
    fn of(&self, group: &str, value: &str) -> Option<usize> {
        self.options.get(&format!("{group}:{value}")).copied()
    }

    /// How many more starts dropping this group's limit would show.
    fn more(&self, group: &str, value: &str) -> usize {
        self.of(group, value).unwrap_or(0).saturating_sub(self.current)
    }
}

impl Found<'_> {
    fn rows(&self, filters: &Filters) -> Vec<Row> {
        let origin = filters.origin(self.home);
        let until = filters.until * 3600;
        self.clubs
            .iter()
            .filter(|club| filters.members || guests(club))
            .filter_map(|club| {
                let distance = distance(origin, club);
                if filters.within > 0 && distance > filters.within as f64 {
                    return None;
                }
                let mut fetched_at = i64::MAX;
                let days: Vec<DayFree> = filters
                    .dates
                    .iter()
                    .filter_map(|date| {
                        let stored = self.days.get(&(club.id, date.clone()))?;
                        fetched_at = fetched_at.min(stored.fetched_at);
                        let courts: Vec<Court> =
                            stored.courts.0.iter().filter(|c| keeps(filters, c)).cloned().collect();
                        if courts.is_empty() {
                            return None;
                        }
                        let after =
                            (filters.from * 3600).max(if date == self.today { time::seconds_today() } else { 0 });
                        let source = Source::parse(&club.source);
                        let (free, costs): (Vec<Slot>, Vec<Option<(i64, i64)>>) =
                            slots::free_for(&courts, &stored.busy.0, after, filters.length * 60)
                                .into_iter()
                                .filter(|slot| slot.end <= until)
                                .map(|slot| {
                                    let court = courts.iter().find(|c| c.id == slot.court);
                                    let cost = court.and_then(|court| {
                                        Some((
                                            self.rates.cost(source, court, date, &slot)?,
                                            self.rates.hourly(source, court, date, &slot)?,
                                        ))
                                    });
                                    (slot, cost)
                                })
                                .filter(|(_, cost)| {
                                    filters.price == 0 || cost.is_some_and(|(_, hourly)| hourly <= filters.price * 100)
                                })
                                .unzip();
                        Some(DayFree {
                            date: date.clone(),
                            courts,
                            free,
                            costs,
                        })
                    })
                    .collect();
                if days.is_empty() {
                    return None;
                }
                Some(Row {
                    km: distance,
                    club: club.clone(),
                    days,
                    fetched_at,
                })
            })
            .collect()
    }

    fn count(&self, filters: &Filters) -> usize {
        self.rows(filters).iter().map(Row::total).sum()
    }

    fn facets(&self, filters: &Filters, sports: &[String], surfaces: &BTreeSet<String>) -> Facets {
        let mut options = BTreeMap::new();
        let mut put = |key: String, changed: Filters| {
            options.insert(key, self.count(&changed));
        };
        for sport in sports {
            put(
                format!("sport:{sport}"),
                Filters {
                    sport: sport.clone(),
                    surface: String::new(),
                    ..filters.clone()
                },
            );
        }
        for (_, from, until) in PRESETS.iter().copied().chain([("", EARLIEST, LATEST)]) {
            put(
                format!("when:{from}-{until}"),
                Filters {
                    from,
                    until,
                    ..filters.clone()
                },
            );
        }
        for (minutes, _) in LENGTHS {
            put(
                format!("length:{minutes}"),
                Filters {
                    length: minutes,
                    ..filters.clone()
                },
            );
        }
        for place in ["", "indoor", "outdoor", "unknown"] {
            put(
                format!("place:{place}"),
                Filters {
                    place: place.into(),
                    ..filters.clone()
                },
            );
        }
        for name in surfaces.iter().map(String::as_str).chain([""]) {
            put(
                format!("surface:{name}"),
                Filters {
                    surface: name.into(),
                    ..filters.clone()
                },
            );
        }
        for km in WITHIN.iter().copied().chain([0]) {
            put(
                format!("within:{km}"),
                Filters {
                    within: km,
                    ..filters.clone()
                },
            );
        }
        for price in PRICES.iter().copied().chain([0]) {
            put(
                format!("price:{price}"),
                Filters {
                    price,
                    ..filters.clone()
                },
            );
        }
        for members in [false, true] {
            put(
                format!("members:{}", u8::from(members)),
                Filters {
                    members,
                    ..filters.clone()
                },
            );
        }
        let cleared = self.count(&Filters {
            from: EARLIEST,
            until: LATEST,
            length: 60,
            place: String::new(),
            surface: String::new(),
            within: 0,
            price: 0,
            ..filters.clone()
        });
        let unpriced = self
            .rows(&Filters {
                price: 0,
                ..filters.clone()
            })
            .iter()
            .flat_map(|row| &row.days)
            .flat_map(|day| &day.costs)
            .filter(|cost| cost.is_none())
            .count();
        Facets {
            options,
            current: self.count(filters),
            cleared,
            unpriced,
        }
    }
}

#[page("/")]
async fn day_page(cx: &Cx) -> Result<impl View> {
    let site = app_context::<Site>(cx).clone();
    let db = app_context::<Db>(cx).clone();
    let dates = time::dates(site.days);
    let today = time::today();
    let all_sports = known_sports(&db, &app_context::<crate::discover::Clubs>(cx).ids()).await;
    let filters = Filters::of(cx, &dates, &all_sports);
    let date = filters.first_date().to_string();
    let several = filters.dates.len() > 1;
    let iphone = crate::source::on_iphone(cx);
    let clubs = db
        .clubs(&app_context::<crate::discover::Clubs>(cx).ids())
        .await
        .map_err(topcoat::Error::msg)?;
    let mut days: BTreeMap<(i64, String), db::Day> = BTreeMap::new();
    for day in &filters.dates {
        for stored in db.days_on(day).await.map_err(topcoat::Error::msg)? {
            days.insert((stored.club_id, day.clone()), stored);
        }
    }
    let surfaces_here: BTreeSet<String> = clubs
        .iter()
        .flat_map(|club| club.courts.0.iter())
        .filter(|court| court.sport == filters.sport)
        .map(|court| surface(&court.surface).to_string())
        .collect();
    let members_only = clubs.iter().filter(|club| !guests(club)).count();
    let rates = Rates::load(&db).await;
    let found = Found {
        clubs: &clubs,
        days: &days,
        home: site.home,
        today: &today,
        rates: &rates,
    };
    let facets = found.facets(&filters, &all_sports, &surfaces_here);
    let mut rows = found.rows(&filters);
    if filters.sort == "near" {
        rows.sort_by(|a, b| {
            (a.total() == 0, a.km)
                .partial_cmp(&(b.total() == 0, b.km))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    } else {
        rows.sort_by(|a, b| {
            let (sa, sb) = (a.soonest(), b.soonest());
            (sa.is_none(), sa, a.km)
                .partial_cmp(&(sb.is_none(), sb, b.km))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }
    let oldest = rows.iter().map(|r| r.fetched_at).min();
    let pins: Vec<Pin> = rows
        .iter()
        .map(|row| {
            let mut times: Vec<PinTime> = Vec::new();
            for day in &row.days {
                let mut starts: Vec<i64> = day.free.iter().map(|s| s.start).collect();
                starts.sort();
                starts.dedup();
                times.extend(starts.into_iter().map(|start| PinTime {
                    label: if several {
                        format!("{} {}", time::weekday(&day.date), clock(start))
                    } else {
                        clock(start)
                    },
                    book: book(&row.club, &day.date, &filters.sport, Some(start), iphone),
                }));
            }
            times.truncate(8);
            let soonest = row.soonest().map_or(date.as_str(), |(d, _)| d);
            Pin {
                id: row.club.id,
                name: row.club.name.clone(),
                lat: row.club.latitude,
                lng: row.club.longitude,
                km: format!("{:.1} km", row.km),
                free: row.total(),
                times,
                book: book(&row.club, soonest, &filters.sport, None, iphone),
                book_label: book_label(&row.club, soonest, iphone),
                about: format!("{} · {}", surfaces(&row.courts()), kind(&row.club)),
            }
        })
        .collect();
    let pins = serde_json::to_string(&pins).unwrap_or_else(|_| "[]".into());
    let (open, full): (Vec<Row>, Vec<Row>) = rows.into_iter().partition(|r| r.total() > 0);
    let free_total: usize = open.iter().map(Row::total).sum();
    let sport_label = sport_name(&filters.sport);
    let hours: Vec<i64> = (filters.from.max(EARLIEST)..filters.until.min(LATEST)).collect();
    let share_key = app_context::<crate::auth::Key>(cx).0.clone();
    let by_day: Vec<Vec<Option<Pick>>> = open
        .iter()
        .map(|row| {
            filters
                .dates
                .iter()
                .map(|d| {
                    let day = row.days.iter().find(|day| day.date == *d)?;
                    Some(pick(&row.club, day, day.cheapest(|_| true)?, &share_key))
                })
                .collect()
        })
        .collect();
    let by_hours: Vec<Vec<Option<Pick>>> = open
        .iter()
        .map(|row| {
            let day = &row.days[0];
            hours
                .iter()
                .map(|hour| {
                    Some(pick(
                        &row.club,
                        day,
                        day.cheapest(|s| s.start / 3600 == *hour)?,
                        &share_key,
                    ))
                })
                .collect()
        })
        .collect();
    let length_label = LENGTHS
        .iter()
        .find(|(m, _)| *m == filters.length)
        .map_or("1 h", |(_, l)| *l);
    let summary = format!(
        "{} {} at {} {}",
        free_total,
        if free_total == 1 { "start" } else { "starts" },
        open.len(),
        if open.len() == 1 { "club" } else { "clubs" }
    );
    let origin = filters.origin(site.home);
    let map_view = filters.view == "map";
    let when = if several {
        format!("{} days", filters.dates.len())
    } else {
        time::label(&date)
    };
    let weekend = time::weekend(&dates);
    let next = |n: usize| dates.iter().take(n).cloned().collect::<Vec<_>>();
    let presets: Vec<(&str, Vec<String>)> = [
        ("This weekend", weekend),
        ("Next 3 days", next(3)),
        ("Next 7 days", next(7)),
    ]
    .into_iter()
    .filter(|(_, days)| days.len() > 1)
    .collect();
    let discovered = app_context::<crate::discover::Clubs>(cx).state();
    let venues: Vec<String> = clubs
        .iter()
        .filter(|club| Source::parse(&club.source) == Source::Eversports)
        .map(|club| club.name.clone())
        .collect();
    Ok(view! {
        nav(current: "courts")
        <div class="days">
            <nav class="chips dates" aria-label="Days">
                <a class=(if filters.several { "chip toggle on" } else { "chip toggle" }) href=(filters.with_several(!filters.several)) aria-pressed=(filters.several.to_string())>"Several days"</a>
                for d in &dates {
                    <a class=(chip(filters.dates.contains(d))) aria-current=(current(filters.dates.contains(d))) href=(filters.with_date(d))>(time::label(d))</a>
                }
            </nav>
            if filters.several {
                <div class="chips presets">
                    for (label, days) in &presets {
                        <a class=(chip(filters.dates == *days)) href=(filters.with_dates(days.clone()))>(*label)</a>
                    }
                    <span class="quiet hint">"Tap days to add or remove them."</span>
                </div>
            }
        </div>
        filter_form(filters: &filters, sports: &all_sports, surfaces: &surfaces_here, members_only: members_only, facets: &facets)
        <div class=(if map_view { "split show-map" } else { "split" })>
            <section class="results" aria-label="Free courts">
                <div class="results-head">
                    <p class="summary">
                        <strong>(when.as_str())</strong>
                        (format!(" · {summary}"))
                        if let Some(at) = oldest {
                            <span class="quiet">(format!(" · updated {}", time::ago(at)))</span>
                        }
                    </p>
                    <nav class="seg views" aria-label="View">
                        <a href=(filters.with_view("list")) aria-current=(current(!map_view))>"List"</a>
                        <a href=(filters.with_view("map")) aria-current=(current(map_view))>"Map"</a>
                    </nav>
                </div>
                if filters.sport == "padel" {
                    <p class="note">"Padel here is outdoors: two clubs on GotCourts and SIHLSPORTS in Langnau on Eversports. The padel halls in Zurich (Padelta, PDL, Unipadel, Yakin Arena) book on Playtomic, which this app doesn't read yet."</p>
                }
                if open.is_empty() && full.is_empty() {
                    <p class="empty">"No court of these fits. Widen the hours, the distance or the length, or add days."</p>
                }
                if !open.is_empty() && several {
                    <section class="overview" aria-label="Cheapest free start by day">
                        <div class="grid-wrap">
                            <table class="grid">
                                <thead>
                                    <tr>
                                        <th scope="col"><span class="sr">"Club"</span></th>
                                        for d in &filters.dates {
                                            <th scope="col">(time::weekday(d))</th>
                                        }
                                    </tr>
                                </thead>
                                <tbody>
                                    for (row, picks) in open.iter().zip(&by_day) {
                                        <tr>
                                            <th scope="row"><a href=(format!("#club-{}", row.club.id))>(row.club.name.as_str())</a></th>
                                            for found in picks {
                                                grid_cell(found: found)
                                            }
                                        </tr>
                                    }
                                </tbody>
                            </table>
                        </div>
                        price_legend(what: format!("Cheapest free start each day, CHF for {length_label}."))
                    </section>
                }
                if !open.is_empty() && !several && hours.len() > 1 {
                    <section class="overview" aria-label="Cheapest free court by hour">
                        <div class="grid-wrap">
                            <table class="grid">
                                <thead>
                                    <tr>
                                        <th scope="col"><span class="sr">"Club"</span></th>
                                        for hour in &hours {
                                            <th scope="col">(format!("{hour}"))</th>
                                        }
                                    </tr>
                                </thead>
                                <tbody>
                                    for (row, picks) in open.iter().zip(&by_hours) {
                                        <tr>
                                            <th scope="row"><a href=(format!("#club-{}", row.club.id))>(row.club.name.as_str())</a></th>
                                            for found in picks {
                                                grid_cell(found: found)
                                            }
                                        </tr>
                                    }
                                </tbody>
                            </table>
                        </div>
                        price_legend(what: format!("Cheapest court free for {length_label} from each hour, in CHF."))
                    </section>
                }
                if !open.is_empty() {
                    <div class="ask-bar" data-ask-bar="1">
                        <p class="quiet hint" data-ask-hint="1">"Pick a time to send that court to someone. The number beside it is what booking it costs, in CHF."</p>
                        <button type="button" class="pill quiet" data-ask-start="1" hidden=(true)>"Ask which one"</button>
                    </div>
                }
                <div class="cards">
                    for row in &open {
                        club_card(row: row, sport: &filters.sport, several: several, share_key: share_key.as_str(), iphone: iphone)
                    }
                </div>
                if !full.is_empty() {
                    <section class="full">
                        <h2>(format!("Nothing free for {sport_label} here"))</h2>
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
                coverage(discovered: &discovered, venues: &venues)
            </section>
            <section class="map-pane" aria-label="Map">
                <div id="map" class="map" data-pins=(pins.as_str()) data-origin=(format!("{},{}", origin.0, origin.1)) data-here=(if filters.near.is_some() { "1" } else { "" })></div>
            </section>
        </div>
    })
}

/// Where the clubs come from, and what GotCourts' map couldn't say.
#[component]
async fn coverage(discovered: &crate::discover::Discovered, venues: &[String]) -> Result<impl View> {
    let holes = discovered.holes.len();
    let eversports = match venues {
        [] => String::new(),
        [one] => format!("{one} books on Eversports. "),
        [rest @ .., last] => format!("{} and {last} book on Eversports. ", rest.join(", ")),
    };
    Ok(view! {
        <p class="coverage quiet">
            if discovered.at > 0 {
                (format!(
                    "{} clubs from GotCourts' map, checked {}: of {} clubs it lists around Zurich, {} take bookings on GotCourts. ",
                    discovered.clubs.len(),
                    time::ago(discovered.at),
                    discovered.listed,
                    discovered.booking
                ))
                if holes > 0 {
                    (format!(
                        "The map can't answer for {} small {} around Hottingen (GotCourts fails there), so a club right there could be missing.",
                        holes,
                        if holes == 1 { "spot" } else { "spots" }
                    ))
                }
            } else {
                "Looking for clubs on GotCourts' map; this takes a few minutes after a start. "
            }
            (eversports.as_str())
        </p>
    })
}

fn count(facets: &Facets, group: &str, value: &str) -> String {
    facets.of(group, value).map(|n| n.to_string()).unwrap_or_default()
}

/// "+12 without this filter", when dropping the group's limit shows more.
fn more(facets: &Facets, group: &str, open: &str) -> String {
    match facets.more(group, open) {
        0 => String::new(),
        n => format!("+{n} without this filter"),
    }
}

#[component]
async fn filter_form(
    filters: &Filters,
    sports: &[String],
    surfaces: &BTreeSet<String>,
    members_only: usize,
    facets: &Facets,
) -> Result<impl View> {
    let summary = filters.summary(&sport_name(&filters.sport));
    let from_hours: Vec<i64> = (EARLIEST..LATEST).collect();
    let until_hours: Vec<i64> = (EARLIEST + 1..=LATEST).collect();
    let any_time = format!("{EARLIEST}-{LATEST}");
    let when_more = more(facets, "when", &any_time);
    let length_more = more(facets, "length", "60");
    let place_more = if filters.place.is_empty() {
        String::new()
    } else {
        more(facets, "place", "")
    };
    let surface_more = if filters.surface.is_empty() {
        String::new()
    } else {
        more(facets, "surface", "")
    };
    let within_more = more(facets, "within", "0");
    let price_more = more(facets, "price", "0");
    let unpriced_note = match facets.unpriced {
        0 => "Francs, as GotCourts and Eversports quote them without signing in. A court without a known price shows only under Any.".to_string(),
        1 => "Francs, as GotCourts and Eversports quote them without signing in. One free start has no known price: it shows only under Any.".to_string(),
        n => format!("Francs, as GotCourts and Eversports quote them without signing in. {n} free starts have no known price: they show only under Any."),
    };
    let members_more = if filters.members {
        String::new()
    } else {
        more(facets, "members", "1")
    };
    let cleared = facets.cleared.saturating_sub(facets.current);
    let clear_link = Filters {
        dates: filters.dates.clone(),
        several: filters.several,
        sport: filters.sport.clone(),
        view: filters.view.clone(),
        near: filters.near,
        sort: filters.sort.clone(),
        members: filters.members,
        ..Filters::default()
    }
    .link();
    Ok(view! {
        <details class="filters" data-filters="">
            <summary>
                <span class="filters-label">"Filters"</span>
                <span class="filters-summary">(summary.as_str())</span>
                if cleared > 0 {
                    <span class="filters-more">(format!("+{cleared} without them"))</span>
                }
            </summary>
            <form method="get" action="/" data-autosubmit="">
                <input type="hidden" name="f" value="1">
                <input type="hidden" name="date" value=(filters.dates.join(","))>
                <input type="hidden" name="several" value=(if filters.several { "1" } else { "" })>
                <input type="hidden" name="view" value=(filters.view.as_str())>
                <input type="hidden" name="near" value=(filters.near.map(|(a, b)| format!("{a:.4},{b:.4}")).unwrap_or_default())>
                if sports.len() > 1 {
                    <fieldset class="group">
                        <legend>"Sport"</legend>
                        <div class="seg">
                            for sport in sports {
                                <label><input type="radio" name="sport" value=(sport.as_str()) checked=(*sport == filters.sport)> (sport_name(sport)) <span class="count">(count(facets, "sport", sport))</span></label>
                            }
                        </div>
                    </fieldset>
                }
                <fieldset class="group">
                    <legend>"When" <span class="more">(when_more.as_str())</span></legend>
                    <div class="row">
                        for (label, from, until) in PRESETS {
                            <button type="submit" name="when" value=(format!("{from}-{until}")) class=(if filters.from == from && filters.until == until { "chip on" } else { "chip" })>(label) <span class="count">(count(facets, "when", &format!("{from}-{until}")))</span></button>
                        }
                        <button type="submit" name="when" value=(any_time.as_str()) class=(if filters.from == EARLIEST && filters.until == LATEST { "chip on" } else { "chip" })>"Any time" <span class="count">(count(facets, "when", &any_time))</span></button>
                    </div>
                    <div class="row hours">
                        <label>"From"
                            <select name="from">
                                for hour in &from_hours {
                                    <option value=(hour.to_string()) selected=(*hour == filters.from)>(format!("{hour}:00"))</option>
                                }
                            </select>
                        </label>
                        <label>"Until"
                            <select name="until">
                                for hour in &until_hours {
                                    <option value=(hour.to_string()) selected=(*hour == filters.until)>(format!("{hour}:00"))</option>
                                }
                            </select>
                        </label>
                    </div>
                </fieldset>
                <fieldset class="group">
                    <legend>"How long" <span class="more">(length_more.as_str())</span></legend>
                    <div class="seg">
                        for (minutes, label) in LENGTHS {
                            <label><input type="radio" name="length" value=(minutes.to_string()) checked=(minutes == filters.length)> (label) <span class="count">(count(facets, "length", &minutes.to_string()))</span></label>
                        }
                    </div>
                </fieldset>
                <fieldset class="group">
                    <legend>"Price an hour" <span class="more">(price_more.as_str())</span></legend>
                    <div class="seg">
                        <label><input type="radio" name="price" value="0" checked=(filters.price == 0)> "Any" <span class="count">(count(facets, "price", "0"))</span></label>
                        for chf in PRICES {
                            <label><input type="radio" name="price" value=(chf.to_string()) checked=(chf == filters.price)> (format!("≤ {chf}")) <span class="count">(count(facets, "price", &chf.to_string()))</span></label>
                        }
                    </div>
                    <p class="quiet hint">(unpriced_note.as_str())</p>
                </fieldset>
                <fieldset class="group">
                    <legend>"Court" <span class="more">(if place_more.is_empty() { surface_more.as_str() } else { place_more.as_str() })</span></legend>
                    <div class="seg">
                        <label><input type="radio" name="place" value="" checked=(filters.place.is_empty())> "Any" <span class="count">(count(facets, "place", ""))</span></label>
                        <label><input type="radio" name="place" value="indoor" checked=(filters.place == "indoor")> "Indoor" <span class="count">(count(facets, "place", "indoor"))</span></label>
                        <label><input type="radio" name="place" value="outdoor" checked=(filters.place == "outdoor")> "Outdoor" <span class="count">(count(facets, "place", "outdoor"))</span></label>
                        if filters.place == "unknown" || facets.of("place", "unknown").unwrap_or(0) > 0 {
                            <label><input type="radio" name="place" value="unknown" checked=(filters.place == "unknown")> "Not stated" <span class="count">(count(facets, "place", "unknown"))</span></label>
                        }
                    </div>
                    if surfaces.len() > 1 {
                        <label class="inline">"Surface"
                            <select name="surface">
                                <option value="">(format!("Any surface ({})", count(facets, "surface", "")))</option>
                                for name in surfaces {
                                    <option value=(name.as_str()) selected=(*name == filters.surface)>(format!("{name} ({})", count(facets, "surface", name)))</option>
                                }
                            </select>
                        </label>
                    }
                </fieldset>
                <fieldset class="group">
                    <legend>"Where" <span class="more">(within_more.as_str())</span></legend>
                    <div class="seg">
                        <label><input type="radio" name="within" value="0" checked=(filters.within == 0)> "Anywhere" <span class="count">(count(facets, "within", "0"))</span></label>
                        for km in WITHIN {
                            <label><input type="radio" name="within" value=(km.to_string()) checked=(km == filters.within)> (format!("{km} km")) <span class="count">(count(facets, "within", &km.to_string()))</span></label>
                        }
                    </div>
                    <div class="row">
                        <span class="quiet">(if filters.near.is_some() { "From where you are" } else { "From home" })</span>
                        <button type="button" class="chip" data-locate="">(if filters.near.is_some() { "Update my location" } else { "Use my location" })</button>
                        if filters.near.is_some() {
                            <button type="button" class="chip" data-home="">"From home"</button>
                        }
                    </div>
                </fieldset>
                <fieldset class="group">
                    <legend>"Order and access" <span class="more">(members_more.as_str())</span></legend>
                    <div class="seg">
                        <label><input type="radio" name="sort" value="soon" checked=(filters.sort != "near")> "Soonest"</label>
                        <label><input type="radio" name="sort" value="near" checked=(filters.sort == "near")> "Nearest"</label>
                    </div>
                    <label class="check">
                        <input type="checkbox" name="members" value="1" checked=(filters.members)>
                        (format!("Members-only clubs too ({members_only} {}, {} free)", if members_only == 1 { "club" } else { "clubs" }, count(facets, "members", "1")))
                    </label>
                </fieldset>
                <div class="filters-foot">
                    <span class="quiet">(format!("{} free starts with these filters", facets.current))</span>
                    if cleared > 0 {
                        <a class="chip" href=(clear_link.as_str())>(format!("Clear filters: +{cleared}"))</a>
                    }
                </div>
                <noscript><button class="pill" type="submit">"Show"</button></noscript>
            </form>
        </details>
    })
}

#[component]
pub async fn nav(cx: &Cx, current: &'static str) -> Result<impl View> {
    let admin = crate::auth::admin(cx);
    Ok(view! {
        <header class="top">
            <a class="brand" href="/">"Courts"</a>
            <nav class="tabs" aria-label="Sections">
                <a href="/" aria-current=(if current == "courts" { "page" } else { "false" })>"Free courts"</a>
                <a href="/watches" aria-current=(if current == "watches" { "page" } else { "false" })>"Watches"</a>
                if admin {
                    <a href="/users" aria-current=(if current == "users" { "page" } else { "false" })>"Users"</a>
                    <a href="/feedback" aria-current=(if current == "feedback" { "page" } else { "false" })>"Feedback"</a>
                }
            </nav>
        </header>
    })
}

/// One cell of an overview grid: the cheapest free slot in it, and its
/// share page.
struct Pick {
    class: &'static str,
    href: String,
    label: String,
    title: String,
}

fn pick(club: &db::Club, day: &DayFree, at: usize, key: &str) -> Pick {
    let slot = &day.free[at];
    let court = day.courts.iter().find(|c| c.id == slot.court);
    let (href, about) = court.map_or_else(Default::default, |court| {
        (
            shared_path(key, club.id, court, &day.date, slot),
            format!("{}, {}", court.label, slot_title(court, slot, day.costs[at])),
        )
    });
    let (class, label, about) = match day.costs[at] {
        Some((total, hourly)) => (price_heat(Some(hourly)), ((total + 50) / 100).to_string(), about),
        None => (price_heat(None), "–".to_string(), format!("{about} · price not known")),
    };
    Pick {
        class,
        href,
        label,
        title: format!("{} {about}", time::weekday(&day.date)),
    }
}

#[component]
async fn grid_cell(found: &Option<Pick>) -> Result<impl View> {
    Ok(view! {
        if let Some(found) = found {
            <td class=(found.class)><a href=(found.href.as_str()) title=(found.title.as_str()) aria-label=(found.title.as_str())>(found.label.as_str())</a></td>
        } else {
            <td class="cell none"></td>
        }
    })
}

/// What a grid's numbers and colours mean.
#[component]
async fn price_legend(what: String) -> Result<impl View> {
    Ok(view! {
        <p class="legend quiet">
            (what.as_str())
            <span class="keys">
                <span class="key"><span class="swatch cell cheap"></span>"up to 30 an hour"</span>
                <span class="key"><span class="swatch cell fair"></span>"up to 50"</span>
                <span class="key"><span class="swatch cell dear"></span>"more"</span>
                <span class="key"><span class="swatch cell unpriced">"–"</span>"price not known"</span>
            </span>
        </p>
    })
}

/// A day of a club card: the day and each court's free starts.
type CardDay<'a> = (&'a DayFree, Vec<(&'a Court, Vec<&'a Slot>)>);

fn shared_path(key: &str, club: i64, court: &Court, date: &str, slot: &Slot) -> String {
    crate::share::path(key, &shared(club, court, date, slot))
}

fn shared_title(court: &Court, date: &str, slot: &Slot, club: &str) -> String {
    crate::share::title(&court.sport, &shared(0, court, date, slot), club)
}

/// The slot to share: as far as booking it reaches, to the end of a cell.
fn shared(club: i64, court: &Court, date: &str, slot: &Slot) -> crate::share::Shared {
    crate::share::Shared {
        club,
        court: slot.court,
        date: date.to_string(),
        start: slot.start,
        end: slots::booked_end(court, slot),
    }
}

#[component]
async fn club_card(row: &Row, sport: &str, several: bool, share_key: &str, iphone: bool) -> Result<impl View> {
    let club = &row.club;
    let courts = row.courts();
    let first = row.soonest().map_or(row.days[0].date.clone(), |(d, _)| d.to_string());
    let source = Source::parse(&club.source);
    let book_at = book(club, &first, sport, None, iphone);
    let book_title = match source {
        Source::GotCourts if iphone => "Opens the GotCourts app; pick the day there".to_string(),
        Source::GotCourts => "Opens GotCourts; pick the day there".to_string(),
        Source::Eversports => format!("Opens Eversports on {}", time::label(&first)),
    };
    let label = book_label(club, &first, iphone);
    let days: Vec<CardDay> = row
        .days
        .iter()
        .filter(|day| !day.free.is_empty())
        .map(|day| {
            let free = day
                .courts
                .iter()
                .map(|court| {
                    (
                        court,
                        day.free.iter().filter(|s| s.court == court.id).collect::<Vec<_>>(),
                    )
                })
                .filter(|(_, free)| !free.is_empty())
                .collect();
            (day, free)
        })
        .collect();
    Ok(view! {
        <section class="club" id=(format!("club-{}", club.id)) data-club=(club.id.to_string())>
            <header>
                <div>
                    <h2 lang="de"><a href=(format!("/clubs/{}", club.id))>(club.name.as_str())</a></h2>
                    <p class="meta">
                        (format!("{:.1} km · {} · {}", row.km, surfaces(&courts), club.city))
                        if !kind(club).is_empty() {
                            (format!(" · {}", kind(club)))
                        }
                    </p>
                </div>
                <div class="club-actions">
                    <button type="button" class="icon-button" data-show-on-map=(club.id.to_string()) aria-label=(format!("Show {} on the map", club.name))>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 21s-6.5-5.6-6.5-11a6.5 6.5 0 0 1 13 0c0 5.4-6.5 11-6.5 11Z"></path><circle cx="12" cy="10" r="2.3"></circle></svg>
                    </button>
                    <a class="pill" href=(book_at.as_str()) target="_blank" rel="noopener" title=(book_title.as_str())>(label.as_str())</a>
                </div>
            </header>
            for (day, free) in &days {
                if several {
                    <h3 class="day-head">(time::label(&day.date))</h3>
                }
                <ul class="courts">
                    for (court, slots) in free {
                        <li>
                            <span class="court">
                                (court.label.as_str())
                                <small>(format!("{}, {}", surface(&court.surface), gotcourts::cover_label(court)))</small>
                            </span>
                            <span class="times">
                                for slot in slots {
                                    <a class="time" href=(shared_path(share_key, club.id, court, &day.date, slot)) data-slot=(shared_title(court, &day.date, slot, &club.name)) title=(format!("Share {}", slot_title(court, slot, day.cost(slot))))>
                                        (clock(slot.start))
                                        if let Some((total, _)) = day.cost(slot) {
                                            <span class="price">(chf(total))</span>
                                        }
                                    </a>
                                }
                            </span>
                        </li>
                    }
                </ul>
            }
        </section>
    })
}

/// "18:15–19:15" and, when known, what that costs; where the court is
/// booked in cells, until the end of the last cell it takes.
fn slot_title(court: &Court, slot: &Slot, cost: Option<(i64, i64)>) -> String {
    let span = format!("{}–{}", clock(slot.start), clock(slots::booked_end(court, slot)));
    match cost {
        Some((total, _)) => format!("{span} · {}", crate::share::francs(total)),
        None => span,
    }
}

#[page("/clubs/{id}")]
async fn club_page(cx: &Cx) -> Result<impl View> {
    let site = app_context::<Site>(cx).clone();
    let db = app_context::<Db>(cx).clone();
    let id: i64 = path_param::<Id>(cx).parse().map_err(|_| not_found())?;
    let club = db.club(id).await.map_err(topcoat::Error::msg)?.ok_or_else(not_found)?;
    let today = time::today();
    let iphone = crate::source::on_iphone(cx);
    let book_there = if in_app(&club, iphone) {
        "Book in the app"
    } else {
        "Book there"
    };
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
                <dt>(Source::parse(&club.source).name())</dt>
                <dd><a href=(book(&club, &today, courts.first().map_or("tennis", |c| c.sport.as_str()), None, iphone)) target="_blank" rel="noopener">(book_there)</a></dd>
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
                            <td>(format!("{}, {}", surface(&court.surface), gotcourts::cover_label(court)))</td>
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

#[topcoat::router::query_params]
struct WatchesQuery {
    invite: Option<String>,
}

#[page("/watches")]
async fn watches_page(cx: &Cx) -> Result<impl View> {
    let invite = topcoat::router::query_params::<WatchesQuery>(cx)
        .map_err(|error| topcoat::Error::msg(error.to_string()))?
        .invite
        .clone()
        .filter(|token| token.len() == 32 && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    let origin = app_context::<crate::auth::Origin>(cx)
        .0
        .trim_end_matches('/')
        .to_string();
    let me = crate::auth::person(cx)?;
    let site = app_context::<Site>(cx).clone();
    let db = app_context::<Db>(cx).clone();
    let clubs = db
        .clubs(&app_context::<crate::discover::Clubs>(cx).ids())
        .await
        .map_err(topcoat::Error::msg)?;
    let mut asked = Vec::new();
    for ask in db.asks_of(me.id).await.map_err(topcoat::Error::msg)? {
        asked.push(asked_line(&db, &ask).await?);
    }
    let mut listed = Vec::new();
    for watch in db.watches_of(me.id).await.map_err(topcoat::Error::msg)? {
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
    let all_sports = known_sports(&db, &app_context::<crate::discover::Clubs>(cx).ids()).await;
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
            if !asked.is_empty() {
                <section class="asked" id="asked">
                    <h2>"Asked which one"</h2>
                    <ul class="watch-list">
                        for (url, what, picks) in &asked {
                            <li>
                                <span>
                                    <a href=(url.as_str())><strong>(what.as_str())</strong></a>
                                    <br>
                                    <span class="quiet">(picks.as_str())</span>
                                </span>
                            </li>
                        }
                    </ul>
                </section>
            }
            <section class="devices" id="devices">
                <h2>"Other devices"</h2>
                <p class="quiet">(format!("Signed in as {}.", me.name))</p>
                if let Some(token) = &invite {
                    <div class="invite">
                        <img src=(format!("/invites/{token}/qr.svg")) width="240" height="240" alt="QR code of the sign-in link">
                        <div class="invite-text">
                            <p>(format!("Scan this with the phone's camera, or send it the link. It works once, within a week, and keeps that device signed in as {}.", me.name))</p>
                            <p class="invite-link"><code>(format!("{origin}/invite/{token}"))</code></p>
                            <div class="row">
                                <button type="button" class="pill" data-share=(format!("{origin}/invite/{token}"))>"Share link"</button>
                                <button type="button" class="pill quiet" data-copy-link=(format!("{origin}/invite/{token}"))>"Copy"</button>
                            </div>
                        </div>
                    </div>
                } else {
                    <p class="quiet">"Sign in on a phone or another computer: make a link, then scan its QR code or send it over."</p>
                    <div class="row">
                        <form method="post" action="/invites">
                            <button class="pill quiet" type="submit">"Make a sign-in link"</button>
                        </form>
                        <form method="post" action="/sign-out">
                            <button class="pill quiet" type="submit">"Sign out here"</button>
                        </form>
                    </div>
                }
            </section>
        </article>
    })
}

/// An ask on the Watches page: its link, its options in a line, and who picked.
async fn asked_line(db: &Db, ask: &db::Ask) -> Result<(String, String, String)> {
    let mut names = Vec::new();
    for shared in &ask.options.0 {
        let club = db.club(shared.club).await.map_err(topcoat::Error::msg)?;
        names.push(crate::ask::short(
            shared,
            club.as_ref().map_or("a club", |club| club.name.as_str()),
        ));
    }
    let what = match names.as_slice() {
        [one] => one.clone(),
        [first, ..] => format!("{first} and {} more", names.len() - 1),
        [] => "Nothing".into(),
    };
    let picks = if ask.picks.0.is_empty() {
        format!("Sent {}, nobody picked yet", time::ago(ask.created_at))
    } else {
        ask.picks
            .0
            .iter()
            .map(|pick| {
                let chose = names.get(pick.option).map_or("?", String::as_str);
                format!("{} picked {chose}", pick.name)
            })
            .collect::<Vec<_>>()
            .join(" · ")
    };
    Ok((format!("/a/{}", ask.token), what, picks))
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
    let me = crate::auth::person(cx)?;
    let db = app_context::<Db>(cx).clone();
    let site = app_context::<Site>(cx);
    if time::dates(site.days).contains(&new.date) && new.earliest < new.latest {
        let club = new.club.as_deref().and_then(|c| c.parse().ok());
        let watch = db
            .add_watch(
                me.id,
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
    let me = crate::auth::person(cx)?;
    let db = app_context::<Db>(cx).clone();
    if let Ok(id) = path_param::<Id>(cx).parse::<i64>() {
        let found = db.watch(id).await.map_err(topcoat::Error::msg)?;
        if found.is_some_and(|watch| watch.person_id == Some(me.id)) {
            db.forget_watch(id).await.map_err(topcoat::Error::msg)?;
        }
    }
    Ok(see_other("/watches"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gotcourts::Busy;
    use toasty::Json;

    fn court(id: i64, indoor: bool) -> Court {
        Court {
            id,
            sport: "tennis".into(),
            label: format!("Platz {id}"),
            surface: "clay".into(),
            indoor: Some(indoor),
            cover: if indoor { "indoor" } else { "outdoor" }.into(),
            ball_machine: false,
            order: id,
            interval: 60,
            durations: vec![60],
            opening: 8 * 3600,
            closing: 12 * 3600,
            cells: Vec::new(),
        }
    }

    const DATE: &str = "2099-01-01";

    /// Lengg on DATE: indoor court 1 free 08–12, outdoor court 2 taken 09–10.
    fn lengg() -> (Vec<db::Club>, BTreeMap<(i64, String), db::Day>) {
        let club = db::Club {
            id: 1,
            source: "gotcourts".into(),
            name: "Lengg".into(),
            canonical: "lengg".into(),
            address: String::new(),
            zip: String::new(),
            city: "Zürich".into(),
            latitude: 47.3779,
            longitude: 8.5403,
            phone: String::new(),
            web: String::new(),
            kind: "public".into(),
            courts: Json(vec![court(1, true), court(2, false)]),
            checked_at: 0,
        };
        let date = DATE.to_string();
        let day = db::Day {
            club_id: 1,
            date: date.clone(),
            fetched_at: 0,
            courts: Json(vec![court(1, true), court(2, false)]),
            busy: Json(vec![Busy {
                court: Some(2),
                start: 9 * 3600,
                end: 10 * 3600,
                note: None,
            }]),
        };
        (vec![club], BTreeMap::from([((1, date), day)]))
    }

    #[test]
    fn each_choice_counts_what_it_would_show() {
        let (clubs, days) = lengg();
        let date = DATE.to_string();
        let rates = Rates::default();
        let found = Found {
            clubs: &clubs,
            days: &days,
            home: (47.3779, 8.5403),
            today: "2000-01-01",
            rates: &rates,
        };
        let filters = Filters {
            dates: vec![date],
            place: "indoor".into(),
            ..Filters::default()
        };
        let facets = found.facets(&filters, &["tennis".into()], &BTreeSet::from(["clay".to_string()]));
        // Indoor: 08–12 on court 1 is four starts; outdoor court 2 has three.
        assert_eq!(facets.current, 4);
        assert_eq!(facets.of("place", "outdoor"), Some(3));
        assert_eq!(facets.of("place", ""), Some(7));
        assert_eq!(facets.more("place", ""), 3);
        // Two hours in a row: indoor 08, 09, 10.
        assert_eq!(facets.of("length", "120"), Some(3));
        assert_eq!(facets.cleared, 7);
    }

    #[test]
    fn a_price_limit_keeps_what_costs_less_and_the_cheapest_leads_a_cell() {
        let (clubs, days) = lengg();
        let mut indoor = crate::tariffs::Week(vec![vec![Some(4000); 24]; 7]);
        indoor.0.iter_mut().for_each(|day| day[11] = Some(6500));
        let mut outdoor = crate::tariffs::Week(vec![vec![None; 24]; 7]);
        outdoor.0.iter_mut().for_each(|day| day[10] = Some(3000));
        let rates = Rates::new(std::collections::HashMap::from([(1, indoor), (2, outdoor)]));
        let found = Found {
            clubs: &clubs,
            days: &days,
            home: (47.3779, 8.5403),
            today: "2000-01-01",
            rates: &rates,
        };
        let filters = Filters {
            dates: vec![DATE.to_string()],
            ..Filters::default()
        };
        let facets = found.facets(&filters, &["tennis".into()], &BTreeSet::new());
        // Court 1 costs 40 an hour but 65 from 11; court 2 has a price only at 10.
        assert_eq!(facets.current, 7);
        assert_eq!(facets.of("price", "30"), Some(1));
        assert_eq!(facets.of("price", "40"), Some(4));
        assert_eq!(facets.of("price", "60"), Some(4));
        assert_eq!(facets.unpriced, 2);
        let rows = found.rows(&filters);
        let day = &rows[0].days[0];
        let best = &day.free[day.cheapest(|_| true).unwrap()];
        assert_eq!((best.court, best.start), (2, 10 * 3600));
        let at_eight = &day.free[day.cheapest(|s| s.start / 3600 == 8).unwrap()];
        assert_eq!((at_eight.court, at_eight.start), (1, 8 * 3600));
        let cell = pick(
            &rows[0].club,
            day,
            day.cheapest(|s| s.start / 3600 == 11).unwrap(),
            "a key of enough length",
        );
        assert_eq!((cell.class, cell.label.as_str()), ("cell dear", "65"));
        assert!(cell.href.starts_with("/s/1-1-20990101-1100-60-"));
        let unknown = day.cheapest(|s| s.court == 2 && s.start / 3600 == 8).unwrap();
        let cell = pick(&rows[0].club, day, unknown, "a key of enough length");
        assert_eq!((cell.class, cell.label.as_str()), ("cell unpriced", "–"));
        assert!(cell.title.ends_with("price not known"));
        let limited = found.rows(&Filters {
            price: 40,
            ..filters.clone()
        });
        assert!(
            limited[0].days[0]
                .costs
                .iter()
                .all(|cost| cost.is_some_and(|(_, hourly)| hourly <= 4000))
        );
    }
}
