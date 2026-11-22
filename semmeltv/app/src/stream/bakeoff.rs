//! The bake-off over HTTP: the full oven's answer to `open`, the challenger's
//! `/api/challenges`, and the defender's `/api/stream/{id}/challenge`, which
//! the stream's id unlocks like the rest of the stream.

use super::{Full as Limit, Streams, short};
use crate::api::problem;
use crate::auth::gate::Who;
use crate::challenge::bake::{Calibration, GAME, Input, SEMMELS};
use crate::challenge::{Challenger, Effect, Refusal, Seat, Side, View, Watcher};
use crate::profile::ProfileId;
use crate::screen::Device;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::time::Instant;

const CALIBRATION: &str = "bake-off calibration";
pub const FULL_HEADING: &str = "Der Ofen ist voll";

pub fn routes() -> Router<Streams> {
    Router::new()
        .route("/api/challenges", post(challenge))
        .route("/api/challenges/practice", post(practice))
        .route("/api/challenges/{id}", get(challenger_view).delete(withdraw))
        .route("/api/challenges/{id}/presses", post(challenger_presses))
        .route("/api/stream/{id}/challenge", get(defender_view))
        .route("/api/stream/{id}/challenge/accept", post(accept))
        .route("/api/stream/{id}/challenge/drop-off", post(drop_off))
        .route("/api/stream/{id}/challenge/presses", post(defender_presses))
}

pub async fn load_calibration(streams: &Streams) {
    let stored = streams.core.setting(CALIBRATION).await;
    if let Some(calibration) = stored.and_then(|text| serde_json::from_str::<Calibration>(&text).ok()) {
        streams.core.arena().calibration = calibration;
    }
}

/// Keeps what the oven learnt about each input across restarts.
pub async fn save_calibration(core: &crate::core::Core) {
    let text = serde_json::to_string(&core.arena().calibration);
    if let Ok(text) = text {
        core.set_setting(CALIBRATION, &text).await;
    }
}

#[derive(Serialize)]
pub struct Full {
    game: &'static str,
    heading: &'static str,
    message: String,
    watchers: Vec<Listed>,
    resting_ms: Option<u64>,
}

#[derive(Serialize)]
struct Listed {
    #[serde(flatten)]
    watcher: Watcher,
    busy: bool,
}

#[derive(Serialize)]
struct FullAnswer {
    error: String,
    full: Full,
}

/// Who watches whose end would make room under `limit`, for the challenge
/// sheet; without a limit, everyone who can be challenged.
async fn watchers(streams: &Streams, except: Option<&Device>, limit: Option<Limit>) -> Vec<Watcher> {
    let from_pcloud = limit == Some(Limit::FromPcloud);
    let live: Vec<_> = streams
        .live
        .lock()
        .unwrap()
        .values()
        .filter(|live| !from_pcloud || !live.downloaded)
        .cloned()
        .collect();
    let mut found = Vec::new();
    for live in live {
        if except.is_some_and(|device| live.device.as_ref().is_some_and(|other| other.id == device.id)) {
            continue;
        }
        let person = streams
            .core
            .profile_name(live.profile)
            .await
            .unwrap_or_else(|| "Someone".to_string());
        found.push(Watcher {
            seat: Seat::Stream(live.id.clone()),
            profile: live.profile,
            person,
            device: super::device_name(live.device.as_ref()).to_string(),
            title: live.name.clone(),
        });
    }
    found.sort_by(|a, b| (&a.person, &a.device).cmp(&(&b.person, &b.device)));
    let tv = if limit == Some(Limit::Streams) {
        None
    } else {
        tv_watcher(streams, from_pcloud).await
    };
    if let Some(tv) = tv {
        found.insert(0, tv);
    }
    found
}

/// The TV, while it plays a film here; with `from_pcloud`, only while that
/// film isn't downloaded.
async fn tv_watcher(streams: &Streams, from_pcloud: bool) -> Option<Watcher> {
    if !streams.core.arena().tv_can_bake {
        return None;
    }
    let playing = streams.core.now_playing()?;
    if !matches!(streams.core.screen().on, crate::screen::OnScreen::Tv) {
        return None;
    }
    let file = super::tv_file(&streams.core.library(), &playing.playing);
    if from_pcloud && file.is_none_or(|file| streams.core.is_downloaded(&file)) {
        return None;
    }
    let profile = streams.core.tv_profile();
    let person = streams
        .core
        .profile_name(profile)
        .await
        .unwrap_or_else(|| "Someone".to_string());
    Some(Watcher {
        seat: Seat::Tv,
        profile,
        person,
        device: "the TV".to_string(),
        title: playing.title,
    })
}

