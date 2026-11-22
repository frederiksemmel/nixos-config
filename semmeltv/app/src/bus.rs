use crate::audio;
use crate::command::Command;
use crate::library::Library;
use crate::profile::ProfileId;
use crate::progress::Progress;
use crate::screen;
use crate::segments::Kind;
use crate::up_next;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;

const BACKLOG: usize = 64;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Event {
    Voice(Voice),
    Playback {
        now_playing: Option<NowPlaying>,
    },
    Audio(audio::View),
    Screen {
        screen: screen::View,
    },
    #[serde(skip)]
    Home(Arc<Home>),
    #[serde(skip)]
    Key(Command),
    #[serde(skip)]
    Message(String),
    #[serde(skip)]
    Panel(Option<Panel>),
    #[serde(skip)]
    UpNext(Option<up_next::Card>),
    #[serde(skip)]
    Together(Together),
}

impl Event {
    pub fn for_phones(&self) -> bool {
        matches!(
            self,
            Self::Voice(_) | Self::Playback { .. } | Self::Audio(_) | Self::Screen { .. }
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Home {
    pub library: Arc<Library>,
    pub profile: ProfileId,
    pub people: Vec<(ProfileId, String)>,
    pub progress: Arc<Progress>,
    pub offline: bool,
    pub downloaded: Arc<HashSet<PathBuf>>,
    pub downloading: Option<PathBuf>,
    pub problem: Option<String>,
}

/// Who the TV's profile can watch `show` with: partners first, with where
/// they are, then everyone else.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Together {
    pub show: String,
    pub people: Vec<Mate>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mate {
    pub id: ProfileId,
    pub name: String,
    pub line: String,
    pub paired: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Panel {
    pub heading: String,
    pub status: String,
    pub hold: Option<Duration>,
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
    #[serde(flatten)]
    pub playing: Playing,
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
    pub skip: Option<Skip>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Skip {
    pub kind: Kind,
    pub label: &'static str,
}

impl From<Kind> for Skip {
    fn from(kind: Kind) -> Self {
        Self {
            kind,
            label: kind.label(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Playing {
    Movie {
        movie: String,
    },
    Episode {
        show: String,
        season: u32,
        episode: u32,
    },
    #[default]
    File,
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
    screen: Arc<Mutex<screen::View>>,
    home: Arc<Mutex<Arc<Home>>>,
}

impl Default for Bus {
    fn default() -> Self {
        Self {
            events: broadcast::Sender::new(BACKLOG),
            now_playing: Arc::default(),
            audio: Arc::default(),
            screen: Arc::default(),
            home: Arc::default(),
        }
    }
}

impl Bus {
    pub fn subscribe(&self) -> (Option<NowPlaying>, broadcast::Receiver<Event>) {
        let now_playing = self.now_playing.lock().unwrap();
        (now_playing.clone(), self.events.subscribe())
    }

    pub fn home_state(&self) -> Arc<Home> {
        self.home.lock().unwrap().clone()
    }

    pub fn home(&self, home: Home) {
        let mut current = self.home.lock().unwrap();
        if **current != home {
            let home = Arc::new(home);
            *current = home.clone();
            let _ = self.events.send(Event::Home(home));
        }
    }

    pub fn key(&self, command: Command) {
        let _ = self.events.send(Event::Key(command));
    }

    pub fn message(&self, text: String) {
        let _ = self.events.send(Event::Message(text));
    }

    pub fn together(&self, together: Together) {
        let _ = self.events.send(Event::Together(together));
    }

    pub fn panel(&self, panel: Option<Panel>) {
        let _ = self.events.send(Event::Panel(panel));
    }

    pub fn up_next(&self, card: Option<up_next::Card>) {
        let _ = self.events.send(Event::UpNext(card));
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

    pub fn screen_state(&self) -> screen::View {
        self.screen.lock().unwrap().clone()
    }

    pub fn screen(&self, view: screen::View) {
        let mut current = self.screen.lock().unwrap();
        if *current != view {
            *current = view.clone();
            let _ = self.events.send(Event::Screen { screen: view });
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
                _ => panic!("only playback events were sent"),
            })
            .collect();
        assert_eq!(positions, [Some(10.1), Some(11.2), None]);
        assert_eq!(bus.now_playing(), None);
    }

    #[test]
    fn the_skip_prompt_the_tv_shows_is_announced_at_once_and_named_like_on_the_tv() {
        let bus = Bus::default();
        let (_, mut events) = bus.subscribe();
        let at = |skip: Option<Kind>| {
            Some(NowPlaying {
                position: 30.2,
                skip: skip.map(Skip::from),
                ..NowPlaying::default()
            })
        };
        bus.playback(at(None));
        bus.playback(at(Some(Kind::Intro)));
        bus.playback(at(None));
        let skips: Vec<_> = std::iter::from_fn(|| events.try_recv().ok())
            .map(|event| serde_json::to_value(event).unwrap()["now_playing"]["skip"].clone())
            .collect();
        assert_eq!(
            skips,
            [
                json!(null),
                json!({ "kind": "intro", "label": "Skip intro" }),
                json!(null)
            ]
        );
        assert_eq!(
            serde_json::to_value(Skip::from(Kind::Recap)).unwrap(),
            json!({ "kind": "recap", "label": "Skip recap" })
        );
        assert_eq!(
            serde_json::to_value(Skip::from(Kind::Next)).unwrap(),
            json!({ "kind": "next", "label": "Play next" })
        );
    }

    #[test]
    fn what_plays_is_a_movie_an_episode_or_a_file_and_never_a_mix() {
        let json = |playing| {
            let value = serde_json::to_value(NowPlaying {
                playing,
                ..NowPlaying::default()
            })
            .unwrap();
            ["kind", "movie", "show", "season", "episode"].map(|key| value.get(key).cloned())
        };
        let movie = Playing::Movie {
            movie: "dune-2021".to_string(),
        };
        assert_eq!(
            json(movie),
            [Some(json!("movie")), Some(json!("dune-2021")), None, None, None]
        );
        let episode = Playing::Episode {
            show: "dark".to_string(),
            season: 2,
            episode: 3,
        };
        assert_eq!(
            json(episode),
            [
                Some(json!("episode")),
                None,
                Some(json!("dark")),
                Some(json!(2)),
                Some(json!(3))
            ]
        );
        assert_eq!(json(Playing::File), [Some(json!("file")), None, None, None, None]);
    }
}
