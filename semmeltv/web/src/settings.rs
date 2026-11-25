use crate::api::{Api, Audio, Link, PairState, TvAbilities};
use crate::layout::back;
use crate::live::Live;
use crate::playing::headphones_icon;
use crate::storage::storage_icon;
use crate::subtitles::{language_order, style, switch};
use crate::usage::usage_icon;
use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::router::page;
use topcoat::runtime::{Event, connected, link, procedure, signal};
use topcoat::view::{View, attributes, component, emit, live, view};

fn tv_name(abilities: &TvAbilities) -> String {
    match abilities.kind.as_deref() {
        Some("samsung") => "Samsung TV".to_string(),
        Some(_) => "TV".to_string(),
        None => "TV, not reachable right now".to_string(),
    }
}

fn tv_detail(abilities: &TvAbilities) -> &'static str {
    if abilities.volume {
        "On, off and input over HDMI, volume over the network"
    } else {
        "On, off and input over HDMI"
    }
}

fn pairing(paired: Option<bool>) -> (&'static str, &'static str) {
    match paired {
        Some(true) => ("Paired", "Pair again"),
        _ => ("Not paired yet, so the volume keys won't work", "Pair with the TV"),
    }
}

fn link_text(state: Link) -> &'static str {
    match state {
        Link::Connected => "Connected",
        Link::Connecting => "Connecting…",
        Link::Disconnected => "Paired, not connected",
    }
}

#[derive(Debug, PartialEq)]
enum PairButton {
    Pair,
    Cancel,
    Wait,
}

struct PairRow {
    own: Option<(String, String)>,
    detail: String,
    button: PairButton,
    note: String,
}

fn pair_row(audio: &Audio, me: Option<i64>) -> PairRow {
    let button = match audio.pairing.as_ref().map(|pairing| pairing.state) {
        Some(PairState::Scanning) => PairButton::Cancel,
        Some(PairState::Pairing) => PairButton::Wait,
        _ => PairButton::Pair,
    };
    let mine = audio
        .headphones
        .as_ref()
        .filter(|headphones| me.is_some_and(|me| headphones.owners.contains(&me)));
    let own = match mine {
        Some(headphones) => Some((headphones.name.clone(), link_text(headphones.state).to_string())),
        None if me.is_some_and(|me| audio.with_headphones.contains(&me)) => {
            Some(("My headphones".to_string(), "Not on the TV right now".to_string()))
        }
        None => None,
    };
    let detail = match &own {
        Some((name, _)) if mine.is_some() => format!("Replaces {name}"),
        Some(_) => "Replaces your headphones".to_string(),
        None => "AirPods or other Bluetooth headphones, yours alone, to hear the TV on them".to_string(),
    };
    let note = match &audio.pairing {
        Some(pairing) if matches!(pairing.state, PairState::Scanning | PairState::Pairing) => pairing.message.clone(),
        _ => String::new(),
    };
    PairRow {
        own,
        detail,
        button,
        note,
    }
}

#[component]
async fn headphones_settings(cx: &Cx) -> Result<impl View> {
    let me = crate::auth::me(cx).map(|me| me.person.id);
    Ok(live! {
        let mut audio = app_context::<Live>(cx).audio.clone();
        loop {
            let state = audio.borrow_and_update().clone();
            let token = match state {
                Some(audio) => {
                    let row = pair_row(&audio, me);
                    emit! {
                        <section class="settings headphones">
                            <h2>"Headphones"</h2>
                            <div class="setting-group">
                                if let Some((name, state)) = &row.own {
                                    <div class="setting setting-row">
                                        <span class="setting-name">(name.as_str()) <small>(state.as_str())</small></span>
                                        <button type="button" class="pill quiet" data-post="/api/audio/headphones" data-method="DELETE">"Forget"</button>
                                    </div>
                                }
                                <div class="setting setting-row">
                                    <span class="setting-name">"New headphones" <small>(row.detail.as_str())</small></span>
                                    match row.button {
                                        PairButton::Cancel => <button type="button" class="pill quiet" data-post="/api/audio/pair" data-method="DELETE">"Cancel"</button>,
                                        PairButton::Wait => <span class="pill quiet" aria-busy="true">"Pairing…"</span>,
                                        PairButton::Pair => <button type="button" class="pill" data-post="/api/audio/pair">
                                            headphones_icon()
                                            "Pair"
                                        </button>,
                                    }
                                    <p class="answer" aria-live="polite">(row.note.as_str())</p>
                                </div>
                            </div>
                        </section>
                    }?
                }
                None => emit! {}?,
            };
            if !connected(cx) || audio.changed().await.is_err() {
                break Ok(token);
            }
        }
    })
}

