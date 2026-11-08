use crate::api::{Api, NowPlaying, Pick, Voice};
use crate::live::{Live, VoiceLog};
use std::time::Duration;
use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::runtime::{connected, link, procedure, shard};
use topcoat::view::{View, attributes, component, emit, live, view};

const SETTLE: Duration = Duration::from_secs(4);
const ALLOWED_CONTROLS: [&str; 6] = ["pause", "resume", "toggle", "stop", "mute", "unmute"];
const LONGEST_COMMAND: usize = 200;

#[procedure]
pub async fn play_movie(cx: &Cx, key: String) -> Result<String> {
    Ok(app_context::<Api>(cx).play(&Pick::Movie { movie: key }).await)
}

#[procedure]
pub async fn play_show(cx: &Cx, key: String) -> Result<String> {
    let pick = Pick::Show {
        show: key,
        season: None,
        episode: None,
    };
    Ok(app_context::<Api>(cx).play(&pick).await)
}

#[procedure]
pub async fn play_episode(cx: &Cx, key: String, season: u32, episode: u32) -> Result<String> {
    let pick = Pick::Show {
        show: key,
        season: Some(season),
        episode: Some(episode),
    };
    Ok(app_context::<Api>(cx).play(&pick).await)
}

#[procedure]
pub async fn control(cx: &Cx, name: String) -> Result<String> {
    if !ALLOWED_CONTROLS.contains(&name.as_str()) {
        return Ok(format!("No such control: {name}"));
    }
    Ok(app_context::<Api>(cx).control(&name).await)
}

#[procedure]
pub async fn seek(cx: &Cx, seconds: i32) -> Result<String> {
    Ok(app_context::<Api>(cx)
        .seek_by(f64::from(seconds.clamp(-3600, 3600)))
        .await)
}

#[procedure]
pub async fn tell(cx: &Cx, text: String) -> Result<String> {
    let text: String = text.chars().take(LONGEST_COMMAND).collect();
    Ok(app_context::<Api>(cx).command(&text).await)
}

pub fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    match hours {
        0 => format!("{minutes}:{seconds:02}"),
        _ => format!("{hours}:{minutes:02}:{seconds:02}"),
    }
}

