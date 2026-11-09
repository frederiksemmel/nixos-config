use serde_json::{Value, json};
use std::fs;
use std::path::Path;

const DRM: &str = "/sys/class/drm";

pub fn connected() -> Option<bool> {
    let entries = fs::read_dir(DRM).ok()?;
    let statuses = entries.filter_map(Result::ok).filter_map(|entry| {
        let name = entry.file_name().into_string().ok()?;
        name.contains("-HDMI-")
            .then(|| fs::read_to_string(Path::new(DRM).join(&name).join("status")).ok())
            .flatten()
    });
    any_connected(statuses)
}

fn any_connected(statuses: impl IntoIterator<Item = String>) -> Option<bool> {
    let mut seen = false;
    for status in statuses {
        if status.trim() == "connected" {
            return Some(true);
        }
        seen = true;
    }
    seen.then_some(false)
}

#[derive(Default)]
pub struct Plug {
    was: Option<bool>,
}

impl Plug {
    pub fn plugged_in(&mut self, now: Option<bool>) -> bool {
        let edge = self.was == Some(false) && now == Some(true);
        if now.is_some() {
            self.was = now;
        }
        edge
    }
}

pub fn restore_audio(playing: Option<i64>, wanted: Option<i64>) -> Option<Value> {
    match (playing, wanted) {
        (Some(_), _) => Some(json!(["ao-reload"])),
        (None, Some(id)) => Some(json!(["set_property", "aid", id])),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn statuses(list: &[&str]) -> Vec<String> {
        list.iter().map(|status| format!("{status}\n")).collect()
    }

    #[test]
    fn any_connected_hdmi_port_counts() {
        assert_eq!(any_connected(statuses(&["disconnected", "connected"])), Some(true));
        assert_eq!(any_connected(statuses(&["disconnected", "disconnected"])), Some(false));
        assert_eq!(any_connected(statuses(&[])), None);
    }

    #[test]
    fn only_a_tv_coming_back_is_a_plug_in() {
        let mut plug = Plug::default();
        assert!(!plug.plugged_in(Some(true)), "connected from the start");
        assert!(!plug.plugged_in(Some(false)));
        assert!(!plug.plugged_in(None), "an unreadable status changes nothing");
        assert!(plug.plugged_in(Some(true)));
        assert!(!plug.plugged_in(Some(true)), "once per return");
        let mut late = Plug::default();
        assert!(!late.plugged_in(None));
        assert!(!late.plugged_in(Some(true)), "first reading after unreadable ones");
    }

    #[test]
    fn audio_comes_back_on_the_track_that_played() {
        assert_eq!(restore_audio(Some(1), Some(1)), Some(json!(["ao-reload"])));
        assert_eq!(restore_audio(None, Some(2)), Some(json!(["set_property", "aid", 2])));
        assert_eq!(restore_audio(None, None), None, "a file without audio stays silent");
    }
}
