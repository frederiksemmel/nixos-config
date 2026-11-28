//! Finding a club by name: the field on the day page lists the clubs that
//! match as you type (day.js), and without JavaScript the form goes to
//! `/find`, which opens the one club that matches or lists them on `/clubs`.

use crate::db::{self, Db};
use crate::pages::{km_from_home, sport_name};
use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::router::error::{SeeOther, see_other};
use topcoat::router::{page, query_params, route};
use topcoat::view::{View, component, view};

/// A club as the search knows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: i64,
    pub name: String,
    /// Where it is: its town, or the sites it is split into.
    pub place: String,
    /// Everything it can be found by, in `plain` form.
    pub words: String,
}

/// Lower case without accents, words apart by single spaces; day.js
/// `plainText` does the same to what is typed.
pub fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        let c = match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ý' | 'ÿ' => 'y',
            c if c.is_alphanumeric() => c,
            _ => ' ',
        };
        if c != ' ' || !out.is_empty() && !out.ends_with(' ') {
            out.push(c);
        }
    }
    out.trim_end().to_string()
}

pub fn entries(clubs: &[db::Club]) -> Vec<Entry> {
    let mut entries: Vec<Entry> = clubs
        .iter()
        .map(|club| {
            let sites: Vec<&str> = crate::sites::of(club.id).map(|site| site.word).collect();
            let place = if sites.is_empty() {
                club.city.clone()
            } else {
                sites.join(" · ")
            };
            let mut sports: Vec<String> = club.courts.0.iter().map(|court| sport_name(&court.sport)).collect();
            sports.sort();
            sports.dedup();
            Entry {
                id: club.id,
                name: club.name.clone(),
                words: plain(&format!(
                    "{} {} {} {} {}",
                    club.name,
                    club.city,
                    club.zip,
                    sites.join(" "),
                    sports.join(" ")
                )),
                place,
            }
        })
        .collect();
    entries.sort_by_key(|entry| plain(&entry.name));
    entries
}

/// The entries every typed word occurs in, those whose name starts with
/// it first.
pub fn matching<'a>(entries: &'a [Entry], query: &str) -> Vec<&'a Entry> {
    let query = plain(query);
    let words: Vec<&str> = query.split(' ').filter(|word| !word.is_empty()).collect();
    let mut found: Vec<&Entry> = entries
        .iter()
        .filter(|entry| words.iter().all(|word| entry.words.contains(word)))
        .collect();
    found.sort_by_key(|entry| {
        let name = plain(&entry.name);
        (!name.starts_with(&query), !words.iter().all(|word| name.contains(word)))
    });
    found
}

#[query_params]
struct Query {
    q: Option<String>,
}

fn typed(cx: &Cx) -> String {
    query_params::<Query>(cx)
        .ok()
        .and_then(|query| query.q.clone())
        .unwrap_or_default()
        .trim()
        .to_string()
}

async fn known(cx: &Cx) -> Result<Vec<db::Club>> {
    app_context::<Db>(cx)
        .clubs(&app_context::<crate::discover::Clubs>(cx).ids())
        .await
        .map_err(topcoat::Error::msg)
}

#[route(GET "/find")]
async fn find(cx: &Cx) -> Result<SeeOther> {
    let query = typed(cx);
    let all = entries(&known(cx).await?);
    let found = matching(&all, &query);
    if let [only] = found.as_slice() {
        return Ok(see_other(format!("/clubs/{}", only.id)));
    }
    let query: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
    Ok(see_other(format!("/clubs?q={query}")))
}

/// The field, and the clubs it can find for day.js to show.
#[component]
pub async fn club_search(entries: &[Entry], query: &str) -> Result<impl View> {
    Ok(view! {
        <form class="find" role="search" action="/find" method="get" data-find="1">
            <label class="find-field">
                <span class="sr">"Find a club"</span>
                <svg class="find-icon" viewBox="0 0 24 24" aria-hidden="true"><circle cx="10.5" cy="10.5" r="6.5"/><path d="M15.5 15.5 20 20"/></svg>
                <input type="search" name="q" value=(query) placeholder="Find a club" autocomplete="off" autocapitalize="off" spellcheck="false" enterkeyhint="go" data-find-input="1">
            </label>
            <ul class="find-hits" data-find-hits="1" hidden=(true)>
                for entry in entries {
                    <li data-words=(entry.words.as_str()) hidden=(true)>
                        <a href=(format!("/clubs/{}", entry.id))>
                            <span class="find-name">(entry.name.as_str())</span>
                            <span class="quiet">(entry.place.as_str())</span>
                        </a>
                    </li>
                }
                <li class="quiet find-none" data-find-none="1" hidden=(true)>"No club is called that. Try part of its name or its town."</li>
            </ul>
        </form>
    })
}

