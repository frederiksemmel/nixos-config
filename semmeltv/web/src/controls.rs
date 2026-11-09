use crate::api::{Api, NowPlaying};
use crate::layout::back;
use crate::live::Live;
use crate::playing::{clock, control, seek, subtitles_icon};
use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::router::page;
use topcoat::runtime::{connected, link, procedure, signal};
use topcoat::view::{View, attributes, component, emit, live, view};

const TV_CONTROLS: [&str; 7] = ["on", "off", "semmeltv", "volume-up", "volume-down", "mute", "pair"];

#[procedure]
pub async fn tv(cx: &Cx, name: String) -> Result<String> {
    if !TV_CONTROLS.contains(&name.as_str()) {
        return Ok(format!("No such TV control: {name}"));
    }
    Ok(app_context::<Api>(cx).tv(&name).await)
}

fn now_line(now: Option<&NowPlaying>) -> String {
    match now {
        None => "Nothing is playing. Start something from the library first.".to_string(),
        Some(now) if now.loading => format!("Loading {}…", now.title),
        Some(now) => {
            let state = if now.paused { "Paused" } else { "Playing" };
            let at = format!("{} / {}", clock(now.position), clock(now.duration));
            format!("{state}: {} · {at}", now.title)
        }
    }
}

#[component]
async fn status(cx: &Cx) -> Result<impl View> {
    Ok(live! {
        let mut playback = app_context::<Live>(cx).playback.clone();
        loop {
            let line = now_line(playback.borrow_and_update().as_ref());
            let token = emit! { <p class="controls-status">(line)</p> }?;
            if !connected(cx) || playback.changed().await.is_err() {
                break Ok(token);
            }
        }
    })
}

#[page("/controls")]
async fn controls_page(cx: &Cx) -> Result<impl View> {
    let tv_said = signal(cx, String::new);
    let volume_said = signal(cx, String::new);
    let playback_said = signal(cx, String::new);
    let pairing_said = signal(cx, String::new);
    let abilities = app_context::<Api>(cx).tv_abilities().await.unwrap_or_default();
    Ok(view! {
        <article class="controls">
            back(href: "/")
            <h1>"Controls"</h1>
            <section class="settings">
                <h2>"TV"</h2>
                <div class="pad">
                    <button type="button" @click=$(async |_e| { tv_said.set(tv("on".to_owned()).await); })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3v8"></path><path d="M6.6 6.6a8 8 0 1 0 10.8 0"></path></svg>
                        <span>"On"</span>
                    </button>
                    <button type="button" @click=$(async |_e| { tv_said.set(tv("off".to_owned()).await); })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3v8" opacity="0.45"></path><path d="M6.6 6.6a8 8 0 1 0 10.8 0" opacity="0.45"></path><path d="M4 4l16 16"></path></svg>
                        <span>"Off"</span>
                    </button>
                    <button type="button" @click=$(async |_e| { tv_said.set(tv("semmeltv".to_owned()).await); })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="5" width="18" height="12" rx="2"></rect><path d="M8 20.5h8"></path><path d="m10.5 8.5 3.5 2.5-3.5 2.5z"></path></svg>
                        <span>"SemmelTV"</span>
                    </button>
                </div>
                <p class="controls-said" aria-live="polite">$(tv_said.get())</p>
            </section>
            <section class="settings">
                <h2>"TV volume"</h2>
                <div class="pad">
                    <button type="button" @click=$(async |_e| { volume_said.set(tv("volume-down".to_owned()).await); })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"></path><path d="M15.5 12h5"></path></svg>
                        <span>"Quieter"</span>
                    </button>
                    <button type="button" @click=$(async |_e| { volume_said.set(tv("mute".to_owned()).await); })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"></path><path d="m15.5 9.5 5 5M20.5 9.5l-5 5"></path></svg>
                        <span>"Mute"</span>
                    </button>
                    <button type="button" @click=$(async |_e| { volume_said.set(tv("volume-up".to_owned()).await); })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"></path><path d="M15.5 12h5M18 9.5v5"></path></svg>
                        <span>"Louder"</span>
                    </button>
                </div>
                <p class="controls-said" aria-live="polite">$(volume_said.get())</p>
                <p class="setting-note">"The TV's own volume, set over the network."</p>
            </section>
            if abilities.pairing {
                <section class="settings">
                    <h2>"Pairing"</h2>
                    <div class="pad">
                        <button type="button" @click=$(async |_e| {
                            pairing_said.set("Press Allow on the TV".to_owned());
                            pairing_said.set(tv("pair".to_owned()).await);
                        })>
                            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M9.5 14.5 14.5 9.5"></path><path d="M11 6.5 13 4.5a4 4 0 0 1 6.5 6.5l-2 2"></path><path d="M13 17.5 11 19.5a4 4 0 0 1-6.5-6.5l2-2"></path></svg>
                            <span>"Pair with TV"</span>
                        </button>
                    </div>
                    <p class="controls-said" aria-live="polite">$(pairing_said.get())</p>
                    <p class="setting-note">"Once per TV: the TV asks whether to allow SemmelTV, and its volume works after you press Allow."</p>
                </section>
            }
            <section class="settings">
                <h2>"Playback"</h2>
                status()
                <div class="pad four">
                    <button type="button" @click=$(async |_e| { playback_said.set(seek(-30i32).await); })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M11 6 5 12l6 6M19 6l-6 6 6 6"></path></svg>
                        <span>"Back 30s"</span>
                    </button>
                    <button type="button" @click=$(async |_e| { playback_said.set(control("toggle".to_owned()).await); })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M5 4.5v15l9-7.5zM15 4.5h3v15h-3z"></path></svg>
                        <span>"Play / pause"</span>
                    </button>
                    <button type="button" @click=$(async |_e| { playback_said.set(seek(30i32).await); })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m13 6 6 6-6 6M5 6l6 6-6 6"></path></svg>
                        <span>"Ahead 30s"</span>
                    </button>
                    <button type="button" @click=$(async |_e| { playback_said.set(control("stop".to_owned()).await); })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M6 6h12v12H6z"></path></svg>
                        <span>"Stop"</span>
                    </button>
                </div>
                <p class="controls-said" aria-live="polite">$(playback_said.get())</p>
            </section>
            link(href: "/subtitles", attrs: attributes! { cx => class="controls-row" },
                subtitles_icon()
                <span>"Subtitles"</span>
                <svg class="controls-row-arrow" viewBox="0 0 24 24" aria-hidden="true"><path d="m9 6 6 6-6 6"></path></svg>
            )
        </article>
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_says_when_nothing_plays() {
        assert!(now_line(None).starts_with("Nothing is playing"));
        let now = NowPlaying {
            title: "Isle of Dogs".to_string(),
            position: 65.0,
            duration: 6060.0,
            paused: true,
            ..NowPlaying::default()
        };
        assert_eq!(now_line(Some(&now)), "Paused: Isle of Dogs · 1:05 / 1:41:00");
    }
}
