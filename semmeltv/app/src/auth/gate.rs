use super::{Auth, Person, Resolved, SESSION_LIFETIME};
use crate::profile::ProfileId;
use axum::Json;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use std::convert::Infallible;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use sha2::{Digest, Sha256};

pub const COOKIE: &str = "semmeltv_session";

#[derive(Clone)]
pub struct Gate {
    pub auth: Auth,
    pub service: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Caller {
    Person(Person),
    Service,
}

pub struct Who(pub ProfileId);

impl<S: Send + Sync> FromRequestParts<S> for Who {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(Who(parts.extensions.get::<Person>().map_or(ProfileId::FIRST, |person| person.id)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Access {
    Open,
    Stream,
    Service,
    Signed,
    Admin,
}

pub fn access(method: &Method, path: &str) -> Access {
    let segments = path.trim_start_matches('/').split('/').collect::<Vec<_>>();
    match segments.as_slice() {
        ["api", "auth", "choices"]
        | ["api", "auth", "password"]
        | ["api", "auth", "passkey", "start" | "finish"]
        | ["api", "auth", "invite", _]
        | ["api", "log"] => Access::Open,
        ["api", "stream", id, ..] if *id != "open" => Access::Stream,
        ["api", "events"] if method == Method::GET => Access::Service,
        ["api", "storage", ..] | ["api", "tv", "pair"] | ["api", "auth", "members", ..] | ["api", "downloads", ..] => {
            Access::Admin
        }
        _ => Access::Signed,
    }
}

pub fn tokens(headers: &HeaderMap) -> Vec<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .filter(|(name, _)| *name == COOKIE)
        .flat_map(|(_, value)| value.split('.'))
        .filter(|token| !token.is_empty() && token.len() <= 64)
        .map(str::to_string)
        .collect()
}

pub fn cookie(tokens: &[String]) -> HeaderValue {
    let value = if tokens.is_empty() {
        format!("{COOKIE}=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Lax")
    } else {
        format!(
            "{COOKIE}={}; Path=/; Max-Age={SESSION_LIFETIME}; HttpOnly; Secure; SameSite=Lax",
            tokens.join(".")
        )
    };
    HeaderValue::from_str(&value).expect("tokens are base64url")
}

fn same(a: &str, b: &str) -> bool {
    Sha256::digest(a.as_bytes()) == Sha256::digest(b.as_bytes())
}

fn refuse(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

pub fn origin(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .filter(|value| *value != "null")
        .map(|value| value.trim_end_matches('/').to_string())
}

fn writes(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

pub async fn guard(State(gate): State<Gate>, mut request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let access = access(&method, &path);

    if writes(&method)
        && let Some(origin) = origin(request.headers())
        && !gate.auth.allows_origin(&origin)
    {
        eprintln!("auth: refused {method} {path} from origin {origin}");
        return refuse(StatusCode::FORBIDDEN, "This address may not change anything on the TV");
    }

    let bearer = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_string);
    if let Some(bearer) = bearer {
        let known = gate.service.as_deref().is_some_and(|service| same(service, &bearer));
        if known && access == Access::Service {
            request.extensions_mut().insert(Caller::Service);
            return next.run(request).await;
        }
        return refuse(StatusCode::FORBIDDEN, "That token is not good for this");
    }

    let tokens = tokens(request.headers());
    let resolved = if tokens.is_empty() {
        Resolved {
            active: None,
            others: Vec::new(),
            rewrite: false,
        }
    } else {
        let auth = gate.auth.clone();
        match tokio::task::spawn_blocking(move || auth.resolve(&tokens)).await {
            Ok(Ok(resolved)) => resolved,
            Ok(Err(refusal)) => {
                eprintln!("auth: resolving a session: {refusal}");
                return refuse(StatusCode::SERVICE_UNAVAILABLE, &refusal.to_string());
            }
            Err(error) => {
                eprintln!("auth: resolving a session: {error}");
                return refuse(StatusCode::SERVICE_UNAVAILABLE, "The TV couldn't check who you are");
            }
        }
    };

    let person = resolved.person().cloned();
    match (&access, &person) {
        (Access::Open | Access::Stream, _) => {}
        (_, None) => {
            let mut response = refuse(StatusCode::UNAUTHORIZED, "Not signed in");
            if resolved.rewrite {
                response.headers_mut().append(header::SET_COOKIE, cookie(&resolved.tokens()));
            }
            return response;
        }
        (Access::Admin, Some(person)) if !person.admin() => {
            return refuse(StatusCode::FORBIDDEN, "Only an admin can do that");
        }
        _ => {}
    }

    if let Some(person) = person {
        request.extensions_mut().insert(Caller::Person(person.clone()));
        request.extensions_mut().insert(person);
    }
    request.extensions_mut().insert(resolved.clone());
    let mut response = next.run(request).await;
    if resolved.rewrite && !response.headers().contains_key(header::SET_COOKIE) {
        response.headers_mut().append(header::SET_COOKIE, cookie(&resolved.tokens()));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_sign_in_and_the_stream_by_its_id_are_open() {
        let get = Method::GET;
        let post = Method::POST;
        assert_eq!(access(&get, "/api/auth/choices"), Access::Open);
        assert_eq!(access(&post, "/api/auth/password"), Access::Open);
        assert_eq!(access(&post, "/api/auth/passkey/start"), Access::Open);
        assert_eq!(access(&post, "/api/auth/invite/abc"), Access::Open);
        assert_eq!(access(&get, "/api/auth/me"), Access::Signed);
        assert_eq!(access(&post, "/api/auth/passkeys/start"), Access::Signed);
        assert_eq!(access(&get, "/api/stream/abc/main.m3u8"), Access::Stream);
        assert_eq!(access(&post, "/api/stream/open"), Access::Signed);
        assert_eq!(access(&get, "/api/events"), Access::Service);
        assert_eq!(access(&get, "/api/library"), Access::Signed);
        assert_eq!(access(&get, "/api/movies/dune/poster"), Access::Signed);
        assert_eq!(access(&post, "/api/play"), Access::Signed);
        assert_eq!(access(&get, "//api/auth/choices/../../library"), Access::Signed);
    }

    #[test]
    fn storage_tv_pairing_members_and_downloads_are_for_admins() {
        assert_eq!(access(&Method::GET, "/api/storage"), Access::Admin);
        assert_eq!(access(&Method::POST, "/api/storage/remove"), Access::Admin);
        assert_eq!(access(&Method::POST, "/api/tv/pair"), Access::Admin);
        assert_eq!(access(&Method::POST, "/api/tv/on"), Access::Signed);
        assert_eq!(access(&Method::GET, "/api/auth/members"), Access::Admin);
        assert_eq!(access(&Method::POST, "/api/auth/members/2/invite"), Access::Admin);
        assert_eq!(access(&Method::POST, "/api/downloads/confirm"), Access::Admin);
    }

    #[test]
    fn the_cookie_holds_the_device_sessions_in_order() {
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, HeaderValue::from_static("other=1; semmeltv_session=aaa.bbb; x=y"));
        assert_eq!(tokens(&headers), vec!["aaa".to_string(), "bbb".to_string()]);
        let set = cookie(&["aaa".to_string(), "bbb".to_string()]);
        let set = set.to_str().unwrap();
        assert!(set.starts_with("semmeltv_session=aaa.bbb;"));
        assert!(set.contains("HttpOnly") && set.contains("Secure") && set.contains("SameSite=Lax"));
        assert!(cookie(&[]).to_str().unwrap().contains("Max-Age=0"));
    }
}
