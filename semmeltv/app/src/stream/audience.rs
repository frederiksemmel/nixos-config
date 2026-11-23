//! Who watches right now, on the TV or on a phone: beast asks before a deploy
//! and holds it until nobody does.

use super::Streams;
use axum::Json;
use axum::extract::State;
use serde::Serialize;
use std::time::{Duration, Instant};

/// A film paused this long on the TV counts as left, not watched.
pub const LEFT_PAUSED: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Viewer {
    pub person: String,
    pub device: String,
    pub title: String,
    pub paused: bool,
}

/// Since when the TV's film is paused, as far as anyone looked.
#[derive(Debug, Default)]
pub struct Pause {
    since: Option<Instant>,
}

impl Pause {
    /// Notes whether the TV's film is paused at `now`, and says whether it
    /// has been paused for so long that nobody watches it any more.
    pub fn left(&mut self, paused: bool, now: Instant) -> bool {
        if !paused {
            self.since = None;
            return false;
        }
        let since = *self.since.get_or_insert(now);
        now.duration_since(since) >= LEFT_PAUSED
    }
}

/// Looks at the TV's film, so a long pause counts from when it began.
pub fn look(streams: &Streams) {
    let paused = streams.core.now_playing().is_some_and(|playing| playing.paused);
    streams.pause.lock().unwrap().left(paused, Instant::now());
}

pub async fn viewers(streams: &Streams) -> Vec<Viewer> {
    let mut found = Vec::new();
    if let Some(playing) = streams.core.now_playing()
        && !streams.pause.lock().unwrap().left(playing.paused, Instant::now())
    {
        let person = streams.core.profile_name(streams.core.tv_profile()).await;
        found.push(Viewer {
            person: person.unwrap_or_else(|| "Someone".to_string()),
            device: "the TV".to_string(),
            title: playing.title,
            paused: playing.paused,
        });
    }
    let live: Vec<_> = streams.live.lock().unwrap().values().cloned().collect();
    for live in live {
        let person = streams.core.profile_name(live.profile).await;
        found.push(Viewer {
            person: person.unwrap_or_else(|| "Someone".to_string()),
            device: super::device_name(live.device.as_ref()).to_string(),
            title: live.name.clone(),
            paused: false,
        });
    }
    found
}

pub async fn watching(State(streams): State<Streams>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "watching": viewers(&streams).await }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_film_paused_for_an_hour_on_the_tv_counts_as_left() {
        let start = Instant::now();
        let mut pause = Pause::default();
        assert!(!pause.left(false, start));
        assert!(!pause.left(true, start));
        assert!(!pause.left(true, start + LEFT_PAUSED - Duration::from_secs(1)));
        assert!(pause.left(true, start + LEFT_PAUSED));
        assert!(!pause.left(false, start + LEFT_PAUSED), "playing again");
        assert!(!pause.left(true, start + LEFT_PAUSED * 2), "a new pause starts over");
    }
}
