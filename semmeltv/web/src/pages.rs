use crate::api::{About, Api, Card, CastMember, Found, Kind, MoreScores, Next, Played, Ratings, Watch, WatchingCard};
use crate::playing::{play_episode, play_movie, play_show, tell};
use crate::public::Public;
use std::hash::{DefaultHasher, Hash, Hasher};
use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::router::error::not_found;
use topcoat::router::{page, path_param};
use topcoat::runtime::{Event, link, shard, signal};
use topcoat::view::{View, attributes, component, view};

const LONGEST_QUERY: usize = 100;
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

#[component]
async fn watching_tile(cx: &Cx, public: &Public, watching: &WatchingCard) -> Result<impl View> {
    let card = &watching.card;
    let image = card.backdrop.as_deref().map(|path| public.url(path));
    let attrs = attributes! { cx => class="tile-link" };
    let key = card.key.clone();
    let is_movie = card.kind == Kind::Movie;
    let said = signal(cx, String::new);
    Ok(view! {
        <div class="tile watching">
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

#[page("/")]
async fn home(cx: &Cx) -> Result<impl View> {
    let query = signal(cx, String::new);
    let section = signal(cx, || "all".to_owned());
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
        <nav class="chips" :hidden=$(!query.get().trim().is_empty())>
            <button type="button" :class=$(if section.get() == "all" { "chip on" } else { "chip" })
                @click=$(|_e| section.set("all".to_owned()))>"All"</button>
            <button type="button" :class=$(if section.get() == "shows" { "chip on" } else { "chip" })
                @click=$(|_e| section.set("shows".to_owned()))>"Series"</button>
            <button type="button" :class=$(if section.get() == "movies" { "chip on" } else { "chip" })
                @click=$(|_e| section.set("movies".to_owned()))>"Movies"</button>
        </nav>
        browse(query: $(query.get()), section: $(section.get()))
    })
}

#[shard]
async fn browse(cx: &Cx, query: String, section: String) -> Result<impl View> {
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
    let show_series = section != "movies";
    let show_movies = section != "shows";
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
                if section == "all" && !library.watching.is_empty() {
                    <section class="shelf">
                        <h2>"Currently Watching"</h2>
                        <div class="row">
                            #[key(watching.card.href())]
                            for watching in &library.watching {
                                watching_tile(public: &public, watching: watching)
                            }
                        </div>
                    </section>
                }
                if section == "all" && !library.newest.is_empty() {
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
                if show_series {
                    <section class="shelf">
                        <h2>"Series"</h2>
                        grid(public: &public, cards: &library.shows)
                    </section>
                }
                if show_movies {
                    <section class="shelf">
                        <h2>"Movies"</h2>
                        grid(public: &public, cards: &library.movies)
                    </section>
                }
            },
            (None, None) => "",
        }
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
    let file = movie.watch.tags.line();
    if !file.is_empty() {
        rows.push(("File", file));
    }
    Ok(view! {
        <article class="title detail">
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
                                        let show_key = key.clone();
                                        let (season_number, episode_number) = (episode.season, episode.episode);
                                        let still = episode.still.as_deref().map(|path| public.url(path));
                                        let seen = episode.watched;
                                        <li>
                                            <button type="button" class="episode" @click=$(async |_e| {
                                                let answer = play_episode(show_key, season_number, episode_number).await;
                                                said.set(answer);
                                            })>
                                                <span class="still" style=(tint(&episode.label))>
                                                    if let Some(still) = still {
                                                        <img src=(still) alt="" loading="lazy" decoding="async" onerror="this.remove()">
                                                    }
                                                    if seen {
                                                        <span class="detail-seen" title="Watched">
                                                            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 12.5 4 4 8-9"></path></svg>
                                                        </span>
                                                    }
                                                    progress(watch: &episode.watch)
                                                </span>
                                                <span class="episode-text">
                                                    <b>(episode.label.as_str())</b>
                                                    scores(of: &episode.ratings)
                                                    <span>
                                                        (episode.short_label.as_str()) " · " (episode.watch.details.as_str())
                                                        if seen { " · Watched" }
                                                    </span>
                                                </span>
                                            </button>
                                        </li>
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
