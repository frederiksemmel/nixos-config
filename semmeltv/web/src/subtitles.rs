use crate::api::{Api, SubtitleSettings, SubtitleTrack, Subtitles};
use serde_json::{Value, json};
use topcoat::Result;
use topcoat::context::{Cx, app_context};
use topcoat::router::page;
use topcoat::runtime::{Event, procedure, shard, signal};
use topcoat::view::{View, component, view};

const OFF: i64 = -1;
const SLOTS: [&str; 3] = ["Preferred language", "Second choice", "Third choice"];
const LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("de", "German"),
    ("fr", "French"),
    ("es", "Spanish"),
    ("it", "Italian"),
    ("nl", "Dutch"),
    ("pt", "Portuguese"),
    ("sv", "Swedish"),
    ("da", "Danish"),
    ("no", "Norwegian"),
    ("fi", "Finnish"),
    ("is", "Icelandic"),
    ("pl", "Polish"),
    ("cs", "Czech"),
    ("sk", "Slovak"),
    ("hu", "Hungarian"),
    ("ro", "Romanian"),
    ("bg", "Bulgarian"),
    ("hr", "Croatian"),
    ("sr", "Serbian"),
    ("sl", "Slovenian"),
    ("el", "Greek"),
    ("tr", "Turkish"),
    ("ru", "Russian"),
    ("uk", "Ukrainian"),
    ("he", "Hebrew"),
    ("ar", "Arabic"),
    ("hi", "Hindi"),
    ("th", "Thai"),
    ("vi", "Vietnamese"),
    ("id", "Indonesian"),
    ("ms", "Malay"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
    ("zh", "Chinese"),
    ("ca", "Catalan"),
];

fn change(name: &str, value: &str) -> Option<Value> {
    let number = value.trim().parse::<f64>().ok().filter(|number| number.is_finite())?;
    match name {
        "size" => Some(json!({ "size": number / 100.0 })),
        "lift" => Some(json!({ "position": 100.0 - number })),
        "background" => Some(json!({ "background": number / 100.0 })),
        "outline" | "shadow" => Some(json!({ name: number })),
        _ => None,
    }
}

fn saved(result: std::result::Result<Subtitles, String>, applies: &str) -> String {
    match result {
        Ok(_) => format!("Saved{applies}"),
        Err(error) => error,
    }
}

#[procedure]
pub async fn set_number(cx: &Cx, name: String, value: String) -> Result<String> {
    Ok(match change(&name, &value) {
        Some(changes) => saved(app_context::<Api>(cx).change_subtitles(changes).await, ""),
        None => format!("Can't set {name} to {value}"),
    })
}

#[procedure]
pub async fn set_flag(cx: &Cx, name: String, on: bool) -> Result<String> {
    if !matches!(name.as_str(), "shown" | "restyle") {
        return Ok(format!("No such setting: {name}"));
    }
    let applies = if name == "shown" { " for the next video" } else { "" };
    Ok(saved(
        app_context::<Api>(cx).change_subtitles(json!({ name: on })).await,
        applies,
    ))
}

#[procedure]
pub async fn set_color(cx: &Cx, name: String, value: String) -> Result<String> {
    if !matches!(name.as_str(), "color" | "outline_color") {
        return Ok(format!("No such setting: {name}"));
    }
    Ok(saved(
        app_context::<Api>(cx).change_subtitles(json!({ name: value })).await,
        "",
    ))
}

fn reorder(current: &[String], slot: usize, code: &str) -> Vec<String> {
    let mut slots: Vec<Option<String>> = (0..SLOTS.len()).map(|index| current.get(index).cloned()).collect();
    if let Some(chosen) = slots.get_mut(slot) {
        *chosen = LANGUAGES
            .iter()
            .find(|(known, _)| *known == code)
            .map(|(known, _)| (*known).to_string());
    }
    let mut order: Vec<String> = Vec::new();
    for code in slots.into_iter().flatten() {
        if !order.contains(&code) {
            order.push(code);
        }
    }
    order
}

#[procedure]
pub async fn set_language(cx: &Cx, slot: u32, code: String) -> Result<String> {
    let api = app_context::<Api>(cx);
    let current = match api.subtitles().await {
        Ok(subtitles) => subtitles.settings.languages,
        Err(error) => return Ok(error),
    };
    let order = reorder(&current, slot as usize, &code);
    Ok(match api.change_subtitles(json!({ "languages": order })).await {
        Ok(subtitles) => subtitles.settings.languages.join(","),
        Err(_) => current.join(","),
    })
}