/// The answer to an `open` that finds every place taken.
pub async fn full(streams: &Streams, profile: ProfileId, device: Option<&Device>, limit: Limit) -> Response {
    let message = limit.message();
    let watchers = watchers(streams, device, Some(limit)).await;
    let now = Instant::now();
    let (listed, resting) = {
        let arena = streams.core.arena();
        let listed = watchers
            .into_iter()
            .map(|watcher| Listed {
                busy: arena.busy(&watcher.seat),
                watcher,
            })
            .collect();
        (listed, arena.resting(profile, now))
    };
    let answer = FullAnswer {
        error: message.clone(),
        full: Full {
            game: GAME,
            heading: FULL_HEADING,
            message,
            watchers: listed,
            resting_ms: resting.map(|left| left.as_millis() as u64),
        },
    };
    (StatusCode::TOO_MANY_REQUESTS, Json(answer)).into_response()
}

/// Ends what ran out of time and carries out what the arena decided.
pub fn settle(streams: &Streams) {
    let effects = streams.core.arena().settle(Instant::now());
    apply(streams, effects);
}

fn apply(streams: &Streams, effects: Vec<Effect>) {
    for effect in effects {
        match effect {
            Effect::Stop(Seat::Stream(id)) => {
                let removed = streams.live.lock().unwrap().remove(&id);
                if let Some(live) = removed {
                    eprintln!("stream {}: ended, lost a bake-off", short(&id));
                    live.remux.close();
                }
            }
            Effect::Stop(Seat::Tv | Seat::Practice) => {}
        }
    }
}

