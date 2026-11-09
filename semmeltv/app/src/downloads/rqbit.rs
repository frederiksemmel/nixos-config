use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;

const QUICK: Duration = Duration::from_secs(20);
const METADATA: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct File {
    pub name: String,
    #[serde(default)]
    pub components: Vec<String>,
    pub length: u64,
    #[serde(default)]
    pub included: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Details {
    #[serde(default)]
    pub files: Option<Vec<File>>,
}

#[derive(Debug, Deserialize)]
struct Added {
    details: Details,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Stats {
    pub state: String,
    pub error: Option<String>,
    pub progress_bytes: u64,
    pub uploaded_bytes: u64,
    pub total_bytes: u64,
    pub finished: bool,
    pub live: Option<Live>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Live {
    pub download_speed: Speed,
    pub upload_speed: Speed,
    #[serde(default)]
    pub snapshot: Value,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Speed {
    pub mbps: f64,
}

impl Stats {
    pub fn peers(&self) -> Option<u64> {
        let stats = &self.live.as_ref()?.snapshot["peer_stats"];
        stats["live"].as_u64()
    }

    pub fn seen_peers(&self) -> Option<u64> {
        let stats = &self.live.as_ref()?.snapshot["peer_stats"];
        stats["seen"].as_u64()
    }
}

pub struct Rqbit {
    base: String,
    agent: ureq::Agent,
}

impl Rqbit {
    pub fn new(base: String) -> Self {
        let config = ureq::Agent::config_builder().http_status_as_error(false).build();
        Self {
            base: base.trim_end_matches('/').to_string(),
            agent: config.new_agent(),
        }
    }

    fn answer(&self, mut response: ureq::http::Response<ureq::Body>, what: &str) -> Result<String, String> {
        let status = response.status();
        let body = response
            .body_mut()
            .with_config()
            .limit(16 * 1024 * 1024)
            .read_to_string()
            .map_err(|error| format!("rqbit {what}: {error}"))?;
        if status.is_success() {
            Ok(body)
        } else {
            Err(format!("rqbit {what}: HTTP {status}: {}", body.chars().take(300).collect::<String>()))
        }
    }

    fn post(&self, path: &str, query: &[(&str, String)], body: &str, timeout: Duration) -> Result<String, String> {
        let mut request = self
            .agent
            .post(&format!("{}{path}", self.base))
            .config()
            .timeout_global(Some(timeout))
            .build();
        for (name, value) in query {
            request = request.query(*name, value);
        }
        let response = request
            .send(body)
            .map_err(|error| format!("rqbit {path}: {error}"))?;
        self.answer(response, path)
    }

    fn get(&self, path: &str) -> Result<String, String> {
        let response = self
            .agent
            .get(&format!("{}{path}", self.base))
            .config()
            .timeout_global(Some(QUICK))
            .build()
            .call()
            .map_err(|error| format!("rqbit {path}: {error}"))?;
        self.answer(response, path)
    }

    pub fn files(&self, magnet: &str) -> Result<Details, String> {
        let query = [("list_only", "true".to_string()), ("overwrite", "true".to_string())];
        let body = self.post("/torrents", &query, magnet, METADATA)?;
        let added: Added = serde_json::from_str(&body).map_err(|error| format!("rqbit list: {error}"))?;
        Ok(added.details)
    }

    pub fn add(&self, magnet: &str, folder: &str, only: &[usize]) -> Result<Details, String> {
        let only: Vec<String> = only.iter().map(usize::to_string).collect();
        let query = [
            ("overwrite", "true".to_string()),
            ("output_folder", folder.to_string()),
            ("only_files", only.join(",")),
        ];
        let body = self.post("/torrents", &query, magnet, METADATA)?;
        let added: Added = serde_json::from_str(&body).map_err(|error| format!("rqbit add: {error}"))?;
        Ok(added.details)
    }

    pub fn stats(&self, hash: &str) -> Result<Stats, String> {
        let body = self.get(&format!("/torrents/{hash}/stats/v1"))?;
        serde_json::from_str(&body).map_err(|error| format!("rqbit stats: {error}"))
    }

    pub fn pause(&self, hash: &str) -> Result<(), String> {
        self.post(&format!("/torrents/{hash}/pause"), &[], "", QUICK).map(drop)
    }

    pub fn delete(&self, hash: &str) -> Result<(), String> {
        self.post(&format!("/torrents/{hash}/delete"), &[], "", QUICK).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_v1_reads_progress_upload_and_peers() {
        let json = r#"{"state":"live","file_progress":[5,0],"error":null,"progress_bytes":5,
            "uploaded_bytes":0,"total_bytes":10,"finished":false,
            "live":{"snapshot":{"downloaded_and_checked_bytes":5,"fetched_bytes":5,"uploaded_bytes":0,
              "downloaded_and_checked_pieces":1,"total_piece_download_ms":10,
              "peer_stats":{"queued":3,"connecting":1,"live":12,"seen":80,"dead":2,"not_needed":0,"steals":0}},
              "average_piece_download_time":null,"download_speed":{"mbps":4.5,"human_readable":"4.5 MiB/s"},
              "upload_speed":{"mbps":0.0,"human_readable":"0 MiB/s"},"time_remaining":null}}"#;
        let stats: Stats = serde_json::from_str(json).unwrap();
        assert_eq!(stats.progress_bytes, 5);
        assert_eq!(stats.uploaded_bytes, 0);
        assert_eq!(stats.peers(), Some(12));
        assert_eq!(stats.seen_peers(), Some(80));
        assert!((stats.live.unwrap().download_speed.mbps - 4.5).abs() < 1e-9);
    }

    #[test]
    fn a_list_only_answer_names_the_files() {
        let json = r#"{"id":null,"details":{"info_hash":"ab","name":"Outlander S08","output_folder":"/d",
            "files":[{"name":"Outlander S08/Outlander S08E01.mkv","components":["Outlander S08","Outlander S08E01.mkv"],
            "length":100,"included":true,"attributes":{"symlink":false,"hidden":false,"padding":false,"executable":false}}]},
            "output_folder":"/d","seen_peers":[]}"#;
        let added: Added = serde_json::from_str(json).unwrap();
        let files = added.details.files.unwrap();
        assert_eq!(files[0].components, ["Outlander S08", "Outlander S08E01.mkv"]);
    }
}