#[procedure]
pub async fn choose_track(cx: &Cx, track: i64) -> Result<String> {
    let track = (track != OFF).then_some(track);
    Ok(app_context::<Api>(cx).choose_subtitles(track).await)
}

fn track_name(track: &SubtitleTrack) -> String {
    let mut name = track.label.clone();
    if track.hearing_impaired {
        name.push_str(" (SDH)");
    }
    if track.forced {
        name.push_str(" (forced)");
    }
    name
}

fn track_detail(track: &SubtitleTrack) -> Option<String> {
    track.title.clone().filter(|title| {
        let said = title.to_ascii_lowercase();
        *title != track.label && !(track.hearing_impaired && said == "sdh") && !(track.forced && said == "forced")
    })
}

#[component]
async fn slider(
    cx: &Cx,
    name: &'static str,
    label: &'static str,
    unit: &'static str,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
) -> Result<impl View> {
    let shown = signal(cx, move || format!("{value}"));
    let said = signal(cx, String::new);
    Ok(view! {
        <label class="setting">
            <span class="setting-name">(label) <output>$(shown.get()) (unit)</output></span>
            <input type="range" min=(min) max=(max) step=(step) value=(value)
                @input=$(|e: Event| shown.set(e.target.value))
                @change=$(async |e: Event| {
                    let answer = set_number(name.to_owned(), e.target.value).await;
                    said.set(answer);
                })>
            <span class="setting-said" :hidden=$(said.get().is_empty())>$(said.get())</span>
        </label>
    })
}

#[component]
async fn colour(cx: &Cx, name: &'static str, label: &'static str, value: &str) -> Result<impl View> {
    let said = signal(cx, String::new);
    Ok(view! {
        <label class="setting setting-row">
            <span class="setting-name">(label)</span>
            <input type="color" value=(value.to_ascii_lowercase())
                @change=$(async |e: Event| {
                    let answer = set_color(name.to_owned(), e.target.value).await;
                    said.set(answer);
                })>
            <span class="setting-said" :hidden=$(said.get().is_empty())>$(said.get())</span>
        </label>
    })
}

#[component]
async fn switch(cx: &Cx, name: &'static str, label: &'static str, detail: &'static str, on: bool) -> Result<impl View> {
    let said = signal(cx, String::new);
    Ok(view! {
        <label class="setting setting-row">
            <span class="setting-name">(label) <small>(detail)</small></span>
            <input type="checkbox" class="switch" checked=(on)
                @change=$(async |e: Event| {
                    let answer = set_flag(name.to_owned(), e.target.checked).await;
                    said.set(answer);
                })>
            <span class="setting-said" :hidden=$(said.get().is_empty())>$(said.get())</span>
        </label>
    })
}

#[shard]
async fn language_order(cx: &Cx, initial: String) -> Result<impl View> {
    let order = signal(cx, move || initial);
    let current: Vec<String> = order
        .get()
        .split(',')
        .filter(|code| !code.is_empty())
        .map(str::to_string)
        .collect();
    Ok(view! {
        <div class="languages">
            #[key(slot)]
            for (slot, label) in SLOTS.iter().enumerate() {
                let picked = current.get(slot).cloned().unwrap_or_default();
                let number = slot as u32;
                <label class="setting setting-row">
                    <span class="setting-name">(*label)</span>
                    <select @change=$(async |e: Event| {
                        let answer = set_language(number, e.target.value).await;
                        order.set(answer);
                    })>
                        <option value="" selected=(picked.is_empty())>(if slot == 0 { "Any" } else { "Nothing else" })</option>
                        for &(code, name) in LANGUAGES {
                            <option value=(code) selected=(picked == code)>(name)</option>
                        }
                    </select>
                </label>
            }
        </div>
    })
}

