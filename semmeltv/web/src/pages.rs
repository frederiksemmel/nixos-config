use crate::api::{About, Api, Card, CastMember, Episode, Found, Kind, MoreScores, Next, Played, Ratings, Watch, WatchingCard};
use crate::layout::back;
use crate::playing::{play_episode, play_movie, play_show, tell};
use crate::public::Public;
use std::hash::{DefaultHasher, Hash, Hasher};
use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::router::error::not_found;
use topcoat::router::{page, path_param};
use topcoat::runtime::{Event, Signal, link, procedure, shard, signal};
use topcoat::view::{View, attributes, component, view};

const LONGEST_QUERY: usize = 100;
const OLDEST_DECADE: u16 = 1990;
const RATINGS: [(&str, &str); 5] = [
    ("", "Any"),
    ("best", "Best first"),
    ("6", "6+"),
    ("7", "7+"),
    ("8", "8+"),
];
const YEARS: [(&str, &str); 8] = [
    ("", "Any"),
    ("newest", "Newest"),
    ("oldest", "Oldest"),
    ("2020", "2020s"),
    ("2010", "2010s"),
    ("2000", "2000s"),
    ("1990", "1990s"),
    ("older", "Before 1990"),
];
const BANDS: [(&str, &str); 6] = [
    ("awesome", "9+"),
    ("great", "8+"),
    ("good", "7+"),
    ("regular", "6+"),
    ("bad", "5+"),
    ("garbage", "<5"),
];

path_param!(key);

fn hue(title: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    title.hash(&mut hasher);
    hasher.finish() % 360
}

fn tint(title: &str) -> String {
    format!("--hue:{}", hue(title))
}

fn failed(error: String) -> topcoat::Error {
    topcoat::Error::msg(error)
}

#[component]
async fn scores(of: &Ratings) -> Result<impl View> {
    let imdb = of.imdb.map(|score| format!("{score:.1}"));
    let tomatoes = of.rotten_tomatoes.map(|score| format!("{score}%"));
    let popcorn = of.popcorn.map(|score| format!("{score}%"));
    let fresh = of.fresh.unwrap_or(false);
    let full = of.full.unwrap_or(false);
    Ok(view! {
        if imdb.is_some() || tomatoes.is_some() || popcorn.is_some() {
            <span class="ratings">
                if let Some(tomatoes) = tomatoes {
                    <span class=(if fresh { "rating rt fresh" } else { "rating rt rotten" }) title="Tomatometer">
                        <i aria-hidden="true"></i>
                        (tomatoes)
                    </span>
                }
                if let Some(popcorn) = popcorn {
                    <span class=(if full { "rating popcorn full" } else { "rating popcorn spilled" }) title="Popcornmeter">
                        <i aria-hidden="true"></i>
                        (popcorn)
                    </span>
                }
                if let Some(imdb) = imdb {
                    <span class="rating imdb"><b>"IMDb"</b> (imdb)</span>
                }
            </span>
        }
    })
}

#[component]
async fn tile(cx: &Cx, public: &Public, card: &Card) -> Result<impl View> {
    let image = card
        .poster
        .as_ref()
        .or(card.backdrop.as_ref())
        .map(|path| public.url(path));
    let attrs = attributes! { cx => class="tile" };
    Ok(view! {
        link(href: card.href(), attrs: attrs,
            <span class="art poster" style=(tint(&card.title))>
                <span class="art-title">(card.title.as_str())</span>
                if let Some(image) = image {
                    <img src=(image) alt="" loading="lazy" decoding="async" onerror="this.remove()">
                }
            </span>
            <span class="tile-name">(card.title.as_str())</span>
            <span class="tile-sub">
                (card.subtitle.as_str())
                scores(of: &card.ratings)
            </span>
        )
    })
}

#[procedure]
async fn forget(cx: &Cx, kind: String, key: String) -> Result<(bool, String)> {
    let kind = match kind.as_str() {
        "movie" => Kind::Movie,
        "show" => Kind::Show,
        _ => return Ok((false, format!("No such kind: {kind}"))),
    };
    Ok(app_context::<Api>(cx).dismiss_forgotten(kind, &key).await)
}

#[procedure]
async fn mark(cx: &Cx, kind: String, key: String, watched: bool, season: Option<u32>, episode: Option<u32>) -> Result<(bool, String)> {
    let mut body = serde_json::json!({ "kind": kind, "key": key, "watched": watched });
    if let (Some(season), Some(episode)) = (season, episode) {
        body["season"] = season.into();
        body["episode"] = episode.into();
    }
    Ok(match app_context::<Api>(cx).mark_watched(body).await {
        Ok(now) => (now, String::new()),
        Err(error) => (!watched, error),
    })
}

#[component]
async fn check_icon() -> Result<impl View> {
    Ok(view! {
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 12.5 4 4 8-9"></path></svg>
    })
}

