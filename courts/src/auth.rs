//! One person uses this, with a key from a file: the sign-in page takes it
//! once and the browser keeps it in a cookie.

use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::cookie::{Cookies, SameSite, cookie, cookies, time::Duration};
use topcoat::router::content::Form;
use topcoat::router::error::{SeeOther, redirect, see_other};
use topcoat::router::request::{headers, uri};
use topcoat::router::{Body, Layer, LayerFuture, Next, Path, page, path_param, query_params, route};
use topcoat::view::{View, view};

const COOKIE: &str = "courts_key";
const OPEN: [&str; 9] = [
    "/sign-in",
    "/sign-in/key",
    "/sign-in/link",
    "/invite/",
    "/s/",
    "/sw.js",
    "/icon.svg",
    "/manifest.webmanifest",
    "/_topcoat/",
];

#[derive(Clone)]
pub struct Key(pub String);

/// Where the site is on the web, for the links it makes.
#[derive(Clone)]
pub struct Origin(pub String);

const INVITE_LIFE: i64 = 24 * 60 * 60;

fn hash(token: &str) -> String {
    ring::digest::digest(&ring::digest::SHA256, token.as_bytes())
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Makes a one-time sign-in link, good for a day.
pub async fn invite(cx: &Cx) -> Result<(String, String)> {
    use base64::Engine;
    use ring::rand::SecureRandom;
    let mut bytes = [0u8; 24];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| topcoat::Error::msg("no randomness for a link"))?;
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let now = crate::time::now();
    app_context::<crate::db::Db>(cx)
        .add_invite(&hash(&token), now, now + INVITE_LIFE)
        .await
        .map_err(topcoat::Error::msg)?;
    let url = format!("{}/invite/{token}", app_context::<Origin>(cx).0.trim_end_matches('/'));
    Ok((token, url))
}

path_param!(token);

#[route(GET "/invite/{token}")]
async fn redeem_invite(cx: &Cx) -> Result<SeeOther> {
    let token = path_param::<Token>(cx).to_string();
    let used = app_context::<crate::db::Db>(cx)
        .redeem(&hash(&token), crate::time::now())
        .await
        .map_err(topcoat::Error::msg)?;
    if used {
        keep(cx, &app_context::<Key>(cx).0);
        Ok(see_other("/"))
    } else {
        Ok(see_other("/sign-in?invite=spent"))
    }
}

#[route(POST "/invites")]
async fn make_invite(cx: &Cx) -> Result<SeeOther> {
    let (token, _) = invite(cx).await?;
    Ok(see_other(format!("/watches?invite={token}#devices")))
}

#[route(POST "/api/invites")]
async fn api_invite(cx: &Cx) -> Result<topcoat::router::content::Json<serde_json::Value>> {
    let (_, url) = invite(cx).await?;
    Ok(topcoat::router::content::Json(
        serde_json::json!({"ok": true, "url": url, "hours": INVITE_LIFE / 3600}),
    ))
}

/// The QR code of a sign-in link, for a phone's camera.
pub struct Svg(String);

impl topcoat::router::response::IntoResponse for Svg {
    fn into_response(self, _cx: &Cx) -> Result<topcoat::router::response::Response> {
        Ok(topcoat::router::response::Response::builder()
            .header("Content-Type", "image/svg+xml")
            .header("Cache-Control", "no-store")
            .body(topcoat::router::Body::from(self.0))?)
    }
}

