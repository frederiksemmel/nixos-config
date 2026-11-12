use crate::audio;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

const BACKLOG: usize = 64;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Event {
    Voice(Voice),
    Playback { now_playing: Option<NowPlaying> },
    Audio(audio::View),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Voice {
    Listening,
    Transcribing,
    Heard {
        text: String,
        action: String,
        understood: bool,
    },
    Nothing,
    Cancelled,
    NoMicrophone,
    Failed {
        message: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct NowPlaying {
    pub title: String,
    pub caption: String,
    pub movie: Option<String>,
    pub show: Option<String>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub position: f64,
    pub duration: f64,
    pub paused: bool,
    pub muted: bool,
    pub speed: f64,
    pub subtitle: Option<i64>,
    pub subtitle_delay: f64,
    pub audio: Option<i64>,
    pub loading: bool,
    pub jump: Option<i32>,
}

impl NowPlaying {
    fn same_second(&self, other: &Self) -> bool {
        Self {
            position: self.position.floor(),
            duration: self.duration.floor(),
            ..self.clone()
        } == Self {
            position: other.position.floor(),
            duration: other.duration.floor(),
            ..other.clone()
        }
    }
}

#[derive(Clone)]
pub struct Bus {
    events: broadcast::Sender<Event>,
    now_playing: Arc<Mutex<Option<NowPlaying>>>,
    audio: Arc<Mutex<Option<audio::View>>>,
}

impl Default for Bus {
    fn default() -> Self {
        Self {
            events: broadcast::Sender::new(BACKLOG),
            now_playing: Arc::default(),
            audio: Arc::default(),
        }
    }
}

impl Bus {
    pub fn subscribe(&self) -> (Option<NowPlaying>, broadcast::Receiver<Event>) {
        let now_playing = self.now_playing.lock().unwrap();
        (now_playing.clone(), self.events.subscribe())
    }

    pub fn now_playing(&self) -> Option<NowPlaying> {
        self.now_playing.lock().unwrap().clone()
    }

    pub fn audio_state(&self) -> Option<audio::View> {
        self.audio.lock().unwrap().clone()
    }

    pub fn audio(&self, view: audio::View) {
        let mut current = self.audio.lock().unwrap();
        if current.as_ref() != Some(&view) {
            *current = Some(view.clone());
            let _ = self.events.send(Event::Audio(view));
        }
    }

    pub fn voice(&self, voice: Voice) {
        let _ = self.events.send(Event::Voice(voice));
    }

    pub fn playback(&self, now_playing: Option<NowPlaying>) {
        let mut current = self.now_playing.lock().unwrap();
        let unchanged = match (&*current, &now_playing) {
            (Some(old), Some(new)) => old.same_second(new),
            (None, None) => true,
            _ => false,
        };
        *current = now_playing.clone();
        if !unchanged {
            let _ = self.events.send(Event::Playback { now_playing });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn events_are_flat_json_objects_tagged_with_their_type() {
        let heard = Event::Voice(Voice::Heard {
            text: "play dune".to_string(),
            action: "Playing Dune".to_string(),
            understood: true,
        });
        assert_eq!(
            serde_json::to_value(&heard).unwrap(),
            json!({ "type": "voice", "state": "heard", "text": "play dune", "action": "Playing Dune", "understood": true })
        );
        let stopped = Event::Playback { now_playing: None };
        assert_eq!(
            serde_json::to_value(&stopped).unwrap(),
            json!({ "type": "playback", "now_playing": null })
        );
        assert_eq!(
            serde_json::to_value(Event::Voice(Voice::NoMicrophone)).unwrap(),
            json!({ "type": "voice", "state": "no-microphone" })
        );
    }

    #[test]
    fn playback_is_announced_once_per_second_of_progress() {
        let bus = Bus::default();
        let (_, mut events) = bus.subscribe();
        let at = |position| {
            Some(NowPlaying {
                title: "Dune".to_string(),
                position,
                duration: 9000.0,
                ..NowPlaying::default()
            })
        };
        bus.playback(at(10.1));
        bus.playback(at(10.6));
        bus.playback(at(11.2));
        bus.playback(None);
        let positions: Vec<_> = std::iter::from_fn(|| events.try_recv().ok())
            .map(|event| match event {
                Event::Playback { now_playing } => now_playing.map(|playing| playing.position),
                Event::Voice(_) | Event::Audio(_) => panic!("only playback events were sent"),
            })
            .collect();
        assert_eq!(positions, [Some(10.1), Some(11.2), None]);
        assert_eq!(bus.now_playing(), None);
    }
}
