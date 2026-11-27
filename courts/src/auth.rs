//! Who is signed in. Each browser signs in as a person, through a one-time
//! link an admin makes for them (or for themselves on another device), or
//! with the key from the key file, which signs in as the first admin. The
//! browser then keeps a device token in a cookie.

use crate::db::{Db, Person};
use topcoat::Result;
use topcoat::context::{Cx, app_context, try_request_context};
use topcoat::cookie::{Cookies, SameSite, cookie, cookies, time::Duration};
use topcoat::router::content::Form;
use topcoat::router::error::{SeeOther, not_found, redirect, see_other};
use topcoat::router::request::{headers, uri};
use topcoat::router::{Body, Layer, LayerFuture, Next, Path, page, path_param, query_params, route};
use topcoat::view::{View, view};

/// The key itself, as browsers signed in before there were people keep it.
const KEY_COOKIE: &str = "courts_key";
const DEVICE_COOKIE: &str = "courts_device";
const OPEN: [&str; 10] = [
    "/sign-in",
    "/sign-in/key",
    "/sign-in/link",
    "/invite/",
    "/s/",
    "/a/",
    "/sw.js",
    "/icon.svg",
    "/manifest.webmanifest",
    "/_topcoat/",
];
const ADMINS_ONLY: [&str; 4] = ["/users", "/feedback", "/api/feedback", "/api/invites"];

/// Whom the first admin is called when Courts starts on an empty database.
pub const FIRST_ADMIN: &str = "Frederik";

#[derive(Clone)]
pub struct Key(pub String);

/// Where the site is on the web, for the links it makes.
#[derive(Clone)]
pub struct Origin(pub String);

/// The person this request comes from, if anybody signed in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Me {
    pub id: i64,
    pub name: String,
    pub admin: bool,
}

impl From<&Person> for Me {
    fn from(person: &Person) -> Self {
        Me {
            id: person.id,
            name: person.name.clone(),
            admin: person.admin,
        }
    }
}

#[derive(Clone)]
struct Who(Option<Me>);

pub fn me(cx: &Cx) -> Option<&Me> {
    try_request_context::<Who>(cx).and_then(|who| who.0.as_ref())
}

pub fn signed_in(cx: &Cx) -> bool {
    me(cx).is_some()
}

pub fn admin(cx: &Cx) -> bool {
    me(cx).is_some_and(|me| me.admin)
}

/// The signed-in person; pages behind the sign-in always have one.
pub fn person(cx: &Cx) -> Result<Me> {
    me(cx).cloned().ok_or_else(|| redirect("/sign-in").into())
}

/// A week: invites are sent over chat, and read a day or two later.
const INVITE_LIFE: i64 = 7 * 24 * 60 * 60;