#[route(GET "/invites/{token}/qr.svg")]
async fn invite_qr(cx: &Cx) -> Result<Svg> {
    let token = path_param::<Token>(cx).to_string();
    let url = format!("{}/invite/{token}", app_context::<Origin>(cx).0.trim_end_matches('/'));
    let code = qrcode::QrCode::new(url.as_bytes()).map_err(|error| topcoat::Error::msg(error.to_string()))?;
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(240, 240)
        .dark_color(qrcode::render::svg::Color("#0d1a12"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build();
    Ok(Svg(svg))
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Whether the request carries the key as `Authorization: Bearer KEY`, as
/// the feedback sessions on beast send it.
pub fn bearer(cx: &Cx) -> bool {
    let key = app_context::<Key>(cx);
    !key.0.is_empty()
        && headers(cx)
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .is_some_and(|given| same(given.trim(), &key.0))
}

pub fn signed_in(cx: &Cx) -> bool {
    let key = app_context::<Key>(cx);
    !key.0.is_empty()
        && headers(cx)
            .get_all("cookie")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(';'))
            .filter_map(|pair| pair.trim().split_once('='))
            .any(|(name, value)| name == COOKIE && same(value, &key.0))
}

fn open(path: &str) -> bool {
    OPEN.iter().any(|open| {
        if open.ends_with('/') {
            path.starts_with(open)
        } else {
            path == *open
        }
    })
}

pub struct SignedIn;

impl Layer for SignedIn {
    fn path(&self) -> Option<&Path> {
        None
    }

    fn handle<'a>(&'a self, cx: &'a Cx, body: Body, next: Next<'a>) -> LayerFuture<'a> {
        Box::pin(async move {
            let path = uri(cx).path().to_string();
            if open(&path) || signed_in(cx) || bearer(cx) {
                next.run(cx, body).await
            } else {
                Err(redirect("/sign-in").into())
            }
        })
    }
}

fn keep(cx: &Cx, key: &str) {
    cookies(cx).add(cookie! {
        COOKIE = key.to_string();
        Path = "/";
        Secure;
        HttpOnly;
        SameSite = SameSite::Lax;
        MaxAge = Duration::days(400)
    });
}

#[query_params]
struct SignInQuery {
    wrong: Option<String>,
    invite: Option<String>,
}

#[query_params]
struct LinkQuery {
    key: Option<String>,
}

#[route(GET "/sign-in/link")]
async fn sign_in_link(cx: &Cx) -> Result<SeeOther> {
    let given = query_params::<LinkQuery>(cx)
        .map_err(|error| topcoat::Error::msg(error.to_string()))?
        .key
        .clone()
        .unwrap_or_default();
    Ok(check(cx, &given))
}

fn check(cx: &Cx, given: &str) -> SeeOther {
    let key = app_context::<Key>(cx);
    if !key.0.is_empty() && same(given.trim(), &key.0) {
        keep(cx, given.trim());
        see_other("/")
    } else {
        see_other("/sign-in?wrong=1")
    }
}

#[page("/sign-in")]
async fn sign_in(cx: &Cx) -> Result<impl View> {
    let query = query_params::<SignInQuery>(cx).map_err(|error| topcoat::Error::msg(error.to_string()))?;
    let wrong = query.wrong.is_some();
    let spent = query.invite.is_some();
    Ok(view! {
        <article class="sign-in">
            <h1>"Courts"</h1>
            <p>"Free tennis, padel and squash courts in Zurich, from GotCourts. Open a sign-in link on this device, or enter the key."</p>
            <form method="post" action="/sign-in/key">
                <label>"Key" <input type="password" name="key" autocomplete="current-password" autofocus=(true)></label>
                <button class="pill" type="submit">"Sign in"</button>
            </form>
            if wrong {
                <p class="wrong">"That key is not the one."</p>
            }
            if spent {
                <p class="wrong">"That sign-in link was used already or is older than a day. Make a new one on a device that is signed in (Watches, Other devices), or ask Claude for one."</p>
            }
        </article>
    })
}

#[derive(serde::Deserialize)]
struct Given {
    key: String,
}

#[route(POST "/sign-in/key")]
async fn sign_in_key(cx: &Cx, Form(given): Form<Given>) -> Result<SeeOther> {
    Ok(check(cx, &given.key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_sign_in_and_what_it_needs_are_open() {
        assert!(open("/sign-in"));
        assert!(open("/_topcoat/assets/style.css"));
        assert!(open("/sw.js"));
        assert!(open("/sign-in/key"));
        assert!(open("/invite/abc"));
        assert!(open("/s/20-111-20261129-1800-60-abc"));
        assert!(!open("/s"));
        assert!(!open("/invites"));
        assert!(!open("/"));
        assert!(!open("/watches"));
        assert!(!open("/sign-in/elsewhere"));
        assert!(same("abc", "abc"));
        assert!(!same("abc", "abd"));
        assert!(!same("abc", "abcd"));
    }
}
