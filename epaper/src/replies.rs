use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use rand::seq::IndexedRandom;
use serde::{Deserialize, Deserializer, Serialize};

const MODES_DIR: &str = "modes";
const LOG_FILE: &str = "replies.jsonl";
const MIN_WEIGHT: f64 = 0.25;
const MAX_WEIGHT: f64 = 8.0;

/// The mode of a chat that picked none, and the one whose variants fill a message another mode lacks.
pub const DEFAULT_MODE: &str = "swiss";

/// What a reply has to get across; each mode words it in its own variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Message {
    Wait,
    Written,
    Ring,
    Rotated,
    Busy,
    Failed,
    #[serde(alias = "styled")]
    Switched,
}

impl Message {
    const ALL: [Message; 7] = [
        Message::Wait,
        Message::Written,
        Message::Ring,
        Message::Rotated,
        Message::Busy,
        Message::Failed,
        Message::Switched,
    ];

    fn name(self) -> &'static str {
        match self {
            Message::Wait => "wait",
            Message::Written => "written",
            Message::Ring => "ring",
            Message::Rotated => "rotated",
            Message::Busy => "busy",
            Message::Failed => "failed",
            Message::Switched => "switched",
        }
    }

    /// Said when no mode has a variant for this message.
    fn fallback(self) -> &'static str {
        match self {
            Message::Wait => "Wait for it…",
            Message::Written => "Written it is",
            Message::Ring => "ine gluegt",
            Message::Rotated => "Rotated to {deg}°",
            Message::Busy => "Still drawing, send it again in a moment",
            Message::Failed => "The display failed, see the journal",
            Message::Switched => "From now on I answer in {mode} mode",
        }
    }
}

/// Reads a mode file: `[message]` starts a section, and every other line
/// that is neither blank nor starts with `#` is one variant of that message.
pub fn parse(text: &str) -> HashMap<Message, Vec<String>> {
    let mut variants: HashMap<Message, Vec<String>> = HashMap::new();
    let mut section = None;
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let name = name.trim().to_lowercase();
            section = Message::ALL.into_iter().find(|m| m.name() == name);
            if section.is_none() {
                tracing::warn!("unknown message [{name}] in a mode file");
            }
        } else if let Some(message) = section {
            variants.entry(message).or_default().push(line.to_owned());
        }
    }
    variants
}

/// The mode files, `<mode>.txt` in one directory, reread whenever one changes on disk.
struct Modes {
    dir: Option<PathBuf>,
    seen: Vec<(PathBuf, Option<SystemTime>)>,
    modes: HashMap<String, HashMap<Message, Vec<String>>>,
}

impl Modes {
    fn files(dir: &Path) -> Vec<(PathBuf, Option<SystemTime>)> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut files: Vec<_> = entries
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "txt"))
            .map(|path| {
                let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
                (path, modified)
            })
            .collect();
        files.sort();
        files
    }

    fn refresh(&mut self) {
        let Some(dir) = &self.dir else { return };
        let files = Self::files(dir);
        if files == self.seen {
            return;
        }
        self.modes.clear();
        for (path, _) in &files {
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            match std::fs::read_to_string(path) {
                Ok(text) => {
                    let variants = parse(&text);
                    let counts: Vec<String> = Message::ALL
                        .iter()
                        .map(|&m| format!("{}={}", m.name(), variants.get(&m).map_or(0, Vec::len)))
                        .collect();
                    tracing::info!("read {}: {}", path.display(), counts.join(" "));
                    self.modes.insert(name.to_lowercase(), variants);
                }
                Err(err) => tracing::warn!("reading {}: {err}", path.display()),
            }
        }
        self.seen = files;
    }

    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.modes.keys().cloned().chain([DEFAULT_MODE.to_owned()]).collect();
        names.sort();
        names.dedup();
        names
    }
}

/// Where a reply sits: the chat (`tg:<id>` or `wa:<jid>`) and the message id in it.
type Spot = (String, String);

