//! "Which of these suits you?": a few free slots sent as one link. Whoever
//! opens it needs no sign-in: they tap the one they want, and the person who
//! sent it gets a notification and books it; or they book it themselves.

use crate::db::{self, Db};
use crate::push::{Notice, Push};
use crate::share::{self, Now, Price, Shared};
use crate::slots::clock;
use crate::source::Source;
use crate::time;
use serde::{Deserialize, Serialize};
use topcoat::Result;
use topcoat::asset::{Asset, asset};
use topcoat::context::{Cx, app_context};
use topcoat::router::content::{Form, Html, Json};
use topcoat::router::error::{SeeOther, see_other};
use topcoat::router::{StatusCode, path_param, query_params, route};
use topcoat::view::{ViewExt, view};

const SCRIPT: Asset = asset!("./ask.js");
/// The most slots one ask offers.
pub const MOST_OPTIONS: usize = 12;
/// The most picks one ask takes, so a link passed around can't fill the database.
const MOST_PICKS: usize = 60;
const LONGEST_NAME: usize = 40;

/// Someone chose option `option` (counted from 0).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pick {
    pub name: String,
    pub option: usize,
    pub at: i64,
}

/// The slots that share codes name, in order of time, each once; none when
/// a code is made up or there are too many.
pub fn options(key: &str, codes: &[String]) -> Option<Vec<Shared>> {
    let mut found: Vec<Shared> = codes
        .iter()
        .map(|code| share::read(key, code.trim().trim_start_matches("/s/")))
        .collect::<Option<_>>()?;
    found.sort_by(|a, b| (&a.date, a.start, a.club, a.court).cmp(&(&b.date, b.start, b.club, b.court)));
    found.dedup();
    (!found.is_empty() && found.len() <= MOST_OPTIONS).then_some(found)
}

/// A name as given, or "Someone".
pub fn name(given: Option<&str>) -> String {
    let name: String = given
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .take(LONGEST_NAME)
        .collect();
    if name.is_empty() { "Someone".into() } else { name }
}

