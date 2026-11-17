use super::{Auth, Capability, Person, Resolved, SESSION_LIFETIME};
use crate::profile::ProfileId;
use axum::Json;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::convert::Infallible;

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
        Ok(Who(parts
            .extensions
            .get::<Person>()
            .map_or(ProfileId::FIRST, |person| person.id)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Access {
    Open,
    Stream,
    Service,
    Signed,
    Needs(Capability),
    Admin,
    AdminOrService,
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
        ["api", "feedback", ..] if method == Method::GET => Access::AdminOrService,
        ["api", "feedback", _, "status" | "comments"] => Access::AdminOrService,
        ["api", "storage", ..]
        | ["api", "tv", "pair"]
        | ["api", "auth", "members", ..]
        | ["api", "downloads", "jobs", _, "stop"] => Access::Admin,
        ["api", "downloads", ..] => Access::Needs(Capability::Download),
        ["api", "audio", "pair" | "headphones" | "delay"] => Access::Needs(Capability::Headphones),
        ["api", "audio"] if writes(method) => Access::Needs(Capability::Headphones),
        ["api", "play" | "command" | "listen" | "microphone" | "volume", ..]
        | ["api", "audio", "track"]
        | ["api", "subtitles", "track" | "toggle" | "delay"] => Access::Needs(Capability::ControlTv),
        ["api", "playback", _] | ["api", "tv", _] | ["api", "tv-profile"] if writes(method) => {
            Access::Needs(Capability::ControlTv)
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
        if known && matches!(access, Access::Service | Access::AdminOrService) {
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
        match gate.auth.resolve(&tokens).await {
            Ok(resolved) => resolved,
            Err(refusal) => {
                eprintln!("auth: resolving a session: {refusal}");
                return refuse(StatusCode::SERVICE_UNAVAILABLE, &refusal.to_string());
            }
        }
    };

    let person = resolved.person().cloned();
    match (&access, &person) {
        (Access::Open | Access::Stream, _) => {}
        (_, None) => {
            let mut response = refuse(StatusCode::UNAUTHORIZED, "Not signed in");
            if resolved.rewrite {
                response
                    .headers_mut()
                    .append(header::SET_COOKIE, cookie(&resolved.tokens()));
            }
            return response;
        }
        (Access::Admin | Access::AdminOrService, Some(person)) if !person.admin() => {
            return refuse(StatusCode::FORBIDDEN, "Only an admin can do that");
        }
        (Access::Needs(capability), Some(person)) if !person.can(*capability) => {
            return refuse(StatusCode::FORBIDDEN, capability.refusal());
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
        response
            .headers_mut()
            .append(header::SET_COOKIE, cookie(&resolved.tokens()));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_route_has_its_access() {
        let (get, post, put, delete) = (Method::GET, Method::POST, Method::PUT, Method::DELETE);
        let rows = [
            (&post, "/api/command", Access::Needs(Capability::ControlTv)),
            (&post, "/api/listen", Access::Needs(Capability::ControlTv)),
            (&get, "/api/microphone", Access::Needs(Capability::ControlTv)),
            (&get, "/api/playback", Access::Signed),
            (&post, "/api/playback/seek", Access::Needs(Capability::ControlTv)),
            (&post, "/api/playback/toggle", Access::Needs(Capability::ControlTv)),
            (
                &post,
                "/api/playback/skip-segment",
                Access::Needs(Capability::ControlTv),
            ),
            (&get, "/api/tv", Access::Signed),
            (&post, "/api/tv/volume-up", Access::Needs(Capability::ControlTv)),
            (&post, "/api/tv/mute", Access::Needs(Capability::ControlTv)),
            (&get, "/api/tv-profile", Access::Signed),
            (&post, "/api/tv-profile", Access::Needs(Capability::ControlTv)),
            (&get, "/api/screen", Access::Signed),
            (&post, "/api/screen", Access::Signed),
            (&post, "/api/volume/up", Access::Needs(Capability::ControlTv)),
            (&get, "/api/audio", Access::Signed),
            (&post, "/api/audio", Access::Needs(Capability::Headphones)),
            (&post, "/api/audio/pair", Access::Needs(Capability::Headphones)),
            (&delete, "/api/audio/pair", Access::Needs(Capability::Headphones)),
            (&delete, "/api/audio/headphones", Access::Needs(Capability::Headphones)),
            (&post, "/api/audio/delay", Access::Needs(Capability::Headphones)),
            (&post, "/api/audio/track", Access::Needs(Capability::ControlTv)),
            (&get, "/api/audio/dialogue", Access::Signed),
            (&post, "/api/audio/dialogue", Access::Signed),
            (&get, "/api/subtitles", Access::Signed),
            (&put, "/api/subtitles", Access::Signed),
            (&post, "/api/subtitles/track", Access::Needs(Capability::ControlTv)),
            (&post, "/api/subtitles/toggle", Access::Needs(Capability::ControlTv)),
            (&post, "/api/subtitles/delay", Access::Needs(Capability::ControlTv)),
            (&post, "/api/subtitles/fetch", Access::Signed),
            (&post, "/api/watched", Access::Signed),
            (&post, "/api/stream/abc/progress", Access::Stream),
            (&put, "/api/auth/members/2/capabilities/control-tv", Access::Admin),
            (&delete, "/api/auth/members/2/capabilities/headphones", Access::Admin),
            (&get, "/api/auth/choices", Access::Open),
            (&post, "/api/auth/password", Access::Open),
            (&post, "/api/auth/passkey/start", Access::Open),
            (&post, "/api/auth/invite/abc", Access::Open),
            (&get, "/api/auth/me", Access::Signed),
            (&post, "/api/auth/passkeys/start", Access::Signed),
            (&get, "/api/stream/abc/main.m3u8", Access::Stream),
            (&post, "/api/stream/open", Access::Signed),
            (&get, "/api/events", Access::Service),
            (&get, "/api/library", Access::Signed),
            (&get, "/api/movies/dune/poster", Access::Signed),
            (&post, "/api/play", Access::Needs(Capability::ControlTv)),
            (&get, "//api/auth/choices/../../library", Access::Signed),
            (&get, "/api/storage", Access::Admin),
            (&post, "/api/storage/remove", Access::Admin),
            (&post, "/api/tv/pair", Access::Admin),
            (&post, "/api/tv/on", Access::Needs(Capability::ControlTv)),
            (&get, "/api/auth/members", Access::Admin),
            (&post, "/api/auth/members/2/invite", Access::Admin),
            (&post, "/api/downloads/confirm", Access::Needs(Capability::Download)),
            (&get, "/api/downloads", Access::Needs(Capability::Download)),
            (&post, "/api/downloads/jobs/abc/stop", Access::Admin),
            (&put, "/api/auth/members/2/capabilities/download", Access::Admin),
            (&post, "/api/feedback", Access::Signed),
            (&get, "/api/feedback", Access::AdminOrService),
            (&get, "/api/feedback/20261115-131211-ab12", Access::AdminOrService),
            (
                &get,
                "/api/feedback/files/20261115-131211-ab12-tv.jpg",
                Access::AdminOrService,
            ),
            (
                &post,
                "/api/feedback/20261115-131211-ab12/status",
                Access::AdminOrService,
            ),
            (
                &post,
                "/api/feedback/20261115-131211-ab12/comments",
                Access::AdminOrService,
            ),
        ];
        for (method, path, want) in rows {
            assert_eq!(access(method, path), want, "{method} {path}");
        }
    }

    #[test]
    fn the_cookie_holds_the_device_sessions_in_order() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("other=1; semmeltv_session=aaa.bbb; x=y"),
        );
        assert_eq!(tokens(&headers), vec!["aaa".to_string(), "bbb".to_string()]);
        let set = cookie(&["aaa".to_string(), "bbb".to_string()]);
        let set = set.to_str().unwrap();
        assert!(set.starts_with("semmeltv_session=aaa.bbb;"));
        assert!(set.contains("HttpOnly") && set.contains("Secure") && set.contains("SameSite=Lax"));
        assert!(cookie(&[]).to_str().unwrap().contains("Max-Age=0"));
    }
}