#[component]
async fn watching_tile(cx: &Cx, public: &Public, watching: &WatchingCard, forgettable: bool) -> Result<impl View> {
    let card = &watching.card;
    let image = card.backdrop.as_deref().map(|path| public.url(path));
    let attrs = attributes! { cx => class="tile-link" };
    let key = card.key.clone();
    let is_movie = card.kind == Kind::Movie;
    let kind = if is_movie { "movie" } else { "show" };
    let said = signal(cx, String::new);
    let gone = signal(cx, || false);
    Ok(view! {
        <div :class=$(if gone.get() { "tile watching gone" } else { "tile watching" })>
            link(href: card.href(), attrs: attrs,
                <span class="art" style=(tint(&card.title))>
                    <span class="art-title">(card.title.as_str())</span>
                    if let Some(image) = image {
                        <img src=(image) alt="" loading="lazy" decoding="async" onerror="this.remove()">
                    }
                    <span class="progress"><i style=(format!("width:{:.1}%", watching.progress * 100.0))></i></span>
                </span>
                <span class="tile-name">(card.title.as_str())</span>
                <span class="tile-sub">(watching.detail.as_str())</span>
            )
            <button type="button" class="resume" aria-label=(format!("Play {}", card.title))
                @click=$(async |_e| {
                    let answer = if is_movie { play_movie(key).await } else { play_show(key).await };
                    said.set(answer);
                })>
                play_icon()
            </button>
            if forgettable {
                <button type="button" class="forget" aria-label=(format!("Remove {} from Started and Forgotten", card.title))
                    @click=$(async |_e| {
                        gone.set(true);
                        let answer = forget(kind.to_owned(), key.to_owned()).await;
                        gone.set(answer.0);
                        said.set(answer.1);
                    })>
                    <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 7l10 10M17 7 7 17"></path></svg>
                </button>
            }
            <span class="tile-said" :hidden=$(said.get().is_empty())>$(said.get())</span>
        </div>
    })
}

#[component]
async fn grid(public: &Public, cards: &[Card]) -> Result<impl View> {
    Ok(view! {
        <div class="grid">
            #[key(card.href())]
            for card in cards {
                tile(public: public, card: card)
            }
        </div>
    })
}

fn decade(year: &str) -> Option<std::ops::Range<u16>> {
    match year {
        "older" => Some(0..OLDEST_DECADE),
        _ => {
            let start: u16 = year.parse().ok()?;
            Some(start..start + 10)
        }
    }
}

fn arrange(cards: &[Card], rating: &str, year: &str) -> Vec<Card> {
    let least: Option<f64> = rating.parse().ok();
    let span = decade(year);
    let mut kept: Vec<Card> = cards
        .iter()
        .filter(|card| least.is_none_or(|least| card.ratings.imdb.is_some_and(|imdb| imdb >= least)))
        .filter(|card| {
            span.as_ref()
                .is_none_or(|span| card.year().is_some_and(|year| span.contains(&year)))
        })
        .cloned()
        .collect();
    let by_year = |newest: bool| {
        move |a: &Card, b: &Card| match (a.year(), b.year()) {
            (Some(a), Some(b)) if newest => b.cmp(&a),
            (Some(a), Some(b)) => a.cmp(&b),
            (a, b) => b.is_some().cmp(&a.is_some()),
        }
    };
    let by_rating = |a: &Card, b: &Card| match (a.ratings.imdb, b.ratings.imdb) {
        (Some(a), Some(b)) => b.total_cmp(&a),
        (a, b) => b.is_some().cmp(&a.is_some()),
    };
    if !rating.is_empty() {
        kept.sort_by(by_rating);
    }
    match year {
        "newest" => kept.sort_by(by_year(true)),
        "oldest" => kept.sort_by(by_year(false)),
        _ if rating.is_empty() && span.is_some() => kept.sort_by(by_year(true)),
        _ => {}
    }
    if !year.is_empty() {
        for card in &mut kept {
            if card.kind == Kind::Show
                && let Some(years) = &card.years
            {
                card.subtitle = years.clone();
            }
        }
    }
    kept
}