/// "Anna and Ben", "Anna, Ben and Cleo".
pub fn and(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// "Sat 18:00 · TC Lengg": an option in a line.
pub fn short(shared: &Shared, club: &str) -> String {
    format!("{} {} · {club}", time::weekday(&shared.date), clock(shared.start))
}

#[derive(Deserialize)]
struct NewAsk {
    codes: Vec<String>,
}

#[route(POST "/api/asks")]
async fn api_new_ask(cx: &Cx, Json(new): Json<NewAsk>) -> Result<Json<serde_json::Value>> {
    let me = crate::auth::person(cx)?;
    let key = app_context::<crate::auth::Key>(cx).0.clone();
    let Some(options) = options(&key, &new.codes) else {
        return Ok(Json(serde_json::json!({
            "ok": false,
            "message": format!("Choose between 1 and {MOST_OPTIONS} times from this page"),
        })));
    };
    let count = options.len();
    let token = crate::auth::token()?;
    app_context::<Db>(cx)
        .add_ask(&token, me.id, options, time::now())
        .await
        .map_err(topcoat::Error::msg)?;
    Ok(Json(serde_json::json!({
        "ok": true,
        "url": format!("{}/a/{token}", crate::auth::origin(cx)),
        "title": format!("Which court suits you? {count} options"),
    })))
}

/// One option as the page shows it.
struct Choice {
    index: usize,
    headline: String,
    date: String,
    hours: String,
    court: String,
    cost: String,
    status_class: &'static str,
    status: String,
    when_class: &'static str,
    free: bool,
    book: String,
    book_label: String,
    map: String,
    place: String,
    picked_by: String,
}

async fn choices(cx: &Cx, ask: &db::Ask) -> Result<Vec<(Choice, String)>> {
    let db = app_context::<Db>(cx).clone();
    let prices = app_context::<share::Prices>(cx).clone();
    let iphone = crate::source::on_iphone(cx);
    let (today, seconds) = (time::today(), time::seconds_today());
    let mut found = Vec::new();
    for (index, shared) in ask.options.0.iter().enumerate() {
        if let Some(place) = share::place(&db, shared.clone()).await? {
            found.push((index, place));
        }
    }
    let asked: Vec<_> = found
        .iter()
        .map(|(_, (shared, club, court, _))| {
            let (prices, shared, club, court) = (prices.clone(), shared.clone(), club.clone(), court.clone());
            tokio::spawn(async move { prices.of(&shared, &club, &court).await })
        })
        .collect();
    let mut made = Vec::new();
    for ((index, (shared, club, court, day)), price) in found.into_iter().zip(asked) {
        let price = price.await.unwrap_or(Price::Unknown);
        let state = share::now(day.as_ref(), &shared, &today, seconds);
        let sport = crate::pages::sport_name(&court.sport);
        let source = Source::parse(&club.source);
        let app = source.app_url(club.id).filter(|_| iphone);
        let book_label = format!("Book {} {}", if app.is_some() { "in" } else { "on" }, source.name());
        let book = app.unwrap_or_else(|| {
            source.booking_url(
                &club.canonical,
                Some(&shared.date),
                Some(&court.sport),
                Some(shared.start),
            )
        });
        let (status_class, status) = match state {
            Now::Free { checked } => ("status free", format!("Free · checked {}", time::ago(checked))),
            Now::Taken { checked } => (
                "status taken",
                format!("No longer free · checked {}", time::ago(checked)),
            ),
            Now::Over => ("status over", "Over".to_string()),
            Now::Unknown => ("status unknown", "Can't be checked right now".to_string()),
        };
        let names: Vec<&str> = ask
            .picks
            .0
            .iter()
            .filter(|pick| pick.option == index)
            .map(|pick| pick.name.as_str())
            .collect();
        let line = short(&shared, &club.name);
        made.push((
            Choice {
                index,
                headline: share::headline(&sport, &club.name),
                date: time::parse(&shared.date).map_or(shared.date.clone(), |d| d.strftime("%A %-d %B").to_string()),
                hours: format!("{}–{}", clock(shared.start), clock(shared.end)),
                court: format!(
                    "{}, {}, {}",
                    court.label,
                    crate::pages::surface(&court.surface),
                    crate::gotcourts::cover_label(&court)
                ),
                cost: match price {
                    Price::Rappen(rappen) => {
                        format!("{} for {}", share::francs(rappen), share::length(shared.minutes()))
                    }
                    Price::NotListed => "Price not listed".into(),
                    Price::Unknown => "Price couldn't be checked just now".into(),
                },
                status_class,
                status,
                when_class: match state {
                    Now::Free { .. } => "when",
                    Now::Taken { .. } | Now::Over => "when gone",
                    Now::Unknown => "when unsure",
                },
                free: matches!(state, Now::Free { .. } | Now::Unknown),
                book,
                book_label,
                map: share::maps(&club),
                place: if !club.zip.is_empty() && club.address.contains(&club.zip) {
                    club.address.clone()
                } else {
                    format!("{}, {} {}", club.address, club.zip, club.city)
                },
                picked_by: if names.is_empty() {
                    String::new()
                } else {
                    format!("Picked by {}", and(&names))
                },
            },
            line,
        ));
    }
    Ok(made)
}

path_param!(token);

#[query_params]
struct AskQuery {
    picked: Option<usize>,
}

#[route(GET "/a/{token}")]
async fn ask_page(cx: &Cx) -> Result<(StatusCode, Html<String>)> {
    let token = path_param::<Token>(cx).to_string();
    let db = app_context::<Db>(cx).clone();
    let found = db.ask(&token).await.map_err(topcoat::Error::msg)?;
    let Some(ask) = found else {
        let html = view! {
            <!DOCTYPE html>
            <html lang="en">
                <head>
                    <meta charset="utf-8">
                    <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
                    <meta name="color-scheme" content="dark">
                    <meta name="robots" content="noindex">
                    <title>"Courts"</title>
                    <link rel="icon" href=(crate::layout::ICON_URL) type="image/svg+xml">
                    <link rel="stylesheet" href=(crate::layout::STYLE)>
                </head>
                <body>
                    <main class="shared">
                        <article class="slot">
                            <h1>"These courts are gone"</h1>
                            <p class="quiet">"The days in this link are over, or the link was cut off when it was copied. Ask whoever sent it for a new one."</p>
                        </article>
                    </main>
                </body>
            </html>
        }
        .single()
        .await?
        .render(cx);
        return Ok((StatusCode::NOT_FOUND, Html(html)));
    };
    let picked = query_params::<AskQuery>(cx)
        .map_err(|error| topcoat::Error::msg(error.to_string()))?
        .picked;
    let sender = db
        .person(ask.person_id)
        .await
        .map_err(topcoat::Error::msg)?
        .map_or_else(|| "Someone".to_string(), |person| person.name);
    let mine = crate::auth::me(cx).is_some_and(|me| me.id == ask.person_id);
    let made = choices(cx, &ask).await?;
    let lines: Vec<String> = made.iter().map(|(_, line)| line.clone()).collect();
    let choices: Vec<Choice> = made.into_iter().map(|(choice, _)| choice).collect();
    let open = choices.iter().filter(|choice| choice.free).count();
    let heading = "Which court suits you?";
    let title = format!("{sender} asks: which court suits you?");
    let description = lines.join(" · ");
    let here = format!("{}/a/{token}", crate::auth::origin(cx));
    let lede = if mine {
        format!(
            "You sent these {} to choose from. Whoever opens the link taps the one they want, and you get a notification.",
            if choices.len() == 1 { "court" } else { "courts" }
        )
    } else {
        format!(
            "{sender} found these free courts. Tap the one you'd like: {sender} gets told and books it. Or book it yourself."
        )
    };
    let thanks = picked
        .and_then(|index| ask.options.0.get(index).map(|shared| (index, shared)))
        .and_then(|(index, _)| choices.iter().find(|choice| choice.index == index))
        .map(|choice| {
            format!(
                "Thanks. {sender} is told you'd like {} on {}, {}.",
                choice.headline, choice.date, choice.hours
            )
        });
    let action = format!("/a/{token}/pick");
    let signed_in = crate::auth::signed_in(cx);
    let html = view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
                <meta name="theme-color" content="#10241a">
                <meta name="color-scheme" content="dark">
                <meta name="robots" content="noindex">
                <title>(title.as_str())</title>
                <meta name="description" content=(description.as_str())>
                <meta property="og:type" content="website">
                <meta property="og:site_name" content="Courts">
                <meta property="og:title" content=(title.as_str())>
                <meta property="og:description" content=(description.as_str())>
                <meta property="og:url" content=(here.as_str())>
                <meta name="twitter:card" content="summary">
                <link rel="icon" href=(crate::layout::ICON_URL) type="image/svg+xml">
                <link rel="stylesheet" href=(crate::layout::STYLE)>
                <script type="module" src=(SCRIPT)></script>
            </head>
            <body>
                <main class="shared ask">
                    <header class="ask-head">
                        <h1>(heading)</h1>
                        <p class="lede">(lede.as_str())</p>
                        if mine {
                            <div class="slot-actions">
                                <button type="button" class="pill" data-share=(here.as_str()) data-title=(title.as_str())>"Send the link again"</button>
                            </div>
                        }
                    </header>
                    if let Some(thanks) = &thanks {
                        <p class="thanks" role="status">(thanks.as_str())</p>
                    }
                    if open == 0 {
                        <p class="note">"None of these is free any more."</p>
                    }
                    <form class="ask-form" method="post" action=(action.as_str())>
                        if !mine {
                            <label class="ask-name">"Your name"
                                <input type="text" name="name" maxlength="40" autocomplete="given-name" enterkeyhint="done" placeholder=(format!("So {sender} knows who it is"))>
                            </label>
                        }
                        <ol class="options">
                            for choice in &choices {
                                <li class="option">
                                    <p class=(choice.when_class)>
                                        <span class="date">(choice.date.as_str())</span>
                                        <span class="hours">(choice.hours.as_str())</span>
                                    </p>
                                    <h2 lang="de">(choice.headline.as_str())</h2>
                                    <p class="meta">(choice.court.as_str())<br>(choice.cost.as_str())<br><a href=(choice.map.as_str()) target="_blank" rel="noopener">(choice.place.as_str())</a></p>
                                    <p class=(choice.status_class)><span class="dot" aria-hidden="true"></span>(choice.status.as_str())</p>
                                    if !choice.picked_by.is_empty() {
                                        <p class="picked-by">(choice.picked_by.as_str())</p>
                                    }
                                    <div class="slot-actions">
                                        if choice.free && !mine {
                                            <button class="pill" type="submit" name="option" value=(choice.index.to_string())>"This one"</button>
                                        }
                                        <a class=(if mine && choice.free { "pill" } else { "pill quiet" }) href=(choice.book.as_str()) target="_blank" rel="noopener">(choice.book_label.as_str())</a>
                                    </div>
                                </li>
                            }
                        </ol>
                    </form>
                    <footer class="shared-foot quiet">
                        "Sent from Courts, which finds free courts on GotCourts and Eversports. Times and prices are theirs."
                        if signed_in {
                            " " <a href="/">"Back to Courts"</a>
                        }
                    </footer>
                </main>
            </body>
        </html>
    }
    .single()
    .await?
    .render(cx);
    Ok((StatusCode::OK, Html(html)))
}

