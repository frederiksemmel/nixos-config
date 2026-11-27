use crate::auth;
use crate::brand;
use crate::playing;
use crate::public::Public;
use crate::version;
use topcoat::Result;
use topcoat::asset::{Asset, asset};
use topcoat::context::{Cx, app_context};
use topcoat::router::error::NotFoundError;
use topcoat::router::{Slot, StatusCode, layout, not_found};
use topcoat::runtime::link;
use topcoat::view::{View, attributes, component, error_boundary, view};

const STYLE: Asset = asset!("./style.css");
const APP: Asset = asset!("./app.js");
const PHONE: Asset = asset!("./phone.js");
const TOAST: Asset = asset!("./toast.js");
const REPORT: Asset = asset!("./report.js");
const MAINTENANCE: Asset = asset!("./maintenance.js");
const ACT: Asset = asset!("./act.js");
const PHONE_STYLE: Asset = asset!("./phone.css");
const AUTH: Asset = asset!("./auth.js");
const LIVE: Asset = asset!("./live.js");
const FEEDBACK: Asset = asset!("./feedback.js");
const SHARE: Asset = asset!("./share.js");
const DRAFTS: Asset = asset!("./drafts.js");
const NOTIFY: Asset = asset!("./notify.js");
const FEEDBACK_MODE: Asset = asset!("../../../feedback/client/feedback-mode.js");
const FEEDBACK_MODE_STYLE: Asset = asset!("../../../feedback/client/feedback-mode.css");

#[layout("/")]
async fn root(cx: &Cx, slot: Slot<'_>) -> Result<impl View> {
    let public = app_context::<Public>(cx);
    let signed_in = auth::me(cx).is_some();
    let controls_tv = auth::controls_tv(cx);
    Ok(view! {
        <!DOCTYPE html>
        <html lang="en" data-api=(public.api.as_str()) data-build=(version::id()) data-controls-tv=(controls_tv.to_string())
            data-feedback-project="semmeltv" data-feedback-launcher="off" data-feedback-list=(if auth::me(cx).is_some_and(|me| me.person.admin()) { "/feedback" } else { "" })>
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
                <meta name="description" content=(brand::DESCRIPTION)>
                <meta name="theme-color" content=(brand::THEME)>
                <meta name="color-scheme" content="dark">
                <meta name="mobile-web-app-capable" content="yes">
                <meta name="apple-mobile-web-app-capable" content="yes">
                <meta name="apple-mobile-web-app-title" content="SemmelTV">
                <title>"SemmelTV"</title>
                <link rel="icon" href=(brand::FAVICON) type="image/svg+xml">
                <link rel="icon" href=(brand::FAVICON_PNG) type="image/png" sizes="32x32">
                <link rel="apple-touch-icon" href=(brand::TOUCH_ICON)>
                <link rel="manifest" href="/manifest.webmanifest">
                <link rel="stylesheet" href=(STYLE)>
                <link rel="stylesheet" href=(PHONE_STYLE)>
                <script src=(REPORT)></script>
                <script src=(MAINTENANCE)></script>
                <script src=(ACT)></script>
                if signed_in {
                    <script src=(LIVE)></script>
                    <script src=(DRAFTS)></script>
                }
                topcoat::runtime::script()
                <script type="module" src=(TOAST)></script>
                <script type="module" src=(AUTH)></script>
                if signed_in {
                    <script type="module" src=(APP)></script>
                    <script type="module" src=(PHONE)></script>
                    <script type="module" src=(FEEDBACK)></script>
                    <link rel="stylesheet" href=(FEEDBACK_MODE_STYLE)>
                    <script type="module" src=(FEEDBACK_MODE)></script>
                    <script type="module" src=(SHARE)></script>
                    <script type="module" src=(NOTIFY)></script>
                }
            </head>
            <body>
                if !signed_in {
                    <header class="top">
                        <span class="brand"><img src=(brand::WORDMARK) width="296" height="132" alt="SemmelTV"></span>
                    </header>
                    <main>(slot)</main>
                } else {
                <header class="top">
                    link(href: "/", attrs: attributes! { cx => class="brand" }, <img src=(brand::WORDMARK) width="296" height="132" alt="SemmelTV">)
                    if controls_tv {
                        playing::mode_switch()
                        <span class="mode-note" id="mode-note" role="status"></span>
                    }
                    <button class="top-link feedback-open" id="feedback-open" type="button" aria-label="Feedback: mark something on the page and comment on it" data-feedback-open="">
                        <svg viewBox="0 0 24 24" aria-hidden="true">
                            <path d="M7 16.5H5.5a2 2 0 0 1-2-2v-8a2 2 0 0 1 2-2h13a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H12l-5 4Z"></path>
                            <path d="M12 7.5v3.6M12 13.9v.1"></path>
                        </svg>
                    </button>
                    link(href: "/settings", attrs: attributes! { cx => class="top-link" aria-label="Settings" }, playing::settings_icon())
                    auth::profile_button()
                </header>
                playing::notices()
                <main>
                    error_boundary(
                        fallback: |error| {
                            let missing = error.downcast_ref::<NotFoundError>().is_some();
                            Ok(view! {
                                if missing {
                                    (StatusCode::NOT_FOUND)
                                    trouble(title: "Nicht in der Auslage", note: "Das gibt es hier nicht, oder es wurde aus dem Sortiment genommen.", burnt: false)
                                } else {
                                    (StatusCode::SERVICE_UNAVAILABLE)
                                    trouble(title: "Wir haben es versemmelt", note: "Wir kriegen es gerade nicht gebacken: Die TV-App antwortet nicht. Vielleicht wird der Ofen neu angeheizt, versuch es gleich nochmal.", burnt: true)
                                }
                            })
                        },
                        (slot)
                    )
                </main>
                if controls_tv {
                    playing::player()
                } else {
                    <script type="module" src=(playing::PLAYER)></script>
                }
                playing::pulse()
                }
            </body>
        </html>
    })
}

not_found!("/");

#[component]
async fn trouble(cx: &Cx, title: &'static str, note: &'static str, burnt: bool) -> Result<impl View> {
    Ok(view! {
        <article class="page trouble">
            <img class="trouble-roll" src=(brand::FAVICON) width="96" height="96" alt="" data-burnt=(burnt.to_string())>
            <h1>(title)</h1>
            <p>(note)</p>
            link(href: "/", attrs: attributes! { cx => class="pill" }, "Home")
        </article>
    })
}

#[component]
pub async fn back(cx: &Cx, href: &'static str) -> Result<impl View> {
    Ok(view! {
        link(href: href, attrs: attributes! { cx => class="back" },
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m14.5 5.5-6.5 6.5 6.5 6.5"></path></svg>
            "Back"
        )
    })
}