#[component]
async fn sections(cx: &Cx, section: Signal<String>, rating: Signal<String>, year: Signal<String>) -> Result<impl View> {
    let open = signal(cx, String::new);
    Ok(view! {
        <nav class="chips">
        <button type="button" :class=$(if section.get() == "home" { "chip on" } else { "chip" })
            @click=$(|_e| section.set("home".to_owned()))>"Home"</button>
        <button type="button" :class=$(if section.get() == "shows" { "chip on" } else { "chip" })
            @click=$(|_e| section.set("shows".to_owned()))>"Series"</button>
        <button type="button" :class=$(if section.get() == "movies" { "chip on" } else { "chip" })
            @click=$(|_e| section.set("movies".to_owned()))>"Movies"</button>
        <span class="chip-gap" :hidden=$(section.get() == "home")></span>
        <button type="button" aria-label="Rating" :hidden=$(section.get() == "home")
            :class=$(if rating.get().is_empty() { "chip pick" } else { "chip pick on" })
            :aria-expanded=$(if open.get() == "rating" { "true" } else { "false" })
            @click=$(|_e| if open.get() == "rating" { open.set("".to_owned()) } else { open.set("rating".to_owned()) })>
            <svg class="glyph" viewBox="0 0 24 24"><path d="m12 3.5 2.6 5.4 5.9.8-4.3 4.1 1 5.9-5.2-2.8-5.2 2.8 1-5.9-4.3-4.1 5.9-.8z"></path></svg>
            $(if rating.get() == "best" { "Best" } else if rating.get() == "6" { "6+" } else if rating.get() == "7" { "7+" } else if rating.get() == "8" { "8+" } else { "" })
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m7 10 5 5 5-5"></path></svg>
        </button>
        <button type="button" aria-label="Year" :hidden=$(section.get() == "home")
            :class=$(if year.get().is_empty() { "chip pick" } else { "chip pick on" })
            :aria-expanded=$(if open.get() == "year" { "true" } else { "false" })
            @click=$(|_e| if open.get() == "year" { open.set("".to_owned()) } else { open.set("year".to_owned()) })>
            <svg class="glyph" viewBox="0 0 24 24"><rect x="4" y="5.5" width="16" height="14.5" rx="2.5"></rect><path d="M4 10h16M8.5 3.5v4M15.5 3.5v4"></path></svg>
            $(if year.get() == "newest" { "Newest" } else if year.get() == "oldest" { "Oldest" } else if year.get() == "older" { "<1990" } else if year.get() == "2020" { "2020s" } else if year.get() == "2010" { "2010s" } else if year.get() == "2000" { "2000s" } else if year.get() == "1990" { "1990s" } else { "" })
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m7 10 5 5 5-5"></path></svg>
        </button>
        </nav>
        <div class="filter-options" :hidden=$(if section.get() == "home" { true } else { open.get() != "rating" })>
            for (value, label) in RATINGS {
                <button type="button" :class=$(if rating.get() == value { "filter-option on" } else { "filter-option" })
                    @click=$(|_e| { rating.set(value.to_owned()); open.set("".to_owned()); })>
                    (label)
                </button>
            }
        </div>
        <div class="filter-options" :hidden=$(if section.get() == "home" { true } else { open.get() != "year" })>
            for (value, label) in YEARS {
                <button type="button" :class=$(if year.get() == value { "filter-option on" } else { "filter-option" })
                    @click=$(|_e| { year.set(value.to_owned()); open.set("".to_owned()); })>
                    (label)
                </button>
            }
        </div>
    })
}

#[page("/")]
async fn home(cx: &Cx) -> Result<impl View> {
    let query = signal(cx, String::new);
    let section = signal(cx, || "home".to_owned());
    let rating = signal(cx, String::new);
    let year = signal(cx, String::new);
    let said = signal(cx, String::new);
    Ok(view! {
        <div class="search">
            <svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="11" cy="11" r="6.5"></circle><path d="m16 16 4.5 4.5"></path></svg>
            <input
                type="search"
                placeholder="Search, or tell the TV"
                autocomplete="off"
                autocapitalize="off"
                spellcheck="false"
                enterkeyhint="go"
                aria-label="Search the library"
                :value=$(query.get())
                @input=$(|e: Event| query.set(e.target.value))
            >
        </div>
        <div class="tell" :hidden=$(query.get().trim().is_empty())>
            <button type="button" @click=$(async |_e| {
                let answer = tell(query.get()).await;
                said.set(answer);
            })>
                "Tell the TV: “" $(query.get().trim().to_owned()) "”"
            </button>
        </div>
        <p class="said" :hidden=$(said.get().is_empty())>$(said.get())</p>
        <div class="sections" :hidden=$(!query.get().trim().is_empty())>
            sections(section: section.clone(), rating: rating.clone(), year: year.clone())
        </div>
        browse(query: $(query.get()), section: $(section.get()), rating: $(rating.get()), year: $(year.get()))
    })
}

