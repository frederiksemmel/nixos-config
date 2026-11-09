use serde_json::Value;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

const SETTLE: Duration = Duration::from_secs(2);
const TOLERANCE: f64 = 0.005;
const MAX_MULTIPLE: u32 = 5;

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

fn choose(outputs: &Value, video: Video) -> Option<(Mode, Mode)> {
    let output = outputs
        .as_array()?
        .iter()
        .find(|output| output["enabled"].as_bool() == Some(true))?;
    let name = output["name"].as_str()?.to_string();
    let modes = modes_of(output);
    let current = modes.iter().find(|mode| mode.3)?;
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
    if (width, height, refresh) == (current.0, current.1, current.2) {
        return None;
    }
    let mode = |width, height, refresh| Mode {
        output: name.clone(),
        width,
        height,
        refresh,
    };
    Some((mode(width, height, refresh), mode(current.0, current.1, current.2)))
}

pub fn match_video(video: Video) -> Option<Mode> {
    let output = Command::new("wlr-randr")
        .arg("--json")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let outputs = serde_json::from_slice(&output.stdout).ok()?;
    let (matching, previous) = choose(&outputs, video)?;
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

    fn hd(fps: f64) -> Video {
        Video {
            width: 1920,
            height: 1080,
            fps,
        }
    }

    fn uhd(fps: f64) -> Video {
        Video {
            width: 3840,
            height: 2160,
            fps,
        }
    }

    #[test]
    fn a_1080p_film_plays_at_1080p_and_its_exact_rate_and_remembers_the_old_mode() {
        let (matching, previous) = choose(&tv(), hd(23.976024)).unwrap();
        assert_eq!(
            (matching.width, matching.height, matching.refresh),
            (1920, 1080, 23.976)
        );
        assert_eq!((previous.width, previous.height, previous.refresh), (3840, 2160, 30.0));
    }

    #[test]
    fn a_letterboxed_film_still_gets_1080p() {
        let video = Video {
            width: 1920,
            height: 800,
            fps: 24.0,
        };
        let (matching, _) = choose(&tv(), video).unwrap();
        assert_eq!((matching.width, matching.height, matching.refresh), (1920, 1080, 24.0));
    }

    #[test]
    fn a_4k_film_keeps_4k() {
        let (matching, _) = choose(&tv(), uhd(23.976)).unwrap();
        assert_eq!((matching.width, matching.height), (3840, 2160));
    }

    #[test]
    fn a_video_larger_than_every_mode_gets_the_largest() {
        let video = Video {
            width: 7680,
            height: 4320,
            fps: 24.0,
        };
        let (matching, _) = choose(&tv(), video).unwrap();
        assert_eq!((matching.width, matching.height, matching.refresh), (3840, 2160, 24.0));
    }

    #[test]
    fn a_smaller_video_gets_the_smallest_mode_with_its_rate() {
        let small = Video {
            width: 1280,
            height: 720,
            fps: 25.0,
        };
        let (matching, _) = choose(&tv(), small).unwrap();
        assert_eq!((matching.width, matching.height, matching.refresh), (1920, 1080, 25.0));
        let (matching, _) = choose(&tv(), Video { fps: 60.0, ..small }).unwrap();
        assert_eq!((matching.width, matching.height), (1280, 720));
    }

    #[test]
    fn a_film_slightly_off_its_nominal_rate_gets_the_nearest_mode() {
        let (matching, _) = choose(&tv(), hd(23.944723)).unwrap();
        assert_eq!(matching.refresh, 23.976);
    }

    #[test]
    fn an_exact_rate_beats_a_close_one() {
        assert_eq!(choose(&tv(), uhd(24.0)).unwrap().0.refresh, 24.0);
        assert_eq!(choose(&tv(), uhd(29.97003)).unwrap().0.refresh, 29.969999);
    }

    #[test]
    fn a_mode_the_display_already_has_changes_nothing() {
        assert_eq!(choose(&tv(), uhd(30.0)), None);
        assert_eq!(choose(&tv(), uhd(15.0)), None);
    }

    #[test]
    fn a_rate_the_display_lacks_changes_nothing() {
        assert_eq!(choose(&tv(), uhd(50.0)), None);
        assert_eq!(choose(&tv(), hd(1000.0)), None);
    }
}
