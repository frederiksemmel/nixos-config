use crate::api::{Api, Audio, Link, NowPlaying, Output, Pick, TvAbilities, Voice};
use crate::live::Live;
use crate::public::Public;
use std::time::Duration;
use topcoat::Result;
use topcoat::asset::{Asset, asset};
use topcoat::context::{Cx, app_context};
use topcoat::runtime::{connected, link, procedure, shard};
use topcoat::view::{View, attributes, component, emit, live, view};

const SETTLE: Duration = Duration::from_secs(4);
const FRESH: Duration = Duration::from_secs(10);
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

pub fn fetch_body(now: &NowPlaying) -> String {
    match (&now.movie, &now.show, now.season, now.episode) {
        (Some(movie), _, _, _) => serde_json::json!({ "movie": movie }).to_string(),
        (None, Some(show), Some(season), Some(episode)) => {
            serde_json::json!({ "show": show, "season": season, "episode": episode }).to_string()
        }
        _ => String::new(),
    }
}

fn delay_value(delay: f64) -> String {
    format!("{delay:.1}")
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
    delay: String,
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
        delay: delay_value(now.subtitle_delay),
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
pub async fn tv_icon() -> Result<impl View> {
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
                    <span class="bar-phones" aria-label="Sound on headphones">headphones_icon()</span>
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
                                <div class="delay" hidden=(true)>
                                    <span>"Timing"</span>
                                    <button type="button" class="choice" data-post="/api/subtitles/delay" data-step="-0.5" aria-label="Show subtitles half a second earlier">"−0.5 s"</button>
                                    <output aria-live="polite">"0.0 s"</output>
                                    <button type="button" class="choice" data-post="/api/subtitles/delay" data-step="0.5" aria-label="Show subtitles half a second later">"+0.5 s"</button>
                                </div>
                                <button type="button" class="pill quiet find-subtitles" data-post="/api/subtitles/fetch" data-playing="true"
                                    data-pending="Looking for subtitles…" data-refresh="tracks" hidden=(true)>"Find subtitles"</button>
                            </div>
                            <div class="pick" data-tracks="audio" hidden=(true)>
                                <h3>"Audio"</h3>
                                <div class="choices" data-state="audio"></div>
                            </div>
                            <button type="button" class="pill handoff" data-handoff="phone">
                                <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="7" y="2.5" width="10" height="19" rx="2.2"></rect><path d="M11 18.5h2"></path></svg>
                                "Continue on phone"
                            </button>
                            <button type="button" class="stop" data-post="/api/playback/stop">
                                <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M7 7h10v10H7z"></path></svg>
                                "Stop playing"
                            </button>
                        </section>
                        sound()
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
pub async fn headphones_icon() -> Result<impl View> {
    Ok(view! {
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 15v-3a8 8 0 0 1 16 0v3"></path><path class="solid" d="M4 14.5h3.5v6H5.5a1.5 1.5 0 0 1-1.5-1.5zM20 14.5h-3.5v6h2a1.5 1.5 0 0 0 1.5-1.5z"></path></svg>
    })
}

#[derive(Debug, PartialEq)]
struct Sound {
    on_headphones: bool,
    connecting: bool,
    name: String,
    note: String,
}

fn sound_of(audio: &Audio) -> Option<Sound> {
    let headphones = audio.headphones.as_ref()?;
    let connecting = headphones.state == Link::Connecting;
    let name = headphones.name.clone();
    let note = if connecting { format!("Connecting to {name}…") } else { String::new() };
    Some(Sound {
        on_headphones: audio.output == Output::Headphones,
        connecting,
        name,
        note,
    })
}

#[component]
async fn sound(cx: &Cx) -> Result<impl View> {
    Ok(live! {
        let mut audio = app_context::<Live>(cx).audio.clone();
        loop {
            let state = audio.borrow_and_update().as_ref().and_then(sound_of);
            let token = match state {
                Some(sound) => {
                    let on_tv = !sound.on_headphones && !sound.connecting;
                    let waiting = sound.connecting && !sound.on_headphones;
                    emit! {
                        <section class="pick sound" aria-label="Sound" data-output=(if sound.on_headphones { "headphones" } else { "tv" })>
                            <h3>"Sound"</h3>
                            <div class="choices outputs">
                                <button type="button" class="choice" aria-pressed=(on_tv.to_string())
                                    data-post="/api/audio" data-body=r#"{"output":"tv"}"#>
                                    tv_icon()
                                    <span>"TV"</span>
                                </button>
                                <button type="button" class=(if waiting { "choice pending" } else { "choice" })
                                    aria-pressed=(sound.on_headphones.to_string())
                                    data-post="/api/audio" data-body=r#"{"output":"headphones"}"#>
                                    headphones_icon()
                                    <span>(sound.name.as_str())</span>
                                </button>
                            </div>
                            <p class="answer" aria-live="polite">(sound.note.as_str())</p>
                        </section>
                    }?
                }
                None => emit! {}?,
            };
            if !connected(cx) || audio.changed().await.is_err() {
                break Ok(token);
            }
        }
    })
}

#[derive(Debug, PartialEq)]
struct VolumeLook {
    label: String,
    level: Option<u8>,
    muted: bool,
}

fn volume_look(audio: Option<&Audio>) -> Option<VolumeLook> {
    let headphones = audio
        .filter(|audio| audio.output == Output::Headphones)
        .and_then(|audio| audio.headphones.as_ref())?;
    Some(VolumeLook {
        label: format!("{} volume", headphones.name),
        level: headphones.volume.map(|volume| volume.level),
        muted: headphones.volume.is_some_and(|volume| volume.muted),
    })
}

#[component]
async fn volume_keys(cx: &Cx, tv: bool) -> Result<impl View> {
    Ok(live! {
        let mut audio = app_context::<Live>(cx).audio.clone();
        loop {
            let look = volume_look(audio.borrow_and_update().as_ref());
            let shown = look.is_some() || tv;
            let label = look.as_ref().map_or_else(|| "TV volume".to_string(), |look| look.label.clone());
            let reading = match &look {
                Some(VolumeLook { muted: true, .. }) => "Muted".to_string(),
                Some(VolumeLook { level: Some(level), .. }) => format!("{level}%"),
                _ => String::new(),
            };
            let width = look.as_ref().and_then(|look| look.level).filter(|_| !look.as_ref().is_some_and(|look| look.muted)).unwrap_or(0);
            let token = if shown {
                emit! {
                    <div class="volume-block">
                        <div class="volume-head">
                            <span>(label.as_str())</span>
                            <output aria-live="polite">(reading.as_str())</output>
                        </div>
                        if look.as_ref().is_some_and(|look| look.level.is_some()) {
                            <span class="volume-level" aria-hidden="true"><i style=(format!("width:{width}%"))></i></span>
                        }
                        <div class="keys volume" role="group" aria-label=(label.as_str())>
                            <button type="button" class="key" data-post="/api/volume/down" aria-label="Quieter">
                                <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"></path><path d="M15.5 12h5"></path></svg>
                            </button>
                            <button type="button" class="key" data-post="/api/volume/mute" aria-label="Mute">
                                <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"></path><path d="m15.5 9.5 5 5M20.5 9.5l-5 5"></path></svg>
                            </button>
                            <button type="button" class="key" data-post="/api/volume/up" aria-label="Louder">
                                <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"></path><path d="M15.5 12h5M18 9.5v5"></path></svg>
                            </button>
                        </div>
                    </div>
                }?
            } else {
                emit! {}?
            };
            if !connected(cx) || audio.changed().await.is_err() {
                break Ok(token);
            }
        }
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
            volume_keys(tv: abilities.volume)
            if unpaired {
                <div class="pair">
                    <p>"The TV's volume works once the TV allows SemmelTV."</p>
                    <button type="button" class="pill" data-post="/api/tv/pair" data-pending="Press Allow on the TV">"Pair with the TV"</button>
                </div>
            }
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
                            data-subtitle=(track_state(now.subtitle)) data-audio=(track_state(now.audio))
                            data-delay=(delay_value(now.subtitle_delay)) data-fetch=(fetch_body(&now)) hidden=(true)></i>
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
                                        onerror="if (this.dataset.fallback && this.getAttribute('src') !== this.dataset.fallback) { this.src = this.dataset.fallback } else { this.remove() }">
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
        let mut settled = !lasting(&changes.borrow());
        loop {
            let state = changes.borrow_and_update().clone();
            let token = emit! { <i class="voice" data-state=(voice_state(&state, settled)) hidden=(true)></i> }?;
            if !connected(cx) {
                break Ok(token);
            }
            if settled || lasting(&state) || state.is_none() {
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

fn voice_state(state: &Option<Voice>, settled: bool) -> &'static str {
    match state {
        Some(Voice::Listening) => "listening",
        Some(Voice::Transcribing) => "transcribing",
        Some(_) if !settled => "done",
        _ => "",
    }
}

#[shard]
pub async fn notices(cx: &Cx) -> Result<impl View> {
    Ok(live! {
        let mut changes = app_context::<Live>(cx).notices.clone();
        loop {
            let fresh: Vec<_> = changes
                .borrow_and_update()
                .recent
                .iter()
                .filter(|notice| notice.at.elapsed() < FRESH)
                .cloned()
                .collect();
            let token = emit! {
                <div class="notices" hidden=(true)>
                    for notice in &fresh {
                        <i data-notice=(notice.id.as_str()) data-text=(notice.text.as_str())
                            data-detail=(notice.detail.as_str()) data-tone=(notice.tone.name())></i>
                    }
                </div>
            }?;
            if !connected(cx) || changes.changed().await.is_err() {
                break Ok(token);
            }
        }
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
