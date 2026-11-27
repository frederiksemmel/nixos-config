use topcoat::Result;
use topcoat::asset::{Asset, asset};
use topcoat::context::Cx;
use topcoat::router::error::NotFoundError;
use topcoat::router::{Slot, StatusCode, layout, not_found};
use topcoat::view::{View, error_boundary, view};

pub const STYLE: Asset = asset!("./style.css");
const NOTIFY: Asset = asset!("./notify.js");
const DAY: Asset = asset!("./day.js");
const FEEDBACK: Asset = asset!("../../feedback/client/feedback-mode.js");
const FEEDBACK_STYLE: Asset = asset!("../../feedback/client/feedback-mode.css");
pub const ICON_URL: &str = "/icon.svg";

#[layout("/")]
async fn root(cx: &Cx, slot: Slot<'_>) -> Result<impl View> {
    let signed_in = crate::auth::signed_in(cx);
    Ok(view! {
        <!DOCTYPE html>
        <html lang="en" data-feedback-project=(if signed_in { "courts" } else { "" }) data-feedback-send="/api/feedback" data-feedback-list="/feedback">
            <head>
                <meta charset="utf-8">
                <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
                <meta name="theme-color" content="#10241a">
                <meta name="color-scheme" content="dark">
                <meta name="apple-mobile-web-app-capable" content="yes">
                <meta name="apple-mobile-web-app-title" content="Courts">
                <title>"Courts"</title>
                <link rel="icon" href=(ICON_URL) type="image/svg+xml">
                <link rel="manifest" href="/manifest.webmanifest">
                <link rel="stylesheet" href=(STYLE)>
                <script type="module" src=(NOTIFY)></script>
                if signed_in {
                    <script type="module" src=(DAY)></script>
                    <link rel="stylesheet" href=(FEEDBACK_STYLE)>
                    <script type="module" src=(FEEDBACK)></script>
                }
            </head>
            <body>
                <main>
                    error_boundary(
                        fallback: |error| {
                            let missing = error.downcast_ref::<NotFoundError>().is_some();
                            Ok(view! {
                                if missing {
                                    (StatusCode::NOT_FOUND)
                                    <h1>"Not here"</h1>
                                } else {
                                    (StatusCode::INTERNAL_SERVER_ERROR)
                                    <h1>"Something broke"</h1>
                                    <p>(error.to_string())</p>
                                }
                                <p><a href="/">"Back to the courts"</a></p>
                            })
                        },
                        (slot)
                    )
                </main>
            </body>
        </html>
    })
}

not_found!("/");
