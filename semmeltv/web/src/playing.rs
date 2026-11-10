use crate::api::{Api, NowPlaying, Pick, TvAbilities, Voice};
use crate::live::{Live, VoiceLog};
use crate::public::Public;
use std::time::Duration;
use topcoat::Result;
use topcoat::asset::{Asset, asset};
use topcoat::context::{Cx, app_context};
use topcoat::runtime::{connected, link, procedure, shard};
use topcoat::view::{View, attributes, component, emit, live, view};

const SETTLE: Duration = Duration::from_secs(4);
const SCRUB: Asset = asset!("./scrub.js");
const PLAYER: Asset = asset!("./player.js");
const DIRECTIONS: [&str; 2] = ["back", "forward"];
const ALLOWED_CONTROLS: [&str; 4] = ["pause", "resume", "toggle", "stop"];
const LONGEST_COMMAND: usize = 200;
const SPEEDS: [(f64, &str); 5] = [(0.75, "0.75×"), (1.0, "1×"), (1.25, "1.25×"), (1.5, "1.5×"), (2.0, "2×")];

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

fn poster(now: &NowPlaying) -> Option<String> {
    match (&now.movie, &now.show) {
        (Some(movie), _) => Some(format!("/api/movies/{movie}/poster")),
        (None, Some(show)) => Some(format!("/api/shows/{show}/poster")),
        (None, None) => None,
    }
}

fn picture(now: &NowPlaying) -> Option<String> {
    match (&now.movie, &now.show, now.season, now.episode) {
        (Some(movie), _, _, _) => Some(format!("/api/movies/{movie}/backdrop")),
        (None, Some(show), Some(season), Some(episode)) => {
            Some(format!("/api/shows/{show}/episodes/{season}/{episode}/still"))
        }
        (None, Some(show), _, _) => Some(format!("/api/shows/{show}/backdrop")),
        _ => None,
    }
}

fn key(now: &NowPlaying) -> String {
    format!(
        "{}|{}|{}|{}",
        now.movie.as_deref().unwrap_or_default(),
        now.show.as_deref().unwrap_or_default(),
        now.season.unwrap_or_default(),
        now.episode.unwrap_or_default()
    )
}

fn line(now: &NowPlaying) -> String {
    if now.loading {
        "Loading…".to_string()
    } else {
        now.caption.clone()
    }
}

#[derive(PartialEq)]
struct Look {
    key: String,
    title: String,
    line: String,
    paused: bool,
    speed: String,
    subtitle: Option<i64>,
    audio: Option<i64>,
}

fn look(now: Option<&NowPlaying>) -> Option<Look> {
    now.map(|now| Look {
        key: key(now),
        title: now.title.clone(),
        line: line(now),
        paused: now.paused || now.loading,
        speed: speed_value(now.speed),
        subtitle: now.subtitle,
        audio: now.audio,
    })
}

fn track_state(chosen: Option<i64>) -> String {
    chosen.map_or_else(|| "off".to_string(), |id| id.to_string())
}

fn speed_value(speed: f64) -> String {
    format!("{speed:.2}")
}

fn now_class(now: &NowPlaying) -> &'static str {
    if now.paused || now.loading { "now-state paused" } else { "now-state" }
}

#[component]
pub async fn settings_icon() -> Result<impl View> {
    Ok(view! {
        <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M4 7h9M17 7h3M4 17h3M11 17h9"></path>
            <circle cx="15" cy="7" r="2.2"></circle>
            <circle cx="9" cy="17" r="2.2"></circle>
        </svg>
    })
}

#[component]
async fn tv_icon() -> Result<impl View> {
    Ok(view! {
        <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="5" width="18" height="12" rx="2"></rect><path d="M8 20.5h8"></path></svg>
    })
}

#[component]
async fn play_pause() -> Result<impl View> {
    Ok(view! {
        <svg class="when-paused" viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M7 4.5v15l12.5-7.5z"></path></svg>
        <svg class="when-playing" viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M6.5 4.5h4v15h-4zM13.5 4.5h4v15h-4z"></path></svg>
    })
}

