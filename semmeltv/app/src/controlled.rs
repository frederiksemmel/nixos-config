//! Who did what to the TV through the app: every request that controls the
//! TV and succeeds becomes a `Control` row in the usage table, which the
//! admins' Usage page lists.

use crate::auth::gate::{self, Access};
use crate::auth::{self, Capability, Person};
use crate::core::Core;
use crate::library::Library;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::Response;
use serde_json::Value;

/// The largest body read to describe a request; a bigger one still goes
/// through, described by its path alone.
const LONGEST_BODY: usize = 64 * 1024;

pub async fn note(State(core): State<Core>, request: Request, next: Next) -> Response {
    let Some(person) = request.extensions().get::<Person>().cloned() else {
        return next.run(request).await;
    };
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    if !watched(&method, &path) {
        return next.run(request).await;
    }
    let device = auth::http::label(request.headers());
    let (parts, body) = request.into_parts();
    let Ok(bytes) = to_bytes(body, LONGEST_BODY).await else {
        return next.run(Request::from_parts(parts, Body::empty())).await;
    };
    let action = describe(&method, &path, &bytes, &core.library());
    let response = next.run(Request::from_parts(parts, Body::from(bytes))).await;
    if response.status().is_success()
        && let Some(action) = action
        && let Err(error) = core
            .db()
            .note_control(crate::watching::now(), person.id, &device, &action)
            .await
    {
        eprintln!("usage: noting {action}: {error}");
    }
    response
}

fn watched(method: &Method, path: &str) -> bool {
    gate::access(method, path) == Access::Needs(Capability::ControlTv)
        || (method == Method::POST && path == "/api/screen")
}

/// What a request did to the TV, in words for the Usage page, or None when it
/// left the TV alone, such as a phone starting a film on itself.
pub fn describe(method: &Method, path: &str, body: &[u8], library: &Library) -> Option<String> {
    let body: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let delete = method == Method::DELETE;
    let fixed = |text: &str| Some(text.to_string());
    match segments.as_slice() {
        ["api", "screen"] => match (body["action"].as_str(), body["to"].as_str()) {
            (Some("flip"), Some("tv")) => fixed("Moved playback from their phone to the TV"),
            (Some("flip"), Some("phone")) => fixed("Moved playback from the TV to their phone"),
            _ => None,
        },
        ["api", "play"] => Some(format!("Played {}", picked(&body, library))),
        ["api", "command"] => Some(format!("Asked “{}”", body["text"].as_str().unwrap_or_default())),
        ["api", "playback", what] => match *what {
            "pause" => fixed("Paused"),
            "resume" => fixed("Resumed"),
            "toggle" => fixed("Paused or resumed"),
            "stop" => fixed("Stopped"),
            "mute" => fixed("Muted"),
            "unmute" => fixed("Unmuted"),
            "skip-segment" => fixed("Tapped Skip"),
            "hide-skip" => fixed("Hid the Skip button"),
            "seek" => Some(match (body["to"].as_f64(), body["by"].as_f64()) {
                (Some(to), _) => format!("Jumped to {}", clock(to)),
                (_, Some(by)) if by < 0.0 => format!("Jumped back {}", clock(-by)),
                (_, Some(by)) => format!("Jumped ahead {}", clock(by)),
                _ => "Jumped".to_string(),
            }),
            "skip" if body["direction"] == "back" => fixed("Skipped back"),
            "skip" => fixed("Skipped ahead"),
            "speed" => Some(match body["speed"].as_f64() {
                Some(speed) => format!("Set the speed to {speed}×"),
                None => "Changed the speed".to_string(),
            }),
            other => Some(format!("Playback: {other}")),
        },
        ["api", "tv", what] => match *what {
            "on" => fixed("Turned the TV on"),
            "off" => fixed("Turned the TV off"),
            "semmeltv" => fixed("Switched the TV to SemmelTV"),
            "volume-up" => fixed("Volume up"),
            "volume-down" => fixed("Volume down"),
            "mute" => fixed("Muted the TV"),
            other => Some(format!("TV: {other}")),
        },
        ["api", "volume", what] => match *what {
            "up" => fixed("Volume up"),
            "down" => fixed("Volume down"),
            "mute" => fixed("Muted the TV"),
            other => Some(format!("Volume: {other}")),
        },
        ["api", "tv-profile"] => fixed("Showed their profile on the TV"),
        ["api", "audio"] => match body["output"].as_str() {
            Some("tv") => fixed("Switched the sound to the TV"),
            Some("headphones") => fixed("Switched the sound to headphones"),
            _ => fixed("Switched the sound"),
        },
        ["api", "audio", "pair"] if delete => fixed("Stopped pairing headphones"),
        ["api", "audio", "pair"] => fixed("Paired headphones"),
        ["api", "audio", "headphones"] => fixed("Forgot their headphones"),
        ["api", "audio", "delay"] => fixed("Changed the sound delay"),
        ["api", "audio", "track"] => fixed("Changed the audio language"),
        ["api", "subtitles", "track"] => fixed("Changed the subtitles"),
        ["api", "subtitles", "toggle"] => fixed("Turned subtitles on or off"),
        ["api", "subtitles", "delay"] => fixed("Changed the subtitle delay"),
        _ => Some(format!("{method} {path}")),
    }
}

