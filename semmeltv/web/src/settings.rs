use crate::api::{Api, TvAbilities};
use crate::layout::back;
use crate::storage::storage_icon;
use crate::subtitles::{language_order, style, switch};
use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::router::page;
use topcoat::runtime::link;
use topcoat::view::{View, attributes, component, view};

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
    let api = app_context::<Api>(cx);
    let subtitles = api.subtitles().await;
    let abilities = api.tv_abilities().await.unwrap_or_default();
    let (paired, pair) = pairing(abilities.paired);
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
                    if abilities.pairing {
                        <div class="setting setting-row">
                            <span class="setting-name">"Pairing" <small>(paired)</small></span>
                            <button type="button" class="pill" data-post="/api/tv/pair" data-pending="Press Allow on the TV">(pair)</button>
                            <p class="answer" data-said="true" aria-live="polite"></p>
                        </div>
                    }
                </div>
            </section>
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