#[shard]
pub async fn player(cx: &Cx) -> Result<impl View> {
    let abilities = app_context::<Api>(cx).tv_abilities().await.unwrap_or_default();
    Ok(view! {
        <div class="player">
            <section class="bar" aria-label="Now playing">
                bar_progress()
                <button type="button" class="bar-open" data-sheet="open" aria-haspopup="dialog" aria-controls="sheet">
                    bar_info()
                    <svg class="bar-chevron" viewBox="0 0 24 24" aria-hidden="true"><path d="m6 14.5 6-6 6 6"></path></svg>
                </button>
                <button type="button" class="bar-toggle" aria-label="Play or pause"
                    @click=$(async |_e| { control("toggle".to_owned()).await; })>
                    play_pause()
                </button>
            </section>
            <div class="sheet-dim" data-sheet="close" aria-hidden="true"></div>
            <div class="sheet" id="sheet" role="dialog" aria-modal="true" aria-label="Player and TV remote" inert=(true)>
                <div class="sheet-scroll">
                    <div class="sheet-top">
                        <button type="button" class="sheet-close" data-sheet="close" aria-label="Close">
                            <span class="sheet-grabber" aria-hidden="true"></span>
                        </button>
                    </div>
                    <div class="sheet-body">
                        sheet_now()
                        <section class="sheet-playback" aria-label="Playback">
                            <div class="scrub" role="slider" tabindex="0" aria-label="Position" aria-valuemin="0">track()</div>
                            <div class="transport">
                                <button type="button" class="skip" data-skip="back" aria-label="Skip back"
                                    @click=$(async |_e| { skip("back".to_owned()).await; })>
                                    <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M11 6 5 12l6 6M19 6l-6 6 6 6"></path></svg>
                                    <span class="skip-amount" aria-hidden="true"></span>
                                </button>
                                <button type="button" class="transport-toggle" aria-label="Play or pause"
                                    @click=$(async |_e| { control("toggle".to_owned()).await; })>
                                    play_pause()
                                </button>
                                <button type="button" class="skip" data-skip="forward" aria-label="Skip forward"
                                    @click=$(async |_e| { skip("forward".to_owned()).await; })>
                                    <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m13 6 6 6-6 6M5 6l6 6-6 6"></path></svg>
                                    <span class="skip-amount" aria-hidden="true"></span>
                                </button>
                            </div>
                            <div class="pick">
                                <h3>"Speed"</h3>
                                <div class="choices speeds" data-state="speed">
                                    for &(speed, label) in &SPEEDS {
                                        <button type="button" class="choice" data-value=(speed_value(speed))
                                            data-post="/api/playback/speed" data-body=(format!("{{\"speed\":{speed}}}"))>(label)</button>
                                    }
                                </div>
                            </div>
                            <div class="pick" data-tracks="subtitles" hidden=(true)>
                                <h3>"Subtitles"</h3>
                                <div class="choices" data-state="subtitle"></div>
                            </div>
                            <div class="pick" data-tracks="audio" hidden=(true)>
                                <h3>"Audio"</h3>
                                <div class="choices" data-state="audio"></div>
                            </div>
                            <p class="answer" data-said="true" aria-live="polite"></p>
                            <button type="button" class="stop" data-post="/api/playback/stop">
                                <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M7 7h10v10H7z"></path></svg>
                                "Stop playing"
                            </button>
                        </section>
                        remote(abilities: &abilities)
                    </div>
                </div>
            </div>
            <script type="module" src=(SCRUB)></script>
            <script type="module" src=(PLAYER)></script>
        </div>
    })
}

#[component]
async fn remote(abilities: &TvAbilities) -> Result<impl View> {
    let unpaired = abilities.paired == Some(false);
    Ok(view! {
        <section class="remote" aria-label="TV">
            <h3>"TV"</h3>
            <div class="keys">
                <button type="button" class="key" data-post="/api/tv/on">
                    <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3.5v7.5"></path><path d="M6.8 6.8a7.5 7.5 0 1 0 10.4 0"></path></svg>
                    <span>"On"</span>
                </button>
                <button type="button" class="key" data-post="/api/tv/off">
                    <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3.5v7.5" opacity="0.4"></path><path d="M6.8 6.8a7.5 7.5 0 1 0 10.4 0" opacity="0.4"></path><path d="M4.5 4.5l15 15"></path></svg>
                    <span>"Off"</span>
                </button>
                <button type="button" class="key" data-post="/api/tv/semmeltv">
                    tv_icon()
                    <span>"SemmelTV"</span>
                </button>
            </div>
            if abilities.volume {
                <div class="keys volume" role="group" aria-label="TV volume">
                    <button type="button" class="key" data-post="/api/tv/volume-down" aria-label="Quieter">
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"></path><path d="M15.5 12h5"></path></svg>
                    </button>
                    <button type="button" class="key" data-post="/api/tv/mute" aria-label="Mute">
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"></path><path d="m15.5 9.5 5 5M20.5 9.5l-5 5"></path></svg>
                    </button>
                    <button type="button" class="key" data-post="/api/tv/volume-up" aria-label="Louder">
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"></path><path d="M15.5 12h5M18 9.5v5"></path></svg>
                    </button>
                </div>
            }
            if unpaired {
                <div class="pair">
                    <p>"The TV's volume works once the TV allows SemmelTV."</p>
                    <button type="button" class="pill" data-post="/api/tv/pair" data-pending="Press Allow on the TV">"Pair with the TV"</button>
                </div>
            }
            <p class="answer" data-said="true" aria-live="polite"></p>
        </section>
    })
}