#[shard]
async fn browse(cx: &Cx, query: String, section: String, rating: String, year: String) -> Result<impl View> {
    let api = app_context::<Api>(cx);
    let public = app_context::<Public>(cx).clone();
    let query: String = query.trim().chars().take(LONGEST_QUERY).collect();
    let results = if query.is_empty() {
        None
    } else {
        Some(api.search(&query).await)
    };
    let library = if results.is_none() {
        Some(api.library().await)
    } else {
        None
    };
    let rating = if RATINGS.iter().any(|(value, _)| *value == rating) {
        rating
    } else {
        String::new()
    };
    let year = if YEARS.iter().any(|(value, _)| *value == year) {
        year
    } else {
        String::new()
    };
    let filtered = section == "shows" || section == "movies";
    let arranged = match &library {
        Some(Ok(library)) if section == "shows" => arrange(&library.shows, &rating, &year),
        Some(Ok(library)) if section == "movies" => arrange(&library.movies, &rating, &year),
        _ => Vec::new(),
    };
    Ok(view! {
        match (results, library) {
            (Some(Ok(results)), _) => {
                if results.is_empty() {
                    <p class="empty">"Nothing in the library matches “" (query.as_str()) "”."</p>
                } else {
                    <section class="shelf">
                        <h2>"Results"</h2>
                        grid(public: &public, cards: &results)
                    </section>
                }
            },
            (Some(Err(error)), _) | (None, Some(Err(error))) => <p class="empty">(error)</p>,
            (None, Some(Ok(library))) => {
                if filtered {
                    section_grid(public: &public, title: if section == "shows" { "Series" } else { "Movies" },
                        cards: &arranged)
                }
                if !filtered && !library.watching.is_empty() {
                    <section class="shelf">
                        <h2>"Currently Watching"</h2>
                        <div class="row">
                            #[key(watching.card.href())]
                            for watching in &library.watching {
                                watching_tile(public: &public, watching: watching, forgettable: false)
                            }
                        </div>
                    </section>
                }
                if !filtered && !library.newest.is_empty() {
                    <section class="shelf">
                        <h2>"Newly Added"</h2>
                        <div class="row posters">
                            #[key(card.href())]
                            for card in &library.newest {
                                tile(public: &public, card: card)
                            }
                        </div>
                    </section>
                }
                if !filtered && !library.forgotten.is_empty() {
                    <section class="shelf">
                        <h2>"Started and Forgotten"</h2>
                        <div class="row">
                            #[key(forgotten.card.href())]
                            for forgotten in &library.forgotten {
                                watching_tile(public: &public, watching: forgotten, forgettable: true)
                            }
                        </div>
                    </section>
                }
            },
            (None, None) => "",
        }
    })
}

#[component]
async fn section_grid(public: &Public, title: &str, cards: &[Card]) -> Result<impl View> {
    Ok(view! {
        <section class="shelf">
            <h2>(title)</h2>
            if cards.is_empty() {
                <p class="empty">"Nothing here matches these filters."</p>
            } else {
                grid(public: public, cards: cards)
            }
        </section>
    })
}

#[component]
async fn hero(public: &Public, card: &Card) -> Result<impl View> {
    let image = card.backdrop.as_deref().map(|path| public.url(path));
    Ok(view! {
        <div class="hero" style=(tint(&card.title))>
            if let Some(image) = image {
                <img src=(image) alt="" onerror="this.remove()">
            }
        </div>
    })
}

#[component]
async fn progress(watch: &Watch) -> Result<impl View> {
    let fraction = watch.fraction();
    Ok(view! {
        if let Some(fraction) = fraction {
            <span class="progress"><i style=(format!("width:{:.1}%", fraction * 100.0))></i></span>
        }
    })
}

#[component]
async fn play_icon() -> Result<impl View> {
    Ok(view! {
        <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M7 4.5v15l12.5-7.5z"></path></svg>
    })
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    let (hours, minutes, seconds) = (total / 3600, total % 3600 / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn runtime(minutes: u32) -> String {
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes}m"),
        (hours, 0) => format!("{hours}h"),
        (hours, minutes) => format!("{hours}h {minutes}m"),
    }
}

fn long_date(date: &str) -> String {
    let mut parts = date.split('-');
    let parsed = (|| {
        let year: u16 = parts.next()?.parse().ok()?;
        let month: usize = parts.next()?.parse().ok()?;
        let day: u8 = parts.next()?.parse().ok()?;
        Some(format!("{day}\u{a0}{}\u{a0}{year}", MONTHS.get(month.checked_sub(1)?)?))
    })();
    parsed.unwrap_or_else(|| date.to_string())
}

fn listed(names: &[String]) -> Option<String> {
    (!names.is_empty()).then(|| names.join(", "))
}

fn stopped_at(at: Option<f64>, of: Option<f64>) -> Option<String> {
    let at = at.filter(|at| *at > 0.0)?;
    Some(match of.filter(|of| *of > 0.0) {
        Some(of) => format!("Stopped at {} of {}", clock(at), clock(of)),
        None => format!("Stopped at {}", clock(at)),
    })
}

fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|word| word.chars().next())
        .filter(|letter| letter.is_alphabetic())
        .take(2)
        .collect()
}

fn facts(parts: &[Option<String>]) -> String {
    parts
        .iter()
        .flatten()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" · ")
}

#[component]
async fn more_scores(of: &MoreScores) -> Result<impl View> {
    let metacritic = of.metacritic.map(|score| score.to_string());
    let letterboxd = of.letterboxd.map(|score| format!("{score:.1}"));
    let ebert = of
        .ebert
        .map(|score| format!("{score:.1}").trim_end_matches(".0").to_string());
    Ok(view! {
        if let Some(metacritic) = metacritic {
            <span class="detail-score" title="Metascore"><b class="detail-metacritic">(metacritic)</b>"Metacritic"</span>
        }
        if let Some(letterboxd) = letterboxd {
            <span class="detail-score" title="Letterboxd average, out of 5"><b>(letterboxd)</b>"Letterboxd"</span>
        }
        if let Some(ebert) = ebert {
            <span class="detail-score" title="Roger Ebert, out of 4 stars"><b>(ebert) "/4"</b>"Ebert"</span>
        }
    })
}