#[page("/clubs")]
async fn clubs_page(cx: &Cx) -> Result<impl View> {
    let query = typed(cx);
    let clubs = known(cx).await?;
    let all = entries(&clubs);
    let found: Vec<(Entry, String)> = matching(&all, &query)
        .into_iter()
        .map(|entry| {
            let far = clubs
                .iter()
                .find(|club| club.id == entry.id)
                .map_or(String::new(), |club| {
                    let km = crate::sites::of(club.id)
                        .map(|site| km_from_home(cx, &crate::sites::at(club, Some(site))))
                        .reduce(f64::min)
                        .unwrap_or_else(|| km_from_home(cx, club));
                    format!("{km:.1} km")
                });
            (entry.clone(), far)
        })
        .collect();
    let heading = if query.is_empty() {
        "Clubs".to_string()
    } else {
        format!("Clubs for “{query}”")
    };
    Ok(view! {
        crate::pages::nav(current: "courts")
        club_search(entries: &all, query: &query)
        <article class="page">
            <h1>(heading.as_str())</h1>
            if found.is_empty() {
                <p class="empty">"No club is called that. Try part of its name or its town."</p>
            }
            <ul class="club-list">
                for (entry, far) in &found {
                    <li>
                        <a href=(format!("/clubs/{}", entry.id))>(entry.name.as_str())</a>
                        <span class="quiet">(format!("{} · {far}", entry.place))</span>
                    </li>
                }
            </ul>
        </article>
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gotcourts::Court;
    use toasty::Json;

    fn club(id: i64, name: &str, city: &str, sport: &str) -> db::Club {
        db::Club {
            id,
            source: "gotcourts".into(),
            name: name.into(),
            canonical: String::new(),
            address: String::new(),
            zip: "8000".into(),
            city: city.into(),
            latitude: 47.0,
            longitude: 8.5,
            phone: String::new(),
            web: String::new(),
            kind: "public".into(),
            courts: Json(vec![Court {
                id,
                sport: sport.into(),
                label: "Platz 1".into(),
                surface: "clay".into(),
                indoor: None,
                cover: "outdoor".into(),
                ball_machine: false,
                order: 1,
                interval: 60,
                durations: vec![60],
                opening: 0,
                closing: 0,
                cells: Vec::new(),
            }]),
            checked_at: 0,
        }
    }

    fn clubs() -> Vec<db::Club> {
        vec![
            club(6, "Tennis Lengg (Sandplätze)", "Zürich", "tennis"),
            club(21, "ASVZ Zürich", "Zürich", "tennis"),
            club(1_000_000_001, "Squash Arena Uster", "Uster", "squash"),
            club(19, "Tennisclub Uster", "Uster", "tennis"),
        ]
    }

    fn names(query: &str) -> Vec<String> {
        let all = entries(&clubs());
        matching(&all, query)
            .into_iter()
            .map(|entry| entry.name.clone())
            .collect()
    }

    #[test]
    fn plain_drops_case_accents_and_punctuation() {
        assert_eq!(
            plain("  Sandplätze (Hönggerberg)–Zürich! "),
            "sandplatze honggerberg zurich"
        );
    }

    #[test]
    fn a_club_is_found_by_part_of_its_name_its_town_its_site_or_its_sport() {
        assert_eq!(names("lengg"), ["Tennis Lengg (Sandplätze)"]);
        assert_eq!(names("sandplatze"), ["Tennis Lengg (Sandplätze)"]);
        assert_eq!(names("Hongg"), ["ASVZ Zürich"]);
        assert_eq!(names("irchel"), ["ASVZ Zürich"]);
        assert_eq!(names("squash"), ["Squash Arena Uster"]);
        assert_eq!(names("uster tennis"), ["Tennisclub Uster"]);
        assert!(names("padel").is_empty());
    }

    #[test]
    fn names_that_start_with_what_was_typed_come_first() {
        assert_eq!(names("uster"), ["Squash Arena Uster", "Tennisclub Uster"]);
        assert_eq!(names("tennis")[0], "Tennis Lengg (Sandplätze)");
        assert_eq!(names("ten")[..2], ["Tennis Lengg (Sandplätze)", "Tennisclub Uster"]);
    }

    #[test]
    fn a_split_club_says_where_its_sites_are() {
        let all = entries(&clubs());
        let asvz = all.iter().find(|entry| entry.id == 21).unwrap();
        assert_eq!(asvz.place, "Irchel · Hönggerberg");
        assert_eq!(all.iter().find(|entry| entry.id == 19).unwrap().place, "Uster");
    }
}
