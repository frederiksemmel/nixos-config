use crate::api::{Audio, Event, NowPlaying, Output, PairState, Voice};
use futures_util::StreamExt;
use std::collections::VecDeque;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;

const RECONNECT: Duration = Duration::from_secs(2);
const KEPT: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tone {
    Info,
    Ok,
    Error,
}

impl Tone {
    pub fn name(self) -> &'static str {
        match self {
            Tone::Info => "info",
            Tone::Ok => "ok",
            Tone::Error => "error",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub id: String,
    pub at: Instant,
    pub text: String,
    pub detail: String,
    pub tone: Tone,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Notices {
    pub recent: VecDeque<Notice>,
}

struct Raise {
    boot: u128,
    count: u64,
}

impl Raise {
    fn push(&mut self, notices: &mut Notices, (text, detail, tone): (String, String, Tone)) {
        self.count += 1;
        notices.recent.push_back(Notice {
            id: format!("{}-{}", self.boot, self.count),
            at: Instant::now(),
            text,
            detail,
            tone,
        });
        while notices.recent.len() > KEPT {
            notices.recent.pop_front();
        }
    }
}

pub fn voice_notice(voice: &Voice) -> Option<(String, String, Tone)> {
    let plain = |text: &str, tone| Some((text.to_string(), String::new(), tone));
    match voice {
        Voice::Heard {
            text,
            action,
            understood,
        } => Some((
            format!("“{text}”"),
            action.clone(),
            if *understood { Tone::Ok } else { Tone::Error },
        )),
        Voice::Nothing => plain("Didn't catch that", Tone::Info),
        Voice::NoMicrophone => plain("The TV has no microphone", Tone::Error),
        Voice::Failed { .. } => plain("Speech recognition isn't working right now", Tone::Error),
        Voice::Listening | Voice::Transcribing | Voice::Cancelled => None,
    }
}

pub fn audio_notices(before: Option<&Audio>, after: &Audio) -> Vec<(String, String, Tone)> {
    let Some(before) = before else {
        return Vec::new();
    };
    let mut found = Vec::new();
    if let Some(error) = after.error.as_ref().filter(|error| before.error.as_ref() != Some(*error)) {
        found.push((error.clone(), String::new(), Tone::Error));
    }
    let state = |audio: &Audio| audio.pairing.as_ref().map(|pairing| pairing.state);
    if state(after) != state(before)
        && let Some(pairing) = &after.pairing
    {
        match pairing.state {
            PairState::Paired => found.push((pairing.message.clone(), String::new(), Tone::Ok)),
            PairState::Failed => found.push((pairing.message.clone(), String::new(), Tone::Error)),
            PairState::Scanning | PairState::Pairing => {}
        }
    }
    let name = after
        .headphones
        .as_ref()
        .or(before.headphones.as_ref())
        .map_or("the headphones", |headphones| headphones.name.as_str());
    match (before.output, after.output) {
        (Output::Headphones, Output::Tv) => {
            found.push(("Sound is back on the TV".to_string(), format!("{name} stopped playing"), Tone::Info))
        }
        (Output::Tv, Output::Headphones) => found.push((format!("Sound is on {name}"), String::new(), Tone::Ok)),
        _ => {}
    }
    found
}

#[derive(Clone)]
pub struct Live {
    pub playback: watch::Receiver<Option<NowPlaying>>,
    pub voice: watch::Receiver<Option<Voice>>,
    pub audio: watch::Receiver<Option<Audio>>,
    pub notices: watch::Receiver<Notices>,
}

pub fn follow(url: String) -> Live {
    let (playback, playback_seen) = watch::channel(None);
    let (voice, voice_seen) = watch::channel(None);
    let (audio, audio_seen) = watch::channel(None);
    let (notices, notices_seen) = watch::channel(Notices::default());
    let boot = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis());
    let mut raise = Raise { boot, count: 0 };
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
                            Ok(Event::Voice(event)) => {
                                if let Some(notice) = voice_notice(&event) {
                                    notices.send_modify(|list| raise.push(list, notice));
                                }
                                voice.send_replace(Some(event));
                            }
                            Ok(Event::Audio(state)) => {
                                let found = audio_notices(audio.borrow().as_ref(), &state);
                                if !found.is_empty() {
                                    notices.send_modify(|list| found.into_iter().for_each(|notice| raise.push(list, notice)));
                                }
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
        notices: notices_seen,
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
    use crate::api::{Headphones, Link, Pairing};

    fn audio(output: Output, pairing: Option<PairState>, error: Option<&str>) -> Audio {
        Audio {
            output,
            headphones: Some(Headphones {
                name: "AirPods".to_string(),
                state: Link::Connected,
                volume: None,
                audio_delay: None,
            }),
            pairing: pairing.map(|state| Pairing {
                state,
                message: "Paired with AirPods".to_string(),
            }),
            error: error.map(str::to_string),
        }
    }

    #[test]
    fn heard_requests_and_failures_become_notices_but_progress_does_not() {
        let heard = Voice::Heard {
            text: "play dune".to_string(),
            action: "Playing Dune".to_string(),
            understood: true,
        };
        assert_eq!(
            voice_notice(&heard),
            Some(("“play dune”".to_string(), "Playing Dune".to_string(), Tone::Ok))
        );
        assert_eq!(voice_notice(&Voice::Listening), None);
        assert_eq!(voice_notice(&Voice::Nothing).map(|notice| notice.2), Some(Tone::Info));
    }

    #[test]
    fn audio_changes_say_where_the_sound_went_and_what_failed() {
        let on_tv = audio(Output::Tv, None, None);
        let on_phones = audio(Output::Headphones, None, None);
        assert!(audio_notices(None, &on_phones).is_empty());
        assert_eq!(audio_notices(Some(&on_tv), &on_phones)[0].0, "Sound is on AirPods");
        assert_eq!(audio_notices(Some(&on_phones), &on_tv)[0].0, "Sound is back on the TV");
        let failed = audio(Output::Tv, None, Some("AirPods didn't connect"));
        assert_eq!(audio_notices(Some(&on_tv), &failed)[0].2, Tone::Error);
        assert!(audio_notices(Some(&failed), &failed).is_empty());
        let scanning = audio(Output::Tv, Some(PairState::Scanning), None);
        let paired = audio(Output::Tv, Some(PairState::Paired), None);
        assert!(audio_notices(Some(&on_tv), &scanning).is_empty());
        assert_eq!(audio_notices(Some(&scanning), &paired)[0].2, Tone::Ok);
    }
}