pub fn hash(token: &str) -> String {
    ring::digest::digest(&ring::digest::SHA256, token.as_bytes())
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 24 random bytes, URL-safe.
pub fn token() -> Result<String> {
    use base64::Engine;
    use ring::rand::SecureRandom;
    let mut bytes = [0u8; 24];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| topcoat::Error::msg("no randomness for a token"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

pub fn origin(cx: &Cx) -> String {
    app_context::<Origin>(cx).0.trim_end_matches('/').to_string()
}

/// Makes a one-time sign-in link for `person`, good for a week.
pub async fn invite(cx: &Cx, person: i64) -> Result<(String, String)> {
    let token = token()?;
    let now = crate::time::now();
    app_context::<Db>(cx)
        .add_invite(&hash(&token), person, now, now + INVITE_LIFE)
        .await
        .map_err(topcoat::Error::msg)?;
    let url = format!("{}/invite/{token}", origin(cx));
    Ok((token, url))
}

/// The admin the key signs in as: the first one.
async fn key_holder(db: &Db) -> Result<Person> {
    let people = db.people().await.map_err(topcoat::Error::msg)?;
    match people.into_iter().filter(|p| p.admin).min_by_key(|p| p.id) {
        Some(admin) => Ok(admin),
        None => db
            .first_admin(FIRST_ADMIN, crate::time::now())
            .await
            .map_err(topcoat::Error::msg),
    }
}

/// "iPhone", "Mac", …: what kind of device a user agent is.
pub fn kind(agent: &str) -> &'static str {
    [
        ("iPhone", "iPhone"),
        ("iPad", "iPad"),
        ("Android", "Android phone"),
        ("Macintosh", "Mac"),
        ("Windows", "Windows computer"),
        ("CrOS", "Chromebook"),
        ("Linux", "Linux computer"),
    ]
    .into_iter()
    .find(|(mark, _)| agent.contains(mark))
    .map_or("Browser", |(_, name)| name)
}

/// Signs this browser in as `person`, with a new device token.
async fn sign_in_as(cx: &Cx, person: i64) -> Result<()> {
    let token = token()?;
    let agent = headers(cx)
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    app_context::<Db>(cx)
        .add_device(&hash(&token), person, kind(agent), crate::time::now())
        .await
        .map_err(topcoat::Error::msg)?;
    cookies(cx).add(cookie! {
        DEVICE_COOKIE = token;
        Path = "/";
        Secure;
        HttpOnly;
        SameSite = SameSite::Lax;
        MaxAge = Duration::days(400)
    });
    cookies(cx).remove(cookie! { KEY_COOKIE = ""; Path = "/" });
    Ok(())
}

path_param!(token);

#[route(GET "/invite/{token}")]
async fn redeem_invite(cx: &Cx) -> Result<SeeOther> {
    let token = path_param::<Token>(cx).to_string();
    let db = app_context::<Db>(cx).clone();
    let Some(person) = db
        .redeem(&hash(&token), crate::time::now())
        .await
        .map_err(topcoat::Error::msg)?
    else {
        return Ok(see_other("/sign-in?invite=spent"));
    };
    if db.person(person).await.map_err(topcoat::Error::msg)?.is_none() {
        return Ok(see_other("/sign-in?invite=spent"));
    }
    sign_in_as(cx, person).await?;
    Ok(see_other("/"))
}

#[route(POST "/invites")]
async fn make_invite(cx: &Cx) -> Result<SeeOther> {
    let me = person(cx)?;
    let (token, _) = invite(cx, me.id).await?;
    Ok(see_other(format!("/watches?invite={token}#devices")))
}

#[route(POST "/api/invites")]
async fn api_invite(cx: &Cx) -> Result<topcoat::router::content::Json<serde_json::Value>> {
    let me = person(cx)?;
    let (_, url) = invite(cx, me.id).await?;
    Ok(topcoat::router::content::Json(
        serde_json::json!({"ok": true, "url": url, "hours": INVITE_LIFE / 3600}),
    ))
}

#[route(POST "/sign-out")]
async fn sign_out(cx: &Cx) -> Result<SeeOther> {
    if let Some(token) = cookie_value(cx, DEVICE_COOKIE) {
        app_context::<Db>(cx)
            .forget_device(&hash(&token))
            .await
            .map_err(topcoat::Error::msg)?;
    }
    cookies(cx).remove(cookie! { DEVICE_COOKIE = ""; Path = "/" });
    cookies(cx).remove(cookie! { KEY_COOKIE = ""; Path = "/" });
    Ok(see_other("/sign-in"))
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
    let url = format!("{}/invite/{token}", origin(cx));
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

fn cookie_value(cx: &Cx, name: &str) -> Option<String> {
    headers(cx)
        .get_all("cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(given, _)| *given == name)
        .map(|(_, value)| value.to_string())
}

/// Whom the request comes from: the key (as a bearer token or the old
/// cookie) is the first admin; a device token is whoever it signed in.
async fn who(cx: &Cx) -> Result<Option<Me>> {
    let db = app_context::<Db>(cx).clone();
    let key = &app_context::<Key>(cx).0;
    let old_cookie = cookie_value(cx, KEY_COOKIE).is_some_and(|given| !key.is_empty() && same(&given, key));
    if bearer(cx) {
        return Ok(Some(Me::from(&key_holder(&db).await?)));
    }
    if old_cookie {
        let holder = key_holder(&db).await?;
        sign_in_as(cx, holder.id).await?;
        return Ok(Some(Me::from(&holder)));
    }
    let Some(token) = cookie_value(cx, DEVICE_COOKIE) else {
        return Ok(None);
    };
    let Some(device) = db
        .device(&hash(&token), crate::time::now())
        .await
        .map_err(topcoat::Error::msg)?
    else {
        return Ok(None);
    };
    let person = db.person(device.person_id).await.map_err(topcoat::Error::msg)?;
    Ok(person.as_ref().map(Me::from))
}

fn listed(list: &[&str], path: &str) -> bool {
    list.iter().any(|open| {
        if open.ends_with('/') {
            path.starts_with(open)
        } else {
            path == *open || path.starts_with(&format!("{open}/"))
        }
    })
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
        Some(Path::new("/"))
    }

    fn handle<'a>(&'a self, cx: &'a Cx, body: Body, next: Next<'a>) -> LayerFuture<'a> {
        Box::pin(async move {
            let path = uri(cx).path().to_string();
            if path.starts_with("/_topcoat/") {
                return next.run(cx, body).await;
            }
            let found = who(cx).await?;
            if found.is_none() && !open(&path) {
                return Err(redirect("/sign-in").into());
            }
            if listed(&ADMINS_ONLY, &path) && !found.as_ref().is_some_and(|me| me.admin) {
                return Err(not_found().into());
            }
            let cx = cx.with(Who(found));
            next.run(&cx, body).await
        })
    }
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
    check(cx, &given).await
}

async fn check(cx: &Cx, given: &str) -> Result<SeeOther> {
    let key = app_context::<Key>(cx).0.clone();
    if !key.is_empty() && same(given.trim(), &key) {
        let holder = key_holder(app_context::<Db>(cx)).await?;
        sign_in_as(cx, holder.id).await?;
        Ok(see_other("/"))
    } else {
        Ok(see_other("/sign-in?wrong=1"))
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
            <p>"Free tennis, padel and squash courts in Zurich, from GotCourts and Eversports. Open the sign-in link you were sent on this device."</p>
            if spent {
                <p class="wrong">"That sign-in link was used already or is older than a week. Ask whoever sent it for a new one."</p>
            }
            <details class="key" open=(wrong)>
                <summary>"Sign in with the key"</summary>
                <form method="post" action="/sign-in/key">
                    <label>"Key" <input type="password" name="key" autocomplete="current-password"></label>
                    <button class="pill" type="submit">"Sign in"</button>
                </form>
                if wrong {
                    <p class="wrong">"That key is not the one."</p>
                }
            </details>
        </article>
    })
}