/// The variants of every mode, how well each was received, which reply shows which variant, and each chat's mode.
pub struct Replies {
    modes: Modes,
    scores: HashMap<String, i32>,
    sent: HashMap<Spot, String>,
    reactions: HashMap<(Spot, String), Vec<String>>,
    chosen: HashMap<String, String>,
    log: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "lowercase")]
enum Record {
    Sent {
        at: u64,
        #[serde(deserialize_with = "chat_key")]
        chat: String,
        #[serde(rename = "message", deserialize_with = "message_key")]
        id: String,
        #[serde(rename = "kind")]
        message: Message,
        #[serde(rename = "template")]
        variant: String,
    },
    Reaction {
        at: u64,
        #[serde(deserialize_with = "chat_key")]
        chat: String,
        #[serde(rename = "message", deserialize_with = "message_key")]
        id: String,
        #[serde(deserialize_with = "chat_key")]
        user: String,
        old: Vec<String>,
        new: Vec<String>,
    },
    #[serde(alias = "style")]
    Mode {
        at: u64,
        chat: String,
        #[serde(alias = "style")]
        mode: Option<String>,
    },
}

/// A key as the log stores it; the first, Telegram-only log used bare numbers.
#[derive(Deserialize)]
#[serde(untagged)]
enum LoggedKey {
    Number(i64),
    Text(String),
}

fn chat_key<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match LoggedKey::deserialize(d)? {
        LoggedKey::Number(n) => format!("tg:{n}"),
        LoggedKey::Text(s) => s,
    })
}

fn message_key<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match LoggedKey::deserialize(d)? {
        LoggedKey::Number(n) => n.to_string(),
        LoggedKey::Text(s) => s,
    })
}