fn refused(refusal: &Refusal) -> Response {
    let status = match refusal {
        Refusal::NoSuchChallenge | Refusal::NoSuchSeat => StatusCode::NOT_FOUND,
        _ => StatusCode::CONFLICT,
    };
    problem(status, refusal.message())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChallengeBody {
    seat: String,
    device: Option<Device>,
}

#[derive(Serialize)]
struct Created {
    id: String,
}

async fn challenge(State(streams): State<Streams>, Who(profile): Who, Json(body): Json<ChallengeBody>) -> Response {
    settle(&streams);
    let Some(defender) = watchers(&streams, None, None)
        .await
        .into_iter()
        .find(|watcher| watcher.seat == seat_named(&body.seat))
    else {
        return refused(&Refusal::NoSuchSeat);
    };
    let person = streams
        .core
        .profile_name(profile)
        .await
        .unwrap_or_else(|| "Someone".to_string());
    let by = Challenger {
        profile,
        person,
        device: super::device_name(body.device.as_ref()).to_string(),
    };
    let id = super::new_id();
    let named = (by.person.clone(), defender.person.clone());
    let made = streams.core.arena().challenge(id.clone(), by, defender, Instant::now());
    match made {
        Ok(()) => {
            eprintln!("bake-off {}: {} challenges {}", short(&id), named.0, named.1);
            Json(Created { id }).into_response()
        }
        Err(refusal) => refused(&refusal),
    }
}

fn view(streams: &Streams, id: &str, side: Side) -> Response {
    match streams.core.arena().view(id, side, Instant::now()) {
        Some(view) => Json(Seen { game: GAME, side, view }).into_response(),
        None => refused(&Refusal::NoSuchChallenge),
    }
}

#[derive(Serialize)]
struct Seen {
    game: &'static str,
    side: Side,
    #[serde(flatten)]
    view: View,
}

/// A practice bake against the Bäckermeister, for anyone signed in.
async fn practice(State(streams): State<Streams>, Who(profile): Who) -> Response {
    let person = streams
        .core
        .profile_name(profile)
        .await
        .unwrap_or_else(|| "Someone".to_string());
    let by = Challenger {
        profile,
        person,
        device: String::new(),
    };
    let id = super::new_id();
    let mut seed = [0u8; 8];
    rand::fill(&mut seed);
    streams
        .core
        .arena()
        .practice(id.clone(), by, u64::from_le_bytes(seed), Instant::now());
    Json(Created { id }).into_response()
}

fn owned_by(streams: &Streams, id: &str, profile: ProfileId) -> bool {
    streams.core.arena().challenger_of(id) == Some(profile)
}

async fn challenger_view(State(streams): State<Streams>, Who(profile): Who, Path(id): Path<String>) -> Response {
    settle(&streams);
    if !owned_by(&streams, &id, profile) {
        return refused(&Refusal::NoSuchChallenge);
    }
    view(&streams, &id, Side::Challenger)
}

async fn withdraw(State(streams): State<Streams>, Who(profile): Who, Path(id): Path<String>) -> Response {
    if !owned_by(&streams, &id, profile) {
        return refused(&Refusal::NoSuchChallenge);
    }
    let effects = streams.core.arena().withdraw(&id, Instant::now());
    apply(&streams, effects);
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Presses {
    input: Input,
    /// Each Semmel's press in milliseconds after its golden moment on this
    /// device's screen, or null where it burnt.
    presses: [Option<f64>; SEMMELS],
}

impl Presses {
    fn seconds(&self) -> [Option<f64>; SEMMELS] {
        self.presses
            .map(|press| press.filter(|ms| ms.is_finite()).map(|ms| ms / 1000.0))
    }
}

async fn pressed(streams: &Streams, id: &str, side: Side, body: &Presses) -> Response {
    let result = streams
        .core
        .arena()
        .press(id, side, body.input, body.seconds(), Instant::now());
    match result {
        Ok(effects) => {
            let finished = streams
                .core
                .arena()
                .view(id, side, Instant::now())
                .is_some_and(|view| matches!(view, View::Done { .. }));
            apply(streams, effects);
            if finished {
                save_calibration(&streams.core).await;
            }
            view(streams, id, side)
        }
        Err(refusal) => refused(&refusal),
    }
}

async fn challenger_presses(
    State(streams): State<Streams>,
    Who(profile): Who,
    Path(id): Path<String>,
    Json(body): Json<Presses>,
) -> Response {
    if !owned_by(&streams, &id, profile) {
        return refused(&Refusal::NoSuchChallenge);
    }
    pressed(&streams, &id, Side::Challenger, &body).await
}

/// The seat a challenge names: "tv", or a stream's id.
fn seat_named(name: &str) -> Seat {
    if name == "tv" {
        Seat::Tv
    } else {
        Seat::Stream(name.to_string())
    }
}

/// The challenge waiting for the stream `seat`, if there is one.
fn seat_challenge(streams: &Streams, seat: &str) -> Option<String> {
    streams
        .core
        .arena()
        .for_seat(&Seat::Stream(seat.to_string()))
        .map(str::to_string)
}

async fn defender_view(State(streams): State<Streams>, Path(seat): Path<String>) -> Response {
    settle(&streams);
    match seat_challenge(&streams, &seat) {
        Some(id) => view(&streams, &id, Side::Defender),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

async fn accept(State(streams): State<Streams>, Path(seat): Path<String>) -> Response {
    settle(&streams);
    let Some(id) = seat_challenge(&streams, &seat) else {
        return refused(&Refusal::NoSuchChallenge);
    };
    let mut seed = [0u8; 8];
    rand::fill(&mut seed);
    let accepted = streams
        .core
        .arena()
        .accept(&id, u64::from_le_bytes(seed), Instant::now());
    match accepted {
        Ok(()) => {
            eprintln!("bake-off {}: accepted, the oven heats up", short(&id));
            view(&streams, &id, Side::Defender)
        }
        Err(refusal) => refused(&refusal),
    }
}

async fn drop_off(State(streams): State<Streams>, Path(seat): Path<String>) -> Response {
    let Some(id) = seat_challenge(&streams, &seat) else {
        return refused(&Refusal::NoSuchChallenge);
    };
    let dropped = streams.core.arena().drop_off(&id, Instant::now());
    match dropped {
        Ok(effects) => {
            eprintln!("bake-off {}: the person challenged stopped watching", short(&id));
            apply(&streams, effects);
            StatusCode::NO_CONTENT.into_response()
        }
        Err(refusal) => refused(&refusal),
    }
}

async fn defender_presses(
    State(streams): State<Streams>,
    Path(seat): Path<String>,
    Json(body): Json<Presses>,
) -> Response {
    let Some(id) = seat_challenge(&streams, &seat) else {
        return refused(&Refusal::NoSuchChallenge);
    };
    pressed(&streams, &id, Side::Defender, &body).await
}

/// Tells the arena that the stream `id` ended on its own.
pub fn seat_gone(streams: &Streams, id: &str) {
    let effects = streams
        .core
        .arena()
        .seat_gone(&Seat::Stream(id.to_string()), Instant::now());
    apply(streams, effects);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presses_arrive_in_milliseconds_and_count_in_seconds() {
        let body: Presses = serde_json::from_str(r#"{"input":"remote","presses":[220,-35.5,null]}"#).unwrap();
        assert_eq!(body.input, Input::Remote);
        assert_eq!(body.seconds(), [Some(0.22), Some(-0.0355), None]);
        assert!(serde_json::from_str::<Presses>(r#"{"input":"touch","presses":[1,2]}"#).is_err());
    }

    #[test]
    fn the_full_ovens_answer_names_who_watches_and_how_long_the_dough_rests() {
        let answer = FullAnswer {
            error: "Three phones are already watching".to_string(),
            full: Full {
                game: GAME,
                heading: FULL_HEADING,
                message: "Three phones are already watching".to_string(),
                watchers: vec![Listed {
                    watcher: Watcher {
                        seat: Seat::Stream("abc".to_string()),
                        profile: ProfileId(3),
                        person: "Ramu".to_string(),
                        device: "Ramu's iPhone".to_string(),
                        title: "Dune".to_string(),
                    },
                    busy: false,
                }],
                resting_ms: Some(1_000),
            },
        };
        assert_eq!(
            serde_json::to_value(&answer).unwrap(),
            serde_json::json!({
                "error": "Three phones are already watching",
                "full": {
                    "game": "Semmelstechen",
                    "heading": "Der Ofen ist voll",
                    "message": "Three phones are already watching",
                    "watchers": [{
                        "seat": { "kind": "stream", "id": "abc" },
                        "profile": 3,
                        "person": "Ramu",
                        "device": "Ramu's iPhone",
                        "title": "Dune",
                        "busy": false
                    }],
                    "resting_ms": 1000
                }
            })
        );
    }
}
