//! One person uses this, with a key from a file: the sign-in page takes it
//! once and the browser keeps it in a cookie.

use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::cookie::{Cookies, SameSite, cookie, cookies, time::Duration};
use topcoat::router::content::Form;
use topcoat::router::error::{SeeOther, redirect, see_other};
use topcoat::router::request::{headers, uri};
use topcoat::router::{Body, Layer, LayerFuture, Next, Path, page, query_params, route};
use topcoat::view::{View, view};

const COOKIE: &str = "courts_key";
const OPEN: [&str; 7] = [
    "/sign-in",
    "/sign-in/key",
    "/sign-in/link",
    "/sw.js",
    "/icon.svg",
    "/manifest.webmanifest",
    "/_topcoat/",
];

#[derive(Clone)]
pub struct Key(pub String);

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
    Ok(view! {
        <article class="sign-in">
            <h1>"Courts"</h1>
            <p>"Free tennis courts in Zurich, from GotCourts."</p>
            <form method="post" action="/sign-in/key">
                <label>"Key" <input type="password" name="key" autocomplete="current-password" autofocus=(true)></label>
                <button class="pill" type="submit">"Sign in"</button>
            </form>
            if wrong {
                <p class="wrong">"That key is not the one."</p>
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
        assert!(!open("/"));
        assert!(!open("/watches"));
        assert!(!open("/sign-in/elsewhere"));
        assert!(same("abc", "abc"));
        assert!(!same("abc", "abd"));
        assert!(!same("abc", "abcd"));
    }
}
