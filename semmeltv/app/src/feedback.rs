use serde_json::Value;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(2);

pub struct Feedback {
    events: Sender<Value>,
}

impl Feedback {
    pub fn spawn(url: String) -> Self {
        let (events, received) = mpsc::channel::<Value>();
        thread::spawn(move || {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(TIMEOUT))
                .build()
                .into();
            for event in received {
                if let Err(error) = agent.post(&url).send_json(&event) {
                    eprintln!("voice feedback {url}: {error}");
                }
            }
        });
        Self { events }
    }

    pub fn send(&self, event: Value) {
        let _ = self.events.send(event);
    }
}