#[component]
async fn heading(public: &Public, card: &Card, about: &About, line: &str) -> Result<impl View> {
    let poster = card.poster.as_deref().map(|path| public.url(path));
    let original = about.original_title.clone();
    Ok(view! {
        <div class="detail-head">
            <span class="art poster detail-poster" style=(tint(&card.title))>
                <span class="art-title">(card.title.as_str())</span>
                if let Some(poster) = poster {
                    <img src=(poster) alt="" onerror="this.remove()">
                }
            </span>
            <div class="detail-heading">
                <h1>(card.title.as_str())</h1>
                if let Some(original) = original {
                    <p class="detail-original">(original)</p>
                }
                if !line.is_empty() {
                    <p class="detail-line">(line)</p>
                }
                <p class="title-sub detail-ratings">
                    scores(of: &card.ratings)
                    more_scores(of: &about.scores)
                </p>
            </div>
        </div>
        if let Some(tagline) = &about.tagline {
            <p class="detail-tagline">(tagline.as_str())</p>
        }
    })
}

#[component]
async fn trailer_button(about: &About) -> Result<impl View> {
    Ok(view! {
        if let Some(trailer) = &about.trailer {
            <a class="detail-trailer" href=(trailer.as_str()) target="_blank" rel="noopener">
                <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="5.5" width="18" height="13" rx="3.5"></rect><path class="solid" d="m10.2 9.3 4.6 2.7-4.6 2.7z"></path></svg>
                "Trailer"
            </a>
        }
    })
}

