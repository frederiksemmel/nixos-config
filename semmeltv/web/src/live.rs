use crate::api::{Audio, Event, NowPlaying, Voice};
use futures_util::StreamExt;
use std::collections::VecDeque;
use std::time::Duration;
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;

const RECONNECT: Duration = Duration::from_secs(2);
const HISTORY: usize = 5;

#[derive(Clone, Debug, PartialEq)]
pub struct Heard {
    pub text: String,
    pub action: String,
    pub understood: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VoiceLog {
    pub state: Option<Voice>,
    pub heard: VecDeque<Heard>,
}

impl VoiceLog {
    pub fn record(&mut self, voice: Voice) {
        if let Voice::Heard {
            text,
            action,
            understood,
        } = &voice
        {
            self.heard.push_front(Heard {
                text: text.clone(),
                action: action.clone(),
                understood: *understood,
            });
            self.heard.truncate(HISTORY);
        }
        self.state = Some(voice);
    }
}

#[derive(Clone)]
pub struct Live {
    pub playback: watch::Receiver<Option<NowPlaying>>,
    pub voice: watch::Receiver<VoiceLog>,
    pub audio: watch::Receiver<Option<Audio>>,
}

pub fn follow(url: String) -> Live {
    let (playback, playback_seen) = watch::channel(None);
    let (voice, voice_seen) = watch::channel(VoiceLog::default());
    let (audio, audio_seen) = watch::channel(None);
    tokio::spawn(async move {
        loop {
            match tokio_tungstenite::connect_async(&url).await {
                Ok((mut socket, _)) => {
                    while let Some(Ok(message)) = socket.next().await {
                        let Message::Text(text) = message else { continue };
                        match serde_json::from_str(&text) {
                            Ok(Event::Playback { now_playing }) => {
                                playback.send_if_modified(|current| replace(current, now_playing));
                            }
                            Ok(Event::Voice(event)) => voice.send_modify(|log| log.record(event)),
                            Ok(Event::Audio(state)) => {
                                audio.send_if_modified(|current| replace(current, Some(state)));
                            }
                            Err(error) => eprintln!("events: {error}: {text}"),
                        }
                    }
                    playback.send_replace(None);
                    audio.send_replace(None);
                }
                Err(error) => eprintln!("events {url}: {error}"),
            }
            tokio::time::sleep(RECONNECT).await;
        }
    });
    Live {
        playback: playback_seen,
        voice: voice_seen,
        audio: audio_seen,
    }
}

fn replace<T: PartialEq>(current: &mut T, new: T) -> bool {
    let changed = *current != new;
    *current = new;
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_keeps_the_latest_state_and_the_last_few_requests() {
        let mut log = VoiceLog::default();
        for number in 0..7 {
            log.record(Voice::Listening);
            log.record(Voice::Heard {
                text: format!("request {number}"),
                action: "Playing".to_string(),
                understood: true,
            });
        }
        log.record(Voice::Nothing);
        assert_eq!(log.state, Some(Voice::Nothing));
        let texts: Vec<_> = log.heard.iter().map(|heard| heard.text.as_str()).collect();
        assert_eq!(texts, ["request 6", "request 5", "request 4", "request 3", "request 2"]);
    }
}
