use serde_json::Value;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

const SETTLE: Duration = Duration::from_secs(2);
const TOLERANCE: f64 = 0.005;
const MAX_MULTIPLE: u32 = 5;
const UI_SIZE: (u64, u64) = (1920, 1080);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Video {
    pub width: u64,
    pub height: u64,
    pub fps: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mode {
    output: String,
    width: u64,
    height: u64,
    refresh: f64,
}

impl Mode {
    pub fn apply(&self) {
        let mode = format!("{}x{}@{:.6}Hz", self.width, self.height, self.refresh);
        let result = Command::new("wlr-randr")
            .args(["--output", &self.output, "--mode", &mode])
            .stdin(Stdio::null())
            .status();
        if let Err(error) = result {
            eprintln!("wlr-randr: {error}");
        }
    }
}

fn mismatch(refresh: f64, fps: f64) -> f64 {
    (1..=MAX_MULTIPLE)
        .map(|multiple| {
            let target = fps * f64::from(multiple);
            (refresh - target).abs() / target
        })
        .fold(f64::INFINITY, f64::min)
}

fn modes_of(output: &Value) -> Vec<(u64, u64, f64, bool)> {
    let Some(modes) = output["modes"].as_array() else {
        return Vec::new();
    };
    modes
        .iter()
        .filter_map(|mode| {
            Some((
                mode["width"].as_u64()?,
                mode["height"].as_u64()?,
                mode["refresh"].as_f64()?,
                mode["current"].as_bool().unwrap_or(false),
            ))
        })
        .collect()
}

struct Output {
    name: String,
    modes: Vec<(u64, u64, f64, bool)>,
}

impl Output {
    fn enabled(outputs: &Value) -> Option<Self> {
        let output = outputs
            .as_array()?
            .iter()
            .find(|output| output["enabled"].as_bool() == Some(true))?;
        Some(Self {
            name: output["name"].as_str()?.to_string(),
            modes: modes_of(output),
        })
    }

    fn change_to(&self, (width, height, refresh): (u64, u64, f64)) -> Option<Mode> {
        let current = self.modes.iter().find(|mode| mode.3)?;
        if (width, height, refresh) == (current.0, current.1, current.2) {
            return None;
        }
        Some(Mode {
            output: self.name.clone(),
            width,
            height,
            refresh,
        })
    }
}

fn choose(outputs: &Value, video: Video) -> Option<Mode> {
    let output = Output::enabled(outputs)?;
    let modes = &output.modes;
    let area = |mode: &(u64, u64, f64, bool)| mode.0 * mode.1;
    let covering: Vec<_> = modes
        .iter()
        .filter(|mode| mode.0 >= video.width && mode.1 >= video.height)
        .collect();
    let sizes = if covering.is_empty() {
        let largest = modes.iter().map(area).max()?;
        modes.iter().filter(|mode| area(mode) == largest).collect()
    } else {
        covering
    };
    let &&(width, height, refresh, _) = sizes
        .iter()
        .filter(|mode| mismatch(mode.2, video.fps) <= TOLERANCE)
        .min_by(|a, b| {
            area(a)
                .cmp(&area(b))
                .then(mismatch(a.2, video.fps).total_cmp(&mismatch(b.2, video.fps)))
                .then(a.2.total_cmp(&b.2))
        })?;
    output.change_to((width, height, refresh))
}

fn choose_ui(outputs: &Value) -> Option<Mode> {
    let output = Output::enabled(outputs)?;
    let &(width, height, refresh, _) = output
        .modes
        .iter()
        .filter(|mode| (mode.0, mode.1) == UI_SIZE)
        .max_by(|a, b| a.2.total_cmp(&b.2))?;
    output.change_to((width, height, refresh))
}

fn after_film(outputs: &Value) -> Option<Mode> {
    let output = Output::enabled(outputs)?;
    let &(width, height, _, _) = output.modes.iter().find(|mode| mode.3)?;
    if (width, height) == UI_SIZE {
        return None;
    }
    choose_ui(outputs)
}

fn outputs() -> Option<Value> {
    let output = Command::new("wlr-randr")
        .arg("--json")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    serde_json::from_slice(&output.stdout).ok()
}

pub fn match_video(video: Video) -> bool {
    let Some(matching) = outputs().and_then(|outputs| choose(&outputs, video)) else {
        return false;
    };
    matching.apply();
    thread::sleep(SETTLE);
    true
}

pub fn show_ui() {
    if let Some(mode) = outputs().and_then(|outputs| choose_ui(&outputs)) {
        mode.apply();
    }
}

pub fn leave_film() {
    if let Some(mode) = outputs().and_then(|outputs| after_film(&outputs)) {
        mode.apply();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tv() -> Value {
        json!([{
            "name": "HDMI-A-1",
            "enabled": true,
            "modes": [
                { "width": 3840, "height": 2160, "refresh": 30.0, "current": true },
                { "width": 3840, "height": 2160, "refresh": 29.969999, "current": false },
                { "width": 3840, "height": 2160, "refresh": 25.0, "current": false },
                { "width": 3840, "height": 2160, "refresh": 24.0, "current": false },
                { "width": 3840, "height": 2160, "refresh": 23.976, "current": false },
                { "width": 1920, "height": 1080, "refresh": 60.0, "current": false },
                { "width": 1920, "height": 1080, "refresh": 50.0, "current": false },
                { "width": 1920, "height": 1080, "refresh": 30.0, "current": false },
                { "width": 1920, "height": 1080, "refresh": 25.0, "current": false },
                { "width": 1920, "height": 1080, "refresh": 24.0, "current": false },
                { "width": 1920, "height": 1080, "refresh": 23.976, "current": false },
                { "width": 1280, "height": 720, "refresh": 60.0, "current": false },
            ],
        }])
    }

    fn video(width: u64, height: u64, fps: f64) -> Video {
        Video { width, height, fps }
    }

    #[test]
    fn each_video_gets_its_display_mode() {
        let cases = [
            ("1080p film", video(1920, 1080, 23.976024), Some((1920, 1080, 23.976))),
            ("letterboxed film", video(1920, 800, 24.0), Some((1920, 1080, 24.0))),
            ("4k film", video(3840, 2160, 23.976), Some((3840, 2160, 23.976))),
            (
                "larger than every mode",
                video(7680, 4320, 24.0),
                Some((3840, 2160, 24.0)),
            ),
            (
                "smaller, rate only at 1080p",
                video(1280, 720, 25.0),
                Some((1920, 1080, 25.0)),
            ),
            (
                "smaller, rate at its own size",
                video(1280, 720, 60.0),
                Some((1280, 720, 60.0)),
            ),
            (
                "slightly off its nominal rate",
                video(1920, 1080, 23.944723),
                Some((1920, 1080, 23.976)),
            ),
            (
                "exact 24 beats 23.976",
                video(3840, 2160, 24.0),
                Some((3840, 2160, 24.0)),
            ),
            (
                "exact 29.97 beats 30",
                video(3840, 2160, 29.97003),
                Some((3840, 2160, 29.969999)),
            ),
            ("the current mode", video(3840, 2160, 30.0), None),
            ("a multiple of the current rate", video(3840, 2160, 15.0), None),
            ("a rate the display lacks", video(3840, 2160, 50.0), None),
            ("a rate nothing comes near", video(1920, 1080, 1000.0), None),
        ];
        for (name, video, want) in cases {
            let got = choose(&tv(), video).map(|mode| (mode.width, mode.height, mode.refresh));
            assert_eq!(got, want, "{name}");
        }
    }

    #[test]
    fn the_ui_gets_1080p_at_its_fastest_rate() {
        let mut ui_set = tv();
        for mode in ui_set[0]["modes"].as_array_mut().unwrap() {
            let ui = mode["width"] == 1920 && mode["refresh"] == 60.0;
            mode["current"] = json!(ui);
        }
        let no_1080p = json!([{
            "name": "HDMI-A-1",
            "enabled": true,
            "modes": [{ "width": 1280, "height": 720, "refresh": 60.0, "current": true }],
        }]);
        let cases = [
            ("4k now", tv(), Some((1920, 1080, 60.0))),
            ("the ui mode already set", ui_set, None),
            ("a display without 1080p", no_1080p, None),
        ];
        for (name, outputs, want) in cases {
            let got = choose_ui(&outputs).map(|mode| (mode.width, mode.height, mode.refresh));
            assert_eq!(got, want, "{name}");
        }
    }

    #[test]
    fn after_a_film_the_menu_stays_in_its_1080p_mode() {
        let current = |width: u64, refresh: f64| {
            let mut outputs = tv();
            for mode in outputs[0]["modes"].as_array_mut().unwrap() {
                mode["current"] = json!(mode["width"] == width && mode["refresh"] == refresh);
            }
            outputs
        };
        let cases = [
            ("a 1080p 23.976 film", current(1920, 23.976), None),
            ("a 1080p 25 film", current(1920, 25.0), None),
            ("the menu's own mode", current(1920, 60.0), None),
            ("a 4k film", current(3840, 23.976), Some((1920, 1080, 60.0))),
            ("a 720p film", current(1280, 60.0), Some((1920, 1080, 60.0))),
        ];
        for (name, outputs, want) in cases {
            let got = after_film(&outputs).map(|mode| (mode.width, mode.height, mode.refresh));
            assert_eq!(got, want, "{name}");
        }
    }
}