#[derive(serde::Deserialize)]
struct Given {
    key: String,
}

#[route(POST "/sign-in/key")]
async fn sign_in_key(cx: &Cx, Form(given): Form<Given>) -> Result<SeeOther> {
    check(cx, &given.key).await
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
        assert!(open("/a/abc"));
        assert!(!open("/s"));
        assert!(!open("/invites"));
        assert!(!open("/"));
        assert!(!open("/watches"));
        assert!(!open("/users"));
        assert!(!open("/sign-in/elsewhere"));
        assert!(same("abc", "abc"));
        assert!(!same("abc", "abd"));
        assert!(!same("abc", "abcd"));
    }

    #[test]
    fn users_and_feedback_are_for_admins() {
        assert!(listed(&ADMINS_ONLY, "/users"));
        assert!(listed(&ADMINS_ONLY, "/users/3/invite"));
        assert!(listed(&ADMINS_ONLY, "/feedback"));
        assert!(listed(&ADMINS_ONLY, "/api/feedback/4/status"));
        assert!(!listed(&ADMINS_ONLY, "/usersx"));
        assert!(!listed(&ADMINS_ONLY, "/watches"));
        assert!(!listed(&ADMINS_ONLY, "/invites"));
    }

    #[test]
    fn a_device_is_named_by_its_kind() {
        assert_eq!(
            kind("Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15"),
            "iPhone"
        );
        assert_eq!(kind("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)"), "Mac");
        assert_eq!(kind("Mozilla/5.0 (Linux; Android 14; Pixel 8)"), "Android phone");
        assert_eq!(kind("Mozilla/5.0 (X11; Linux x86_64)"), "Linux computer");
        assert_eq!(kind("curl/8"), "Browser");
    }
}