#[component]
async fn tracks(cx: &Cx, subtitles: &Subtitles) -> Result<impl View> {
    let playing = subtitles.playing.clone();
    let initial = playing
        .as_ref()
        .and_then(|playing| playing.track.filter(|_| playing.visible))
        .unwrap_or(OFF);
    let track = signal(cx, move || initial);
    let said = signal(cx, String::new);
    Ok(view! {
        <section class="settings">
            <h2>"Now playing"</h2>
            match playing {
                None => <p class="setting-note">"Nothing is playing. While a video plays, its subtitle tracks show up here."</p>,
                Some(playing) => {
                    if playing.tracks.is_empty() {
                        <p class="setting-note">"This video has no subtitles."</p>
                    } else {
                        <div class="tracks">
                            <button type="button" :class=$(if track.get() == OFF { "track on" } else { "track" })
                                @click=$(async |_e| {
                                    track.set(OFF);
                                    let answer = choose_track(OFF).await;
                                    said.set(answer);
                                })>
                                <b>"Off"</b>
                            </button>
                            #[key(choice.id)]
                            for choice in &playing.tracks {
                                let id = choice.id;
                                <button type="button" :class=$(if track.get() == id { "track on" } else { "track" })
                                    @click=$(async |_e| {
                                        track.set(id);
                                        let answer = choose_track(id).await;
                                        said.set(answer);
                                    })>
                                    <b>(track_name(choice))</b>
                                    if let Some(detail) = track_detail(choice) {
                                        <span>(detail)</span>
                                    }
                                </button>
                            }
                        </div>
                        <p class="setting-said" :hidden=$(said.get().is_empty())>$(said.get())</p>
                    }
                },
            }
        </section>
    })
}

#[component]
async fn style(settings: &SubtitleSettings) -> Result<impl View> {
    Ok(view! {
        <section class="settings">
            <h2>"Look"</h2>
            <div class="setting-group">
            slider(name: "size", label: "Size", unit: "%", value: (settings.size * 100.0).round(), min: 50.0, max: 250.0, step: 5.0)
            slider(name: "lift", label: "Height above the bottom", unit: "%", value: 100.0 - settings.position, min: 0.0, max: 50.0, step: 1.0)
            colour(name: "color", label: "Text colour", value: &settings.color)
            slider(name: "outline", label: "Outline", unit: "", value: settings.outline, min: 0.0, max: 6.0, step: 0.05)
            colour(name: "outline_color", label: "Outline colour", value: &settings.outline_color)
            slider(name: "shadow", label: "Shadow", unit: "", value: settings.shadow, min: 0.0, max: 6.0, step: 0.5)
            slider(name: "background", label: "Background box", unit: "%", value: (settings.background * 100.0).round(), min: 0.0, max: 100.0, step: 5.0)
            switch(name: "restyle", label: "Restyle styled subtitles", detail: "Anime and other subtitles with their own fonts and colours", on: settings.restyle)
            </div>
        </section>
    })
}

#[page("/subtitles")]
async fn subtitles_page(cx: &Cx) -> Result<impl View> {
    let subtitles = app_context::<Api>(cx).subtitles().await.map_err(topcoat::Error::msg)?;
    let settings = subtitles.settings.clone();
    Ok(view! {
        <article class="subtitles">
            <h1>"Subtitles"</h1>
            <p class="setting-note">"The yellow key on the remote shows and hides them while a video plays."</p>
            tracks(subtitles: &subtitles)
            <section class="settings">
                <h2>"Every video"</h2>
                <div class="setting-group">
                    switch(name: "shown", label: "Show when a video starts", detail: "", on: settings.shown)
                    language_order(initial: settings.languages.join(","))
                </div>
            </section>
            style(settings: &settings)
        </article>
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(list: &[&str]) -> Vec<String> {
        list.iter().map(|code| (*code).to_string()).collect()
    }

    #[test]
    fn sliders_become_the_api_settings() {
        assert_eq!(change("size", "125"), Some(json!({ "size": 1.25 })));
        assert_eq!(change("lift", "10"), Some(json!({ "position": 90.0 })));
        assert_eq!(change("background", "40"), Some(json!({ "background": 0.4 })));
        assert_eq!(change("outline", "2.5"), Some(json!({ "outline": 2.5 })));
        assert_eq!(change("size", "big"), None);
        assert_eq!(change("font", "3"), None);
    }

    #[test]
    fn a_language_slot_replaces_its_language_and_drops_repeats() {
        let order = codes(&["en", "de"]);
        assert_eq!(reorder(&order, 0, "de"), codes(&["de"]));
        assert_eq!(reorder(&order, 1, "fr"), codes(&["en", "fr"]));
        assert_eq!(reorder(&order, 2, "es"), codes(&["en", "de", "es"]));
        assert_eq!(reorder(&order, 0, ""), codes(&["de"]));
        assert_eq!(reorder(&order, 1, "klingon"), codes(&["en"]));
    }
}
