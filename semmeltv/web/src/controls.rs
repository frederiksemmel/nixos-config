use crate::api::Api;
use crate::playing::{control, seek, subtitles_icon};
use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::router::page;
use topcoat::runtime::{link, procedure, signal};
use topcoat::view::{View, attributes, view};

const TV_CONTROLS: [&str; 6] = ["on", "off", "semmeltv", "volume-up", "volume-down", "mute"];

#[procedure]
pub async fn tv(cx: &Cx, name: String) -> Result<String> {
    if !TV_CONTROLS.contains(&name.as_str()) {
        return Ok(format!("No such TV control: {name}"));
    }
    Ok(app_context::<Api>(cx).tv(&name).await)
}

#[page("/controls")]
async fn controls_page(cx: &Cx) -> Result<impl View> {
    let said = signal(cx, String::new);
    Ok(view! {
        <article class="controls">
            <h1>"Controls"</h1>
            <p class="controls-said" aria-live="polite">$(said.get())</p>
            <section class="settings">
                <h2>"TV"</h2>
                <div class="pad">
                    <button type="button" @click=$(async |_e| { said.set(tv("on".to_owned()).await); })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3v8"></path><path d="M6.6 6.6a8 8 0 1 0 10.8 0"></path></svg>
                        <span>"On"</span>
                    </button>
                    <button type="button" @click=$(async |_e| { said.set(tv("off".to_owned()).await); })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3v8" opacity="0.45"></path><path d="M6.6 6.6a8 8 0 1 0 10.8 0" opacity="0.45"></path><path d="M4 4l16 16"></path></svg>
                        <span>"Off"</span>
                    </button>
                    <button type="button" @click=$(async |_e| { said.set(tv("semmeltv".to_owned()).await); })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3" y="5" width="18" height="12" rx="2"></rect><path d="M8 20.5h8"></path><path d="m10.5 8.5 3.5 2.5-3.5 2.5z"></path></svg>
                        <span>"SemmelTV"</span>
                    </button>
                </div>
            </section>
            <section class="settings">
                <h2>"TV volume"</h2>
                <div class="pad">
                    <button type="button" @click=$(async |_e| { said.set(tv("volume-down".to_owned()).await); })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"></path><path d="M15.5 12h5"></path></svg>
                        <span>"Quieter"</span>
                    </button>
                    <button type="button" @click=$(async |_e| { said.set(tv("mute".to_owned()).await); })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"></path><path d="m15.5 9.5 5 5M20.5 9.5l-5 5"></path></svg>
                        <span>"Mute"</span>
                    </button>
                    <button type="button" @click=$(async |_e| { said.set(tv("volume-up".to_owned()).await); })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"></path><path d="M15.5 12h5M18 9.5v5"></path></svg>
                        <span>"Louder"</span>
                    </button>
                </div>
                <p class="setting-note">"Pressed on the TV over HDMI-CEC, like its own remote's volume keys."</p>
            </section>
            <section class="settings">
                <h2>"Playback"</h2>
                <div class="pad">
                    <button type="button" @click=$(async |_e| { said.set(seek(-30i32).await); })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M11 6 5 12l6 6M19 6l-6 6 6 6"></path></svg>
                        <span>"Back 30s"</span>
                    </button>
                    <button type="button" @click=$(async |_e| { said.set(control("toggle".to_owned()).await); })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M5 4.5v15l9-7.5zM15 4.5h3v15h-3z"></path></svg>
                        <span>"Play / pause"</span>
                    </button>
                    <button type="button" @click=$(async |_e| { said.set(seek(30i32).await); })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m13 6 6 6-6 6M5 6l6 6-6 6"></path></svg>
                        <span>"Ahead 30s"</span>
                    </button>
                    <button type="button" @click=$(async |_e| { said.set(control("stop".to_owned()).await); })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M6 6h12v12H6z"></path></svg>
                        <span>"Stop"</span>
                    </button>
                    <button type="button" @click=$(async |_e| { said.set(control("mute".to_owned()).await); })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"></path><path d="m15.5 9.5 5 5M20.5 9.5l-5 5"></path></svg>
                        <span>"Mute video"</span>
                    </button>
                    <button type="button" @click=$(async |_e| { said.set(control("unmute".to_owned()).await); })>
                        <svg viewBox="0 0 24 24" aria-hidden="true"><path class="solid" d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"></path><path d="M15.5 9a4.5 4.5 0 0 1 0 6M18 6.5a8 8 0 0 1 0 11"></path></svg>
                        <span>"Unmute video"</span>
                    </button>
                </div>
            </section>
            link(href: "/subtitles", attrs: attributes! { cx => class="controls-row" },
                subtitles_icon()
                <span>"Subtitles"</span>
                <svg class="controls-row-arrow" viewBox="0 0 24 24" aria-hidden="true"><path d="m9 6 6 6-6 6"></path></svg>
            )
        </article>
    })
}
