use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use rand::seq::IndexedRandom;
use serde::{Deserialize, Serialize};

const LINES_FILE: &str = "replies.txt";
const LOG_FILE: &str = "replies.jsonl";
const MIN_WEIGHT: f64 = 0.25;
const MAX_WEIGHT: f64 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Wait,
    Written,
    Ring,
    Rotated,
    Busy,
    Failed,
}

impl Kind {
    const ALL: [Kind; 6] = [
        Kind::Wait,
        Kind::Written,
        Kind::Ring,
        Kind::Rotated,
        Kind::Busy,
        Kind::Failed,
    ];

    fn name(self) -> &'static str {
        match self {
            Kind::Wait => "wait",
            Kind::Written => "written",
            Kind::Ring => "ring",
            Kind::Rotated => "rotated",
            Kind::Busy => "busy",
            Kind::Failed => "failed",
        }
    }

    /// Said when the lines file has nothing for this kind.
    fn fallback(self) -> &'static str {
        match self {
            Kind::Wait => "Wait for it…",
            Kind::Written => "Written it is",
            Kind::Ring => "ine gluegt",
            Kind::Rotated => "Rotated to {deg}°",
            Kind::Busy => "Still drawing, send it again in a moment",
            Kind::Failed => "The display failed, see the journal",
        }
    }
}

/// Reads the lines file: `[kind]` starts a section, and every other line
/// that is neither blank nor starts with `#` is one reply.
pub fn parse(text: &str) -> HashMap<Kind, Vec<String>> {
    let mut lines: HashMap<Kind, Vec<String>> = HashMap::new();
    let mut section = None;
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = Kind::ALL.into_iter().find(|k| k.name() == name.trim());
            if section.is_none() {
                tracing::warn!("unknown section [{name}] in the replies file");
            }
        } else if let Some(kind) = section {
            lines.entry(kind).or_default().push(line.to_owned());
        }
    }
    lines
}

/// The lines file, reread whenever it changes on disk.
struct Lines {
    path: Option<PathBuf>,
    modified: Option<SystemTime>,
    lines: HashMap<Kind, Vec<String>>,
}

impl Lines {
    fn refresh(&mut self) {
        let Some(path) = &self.path else { return };
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if modified == self.modified {
            return;
        }
        self.modified = modified;
        match std::fs::read_to_string(path) {
            Ok(text) => {
                self.lines = parse(&text);
                let counts: Vec<String> = Kind::ALL
                    .iter()
                    .map(|k| format!("{}={}", k.name(), self.lines.get(k).map_or(0, Vec::len)))
                    .collect();
                tracing::info!("read {}: {}", path.display(), counts.join(" "));
            }
            Err(err) => {
                tracing::warn!("reading {}: {err}", path.display());
                self.lines.clear();
            }
        }
    }
}

/// The reply lines, how well each was received, and which message carries which line.
pub struct Replies {
    lines: Lines,
    scores: HashMap<String, i32>,
    sent: HashMap<(i64, i32), String>,
    log: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "lowercase")]
enum Record {
    Sent {
        at: u64,
        chat: i64,
        message: i32,
        kind: Kind,
        template: String,
    },
    Reaction {
        at: u64,
        chat: i64,
        message: i32,
        user: u64,
        old: Vec<String>,
        new: Vec<String>,
    },
}

impl Replies {
    /// Uses `replies.txt` and `replies.jsonl` in `dir`; without a dir, only the fallback lines.
    pub fn open(dir: Option<&Path>) -> Result<Self> {
        let mut replies = Replies {
            lines: Lines {
                path: dir.map(|d| d.join(LINES_FILE)),
                modified: None,
                lines: HashMap::new(),
            },
            scores: HashMap::new(),
            sent: HashMap::new(),
            log: dir.map(|d| d.join(LOG_FILE)),
        };
        replies.lines.refresh();
        if let Some(path) = replies.log.clone()
            && let Ok(file) = std::fs::File::open(&path)
        {
            for line in BufReader::new(file).lines() {
                match serde_json::from_str(&line?) {
                    Ok(record) => replies.apply(&record),
                    Err(err) => tracing::warn!("skipping a line of {}: {err}", path.display()),
                }
            }
        }
        Ok(replies)
    }

    /// Picks a line for `kind`, favouring lines people reacted well to.
    pub fn pick(&mut self, kind: Kind) -> String {
        self.lines.refresh();
        let lines = self.lines.lines.get(&kind).map(Vec::as_slice).unwrap_or_default();
        lines
            .choose_weighted(&mut rand::rng(), |line| self.weight(line))
            .cloned()
            .unwrap_or_else(|_| kind.fallback().to_owned())
    }

    fn weight(&self, line: &str) -> f64 {
        let score = self.scores.get(line).copied().unwrap_or(0);
        2f64.powf(score as f64 / 2.0).clamp(MIN_WEIGHT, MAX_WEIGHT)
    }

    /// Notes that `message` in `chat` now shows `template`.
    pub fn sent(&mut self, chat: i64, message: i32, kind: Kind, template: &str) {
        self.record(Record::Sent {
            at: now(),
            chat,
            message,
            kind,
            template: template.to_owned(),
        });
    }

    /// Scores the line shown in `message` by how `user` changed their reaction.
    pub fn reacted(&mut self, chat: i64, message: i32, user: u64, old: Vec<String>, new: Vec<String>) {
        self.record(Record::Reaction {
            at: now(),
            chat,
            message,
            user,
            old,
            new,
        });
    }

