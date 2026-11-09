use crate::api::{Api, Card, Found, Kind, Ratings, Watch, WatchingCard};
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

#[page("/movies/{key}")]
async fn movie(cx: &Cx) -> Result<impl View> {
    let key = path_param::<Key>(cx).to_string();
    let public = app_context::<Public>(cx).clone();
    let movie = match app_context::<Api>(cx).movie(&key).await.map_err(failed)? {
        Found::Here(movie) => movie,
        Found::Missing => return Err(not_found().into()),
    };
    let said = signal(cx, String::new);
    let label = if movie.watch.fraction().is_some() {
        "Resume"
    } else {
        "Play"
    };
    Ok(view! {
        <article class="title">
            hero(public: &public, card: &movie.card)
            <div class="title-body">
                <h1>(movie.card.title.as_str())</h1>
                <p class="title-sub">
                    (movie.year.map(|year| year.to_string()).unwrap_or_default())
                    scores(of: &movie.card.ratings)
                </p>
                <p class="details">(movie.watch.details.as_str())</p>
                progress(watch: &movie.watch)
                <button type="button" class="play" @click=$(async |_e| {
                    let answer = play_movie(key).await;
                    said.set(answer);
                })>
                    play_icon()
                    (label)
                </button>
                <p class="said" :hidden=$(said.get().is_empty())>$(said.get())</p>
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
    let first = show
        .next
        .as_ref()
        .map(|next| next.season)
        .or_else(|| show.seasons.first().map(|season| season.number))
        .unwrap_or(1);
    let season = signal(cx, move || first);
    let said = signal(cx, String::new);
    let next_label = show
        .next
        .as_ref()
        .map_or_else(|| "Play".to_string(), |next| format!("Play {}", next.label));
    let next_key = key.clone();
    Ok(view! {
        <article class="title">
            hero(public: &public, card: &show.card)
            <div class="title-body">
                <h1>(show.card.title.as_str())</h1>
                <p class="title-sub">
                    (show.card.subtitle.as_str())
                    scores(of: &show.card.ratings)
                </p>
                <button type="button" class="play" @click=$(async |_e| {
                    let answer = play_show(next_key).await;
                    said.set(answer);
                })>
                    play_icon()
                    (next_label)
                </button>
                <p class="said" :hidden=$(said.get().is_empty())>$(said.get())</p>
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
                            <li>
                                <button type="button" class="episode" @click=$(async |_e| {
                                    let answer = play_episode(show_key, season_number, episode_number).await;
                                    said.set(answer);
                                })>
                                    <span class="still" style=(tint(&episode.label))>
                                        if let Some(still) = still {
                                            <img src=(still) alt="" loading="lazy" decoding="async" onerror="this.remove()">
                                        }
                                        progress(watch: &episode.watch)
                                    </span>
                                    <span class="episode-text">
                                        <b>(episode.label.as_str())</b>
                                        <span>(episode.short_label.as_str()) " · " (episode.watch.details.as_str())</span>
                                    </span>
                                </button>
                            </li>
                        }
                    </ol>
                }
            </div>
        </article>
    })
}