fn percent(now: &NowPlaying) -> f64 {
    if now.duration > 0.0 {
        (now.position / now.duration * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    }
}

fn destination(now: &NowPlaying) -> String {
    match (&now.movie, &now.show) {
        (Some(movie), _) => format!("/movies/{movie}"),
        (None, Some(show)) => format!("/shows/{show}"),
        (None, None) => "/".to_string(),
    }
}

#[shard]
pub async fn now_playing() -> Result<impl View> {
    Ok(view! {
        <section class="playing" aria-label="Now playing">
            <div class="playing-track">track()</div>
            <div class="playing-row">
                <div class="playing-info">info()</div>
                <div class="playing-controls">
                    <button type="button" aria-label="Back 30 seconds" @click=$(async |_e| { seek(-30i32).await; })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M11 6 5 12l6 6M19 6l-6 6 6 6"></path></svg>
                    </button>
                    <button type="button" class="playing-toggle" aria-label="Play or pause"
                        @click=$(async |_e| { control("toggle".to_owned()).await; })>
                        <svg class="when-paused" viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M7 4.5v15l12.5-7.5z"></path></svg>
                        <svg class="when-playing" viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M6.5 4.5h4v15h-4zM13.5 4.5h4v15h-4z"></path></svg>
                    </button>
                    <button type="button" aria-label="Forward 30 seconds" @click=$(async |_e| { seek(30i32).await; })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m13 6 6 6-6 6M5 6l6 6-6 6"></path></svg>
                    </button>
                    <button type="button" aria-label="Stop" @click=$(async |_e| { control("stop".to_owned()).await; })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M6 6h12v12H6z"></path></svg>
                    </button>
                </div>
            </div>
        </section>
    })
}

#[component]
async fn track(cx: &Cx) -> Result<impl View> {
    Ok(live! {
        let mut playback = app_context::<Live>(cx).playback.clone();
        loop {
            let width = playback.borrow_and_update().as_ref().map_or(0.0, percent);
            let token = emit! { <i style=(format!("width:{width:.2}%"))></i> }?;
            if !connected(cx) || playback.changed().await.is_err() {
                break Ok(token);
            }
        }
    })
}

#[component]
async fn info(cx: &Cx) -> Result<impl View> {
    Ok(live! {
        let mut playback = app_context::<Live>(cx).playback.clone();
        loop {
            let now = playback.borrow_and_update().clone();
            let token = match now {
                Some(now) => {
                    let line = if now.loading {
                        "Loading…".to_string()
                    } else {
                        let at = format!("{} / {}", clock(now.position), clock(now.duration));
                        match now.caption.as_str() {
                            "" => at,
                            caption => format!("{caption} · {at}"),
                        }
                    };
                    emit! {
                        link(href: destination(&now), attrs: attributes! { cx => class="playing-title" },
                            <strong>(now.title.as_str())</strong>
                            <span>(line)</span>
                            <i class=(if now.paused { "playing-state paused" } else { "playing-state" }) hidden=(true)></i>
                        )
                    }?
                }
                None => emit! { "" }?,
            };
            if !connected(cx) || playback.changed().await.is_err() {
                break Ok(token);
            }
        }
    })
}

#[shard]
pub async fn voice_feedback(cx: &Cx) -> Result<impl View> {
    Ok(live! {
        let mut changes = app_context::<Live>(cx).voice.clone();
        let mut settled = false;
        loop {
            let log = changes.borrow_and_update().clone();
            let token = emit! { panel(log: &log, settled: settled) }?;
            if !connected(cx) {
                break Ok(token);
            }
            let lasting = matches!(log.state, Some(Voice::Listening | Voice::Transcribing));
            settled = false;
            if lasting || settled_already(&log) {
                if changes.changed().await.is_err() {
                    break Ok(token);
                }
            } else {
                match tokio::time::timeout(SETTLE, changes.changed()).await {
                    Ok(Ok(())) => {}
                    Ok(Err(_)) => break Ok(token),
                    Err(_) => settled = true,
                }
            }
        }
    })
}

fn settled_already(log: &VoiceLog) -> bool {
    log.state.is_none()
}

fn state_text(voice: &Voice) -> Option<String> {
    match voice {
        Voice::Listening => Some("Listening…".to_string()),
        Voice::Transcribing => Some("Working it out…".to_string()),
        Voice::Nothing => Some("Didn't hear anything".to_string()),
        Voice::Cancelled => Some("Cancelled".to_string()),
        Voice::NoMicrophone => Some("The TV has no microphone".to_string()),
        Voice::Failed { message } => Some(format!("Voice control failed: {message}")),
        Voice::Heard { .. } => None,
    }
}

fn state_class(voice: &Voice) -> &'static str {
    match voice {
        Voice::Listening => "voice-state listening",
        Voice::Transcribing => "voice-state transcribing",
        _ => "voice-state",
    }
}

#[component]
async fn panel(log: &VoiceLog, settled: bool) -> Result<impl View> {
    let state = log
        .state
        .as_ref()
        .filter(|_| !settled)
        .and_then(|voice| state_text(voice).map(|text| (state_class(voice), text)));
    let latest = log.heard.front().cloned();
    let fresh = !settled && matches!(log.state, Some(Voice::Heard { .. }));
    Ok(view! {
        <section class="voice" aria-live="polite">
            if let Some((class, text)) = state {
                <p class=(class)>(text)</p>
            }
            if let Some(heard) = latest {
                if fresh {
                    <p class=(if heard.understood { "voice-heard" } else { "voice-heard refused" })>
                        <q>(heard.text)</q>
                        <span>(heard.action)</span>
                    </p>
                }
            }
        </section>
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_like_a_player() {
        assert_eq!(clock(65.4), "1:05");
        assert_eq!(clock(3725.0), "1:02:05");
        assert_eq!(clock(-3.0), "0:00");
    }
}