impl Replies {
    /// Uses the mode files in `modes/` and the log `replies.jsonl` in `dir`; without a dir, only the fallbacks.
    pub fn open(dir: Option<&Path>) -> Result<Self> {
        let mut replies = Replies {
            modes: Modes {
                dir: dir.map(|d| d.join(MODES_DIR)),
                seen: Vec::new(),
                modes: HashMap::new(),
            },
            scores: HashMap::new(),
            sent: HashMap::new(),
            reactions: HashMap::new(),
            chosen: HashMap::new(),
            log: dir.map(|d| d.join(LOG_FILE)),
        };
        replies.modes.refresh();
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

    /// Picks a variant of `message` in `chat`'s mode, favouring variants people reacted well to.
    /// A message the mode has no variants of gets one from the default mode.
    pub fn pick(&mut self, chat: &str, message: Message) -> String {
        self.modes.refresh();
        let mode = self.mode_of(chat).to_owned();
        let variants = [mode.as_str(), DEFAULT_MODE]
            .into_iter()
            .filter_map(|mode| self.modes.modes.get(mode)?.get(&message))
            .find(|variants| !variants.is_empty())
            .map(Vec::as_slice)
            .unwrap_or_default();
        variants
            .choose_weighted(&mut rand::rng(), |variant| self.weight(variant))
            .cloned()
            .unwrap_or_else(|_| message.fallback().to_owned())
    }

    /// Every mode by name: one per mode file, and the default.
    pub fn modes(&mut self) -> Vec<String> {
        self.modes.refresh();
        self.modes.names()
    }

    /// The mode `chat` answers in.
    pub fn mode_of(&self, chat: &str) -> &str {
        self.chosen.get(chat).map_or(DEFAULT_MODE, String::as_str)
    }

    /// Makes `chat` answer in `mode` from now on.
    pub fn set_mode(&mut self, chat: &str, mode: &str) {
        self.record(Record::Mode {
            at: now(),
            chat: chat.to_owned(),
            mode: (mode != DEFAULT_MODE).then(|| mode.to_owned()),
        });
    }

    fn weight(&self, variant: &str) -> f64 {
        let score = self.scores.get(variant).copied().unwrap_or(0);
        2f64.powf(score as f64 / 2.0).clamp(MIN_WEIGHT, MAX_WEIGHT)
    }

    /// Notes that the chat message `id` in `chat` now shows `variant` of `message`.
    pub fn sent(&mut self, chat: &str, id: &str, message: Message, variant: &str) {
        self.record(Record::Sent {
            at: now(),
            chat: chat.to_owned(),
            id: id.to_owned(),
            message,
            variant: variant.to_owned(),
        });
    }

    /// The emoji `user` currently has on the chat message `id` in `chat`.
    pub fn reaction_of(&self, chat: &str, id: &str, user: &str) -> Vec<String> {
        let key = ((chat.to_owned(), id.to_owned()), user.to_owned());
        self.reactions.get(&key).cloned().unwrap_or_default()
    }

    /// Whether the chat message `id` in `chat` is one of the bot's replies.
    pub fn is_reply(&self, chat: &str, id: &str) -> bool {
        self.sent.contains_key(&(chat.to_owned(), id.to_owned()))
    }

    /// Scores the variant shown in the chat message `id` by how `user` changed their reaction.
    pub fn reacted(&mut self, chat: &str, id: &str, user: &str, old: Vec<String>, new: Vec<String>) {
        self.record(Record::Reaction {
            at: now(),
            chat: chat.to_owned(),
            id: id.to_owned(),
            user: user.to_owned(),
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
            Record::Sent { chat, id, variant, .. } => {
                self.sent.insert((chat.clone(), id.clone()), variant.clone());
            }
            Record::Reaction {
                chat,
                id,
                user,
                old,
                new,
                ..
            } => {
                let spot = (chat.clone(), id.clone());
                if let Some(variant) = self.sent.get(&spot) {
                    *self.scores.entry(variant.clone()).or_default() += score(new) - score(old);
                }
                self.reactions.insert((spot, user.clone()), new.clone());
            }
            Record::Mode { chat, mode, .. } => match mode {
                Some(mode) => {
                    self.chosen.insert(chat.clone(), mode.clone());
                }
                None => {
                    self.chosen.remove(chat);
                }
            },
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
        | "🤗" | "🫡" | "😘" | "😎" | "🆒" | "💘" | "🦄" | "😇" | "🍾" | "⚡" | "😂" | "🙏" => 1,
        "👎" | "💩" | "🤮" | "🥱" | "😴" | "🤡" | "😐" | "🤨" | "🖕" | "😡" | "🤬" | "💔" | "😢" | "😭" => {
            -1
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SWISS: &str = "# replies\n[wait]\nWait for it…\n\n  Momänt  \n[Written]\nNailed it.\nTadaa ✨\n[nope]\nignored\n[rotated]\n{deg}° it is\n";
    const OFFICE: &str = "[written]\nThat's what she said\n[switched]\nDunder Mifflin, this is Pam.\n";

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("epaper-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join(MODES_DIR)).unwrap();
        dir
    }

    fn sample(name: &str) -> (PathBuf, Replies) {
        let dir = scratch(name);
        std::fs::write(dir.join(MODES_DIR).join("swiss.txt"), SWISS).unwrap();
        std::fs::write(dir.join(MODES_DIR).join("Office.txt"), OFFICE).unwrap();
        std::fs::write(dir.join(MODES_DIR).join("notes.md"), "[written]\nnot a mode\n").unwrap();
        let replies = Replies::open(Some(&dir)).unwrap();
        (dir, replies)
    }

    #[test]
    fn a_mode_file_has_sections_comments_and_blank_lines() {
        let variants = parse(SWISS);
        assert_eq!(variants[&Message::Wait], ["Wait for it…", "Momänt"]);
        assert_eq!(variants[&Message::Written], ["Nailed it.", "Tadaa ✨"]);
        assert_eq!(variants[&Message::Rotated], ["{deg}° it is"]);
        assert!(!variants.contains_key(&Message::Busy));
        assert!(!variants.values().flatten().any(|l| l == "ignored"));
    }

    #[test]
    fn a_chat_in_a_mode_gets_its_variants_and_the_default_ones_where_it_has_none() {
        let (_dir, mut replies) = sample("mode");
        assert_eq!(replies.modes(), ["office", "swiss"]);
        assert_eq!(replies.mode_of("wa:1@g.us"), "swiss");
        replies.set_mode("wa:1@g.us", "office");
        assert_eq!(replies.mode_of("wa:1@g.us"), "office");
        assert_eq!(replies.pick("wa:1@g.us", Message::Written), "That's what she said");
        assert!(["Wait for it…", "Momänt"].contains(&replies.pick("wa:1@g.us", Message::Wait).as_str()));
        assert_ne!(replies.pick("wa:2@g.us", Message::Written), "That's what she said");

        replies.set_mode("wa:1@g.us", "swiss");
        assert_eq!(replies.mode_of("wa:1@g.us"), "swiss");
        assert_ne!(replies.pick("wa:1@g.us", Message::Written), "That's what she said");
    }

    #[test]
    fn the_default_mode_is_listed_without_a_file() {
        let mut replies = Replies::open(Some(&scratch("no-modes"))).unwrap();
        assert_eq!(replies.modes(), ["swiss"]);
    }

    #[test]
    fn a_chats_mode_outlasts_a_restart() {
        let (dir, mut replies) = sample("mode-log");
        replies.set_mode("tg:5", "office");
        replies.set_mode("tg:6", "office");
        replies.set_mode("tg:6", "swiss");
        let replies = Replies::open(Some(&dir)).unwrap();
        assert_eq!(replies.mode_of("tg:5"), "office");
        assert_eq!(replies.mode_of("tg:6"), "swiss");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn modes_set_as_styles_still_read() {
        let dir = scratch("style-log");
        std::fs::write(
            dir.join(LOG_FILE),
            concat!(
                r#"{"event":"style","at":1,"chat":"tg:5","style":"office"}"#,
                "\n",
                r#"{"event":"sent","at":2,"chat":"tg:5","message":"9","kind":"styled","template":"Dunder Mifflin, this is Pam."}"#,
                "\n",
                r#"{"event":"style","at":3,"chat":"tg:6","style":null}"#,
                "\n",
            ),
        )
        .unwrap();
        let replies = Replies::open(Some(&dir)).unwrap();
        assert_eq!(replies.mode_of("tg:5"), "office");
        assert_eq!(replies.mode_of("tg:6"), "swiss");
        assert!(replies.is_reply("tg:5", "9"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_variants_fall_back() {
        let mut replies = Replies::open(None).unwrap();
        assert_eq!(replies.pick("tg:1", Message::Busy), Message::Busy.fallback());
        let (_dir, mut replies) = sample("fallback");
        assert_eq!(replies.pick("tg:1", Message::Failed), Message::Failed.fallback());
        assert!(["Nailed it.", "Tadaa ✨"].contains(&replies.pick("tg:1", Message::Written).as_str()));
    }

    #[test]
    fn an_edited_or_new_mode_file_is_picked_up() {
        let (dir, mut replies) = sample("edited");
        let path = dir.join(MODES_DIR).join("swiss.txt");
        std::fs::write(&path, "[written]\nNew line\n").unwrap();
        let later = SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert_eq!(replies.pick("tg:1", Message::Written), "New line");

        std::fs::write(dir.join(MODES_DIR).join("dune.txt"), "[written]\nThe spice must flow\n").unwrap();
        assert_eq!(replies.modes(), ["dune", "office", "swiss"]);
    }

    #[test]
    fn reactions_move_the_weight_of_the_variant_shown() {
        let (_dir, mut replies) = sample("reactions");
        replies.sent("tg:1", "10", Message::Wait, "Wait for it…");
        replies.sent("tg:1", "10", Message::Written, "Nailed it.");
        replies.reacted("tg:1", "10", "tg:7", vec![], vec!["👍".into()]);
        replies.reacted("tg:1", "10", "tg:8", vec![], vec!["🔥".into()]);
        assert_eq!(replies.scores["Nailed it."], 2);
        assert!(!replies.scores.contains_key("Wait for it…"));

        replies.reacted("tg:1", "10", "tg:8", vec!["🔥".into()], vec!["💩".into()]);
        assert_eq!(replies.scores["Nailed it."], 0);
        replies.reacted("tg:1", "10", "tg:7", vec!["👍".into()], vec!["👎".into()]);
        assert_eq!(replies.scores["Nailed it."], -2);
        assert!(replies.weight("Nailed it.") < replies.weight("Tadaa ✨"));
    }

    #[test]
    fn a_users_current_reaction_is_tracked_for_whatsapp() {
        let (_dir, mut replies) = sample("tracked");
        let (chat, id, user) = ("wa:123@g.us", "3EB0AB", "wa:41000@s.whatsapp.net");
        replies.sent(chat, id, Message::Written, "Nailed it.");
        assert!(replies.is_reply(chat, id));
        assert!(!replies.is_reply(chat, "other"));

        let react = |replies: &mut Replies, emoji: &str| {
            let old = replies.reaction_of(chat, id, user);
            let new = if emoji.is_empty() {
                vec![]
            } else {
                vec![emoji.to_owned()]
            };
            replies.reacted(chat, id, user, old, new);
        };
        react(&mut replies, "👍");
        assert_eq!(replies.scores["Nailed it."], 1);
        react(&mut replies, "💩");
        assert_eq!(replies.scores["Nailed it."], -1);
        react(&mut replies, "");
        assert_eq!(replies.scores["Nailed it."], 0);
        assert!(replies.reaction_of(chat, id, user).is_empty());
    }

    #[test]
    fn reactions_to_unknown_messages_are_ignored() {
        let mut replies = Replies::open(None).unwrap();
        replies.reacted("tg:1", "99", "tg:7", vec![], vec!["👍".into()]);
        assert!(replies.scores.is_empty());
    }

    #[test]
    fn the_log_is_replayed_on_open() {
        let dir = scratch("log");
        {
            let mut replies = Replies::open(Some(&dir)).unwrap();
            replies.sent("wa:1@g.us", "ABC", Message::Written, "Nailed it.");
            replies.reacted("wa:1@g.us", "ABC", "wa:7@lid", vec![], vec!["❤\u{fe0f}".into()]);
        }
        let log = std::fs::read_to_string(dir.join(LOG_FILE)).unwrap();
        assert!(
            log.contains(r#""message":"ABC","kind":"written","template":"Nailed it.""#),
            "{log}"
        );
        let replies = Replies::open(Some(&dir)).unwrap();
        assert_eq!(replies.scores["Nailed it."], 1);
        assert_eq!(replies.reaction_of("wa:1@g.us", "ABC", "wa:7@lid"), ["❤\u{fe0f}"]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_first_telegram_only_log_still_reads() {
        let dir = scratch("legacy");
        std::fs::write(
            dir.join(LOG_FILE),
            concat!(
                r#"{"event":"sent","at":1,"chat":-42,"message":414,"kind":"written","template":"Ich ha's gmacht, Mami!"}"#,
                "\n",
                r#"{"event":"reaction","at":2,"chat":-42,"message":414,"user":7,"old":[],"new":["👍"]}"#,
                "\n",
            ),
        )
        .unwrap();
        let replies = Replies::open(Some(&dir)).unwrap();
        assert_eq!(replies.scores["Ich ha's gmacht, Mami!"], 1);
        assert!(replies.is_reply("tg:-42", "414"));
        assert_eq!(replies.reaction_of("tg:-42", "414", "tg:7"), ["👍"]);
        std::fs::remove_dir_all(dir).ok();
    }
}