#[component]
async fn facts_list(rows: &[(&'static str, String)]) -> Result<impl View> {
    Ok(view! {
        if !rows.is_empty() {
            <dl class="detail-facts">
                for (label, value) in rows {
                    <div>
                        <dt>(*label)</dt>
                        <dd>(value.as_str())</dd>
                    </div>
                }
            </dl>
        }
    })
}

#[component]
async fn cast(public: &Public, members: &[CastMember]) -> Result<impl View> {
    Ok(view! {
        if !members.is_empty() {
            <section class="detail-section detail-cast-section">
                <h2>"Cast"</h2>
                <ul class="detail-cast">
                    for member in members {
                        let photo = member.photo.as_deref().map(|path| public.url(path));
                        <li>
                            <span class="detail-face" style=(tint(&member.name))>
                                <span>(initials(&member.name))</span>
                                if let Some(photo) = photo {
                                    <img src=(photo) alt="" loading="lazy" decoding="async" onerror="this.remove()">
                                }
                            </span>
                            <b>(member.name.as_str())</b>
                            if let Some(character) = &member.character {
                                <span class="detail-role">(character.as_str())</span>
                            }
                        </li>
                    }
                </ul>
            </section>
        }
    })
}

#[component]
async fn links(about: &About) -> Result<impl View> {
    Ok(view! {
        if about.imdb.is_some() || about.tmdb.is_some() {
            <p class="detail-links">
                if let Some(imdb) = &about.imdb {
                    <a href=(imdb.as_str()) target="_blank" rel="noopener">"IMDb"</a>
                }
                if let Some(tmdb) = &about.tmdb {
                    <a href=(tmdb.as_str()) target="_blank" rel="noopener">"TMDB"</a>
                }
                if let Some(trailer) = &about.trailer {
                    <a href=(trailer.as_str()) target="_blank" rel="noopener">"Trailer on YouTube"</a>
                }
            </p>
        }
    })
}

#[component]
async fn movie_mark(cx: &Cx, key: &str, finished: bool, said: Signal<String>) -> Result<impl View> {
    let key = key.to_string();
    let seen = signal(cx, move || finished);
    Ok(view! {
        <button type="button" class="pill quiet mark-film" :aria-pressed=$(if seen.get() { "true" } else { "false" })
            @click=$(async |_e| {
                let wanted = !seen.get();
                seen.set(wanted);
                let answer = mark("movie".to_owned(), key.to_owned(), wanted, None, None).await;
                seen.set(answer.0);
                said.set(answer.1);
            })>
            check_icon()
            $(if seen.get() { "Watched" } else { "Mark as watched" })
        </button>
    })
}

#[component]
async fn episode_row(cx: &Cx, public: &Public, series: &str, episode: &Episode, said: Signal<String>) -> Result<impl View> {
    let show_key = series.to_string();
    let (season_number, episode_number) = (episode.season, episode.episode);
    let still = episode.still.as_deref().map(|path| public.url(path));
    let watched = episode.watched;
    let seen = signal(cx, move || watched);
    let label = episode.short_label.clone();
    Ok(view! {
        <li class="episode-row">
            <button type="button" class="episode" @click=$(async |_e| {
                let answer = play_episode(show_key.to_owned(), season_number, episode_number).await;
                said.set(answer);
            })>
                <span class="still" style=(tint(&episode.label))>
                    if let Some(still) = still {
                        <img src=(still) alt="" loading="lazy" decoding="async" onerror="this.remove()">
                    }
                    <span class="detail-seen" title="Watched" :hidden=$(!seen.get())>check_icon()</span>
                    progress(watch: &episode.watch)
                </span>
                <span class="episode-text">
                    <b>(episode.label.as_str())</b>
                    scores(of: &episode.ratings)
                    <span>(episode.short_label.as_str()) " · " (episode.watch.details.as_str())</span>
                </span>
            </button>
            <button type="button" class="mark-seen" aria-label=(format!("Watched {label}"))
                :aria-pressed=$(if seen.get() { "true" } else { "false" })
                @click=$(async |_e| {
                    let wanted = !seen.get();
                    seen.set(wanted);
                    let answer = mark("show".to_owned(), show_key.to_owned(), wanted, Some(season_number), Some(episode_number)).await;
                    seen.set(answer.0);
                    said.set(answer.1);
                })>
                check_icon()
            </button>
        </li>
    })
}

#[component]
async fn find_subtitles(label: &str, body: &str) -> Result<impl View> {
    Ok(view! {
        <div class="find-subtitles-row">
            <button type="button" class="pill quiet" data-post="/api/subtitles/fetch" data-body=(body)
                data-pending="Looking for subtitles…" data-done="hide">(label)</button>
            <p class="answer" data-said="true" aria-live="polite"></p>
        </div>
    })
}

#[component]
async fn last_played(played: &Played) -> Result<impl View> {
    Ok(view! {
        if let Some(label) = &played.last_played_label {
            <p class="detail-last">"Last played " <b>(label.as_str())</b></p>
        }
    })
}

#[page("/movies/{key}")]
async fn movie(cx: &Cx) -> Result<impl View> {
    let key = path_param::<Key>(cx).to_string();
    let public = app_context::<Public>(cx).clone();
    let movie = match app_context::<Api>(cx).movie(&key).await.map_err(failed)? {
        Found::Here(movie) => movie,
        Found::Missing => return Err(not_found().into()),
    };
    let about = &movie.about;
    let said = signal(cx, String::new);
    let label = if movie.watch.fraction().is_some() {
        "Resume"
    } else {
        "Play"
    };
    let length = movie
        .watch
        .duration
        .map(|seconds| runtime((seconds / 60.0).round() as u32));
    let line = facts(&[
        movie.card.years.clone(),
        movie.watch.runtime_minutes.map(runtime).or(length),
        about.certificate.as_ref().map(|certificate| certificate.label()),
        listed(&about.genres),
    ]);
    let stopped = stopped_at(movie.watch.stopped_at, movie.watch.duration);
    let mut rows = Vec::new();
    if let Some(directors) = listed(&about.directors) {
        rows.push((
            if about.directors.len() > 1 {
                "Directors"
            } else {
                "Director"
            },
            directors,
        ));
    }
    if let Some(released) = &about.released {
        rows.push(("Released", long_date(released)));
    }
    if let Some(language) = about.language.clone().filter(|language| language != "English") {
        rows.push(("Language", language));
    }
    let fetch = (movie.subtitles == Some(0)).then(|| serde_json::json!({ "movie": key }).to_string());
    let file = movie.watch.tags.line();
    if !file.is_empty() {
        rows.push(("File", file));
    }
    Ok(view! {
        <article class="title detail">
            back(href: "/")
            hero(public: &public, card: &movie.card)
            <div class="title-body detail-body">
                <div class="detail-columns">
                    <div class="detail-side">
                        <div class="detail-playback">
                            last_played(played: &movie.played)
                            if let Some(stopped) = stopped {
                                progress(watch: &movie.watch)
                                <p class="detail-stopped">(stopped)</p>
                            }
                            <div class="detail-actions">
                                <button type="button" class="play" @click=$(async |_e| {
                                    let answer = play_movie(key).await;
                                    said.set(answer);
                                })>
                                    play_icon()
                                    (label)
                                </button>
                                trailer_button(about: &movie.about)
                            </div>
                            <p class="said" :hidden=$(said.get().is_empty())>$(said.get())</p>
                            movie_mark(key: key.as_str(), finished: movie.card.finished, said: said.clone())
                            if let Some(fetch) = &fetch {
                                find_subtitles(label: "Find subtitles", body: fetch.as_str())
                            }
                        </div>
                        facts_list(rows: &rows)
                        links(about: &movie.about)
                    </div>
                    <div class="detail-main">
                        heading(public: &public, card: &movie.card, about: &movie.about, line: &line)
                        if let Some(plot) = &movie.about.plot {
                            <p class="detail-plot">(plot.as_str())</p>
                        }
                        cast(public: &public, members: &movie.about.cast)
                    </div>
                </div>
            </div>
        </article>
    })
}

#[page("/shows/{key}")]
async fn show(cx: &Cx) -> Result<impl View> {
    let key = path_param::<Key>(cx).to_string();
    let public = app_context::<Public>(cx).clone();
    let show = match app_context::<Api>(cx).show(&key).await.map_err(failed)? {
        Found::Here(show) => show,
        Found::Missing => return Err(not_found().into()),
    };
    let about = &show.about;
    let first = show
        .next
        .as_ref()
        .map(|next| next.season)
        .or_else(|| show.seasons.first().map(|season| season.number))
        .unwrap_or(1);
    let season = signal(cx, move || first);
    let said = signal(cx, String::new);
    let resuming = show.next.as_ref().and_then(Next::fraction);
    let next_label = match &show.next {
        Some(next) if resuming.is_some() => format!("Resume {}", next.label),
        Some(next) => format!("Play {}", next.label),
        None => "Play".to_string(),
    };
    let next_key = key.clone();
    let next_fetch = show.next.as_ref().and_then(|next| {
        let number = next.episode?;
        let season = show.seasons.iter().find(|season| season.number == next.season)?;
        let episode = season.episodes.iter().find(|episode| episode.episode == number)?;
        (episode.subtitles == Some(0)).then(|| {
            (
                format!("Find subtitles for {}", next.label),
                serde_json::json!({ "show": key, "season": next.season, "episode": number }).to_string(),
            )
        })
    });
    let heat = show.heat();
    let columns = heat.first().map_or(0, |row| row.cells.len() as u32);
    let line = facts(&[
        show.card.years.clone(),
        about.certificate.as_ref().map(|certificate| certificate.label()),
        listed(&about.genres),
    ]);
    let total = show.seasons.iter().map(|season| season.episodes.len()).sum::<usize>();
    let watched = show
        .seasons
        .iter()
        .flat_map(|season| &season.episodes)
        .filter(|episode| episode.watched)
        .count();
    let next_text = show
        .next
        .as_ref()
        .filter(|_| show.played.last_played.is_some())
        .map(|next| {
            let title = next
                .title
                .as_deref()
                .map(|title| format!(" · {title}"))
                .unwrap_or_default();
            format!("{}{title}", next.label)
        });
    let stopped = show
        .next
        .as_ref()
        .and_then(|next| stopped_at(next.stopped_at, next.duration));
    let next_watch = show.next.as_ref().map(|next| Watch {
        details: String::new(),
        stopped_at: next.stopped_at,
        duration: next.duration,
        runtime_minutes: None,
        tags: Default::default(),
    });
    let mut rows = Vec::new();
    if let Some(creators) = listed(&about.creators) {
        rows.push(("Created by", creators));
    }
    if let Some(networks) = listed(&about.networks) {
        rows.push(("Network", networks));
    }
    let aired = match (&about.first_aired, &about.last_aired) {
        (Some(first), Some(last)) if first != last => Some(format!("{} – {}", long_date(first), long_date(last))),
        (Some(first), _) => Some(long_date(first)),
        _ => None,
    };
    if let Some(aired) = aired {
        rows.push(("Aired", aired));
    }
    if let (Some(seasons), Some(count)) = (about.seasons, about.episodes) {
        let plural = |count: u32, noun: &str| format!("{count} {noun}{}", if count == 1 { "" } else { "s" });
        rows.push((
            "Made",
            format!("{} · {}", plural(seasons, "season"), plural(count, "episode")),
        ));
    }
    rows.push(("In the library", show.card.subtitle.clone()));
    if let Some(language) = about.language.clone().filter(|language| language != "English") {
        rows.push(("Language", language));
    }
    Ok(view! {
        <article class="title detail">
            back(href: "/")
            hero(public: &public, card: &show.card)
            <div class="title-body detail-body">
                <div class="detail-columns">
                    <div class="detail-side">
                        <div class="detail-playback">
                            last_played(played: &show.played)
                            if let Some(next_text) = next_text {
                                <p class="detail-next">"Up next " <b>(next_text)</b></p>
                            }
                            if let (Some(stopped), Some(next_watch)) = (stopped, next_watch.as_ref()) {
                                progress(watch: next_watch)
                                <p class="detail-stopped">(stopped)</p>
                            }
                            if watched > 0 {
                                <p class="detail-seen-count">"Watched " (watched.to_string()) " of " (total.to_string()) " episodes"</p>
                            }
                            <div class="detail-actions">
                                <button type="button" class="play" @click=$(async |_e| {
                                    let answer = play_show(next_key).await;
                                    said.set(answer);
                                })>
                                    play_icon()
                                    (next_label)
                                </button>
                                trailer_button(about: &show.about)
                            </div>
                            <p class="said" :hidden=$(said.get().is_empty())>$(said.get())</p>
                            if let Some((label, fetch)) = &next_fetch {
                                find_subtitles(label: label.as_str(), body: fetch.as_str())
                            }
                        </div>
                        facts_list(rows: &rows)
                        links(about: &show.about)
                    </div>
                    <div class="detail-main">
                        heading(public: &public, card: &show.card, about: &show.about, line: &line)
                        if let Some(plot) = &show.about.plot {
                            <p class="detail-plot">(plot.as_str())</p>
                        }
                        cast(public: &public, members: &show.about.cast)
                        <section class="detail-section detail-episodes">
                            <h2>"Episodes"</h2>
                            if show.seasons.len() > 1 {
                                <nav class="chips seasons">
                                    #[key(group.number)]
                                    for group in &show.seasons {
                                        let number = group.number;
                                        <button type="button"
                                            :class=$(if season.get() == number { "chip on" } else { "chip" })
                                            @click=$(|_e| season.set(number))>
                                            (group.label.as_str())
                                        </button>
                                    }
                                </nav>
                            }
                            #[key(group.number)]
                            for group in &show.seasons {
                                let number = group.number;
                                <ol class="episodes" :hidden=$(season.get() != number)>
                                    #[key(episode.episode)]
                                    for episode in &group.episodes {
                                        episode_row(public: &public, series: key.as_str(), episode: episode, said: said.clone())
                                    }
                                </ol>
                            }
                        </section>
                        if !heat.is_empty() {
                            <section class="heat">
                                <h2>"Episode ratings"</h2>
                                <div class="heat-scroll">
                                    <table class="heat-grid">
                                        <thead>
                                            <tr>
                                                <th></th>
                                                for number in 1..=columns {
                                                    <th>(number.to_string())</th>
                                                }
                                            </tr>
                                        </thead>
                                        <tbody>
                                            for row in &heat {
                                                <tr>
                                                    <th>(format!("S{}", row.season))</th>
                                                    for cell in &row.cells {
                                                        <td>
                                                            if let Some(cell) = cell {
                                                                let show_key = key.clone();
                                                                let (season_number, episode_number) = (row.season, cell.episode);
                                                                <button type="button" class=(format!("heat-cell {}", cell.band))
                                                                    aria-label=(format!("Play S{season_number} E{episode_number}, IMDb {}", cell.rating))
                                                                    @click=$(async |_e| {
                                                                        let answer = play_episode(show_key, season_number, episode_number).await;
                                                                        said.set(answer);
                                                                    })>
                                                                    (cell.rating.as_str())
                                                                </button>
                                                            } else {
                                                                <span class="heat-cell"></span>
                                                            }
                                                        </td>
                                                    }
                                                </tr>
                                            }
                                        </tbody>
                                    </table>
                                </div>
                                <p class="heat-legend">
                                    for (band, label) in BANDS {
                                        <span><i class=(format!("heat-cell {band}"))></i>(label)</span>
                                    }
                                </p>
                            </section>
                        }
                    </div>
                </div>
            </div>
        </article>
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn card(key: &str, kind: &str, years: Option<&str>, imdb: Option<f64>) -> Card {
        serde_json::from_value(json!({
            "kind": kind, "key": key, "title": key, "years": years, "subtitle": "8 episodes",
            "poster": null, "backdrop": null,
            "ratings": { "imdb": imdb, "rotten_tomatoes": null, "fresh": null, "popcorn": null, "full": null },
        }))
        .unwrap()
    }

    fn cards() -> Vec<Card> {
        vec![
            card("alien", "movie", Some("1979"), Some(8.5)),
            card("dune", "movie", Some("2021"), Some(8.0)),
            card("home", "movie", None, None),
            card("cats", "movie", Some("2019"), Some(2.8)),
            card("lotus", "show", Some("2021–"), Some(7.9)),
        ]
    }

    fn keys(cards: &[Card]) -> Vec<&str> {
        cards.iter().map(|card| card.key.as_str()).collect()
    }

    #[test]
    fn no_filter_keeps_the_library_order() {
        assert_eq!(
            keys(&arrange(&cards(), "", "")),
            ["alien", "dune", "home", "cats", "lotus"]
        );
    }

    #[test]
    fn best_first_puts_unrated_titles_last() {
        assert_eq!(
            keys(&arrange(&cards(), "best", "")),
            ["alien", "dune", "lotus", "cats", "home"]
        );
        assert_eq!(keys(&arrange(&cards(), "8", "")), ["alien", "dune"]);
    }

    #[test]
    fn years_sort_or_narrow_to_a_decade() {
        assert_eq!(
            keys(&arrange(&cards(), "", "newest")),
            ["dune", "lotus", "cats", "alien", "home"]
        );
        assert_eq!(
            keys(&arrange(&cards(), "", "oldest")),
            ["alien", "cats", "dune", "lotus", "home"]
        );
        assert_eq!(keys(&arrange(&cards(), "", "2020")), ["dune", "lotus"]);
        assert_eq!(keys(&arrange(&cards(), "", "older")), ["alien"]);
        assert_eq!(keys(&arrange(&cards(), "7", "2020")), ["dune", "lotus"]);
    }

    #[test]
    fn titles_from_the_same_year_come_best_first() {
        let mut cards = cards();
        cards[4].ratings.imdb = Some(8.3);
        assert_eq!(
            keys(&arrange(&cards, "", "newest")),
            ["dune", "lotus", "cats", "alien", "home"]
        );
        assert_eq!(
            keys(&arrange(&cards, "best", "newest")),
            ["lotus", "dune", "cats", "alien", "home"]
        );
        assert_eq!(keys(&arrange(&cards, "best", "2020")), ["lotus", "dune"]);
    }

    #[test]
    fn a_year_filter_shows_when_a_series_ran() {
        let arranged = arrange(&cards(), "", "2020");
        assert_eq!(arranged[1].subtitle, "2021–");
    }
}