    fn record(&mut self, record: Record) {
        self.apply(&record);
        if let Some(path) = &self.log
            && let Err(err) = append(path, &record)
        {
            tracing::warn!("writing {}: {err:#}", path.display());
        }
    }

    fn apply(&mut self, record: &Record) {
        match record {
            Record::Sent {
                chat,
                message,
                template,
                ..
            } => {
                self.sent.insert((*chat, *message), template.clone());
            }
            Record::Reaction {
                chat,
                message,
                old,
                new,
                ..
            } => {
                if let Some(template) = self.sent.get(&(*chat, *message)) {
                    *self.scores.entry(template.clone()).or_default() += score(new) - score(old);
                }
            }
        }
    }
}

fn append(path: &Path, record: &Record) -> Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", serde_json::to_string(record)?)?;
    Ok(())
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// +1 when the reaction is liked, −1 when disliked, 0 otherwise.
fn score(emoji: &[String]) -> i32 {
    let points: i32 = emoji.iter().map(|e| emoji_points(e)).sum();
    points.signum()
}

fn emoji_points(emoji: &str) -> i32 {
    match emoji.trim_end_matches('\u{fe0f}') {
        "👍" | "❤" | "🔥" | "🥰" | "👏" | "😁" | "🎉" | "🤩" | "😍" | "❤\u{200d}🔥" | "💯" | "🤣" | "🏆" | "👌"
        | "🤗" | "🫡" | "😘" | "😎" | "🆒" | "💘" | "🦄" | "😇" | "🍾" | "⚡" => 1,
        "👎" | "💩" | "🤮" | "🥱" | "😴" | "🤡" | "😐" | "🤨" | "🖕" | "😡" | "🤬" | "💔" | "😢" | "😭" => {
            -1
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# replies\n[wait]\nWait for it…\n\n  Momänt  \n[written]\nNailed it.\nTadaa ✨\n[nope]\nignored\n[rotated]\n{deg}° it is\n";

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("epaper-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample(name: &str) -> (PathBuf, Replies) {
        let dir = scratch(name);
        std::fs::write(dir.join(LINES_FILE), SAMPLE).unwrap();
        let replies = Replies::open(Some(&dir)).unwrap();
        (dir, replies)
    }

    #[test]
    fn the_lines_file_has_sections_comments_and_blank_lines() {
        let lines = parse(SAMPLE);
        assert_eq!(lines[&Kind::Wait], ["Wait for it…", "Momänt"]);
        assert_eq!(lines[&Kind::Written], ["Nailed it.", "Tadaa ✨"]);
        assert_eq!(lines[&Kind::Rotated], ["{deg}° it is"]);
        assert!(!lines.contains_key(&Kind::Busy));
        assert!(!lines.values().flatten().any(|l| l == "ignored"));
    }

    #[test]
    fn missing_lines_fall_back() {
        let mut replies = Replies::open(None).unwrap();
        assert_eq!(replies.pick(Kind::Busy), Kind::Busy.fallback());
        let (_dir, mut replies) = sample("fallback");
        assert_eq!(replies.pick(Kind::Failed), Kind::Failed.fallback());
        assert!(["Nailed it.", "Tadaa ✨"].contains(&replies.pick(Kind::Written).as_str()));
    }

    #[test]
    fn an_edited_lines_file_is_picked_up() {
        let (dir, mut replies) = sample("edited");
        let path = dir.join(LINES_FILE);
        std::fs::write(&path, "[written]\nNew line\n").unwrap();
        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert_eq!(replies.pick(Kind::Written), "New line");
    }

    #[test]
    fn reactions_move_the_weight_of_the_line_shown() {
        let (_dir, mut replies) = sample("reactions");
        replies.sent(1, 10, Kind::Wait, "Wait for it…");
        replies.sent(1, 10, Kind::Written, "Nailed it.");
        replies.reacted(1, 10, 7, vec![], vec!["👍".into()]);
        replies.reacted(1, 10, 8, vec![], vec!["🔥".into()]);
        assert_eq!(replies.scores["Nailed it."], 2);
        assert!(!replies.scores.contains_key("Wait for it…"));

        replies.reacted(1, 10, 8, vec!["🔥".into()], vec!["💩".into()]);
        assert_eq!(replies.scores["Nailed it."], 0);
        replies.reacted(1, 10, 7, vec!["👍".into()], vec!["👎".into()]);
        assert_eq!(replies.scores["Nailed it."], -2);
        assert!(replies.weight("Nailed it.") < replies.weight("Tadaa ✨"));
    }

    #[test]
    fn reactions_to_unknown_messages_are_ignored() {
        let mut replies = Replies::open(None).unwrap();
        replies.reacted(1, 99, 7, vec![], vec!["👍".into()]);
        assert!(replies.scores.is_empty());
    }

    #[test]
    fn the_log_is_replayed_on_open() {
        let dir = scratch("log");
        {
            let mut replies = Replies::open(Some(&dir)).unwrap();
            replies.sent(1, 10, Kind::Written, "Nailed it.");
            replies.reacted(1, 10, 7, vec![], vec!["❤\u{fe0f}".into()]);
        }
        let replies = Replies::open(Some(&dir)).unwrap();
        assert_eq!(replies.scores["Nailed it."], 1);
        std::fs::remove_dir_all(dir).ok();
    }
}
