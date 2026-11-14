use crate::api::{Api, Audio, Link, PairState, TvAbilities};
use crate::layout::back;
use crate::live::Live;
use crate::playing::headphones_icon;
use crate::storage::storage_icon;
use crate::subtitles::{language_order, style, switch};
use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::router::page;
use topcoat::runtime::{connected, link};
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
    detail: String,
    button: PairButton,
    note: String,
}

fn pair_row(audio: &Audio) -> PairRow {
    let button = match audio.pairing.as_ref().map(|pairing| pairing.state) {
        Some(PairState::Scanning) => PairButton::Cancel,
        Some(PairState::Pairing) => PairButton::Wait,
        _ => PairButton::Pair,
    };
    let detail = match &audio.headphones {
        Some(headphones) => format!("Replaces {}", headphones.name),
        None => "AirPods or other Bluetooth headphones, to hear the TV on them".to_string(),
    };
    let note = match &audio.pairing {
        Some(pairing) if matches!(pairing.state, PairState::Scanning | PairState::Pairing) => pairing.message.clone(),
        _ => String::new(),
    };
    PairRow { detail, button, note }
}

#[component]
async fn headphones_settings(cx: &Cx) -> Result<impl View> {
    Ok(live! {
        let mut audio = app_context::<Live>(cx).audio.clone();
        loop {
            let state = audio.borrow_and_update().clone();
            let token = match state {
                Some(audio) => {
                    let row = pair_row(&audio);
                    emit! {
                        <section class="settings headphones">
                            <h2>"Headphones"</h2>
                            <div class="setting-group">
                                if let Some(headphones) = &audio.headphones {
                                    <div class="setting setting-row">
                                        <span class="setting-name">(headphones.name.as_str()) <small>(link_text(headphones.state))</small></span>
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
    let abilities = api.tv_abilities().await.unwrap_or_default();
    let (paired, pair) = pairing(abilities.paired);
    let admin = crate::auth::me(cx).is_some_and(|me| me.person.admin());
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
                            switch(name: "shown", label: "Show when a video starts", detail: "The yellow key on the remote shows and hides them", on: subtitles.settings.shown)
                            language_order(initial: subtitles.settings.languages.join(","))
                        </div>
                        style(settings: &subtitles.settings)
                    },
                    Err(error) => <p class="setting-note">(error)</p>,
                }
            </section>
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
            headphones_settings()
            if admin {
            <section class="settings">
                <h2>"Library"</h2>
                <div class="setting-group">
                    link(href: "/storage", attrs: attributes! { cx => class="setting row-link" },
                        storage_icon()
                        <span class="setting-name">"Storage" <small>"What the TV keeps offline, and the room left"</small></span>
                        chevron()
                    )
                    link(href: "/downloads", attrs: attributes! { cx => class="setting row-link" },
                        download_icon()
                        <span class="setting-name">"Downloads" <small>"Find a film or a season and add it to the library"</small></span>
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
