use crate::playing;
use crate::public::Public;
use topcoat::Result;
use topcoat::asset::{Asset, asset};
use topcoat::context::{Cx, app_context};
use topcoat::router::{Slot, layout};
use topcoat::runtime::link;
use topcoat::view::{View, attributes, view};

const STYLE: Asset = asset!("./style.css");
const MICROPHONE: Asset = asset!("./microphone.js");

#[layout("/")]
async fn root(cx: &Cx, slot: Slot<'_>) -> Result<impl View> {
    let public = app_context::<Public>(cx);
    Ok(view! {
        <!DOCTYPE html>
        <html lang="en" data-api=(public.api.as_str())>
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
                <meta name="theme-color" content="#0c0c10">
                <meta name="color-scheme" content="dark">
                <meta name="apple-mobile-web-app-capable" content="yes">
                <meta name="apple-mobile-web-app-title" content="SemmelTV">
                <meta name="apple-mobile-web-app-status-bar-style" content="black-translucent">
                <title>"SemmelTV"</title>
                <link rel="stylesheet" href=(STYLE)>
                topcoat::runtime::script()
                <script type="module" src=(MICROPHONE)></script>
            </head>
            <body>
                <header class="top">
                    link(href: "/", attrs: attributes! { cx => class="brand" }, "Semmel" <b>"TV"</b>)
                    <div class="microphone" id="microphone" data-state="off">
                        <span class="microphone-note" id="microphone-note"></span>
                        <button class="microphone-off" id="microphone-off" type="button" aria-label="Turn the microphone off" hidden=(true)>
                            "Mic off"
                        </button>
                        <button class="microphone-talk" id="microphone-talk" type="button" aria-label="Talk to the TV">
                            <svg viewBox="0 0 24 24" aria-hidden="true">
                                <path d="M12 15a3.5 3.5 0 0 0 3.5-3.5v-6a3.5 3.5 0 0 0-7 0v6A3.5 3.5 0 0 0 12 15Z"></path>
                                <path d="M18.5 11.5a6.5 6.5 0 0 1-13 0M12 18v3.5" fill="none"></path>
                            </svg>
                            <i class="microphone-level" id="microphone-level"></i>
                        </button>
                    </div>
                </header>
                playing::voice_feedback()
                <main>(slot)</main>
                playing::now_playing()
            </body>
        </html>
    })
}