pub const DIALOGUE_DETAIL: &str =
    "Voices louder, explosions and music calmer. The green key on the remote switches it on the TV";

#[procedure]
pub async fn set_dialogue(cx: &Cx, on: bool) -> Result<String> {
    Ok(Api::of(cx).set_dialogue(on).await)
}

#[component]
pub async fn dialogue_switch(cx: &Cx, on: bool) -> Result<impl View> {
    let said = signal(cx, String::new);
    Ok(view! {
        <label class="setting setting-row">
            <span class="setting-name">"Dialogue mode" <small>(DIALOGUE_DETAIL)</small></span>
            <input type="checkbox" class="switch" checked=(on)
                @change=$(async |e: Event| {
                    let answer = set_dialogue(e.target.checked).await;
                    said.set(answer);
                })>
            <span class="setting-said" :hidden=$(said.get().is_empty())>$(said.get())</span>
        </label>
    })
}

#[component]
async fn chevron() -> Result<impl View> {
    Ok(view! {
        <svg class="row-chevron" viewBox="0 0 24 24" aria-hidden="true"><path d="m9 6 6 6-6 6"></path></svg>
    })
}

#[component]
async fn download_icon() -> Result<impl View> {
    Ok(view! {
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 4v11M7 10.5l5 5 5-5M5 19.5h14"></path></svg>
    })
}

