use crate::api::{Api, NowPlaying, Pick, Voice};
use crate::live::{Live, VoiceLog};
use std::time::Duration;
use topcoat::Result;
use topcoat::asset::{Asset, asset};
use topcoat::context::{Cx, app_context};
use topcoat::runtime::{connected, link, procedure, shard};
use topcoat::view::{View, attributes, component, emit, live, view};

const SETTLE: Duration = Duration::from_secs(4);
const SCRUB: Asset = asset!("./scrub.js");
const DIRECTIONS: [&str; 2] = ["back", "forward"];
const ALLOWED_CONTROLS: [&str; 4] = ["pause", "resume", "toggle", "stop"];
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
pub async fn skip(cx: &Cx, direction: String) -> Result<String> {
    if !DIRECTIONS.contains(&direction.as_str()) {
        return Ok(format!("No such direction: {direction}"));
    }
    Ok(app_context::<Api>(cx).skip(&direction).await)
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

fn left(now: &NowPlaying) -> String {
    format!("-{}", clock(now.duration - now.position))
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

#[component]
pub async fn subtitles_icon() -> Result<impl View> {
    Ok(view! {
        <svg viewBox="0 0 24 24" aria-hidden="true">
            <rect x="3" y="5" width="18" height="14" rx="2.5"></rect>
            <path d="M7 12.5h4M13 12.5h4M7 15.5h7"></path>
        </svg>
    })
}

#[component]
pub async fn controls_icon() -> Result<impl View> {
    Ok(view! {
        <svg viewBox="0 0 24 24" aria-hidden="true">
            <rect x="7" y="2.5" width="10" height="19" rx="3"></rect>
            <circle cx="12" cy="7.5" r="1.6"></circle>
            <path d="M10 12.5h4M10 15.5h4M10 18.5h4"></path>
        </svg>
    })
}

#[shard]
pub async fn now_playing(cx: &Cx) -> Result<impl View> {
    Ok(view! {
        <section class="playing" aria-label="Now playing">
            <div class="scrub" role="slider" tabindex="0" aria-label="Position" aria-valuemin="0">track()</div>
            <div class="playing-row">
                <div class="playing-info">info()</div>
                <div class="playing-controls">
                    <button type="button" class="skip" data-skip="back" aria-label="Skip back"
                        @click=$(async |_e| { skip("back".to_owned()).await; })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M11 6 5 12l6 6M19 6l-6 6 6 6"></path></svg>
                        <span class="skip-amount" aria-hidden="true"></span>
                    </button>
                    <button type="button" class="playing-toggle" aria-label="Play or pause"
                        @click=$(async |_e| { control("toggle".to_owned()).await; })>
                        <svg class="when-paused" viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M7 4.5v15l12.5-7.5z"></path></svg>
                        <svg class="when-playing" viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M6.5 4.5h4v15h-4zM13.5 4.5h4v15h-4z"></path></svg>
                    </button>
                    <button type="button" class="skip" data-skip="forward" aria-label="Skip forward"
                        @click=$(async |_e| { skip("forward".to_owned()).await; })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m13 6 6 6-6 6M5 6l6 6-6 6"></path></svg>
                        <span class="skip-amount" aria-hidden="true"></span>
                    </button>
                    <button type="button" aria-label="Stop" @click=$(async |_e| { control("stop".to_owned()).await; })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M6 6h12v12H6z"></path></svg>
                    </button>
                    link(href: "/controls", attrs: attributes! { cx => class="playing-more" aria-label="Controls" }, controls_icon())
                </div>
            </div>
            <script type="module" src=(SCRUB)></script>
        </section>
    })
}

#[component]
async fn track(cx: &Cx) -> Result<impl View> {
    Ok(live! {
        let mut playback = app_context::<Live>(cx).playback.clone();
        loop {
            let now = playback.borrow_and_update().clone().unwrap_or_default();
            let at = format!("{:.2}%", percent(&now));
            let time = if now.duration >= 3600.0 { "scrub-time long" } else { "scrub-time" };
            let token = emit! {
                <span class=(time)>(clock(now.position))</span>
                <span class="scrub-rail"
                    data-position=(format!("{:.3}", now.position))
                    data-duration=(format!("{:.3}", now.duration))
                    data-paused=(if now.paused || now.loading { "1" } else { "0" })
                    data-jump=(now.jump.map(|jump| jump.to_string()).unwrap_or_default())>
                    <i class="scrub-fill" style=(format!("width:{at}"))></i>
                    <b class="scrub-thumb" style=(format!("left:{at}"))></b>
                    <output class="scrub-bubble"></output>
                </span>
                <span class=(time)>(left(&now))</span>
            }?;
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
                    let line = if now.loading { "Loading…".to_string() } else { now.caption.clone() };
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
        let mut settled = !lasting(&changes.borrow().state);
        loop {
            let log = changes.borrow_and_update().clone();
            let token = emit! { panel(log: &log, settled: settled) }?;
            if !connected(cx) {
                break Ok(token);
            }
            if settled || lasting(&log.state) || log.state.is_none() {
                if changes.changed().await.is_err() {
                    break Ok(token);
                }
                settled = false;
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

fn lasting(state: &Option<Voice>) -> bool {
    matches!(state, Some(Voice::Listening | Voice::Transcribing))
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

    #[test]
    fn the_time_left_counts_down_to_the_end() {
        let now = NowPlaying {
            position: 754.0,
            duration: 5400.0,
            ..NowPlaying::default()
        };
        assert_eq!(left(&now), "-1:17:26");
    }
}
