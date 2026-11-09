use super::resolve::Wish;
use super::{Outcome, Proposal, SearchBody};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::time::Duration;

const SEARCH_TIME: Duration = Duration::from_secs(120);

#[derive(Clone)]
pub struct Downloads {
    base: String,
    agent: ureq::Agent,
}

impl Downloads {
    pub fn new(base: String) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(SEARCH_TIME))
            .build()
            .new_agent();
        Self {
            base: base.trim_end_matches('/').to_string(),
            agent,
        }
    }

    fn post<T: DeserializeOwned>(&self, path: &str, body: &impl Serialize) -> Result<T, String> {
        self.agent
            .post(&format!("{}{path}", self.base))
            .send_json(body)
            .map_err(|error| format!("downloads: {error}"))?
            .body_mut()
            .read_json()
            .map_err(|error| format!("downloads: {error}"))
    }

    pub fn search(&self, title: String, season: Option<u32>, episode: Option<u32>, said: String) -> Result<Proposal, String> {
        let body = SearchBody {
            text: Some(said),
            wish: Some(Wish {
                title,
                year: None,
                season,
                episode,
            }),
            pick: None,
            from: Some("voice".to_string()),
        };
        self.post("/api/downloads/search", &body)
    }

    pub fn confirm(&self, proposal: &str) -> Result<Outcome, String> {
        self.post(
            "/api/downloads/confirm",
            &serde_json::json!({ "proposal": proposal, "choice": 0 }),
        )
    }

    pub fn dismiss(&self, proposal: &str) -> Result<Outcome, String> {
        self.post("/api/downloads/dismiss", &serde_json::json!({ "proposal": proposal }))
    }
}
