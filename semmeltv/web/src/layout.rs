use crate::auth;
use crate::brand;
use crate::playing;
use crate::public::Public;
use topcoat::Result;
use topcoat::asset::{Asset, asset};
use topcoat::context::{Cx, app_context};
use topcoat::router::error::NotFoundError;
use topcoat::router::{Slot, StatusCode, layout, not_found};
use topcoat::runtime::link;
use topcoat::view::{View, attributes, component, error_boundary, view};

const STYLE: Asset = asset!("./style.css");
const MICROPHONE: Asset = asset!("./microphone.js");
const APP: Asset = asset!("./app.js");
const PHONE: Asset = asset!("./phone.js");
const TOAST: Asset = asset!("./toast.js");
const REPORT: Asset = asset!("./report.js");
const ACT: Asset = asset!("./act.js");
const PHONE_STYLE: Asset = asset!("./phone.css");
const AUTH: Asset = asset!("./auth.js");

#[layout("/")]
async fn root(cx: &Cx, slot: Slot<'_>) -> Result<impl View> {
    let public = app_context::<Public>(cx);
    let signed_in = auth::me(cx).is_some();
    Ok(view! {
        <!DOCTYPE html>
        <html lang="en" data-api=(public.api.as_str())>
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1, maximum-scale=1, user-scalable=no, viewport-fit=cover">
                <meta name="theme-color" content=(brand::THEME)>
                <meta name="color-scheme" content="dark">
                <meta name="apple-mobile-web-app-capable" content="yes">
                <meta name="apple-mobile-web-app-status-bar-style" content="black-translucent">
                <meta name="apple-mobile-web-app-title" content="SemmelTV">
                <title>"SemmelTV"</title>
                <link rel="icon" href=(brand::FAVICON) type="image/svg+xml">
                <link rel="icon" href=(brand::FAVICON_PNG) type="image/png" sizes="32x32">
                <link rel="apple-touch-icon" href=(brand::TOUCH_ICON)>
                <link rel="manifest" href="/manifest.webmanifest">
                <link rel="stylesheet" href=(STYLE)>
                <link rel="stylesheet" href=(PHONE_STYLE)>
                <script src=(REPORT)></script>
                <script src=(ACT)></script>
                topcoat::runtime::script()
                <script type="module" src=(TOAST)></script>
                <script type="module" src=(AUTH)></script>
                if signed_in {
                    <script type="module" src=(MICROPHONE)></script>
                    <script type="module" src=(APP)></script>
                    <script type="module" src=(PHONE)></script>
                }
            </head>
            <body>
                if !signed_in {
                    <header class="top">
                        <span class="brand"><img src=(brand::WORDMARK) alt="SemmelTV"></span>
                    </header>
                    <main>(slot)</main>
                } else {
                <header class="top">
                    link(href: "/", attrs: attributes! { cx => class="brand" }, <img src=(brand::WORDMARK) alt="SemmelTV">)
                    playing::mode_switch()
                    <span class="mode-note" id="mode-note" role="status"></span>
                    <div class="microphone" id="microphone" data-state="idle">
                        <button class="microphone-talk" id="microphone-talk" type="button" aria-label="Talk to the TV" aria-pressed="false">
                            <svg viewBox="0 0 24 24" aria-hidden="true">
                                <path d="M12 15a3.5 3.5 0 0 0 3.5-3.5v-6a3.5 3.5 0 0 0-7 0v6A3.5 3.5 0 0 0 12 15Z"></path>
                                <path d="M18.5 11.5a6.5 6.5 0 0 1-13 0M12 18v3.5" fill="none"></path>
                            </svg>
                            <i class="microphone-level" id="microphone-level"></i>
                        </button>
                    </div>
                    link(href: "/settings", attrs: attributes! { cx => class="top-link" aria-label="Settings" }, playing::settings_icon())
                    auth::profile_button()
                </header>
                playing::voice_feedback()
                playing::notices()
                <main>
                    error_boundary(
                        fallback: |error| {
                            let missing = error.downcast_ref::<NotFoundError>().is_some();
                            Ok(view! {
                                if missing {
                                    (StatusCode::NOT_FOUND)
                                    trouble(title: "Not in the library", note: "Nothing here, or it was taken out of the library.")
                                } else {
                                    (StatusCode::SERVICE_UNAVAILABLE)
                                    trouble(title: "SemmelTV isn't answering", note: "The TV app didn't answer this time. It may be restarting.")
                                }
                            })
                        },
                        (slot)
                    )
                </main>
                playing::player()
                }
            </body>
        </html>
    })
}

not_found!("/");

#[component]
async fn trouble(cx: &Cx, title: &'static str, note: &'static str) -> Result<impl View> {
    Ok(view! {
        <article class="page trouble">
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
