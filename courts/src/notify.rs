use crate::db::{Db, Subscription};
use crate::push::Push;
use serde::Deserialize;
use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::router::Body;
use topcoat::router::content::Json;
use topcoat::router::response::{IntoResponse, Response};
use topcoat::router::route;

const WORKER: &str = include_str!("sw.js");

/// A fixed file at a fixed address: the service worker, or the web manifest.
pub struct Fixed(&'static str, String);

impl IntoResponse for Fixed {
    fn into_response(self, _cx: &Cx) -> Result<Response> {
        Ok(Response::builder()
            .header("Content-Type", self.0)
            .header("Cache-Control", "no-cache")
            .body(Body::from(self.1))?)
    }
}

#[route(GET "/sw.js")]
async fn service_worker() -> Result<Fixed> {
    Ok(Fixed("text/javascript; charset=utf-8", WORKER.to_string()))
}

#[route(GET "/icon.svg")]
async fn app_icon() -> Result<Fixed> {
    Ok(Fixed("image/svg+xml", include_str!("icon.svg").to_string()))
}

#[route(GET "/manifest.webmanifest")]
async fn web_manifest() -> Result<Fixed> {
    let body = serde_json::json!({
        "name": "Courts",
        "short_name": "Courts",
        "start_url": "/",
        "display": "standalone",
        "background_color": "#10241a",
        "theme_color": "#10241a",
        "icons": [{"src": crate::layout::ICON_URL, "sizes": "any", "type": "image/svg+xml"}],
    });
    Ok(Fixed("application/manifest+json", body.to_string()))
}

#[derive(serde::Serialize)]
struct Answer {
    ok: bool,
    key: String,
    message: String,
}

#[route(GET "/api/push")]
async fn push_key(cx: &Cx) -> Result<Json<Answer>> {
    Ok(Json(match app_context::<Push>(cx).key().await {
        Ok(key) => Answer {
            ok: true,
            key,
            message: String::new(),
        },
        Err(message) => Answer {
            ok: false,
            key: String::new(),
            message,
        },
    }))
}

#[derive(Deserialize)]
struct Keys {
    p256dh: String,
    auth: String,
}

#[derive(Deserialize)]
struct Browser {
    endpoint: String,
    keys: Option<Keys>,
}

#[route(POST "/api/push/subscribe")]
async fn push_subscribe(cx: &Cx, Json(browser): Json<Browser>) -> Result<Json<Answer>> {
    let Some(keys) = browser.keys.filter(|_| browser.endpoint.starts_with("https://")) else {
        return Ok(Json(Answer {
            ok: false,
            key: String::new(),
            message: "That is no push subscription".into(),
        }));
    };
    let me = crate::auth::person(cx)?;
    app_context::<Db>(cx)
        .subscribe(Subscription {
            endpoint: browser.endpoint,
            person_id: Some(me.id),
            p256dh: keys.p256dh,
            auth: keys.auth,
            created_at: crate::time::now(),
        })
        .await
        .map_err(topcoat::Error::msg)?;
    Ok(Json(Answer {
        ok: true,
        key: String::new(),
        message: String::new(),
    }))
}

#[route(POST "/api/push/unsubscribe")]
async fn push_unsubscribe(cx: &Cx, Json(browser): Json<Browser>) -> Result<Json<Answer>> {
    app_context::<Db>(cx)
        .unsubscribe(&browser.endpoint)
        .await
        .map_err(topcoat::Error::msg)?;
    Ok(Json(Answer {
        ok: true,
        key: String::new(),
        message: String::new(),
    }))
}
