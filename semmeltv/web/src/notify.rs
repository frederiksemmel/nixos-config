use topcoat::Result;
use topcoat::context::Cx;
use topcoat::router::Body;
use topcoat::router::response::{IntoResponse, Response};
use topcoat::router::route;
use topcoat::view::{View, component, view};

/// The service worker shows the TV's pushes. It lives at the site's root, at
/// a fixed address, so that it serves every page and stays the same worker
/// from one build to the next.
pub const WORKER: &str = include_str!("sw.js");

pub struct Worker;

impl IntoResponse for Worker {
    fn into_response(self, _cx: &Cx) -> Result<Response> {
        Ok(Response::builder()
            .header("Content-Type", "text/javascript; charset=utf-8")
            .header("Cache-Control", "no-cache")
            .body(Body::from(WORKER.to_string()))?)
    }
}

#[route(GET "/sw.js")]
async fn worker() -> Result<Worker> {
    Ok(Worker)
}

pub fn what(admin: bool) -> &'static str {
    if admin {
        "When the TV plays something for you, when someone you watch a series with carries on with it, and when someone else adds to the library"
    } else {
        "When the TV plays something for you, and when someone you watch a series with carries on with it"
    }
}

#[component]
pub async fn notifications(admin: bool) -> Result<impl View> {
    Ok(view! {
        <section class="settings notifications" data-notify="section">
            <h2>"Notifications"</h2>
            <div class="setting-group">
                <div class="setting setting-row">
                    <span class="setting-name">"On this device" <small data-notify="detail">(what(admin))</small></span>
                    <button type="button" class="pill" data-notify="toggle" hidden=(true)>"Turn on"</button>
                    <p class="answer" data-notify="said" aria-live="polite"></p>
                </div>
            </div>
        </section>
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_worker_shows_a_push_and_opens_its_page_when_tapped() {
        assert!(WORKER.contains(r#"addEventListener("push""#));
        assert!(WORKER.contains("showNotification"));
        assert!(WORKER.contains(r#"addEventListener("notificationclick""#));
        assert!(what(true).contains("adds to the library"));
        assert!(!what(false).contains("library"));
    }
}