#[page("/settings")]
async fn settings_page(cx: &Cx) -> Result<impl View> {
    let api = Api::of(cx);
    let subtitles = api.subtitles().await;
    let dialogue = api.dialogue().await;
    let abilities = api.tv_abilities().await.unwrap_or_default();
    let (paired, pair) = pairing(abilities.paired);
    let admin = crate::auth::me(cx).is_some_and(|me| me.person.admin());
    let controls_tv = crate::auth::controls_tv(cx);
    let downloads = crate::auth::downloads(cx);
    let shown = api.tv_profile().await.ok();
    Ok(view! {
        <article class="page">
            back(href: "/")
            <h1>"Settings"</h1>
            <section class="settings">
                <h2>"Subtitles"</h2>
                match subtitles {
                    Ok(subtitles) => {
                        <div class="setting-group">
                            switch(name: "shown", label: "Show when a video starts", detail: "A series keeps what its last episode showed. The yellow key on the remote shows and hides them", on: subtitles.settings.shown)
                            language_order(initial: subtitles.settings.languages.join(","))
                        </div>
                        style(settings: &subtitles.settings)
                    },
                    Err(error) => <p class="setting-note">(error)</p>,
                }
            </section>
            if controls_tv {
            <section class="settings">
                <h2>"TV"</h2>
                <div class="setting-group">
                    <div class="setting">
                        <span class="setting-name">(tv_name(&abilities)) <small>(tv_detail(&abilities))</small></span>
                    </div>
                    if let Some(shown) = shown {
                        <div class="setting setting-row">
                            <span class="setting-name">"Profile on the TV"
                                <small>(if shown.mine { "Yours: the TV's Home shows what you're watching".to_string() } else { format!("{}'s: the TV's Home shows what they're watching", shown.name.unwrap_or_default()) })</small>
                            </span>
                            if !shown.mine {
                                <button type="button" class="pill quiet" data-post="/api/tv-profile" data-done="hide">"Show mine"</button>
                            }
                        </div>
                    }
                    if abilities.pairing && admin {
                        <div class="setting setting-row">
                            <span class="setting-name">"Pairing" <small>(paired)</small></span>
                            <button type="button" class=(if abilities.paired == Some(true) { "pill quiet" } else { "pill" }) data-post="/api/tv/pair" data-pending="Press Allow on the TV">(pair)</button>
                        </div>
                    }
                </div>
            </section>
            }
            <section class="settings">
                <h2>"Sound"</h2>
                match dialogue {
                    Ok(on) => <div class="setting-group">dialogue_switch(on: on)</div>,
                    Err(error) => <p class="setting-note">(error)</p>,
                }
            </section>
            crate::notify::notifications(admin: admin)
            if controls_tv {
                headphones_settings()
            }
            if downloads {
            <section class="settings">
                <h2>"Library"</h2>
                <div class="setting-group">
                    if admin {
                        link(href: "/storage", attrs: attributes! { cx => class="setting row-link" },
                            storage_icon()
                            <span class="setting-name">"Storage" <small>"What the TV keeps offline, and what pCloud holds"</small></span>
                            chevron()
                        )
                    }
                    link(href: "/downloads", attrs: attributes! { cx => class="setting row-link" },
                        download_icon()
                        <span class="setting-name">"Downloads" <small>"Find a film or a season and add it to the library"</small></span>
                        chevron()
                    )
                </div>
            </section>
            }
            if admin {
            <section class="settings">
                <h2>"Usage"</h2>
                <div class="setting-group">
                    link(href: "/usage", attrs: attributes! { cx => class="setting row-link" },
                        usage_icon()
                        <span class="setting-name">"Who watched what" <small>"Hours on the TV and phones, favourites, and when people watch"</small></span>
                        chevron()
                    )
                </div>
            </section>
            }
            if admin {
            <section class="settings">
                <h2>"Feedback"</h2>
                <div class="setting-group">
                    link(href: "/feedback", attrs: attributes! { cx => class="setting row-link" },
                        <svg viewBox="0 0 24 24" aria-hidden="true">
                            <path d="M7 16.5H5.5a2 2 0 0 1-2-2v-8a2 2 0 0 1 2-2h13a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H12l-5 4Z"></path>
                            <path d="M12 7.5v3.6M12 13.9v.1"></path>
                        </svg>
                        <span class="setting-name">"Reports" <small>"What people sent with the speech bubble, newest first"</small></span>
                        chevron()
                    )
                </div>
            </section>
            }
        </article>
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_show_only_your_own_headphones() {
        let audio: Audio = serde_json::from_value(serde_json::json!({
            "output": "tv",
            "headphones": { "name": "AirPods von Frederik", "state": "connected", "owners": [1] },
            "with_headphones": [1, 2],
            "pairing": null,
            "error": null,
        }))
        .unwrap();
        let frederik = pair_row(&audio, Some(1));
        assert_eq!(
            frederik.own,
            Some(("AirPods von Frederik".to_string(), "Connected".to_string()))
        );
        assert_eq!(frederik.detail, "Replaces AirPods von Frederik");
        let mami = pair_row(&audio, Some(2));
        assert_eq!(mami.own.unwrap().0, "My headphones");
        assert_eq!(mami.detail, "Replaces your headphones");
        let miriam = pair_row(&audio, Some(3));
        assert_eq!(miriam.own, None);
        assert!(!miriam.detail.contains("Frederik"));
    }

    #[test]
    fn pairing_says_what_to_do() {
        assert_eq!(pairing(Some(true)).1, "Pair again");
        assert_eq!(pairing(Some(false)).1, "Pair with the TV");
        let samsung = TvAbilities {
            kind: Some("samsung".to_string()),
            volume: true,
            ..TvAbilities::default()
        };
        assert_eq!(tv_name(&samsung), "Samsung TV");
        assert!(tv_detail(&samsung).contains("volume"));
        assert!(tv_name(&TvAbilities::default()).contains("not reachable"));
    }
}