/// "Arrival" or "Dark · S01E02", in the words the plays use.
fn picked(body: &Value, library: &Library) -> String {
    if let Some(key) = body["movie"].as_str() {
        return library
            .movies
            .iter()
            .find(|movie| movie.key == key)
            .map_or_else(|| key.to_string(), |movie| movie.title.clone());
    }
    let Some(key) = body["show"].as_str() else {
        return "something".to_string();
    };
    let title = library
        .shows
        .iter()
        .find(|show| show.key == key)
        .map_or_else(|| key.to_string(), |show| show.title.clone());
    match (body["season"].as_u64(), body["episode"].as_u64()) {
        (Some(season), Some(episode)) => format!("{title} · S{season:02}E{episode:02}"),
        (Some(season), None) => format!("{title} · season {season}"),
        _ => title,
    }
}

fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.0).round() as u64;
    match (seconds / 3600, seconds % 3600 / 60, seconds % 60) {
        (0, 0, seconds) => format!("{seconds} s"),
        (0, minutes, seconds) => format!("{minutes}:{seconds:02}"),
        (hours, minutes, seconds) => format!("{hours}:{minutes:02}:{seconds:02}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn said(method: Method, path: &str, body: Value) -> Option<String> {
        let library = Library::default();
        describe(&method, path, body.to_string().as_bytes(), &library)
    }

    #[test]
    fn each_tv_control_reads_as_what_it_did() {
        let post = || Method::POST;
        let rows = [
            (post(), "/api/playback/pause", json!({}), "Paused"),
            (post(), "/api/playback/toggle", json!({}), "Paused or resumed"),
            (post(), "/api/playback/seek", json!({ "to": 754.4 }), "Jumped to 12:34"),
            (post(), "/api/playback/seek", json!({ "by": -10 }), "Jumped back 10 s"),
            (
                post(),
                "/api/playback/seek",
                json!({ "to": 3_725 }),
                "Jumped to 1:02:05",
            ),
            (
                post(),
                "/api/playback/skip",
                json!({ "direction": "back" }),
                "Skipped back",
            ),
            (
                post(),
                "/api/playback/speed",
                json!({ "speed": 1.5 }),
                "Set the speed to 1.5×",
            ),
            (post(), "/api/playback/skip-segment", json!({}), "Tapped Skip"),
            (post(), "/api/tv/on", json!({}), "Turned the TV on"),
            (post(), "/api/tv/volume-up", json!({}), "Volume up"),
            (post(), "/api/volume/down", json!({}), "Volume down"),
            (post(), "/api/tv-profile", json!({}), "Showed their profile on the TV"),
            (
                post(),
                "/api/audio",
                json!({ "output": "headphones" }),
                "Switched the sound to headphones",
            ),
            (
                Method::DELETE,
                "/api/audio/pair",
                json!({}),
                "Stopped pairing headphones",
            ),
            (
                post(),
                "/api/subtitles/track",
                json!({ "index": 2 }),
                "Changed the subtitles",
            ),
            (
                post(),
                "/api/command",
                json!({ "text": "play dark" }),
                "Asked “play dark”",
            ),
            (post(), "/api/play", json!({ "movie": "arrival" }), "Played arrival"),
            (
                post(),
                "/api/play",
                json!({ "show": "dark", "season": 1, "episode": 2 }),
                "Played dark · S01E02",
            ),
            (
                post(),
                "/api/screen",
                json!({ "action": "flip", "to": "tv", "device": {} }),
                "Moved playback from their phone to the TV",
            ),
        ];
        for (method, path, body, want) in rows {
            assert_eq!(
                said(method.clone(), path, body).as_deref(),
                Some(want),
                "{method} {path}"
            );
        }
    }

    #[test]
    fn a_phone_playing_on_itself_leaves_the_tv_alone() {
        let play = json!({ "action": "play", "device": { "id": "p1" }, "movie": "arrival" });
        assert_eq!(said(Method::POST, "/api/screen", play), None);
        assert_eq!(said(Method::POST, "/api/screen", json!({ "action": "stop" })), None);
    }

    #[test]
    fn the_gate_notes_who_controlled_the_tv_from_which_device_but_not_a_refused_try() {
        use crate::auth::gate::{COOKIE, Gate, guard};
        use crate::auth::{Auth, Role};
        use crate::usage::Activity;
        use axum::Router;
        use axum::http::StatusCode;
        use axum::middleware::from_fn_with_state;
        use axum::routing::post;

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (core, base, anna, otto, anna_token, otto_token) = runtime.block_on(async {
            let (core, _inbox) = Core::detached(Library::default()).await;
            let auth = Auth::new(core.db().clone(), &["https://tv.example.org".to_string()]).unwrap();
            let anna = auth.create_profile("Tante Anna", Role::Viewer).await.unwrap().id;
            let otto = auth.create_profile("Onkel Otto", Role::Viewer).await.unwrap().id;
            auth.grant(anna, Capability::ControlTv, true).await.unwrap();
            for id in [anna, otto] {
                auth.set_password(id, "a long password").await.unwrap();
            }
            let anna_token = auth
                .sign_in_with_password(anna, "a long password", "iPad")
                .await
                .unwrap();
            let otto_token = auth
                .sign_in_with_password(otto, "a long password", "iPad")
                .await
                .unwrap();
            let gate = Gate { auth, service: None };
            let app = Router::new()
                .route("/api/playback/{control}", post(|| async { "done" }))
                .route(
                    "/api/playback/fail",
                    post(|| async { StatusCode::UNPROCESSABLE_ENTITY }),
                )
                .route(
                    "/api/playback/seek",
                    post(|axum::Json(body): axum::Json<Value>| async move { axum::Json(body) }),
                )
                .layer(from_fn_with_state(core.clone(), note))
                .layer(from_fn_with_state(gate, guard));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            tokio::spawn(async move { axum::serve(listener, app).await });
            (core, base, anna, otto, anna_token, otto_token)
        });
        let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        let press = |path: &str, token: &str| {
            agent
                .post(format!("{base}{path}"))
                .header("cookie", format!("{COOKIE}={token}"))
                .header(
                    "user-agent",
                    "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) Version/18.5 Mobile/15E148 Safari/604.1",
                )
                .send_empty()
                .unwrap()
                .status()
                .as_u16()
        };
        assert_eq!(press("/api/playback/pause", &anna_token), 200);
        assert_eq!(press("/api/playback/pause", &otto_token), 403);
        assert_eq!(press("/api/playback/fail", &anna_token), 422);
        let echoed: Value = agent
            .post(format!("{base}/api/playback/seek"))
            .header("cookie", format!("{COOKIE}={anna_token}"))
            .send_json(serde_json::json!({ "to": 60 }))
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();
        assert_eq!(
            echoed,
            serde_json::json!({ "to": 60 }),
            "the body still reaches the handler"
        );
        let noted: Vec<_> = runtime
            .block_on(core.db().usage_since(0))
            .unwrap()
            .into_iter()
            .filter(|event| event.activity == Activity::Control)
            .map(|event| (event.profile, event.device, event.title))
            .collect();
        assert_eq!(
            noted,
            [
                (anna, "iPhone · Safari".to_string(), "Paused".to_string()),
                (anna, "Browser".to_string(), "Jumped to 1:00".to_string()),
            ]
        );
        assert_ne!(anna, otto);
    }

    #[test]
    fn only_tv_controls_and_hand_overs_are_noted() {
        assert!(watched(&Method::POST, "/api/playback/pause"));
        assert!(watched(&Method::POST, "/api/screen"));
        assert!(watched(&Method::POST, "/api/play"));
        assert!(!watched(&Method::GET, "/api/playback"));
        assert!(!watched(&Method::GET, "/api/screen"));
        assert!(!watched(&Method::POST, "/api/subtitles/fetch"));
        assert!(!watched(&Method::PUT, "/api/auth/members/2/capabilities/control-tv"));
    }
}