#[component]
async fn bar_progress(cx: &Cx) -> Result<impl View> {
    Ok(live! {
        let mut playback = app_context::<Live>(cx).playback.clone();
        loop {
            let width = playback.borrow_and_update().as_ref().map_or(0.0, percent);
            let token = emit! { <span class="bar-progress" aria-hidden="true"><i style=(format!("width:{width:.2}%"))></i></span> }?;
            if !connected(cx) || playback.changed().await.is_err() {
                break Ok(token);
            }
        }
    })
}

#[component]
async fn bar_info(cx: &Cx) -> Result<impl View> {
    let public = app_context::<Public>(cx).clone();
    Ok(live! {
        let mut playback = app_context::<Live>(cx).playback.clone();
        'shown: loop {
            let now = playback.borrow_and_update().clone();
            let shown = look(now.as_ref());
            let token = match now {
                Some(now) => {
                    let art = poster(&now).map(|path| public.url(&path));
                    emit! {
                        <span class="bar-art">
                            if let Some(art) = art {
                                <img src=(art) alt="" onerror="this.remove()">
                            }
                        </span>
                        <span class="bar-text">
                            <strong>(now.title.as_str())</strong>
                            <span>(line(&now))</span>
                        </span>
                        <i class=(now_class(&now)) data-key=(key(&now)) data-speed=(speed_value(now.speed))
                            data-subtitle=(track_state(now.subtitle)) data-audio=(track_state(now.audio)) hidden=(true)></i>
                    }?
                }
                None => emit! {
                    <span class="bar-art idle">tv_icon()</span>
                    <span class="bar-text">
                        <strong>"TV remote"</strong>
                        <span>"Nothing is playing"</span>
                    </span>
                }?,
            };
            loop {
                if !connected(cx) || playback.changed().await.is_err() {
                    break 'shown Ok(token);
                }
                if look(playback.borrow().as_ref()) != shown {
                    break;
                }
            }
        }
    })
}

#[component]
async fn sheet_now(cx: &Cx) -> Result<impl View> {
    let public = app_context::<Public>(cx).clone();
    Ok(live! {
        let mut playback = app_context::<Live>(cx).playback.clone();
        'shown: loop {
            let now = playback.borrow_and_update().clone();
            let shown = look(now.as_ref());
            let token = match now {
                Some(now) => {
                    let art = picture(&now).map(|path| public.url(&path));
                    let fallback = poster(&now).map(|path| public.url(&path.replace("/poster", "/backdrop")));
                    emit! {
                        <div class="now">
                            <div class="now-art">
                                if let Some(art) = art {
                                    <img src=(art) alt="" data-fallback=(fallback.unwrap_or_default())
                                        onerror="if (this.dataset.fallback && this.src !== this.dataset.fallback) { this.src = this.dataset.fallback } else { this.remove() }">
                                }
                            </div>
                            link(href: destination(&now), attrs: attributes! { cx => class="now-title" },
                                <h2>(now.title.as_str())</h2>
                                <span>(line(&now))</span>
                            )
                        </div>
                    }?
                }
                None => emit! {
                    <div class="now idle">
                        <h2>"TV remote"</h2>
                        <span>"Nothing is playing. Pick something from the library, or use the TV keys below."</span>
                    </div>
                }?,
            };
            loop {
                if !connected(cx) || playback.changed().await.is_err() {
                    break 'shown Ok(token);
                }
                if look(playback.borrow().as_ref()) != shown {
                    break;
                }
            }
        }
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
                    data-speed=(format!("{:.2}", now.speed))
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
        Voice::Nothing => Some("Didn't catch that".to_string()),
        Voice::Cancelled => Some("Cancelled".to_string()),
        Voice::NoMicrophone => Some("The TV has no microphone".to_string()),
        Voice::Failed { .. } => Some("Speech recognition isn't working right now".to_string()),
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