#[derive(Deserialize)]
struct Chosen {
    option: String,
    name: Option<String>,
}

#[route(POST "/a/{token}/pick")]
async fn pick_option(cx: &Cx, Form(chosen): Form<Chosen>) -> Result<SeeOther> {
    let token = path_param::<Token>(cx).to_string();
    let db = app_context::<Db>(cx).clone();
    let back = format!("/a/{token}");
    let Some(ask) = db.ask(&token).await.map_err(topcoat::Error::msg)? else {
        return Ok(see_other(back));
    };
    let Some((index, shared)) = chosen
        .option
        .parse::<usize>()
        .ok()
        .and_then(|index| ask.options.0.get(index).map(|shared| (index, shared.clone())))
    else {
        return Ok(see_other(back));
    };
    let who = name(chosen.name.as_deref());
    let pick = Pick {
        name: who.clone(),
        option: index,
        at: time::now(),
    };
    if !db
        .add_pick(&token, pick, MOST_PICKS)
        .await
        .map_err(topcoat::Error::msg)?
    {
        return Ok(see_other(back));
    }
    if let Some((shared, club, court, _)) = share::place(&db, shared).await? {
        let notice = Notice {
            title: format!("{who} picked a court"),
            body: share::title(&court.sport, &shared, &club.name),
            url: back.clone(),
            tag: format!("ask-{}", token.chars().take(12).collect::<String>()),
        };
        let push = app_context::<Push>(cx).clone();
        let person = ask.person_id;
        tokio::spawn(async move {
            let sent = push.notify(Some(person), &notice).await;
            eprintln!("ask: {} ({sent} browsers)", notice.title);
        });
    }
    Ok(see_other(format!("{back}?picked={index}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(date: &str, hour: i64) -> Shared {
        Shared {
            club: 20,
            court: 111,
            date: date.into(),
            start: hour * 3600,
            end: (hour + 1) * 3600,
        }
    }

    #[test]
    fn an_ask_takes_signed_slots_in_order_and_once() {
        let late = share::path("k", &slot("2026-11-30", 18));
        let early = share::path("k", &slot("2026-11-29", 9));
        let codes = vec![late.clone(), early.clone(), late.trim_start_matches("/s/").to_string()];
        assert_eq!(
            options("k", &codes),
            Some(vec![slot("2026-11-29", 9), slot("2026-11-30", 18)])
        );
        assert_eq!(options("other", &codes), None);
        assert_eq!(options("k", &[]), None);
        let many: Vec<String> = (6..20)
            .map(|hour| share::path("k", &slot("2026-11-29", hour)))
            .collect();
        assert_eq!(options("k", &many), None);
    }

    #[test]
    fn a_pick_carries_a_tidy_name() {
        assert_eq!(name(None), "Someone");
        assert_eq!(name(Some("   ")), "Someone");
        assert_eq!(name(Some("  Anna \n Muster ")), "Anna Muster");
        assert_eq!(name(Some(&"x".repeat(100))).len(), 40);
        assert_eq!(and(&["Anna"]), "Anna");
        assert_eq!(and(&["Anna", "Ben"]), "Anna and Ben");
        assert_eq!(and(&["Anna", "Ben", "Cleo"]), "Anna, Ben and Cleo");
        assert_eq!(short(&slot("2026-11-29", 18), "Lengg"), "Sun 18:00 · Lengg");
    }
}
