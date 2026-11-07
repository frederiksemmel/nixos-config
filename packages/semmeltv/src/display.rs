use serde_json::Value;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

const SETTLE: Duration = Duration::from_secs(2);
const TOLERANCE: f64 = 0.02;
const MAX_MULTIPLE: u32 = 5;

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

fn fits(refresh: f64, fps: f64) -> bool {
    (1..=MAX_MULTIPLE).any(|multiple| {
        let multiple = f64::from(multiple);
        (refresh - fps * multiple).abs() <= TOLERANCE * multiple
    })
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

fn choose(outputs: &Value, fps: f64) -> Option<(Mode, Mode)> {
    let output = outputs
        .as_array()?
        .iter()
        .find(|output| output["enabled"].as_bool() == Some(true))?;
    let name = output["name"].as_str()?.to_string();
    let modes = modes_of(output);
    let &(width, height, current, _) = modes.iter().find(|mode| mode.3)?;
    if fits(current, fps) {
        return None;
    }
    let refresh = modes
        .iter()
        .filter(|mode| mode.0 == width && mode.1 == height && fits(mode.2, fps))
        .map(|mode| mode.2)
        .max_by(f64::total_cmp)?;
    let mode = |refresh| Mode {
        output: name.clone(),
        width,
        height,
        refresh,
    };
    Some((mode(refresh), mode(current)))
}

pub fn match_frame_rate(fps: f64) -> Option<Mode> {
    let output = Command::new("wlr-randr").arg("--json").stdin(Stdio::null()).output().ok()?;
    let outputs = serde_json::from_slice(&output.stdout).ok()?;
    let (matching, previous) = choose(&outputs, fps)?;
    matching.apply();
    thread::sleep(SETTLE);
    Some(previous)
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
                { "width": 1920, "height": 1080, "refresh": 50.0, "current": false },
            ],
        }])
    }

    #[test]
    fn a_film_switches_the_tv_to_its_exact_rate_and_remembers_the_old_one() {
        let (matching, previous) = choose(&tv(), 23.976024).unwrap();
        assert_eq!(matching.refresh, 23.976);
        assert_eq!(previous.refresh, 30.0);
        assert_eq!(matching.width, 3840);
    }

    #[test]
    fn a_rate_the_display_already_fits_changes_nothing() {
        assert_eq!(choose(&tv(), 30.0), None);
        assert_eq!(choose(&tv(), 15.0), None);
    }

    #[test]
    fn a_rate_the_display_lacks_at_this_resolution_changes_nothing() {
        assert_eq!(choose(&tv(), 50.0), None);
        assert_eq!(choose(&tv(), 1000.0), None);
    }
}
