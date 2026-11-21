//! The bot as a linked device of a WhatsApp account.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow};
use whatsapp_rust::{
    pair_code::PairCodeOptions,
    prelude::{
        Bot as WaBot, Client, Event, EventKind, Jid, MessageBuilderExt, MessageContext, MessageExt, SqliteStore, wa,
    },
    types::message::EditAttribute,
};

use crate::flow::{self, Chat};

const DATABASE: &str = "whatsapp.db";
/// Holds the bot account's number, with country code, while it is being linked.
const PHONE_FILE: &str = "whatsapp-phone";
/// Lists the chats allowed to draw, one JID per line.
const GROUPS_FILE: &str = "whatsapp-groups";
/// Holds the current pairing QR payload while the client is not linked.
const QR_FILE: &str = "whatsapp-qr";
const PAIR_CHECK: Duration = Duration::from_secs(10);
const PAIR_RETRY: Duration = Duration::from_secs(60);
const RESTART_AFTER: Duration = Duration::from_secs(60);
const STALE_AFTER_SECS: i64 = 600;

/// The chats that may draw, from the groups file, reread whenever it changes.
struct Groups {
    path: PathBuf,
    modified: Option<SystemTime>,
    allowed: HashSet<String>,
    ignored: HashSet<String>,
}

/// The JIDs in the groups file: one per line, `#` comments and blank lines skipped.
fn parse_groups(text: &str) -> HashSet<String> {
    text.lines()
        .map(|line| line.split('#').next().unwrap_or_default().trim())
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

impl Groups {
    fn new(path: PathBuf) -> Self {
        Groups {
            path,
            modified: None,
            allowed: HashSet::new(),
            ignored: HashSet::new(),
        }
    }

    fn refresh(&mut self) {
        let modified = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        if modified == self.modified {
            return;
        }
        self.modified = modified;
        self.allowed = std::fs::read_to_string(&self.path)
            .map(|text| parse_groups(&text))
            .unwrap_or_default();
        tracing::info!("whatsapp: {} chat(s) may draw", self.allowed.len());
    }

    /// Lets `chat` draw from now on; true when it could not before.
    fn add(&mut self, chat: &str) -> Result<bool> {
        self.refresh();
        if self.allowed.contains(chat) {
            return Ok(false);
        }
        let mut text = std::fs::read_to_string(&self.path).unwrap_or_default();
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(chat);
        text.push('\n');
        self.save(&text)?;
        Ok(true)
    }

    /// Stops `chat` from drawing; true when it could before.
    fn remove(&mut self, chat: &str) -> Result<bool> {
        self.refresh();
        if !self.allowed.contains(chat) {
            return Ok(false);
        }
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        let kept: String = text
            .lines()
            .filter(|line| line.split('#').next().unwrap_or_default().trim() != chat)
            .map(|line| format!("{line}\n"))
            .collect();
        self.save(&kept)?;
        Ok(true)
    }

    fn save(&mut self, text: &str) -> Result<()> {
        std::fs::write(&self.path, text).with_context(|| self.path.display().to_string())?;
        self.modified = None;
        self.refresh();
        Ok(())
    }

    /// Whether `chat` may draw; logs each other group once.
    fn admits(&mut self, chat: &str, is_group: bool) -> bool {
        self.refresh();
        if self.allowed.contains(chat) {
            return true;
        }
        if is_group && self.ignored.insert(chat.to_owned()) {
            tracing::info!("WhatsApp: ignoring group {chat}");
        }
        false
    }
}

type SharedGroups = Arc<Mutex<Groups>>;

/// One WhatsApp chat.
struct Conversation {
    client: Arc<Client>,
    chat: Jid,
}

impl Chat for Conversation {
    fn key(&self) -> String {
        format!("wa:{}", self.chat)
    }

    async fn send(&self, text: &str) -> Result<String> {
        let sent = self
            .client
            .send_message(self.chat.clone(), wa::Message::text(text))
            .await
            .map_err(|err| anyhow!("sending to WhatsApp: {err}"))?;
        Ok(sent.message_id)
    }

    async fn follow_up(&self, _waiting: &str, text: &str) -> Result<String> {
        self.send(text).await
    }
}

/// Keeps a WhatsApp client running from the session in `dir`, restarting it when it stops.
pub async fn run(dir: PathBuf, bot: flow::Bot) {
    let groups: SharedGroups = Arc::new(Mutex::new(Groups::new(dir.join(GROUPS_FILE))));
    loop {
        if let Err(err) = connect(&dir, &bot, &groups).await {
            tracing::error!("whatsapp: {err:#}");
        } else {
            tracing::warn!("whatsapp: the client stopped");
        }
        tokio::time::sleep(RESTART_AFTER).await;
    }
}

/// When the current pair code runs out; `None` asks for a new one.
type CodeExpiry = Arc<Mutex<Option<Instant>>>;

async fn connect(dir: &Path, bot: &flow::Bot, groups: &SharedGroups) -> Result<()> {
    let database = dir.join(DATABASE);
    let store = SqliteStore::new(&database.to_string_lossy())
        .await
        .map_err(|err| anyhow!("opening {}: {err}", database.display()))?;
    let expiry: CodeExpiry = Arc::default();
    let on_code = expiry.clone();
    let on_refresh = expiry.clone();
    let on_message = bot.clone();
    let message_groups = groups.clone();
    let qr_file = dir.join(QR_FILE);
    let linked_qr_file = qr_file.clone();
    let client = WaBot::builder()
        .with_backend(store)
        .skip_history_sync()
        .on_qr_code(move |code, timeout| {
            match std::fs::write(&qr_file, &code) {
                Ok(()) => tracing::info!(
                    "WhatsApp QR code in {} (valid {} s)",
                    qr_file.display(),
                    timeout.as_secs()
                ),
                Err(err) => tracing::warn!("writing {}: {err}", qr_file.display()),
            }
            async {}
        })
        .on_pair_code(move |code, timeout| {
            *on_code.lock().unwrap() = Some(Instant::now() + timeout);
            let code = match code.split_at_checked(4) {
                Some((a, b)) if code.len() == 8 => format!("{a}-{b}"),
                _ => code,
            };
            tracing::info!("WhatsApp pair code: {code} (valid {} s)", timeout.as_secs());
            async {}
        })
        .on_pair_code_refresh(move |_force_manual, _client| {
            *on_refresh.lock().unwrap() = None;
            async {}
        })
        .on_pair_code_error(|err, _client| {
            tracing::warn!("whatsapp: pairing failed: {}", err.error);
            async {}
        })
        .on_event_for(&[EventKind::PairSuccess], move |event, _client| {
            if let Event::PairSuccess(paired) = &*event {
                tracing::info!("WhatsApp linked as {}", paired.id);
                std::fs::remove_file(&linked_qr_file).ok();
            }
            async {}
        })
        .on_connected(|_client| {
            tracing::info!("whatsapp: connected");
            async {}
        })
        .on_logged_out(|info| {
            tracing::warn!("whatsapp: logged out: {:?}", info.reason);
            async {}
        })
        .on_message(move |ctx| {
            let bot = on_message.clone();
            let groups = message_groups.clone();
            async move {
                tokio::spawn(async move {
                    if let Err(err) = handle(bot, &groups, ctx).await {
                        tracing::warn!("whatsapp: {err:#}");
                    }
                });
            }
        })
        .build()
        .await
        .map_err(|err| anyhow!("starting the WhatsApp client: {err}"))?;
    let pairing = tokio::spawn(pair_when_asked(client.client(), dir.join(PHONE_FILE), expiry));
    tracing::info!("whatsapp: client started");
    client.run().await;
    pairing.abort();
    Ok(())
}

/// The number in the phone file, digits only.
fn phone_number(text: &str) -> Option<String> {
    let digits: String = text.chars().filter(char::is_ascii_digit).collect();
    (digits.len() >= 8).then_some(digits)
}

/// While the client is not linked, asks for a pair code whenever the phone
/// file names a number and no code is valid.
async fn pair_when_asked(client: Arc<Client>, phone_file: PathBuf, expiry: CodeExpiry) {
    let mut announced = false;
    loop {
        tokio::time::sleep(PAIR_CHECK).await;
        if !client.is_connected() {
            continue;
        }
        if client.is_logged_in() {
            return;
        }
        if !announced {
            tracing::info!(
                "whatsapp: not linked; write the bot's number to {} to get a pair code",
                phone_file.display()
            );
            announced = true;
        }
        let Some(phone) = std::fs::read_to_string(&phone_file)
            .ok()
            .and_then(|text| phone_number(&text))
        else {
            continue;
        };
        let valid = expiry.lock().unwrap().is_some_and(|until| Instant::now() < until);
        if valid {
            continue;
        }
        tracing::info!("whatsapp: asking for a pair code");
        let options = PairCodeOptions {
            phone_number: phone,
            ..Default::default()
        };
        if let Err(err) = client.pair_with_code(options).await {
            tracing::warn!("whatsapp: no pair code: {err}");
            *expiry.lock().unwrap() = Some(Instant::now() + PAIR_RETRY);
        }
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

async fn handle(bot: flow::Bot, groups: &SharedGroups, ctx: MessageContext) -> Result<()> {
    let info = &ctx.info;
    if info.source.is_from_me || !matches!(info.edit, EditAttribute::Empty) {
        return Ok(());
    }
    if now() - info.timestamp.timestamp() > STALE_AFTER_SECS {
        tracing::info!("whatsapp: skipping a message from while the bot was away");
        return Ok(());
    }
    let chat = Conversation {
        client: ctx.client.clone(),
        chat: info.source.chat.clone(),
    };
    let jid = info.source.chat.to_string();
    let is_group = info.source.is_group;
    let text = ctx.message.text_content().map(str::to_owned);
    match text.as_deref().and_then(switch) {
        Some(Switch::On) => {
            let added = groups.lock().unwrap().add(&jid)?;
            if added {
                tracing::info!(chat = jid, "whatsapp: hoi, this chat may draw now");
            }
            chat.send(greeting(is_group)).await?;
            return Ok(());
        }
        Some(Switch::Off) => {
            let removed = groups.lock().unwrap().remove(&jid)?;
            if removed {
                tracing::info!(chat = jid, "whatsapp: this chat stopped the bot");
                chat.send(FAREWELL).await?;
            }
            return Ok(());
        }
        None => {}
    }
    if !groups.lock().unwrap().admits(&jid, is_group) {
        return Ok(());
    }
    let base = ctx.message.get_base_message();
    if let Some(reaction) = base.reaction_message.as_option() {
        let Some(message) = reaction.key.as_option().and_then(|key| key.id.clone()) else {
            return Ok(());
        };
        let emoji = reaction.text.clone().unwrap_or_default();
        let (chat, user) = (chat.key(), format!("wa:{}", info.source.sender));
        let mut replies = bot.replies.lock().unwrap();
        if replies.is_reply(&chat, &message) {
            tracing::info!(new = emoji, "whatsapp reaction");
            let old = replies.reaction_of(&chat, &message, &user);
            let new = if emoji.is_empty() { vec![] } else { vec![emoji] };
            replies.reacted(&chat, &message, &user, old, new);
        }
        return Ok(());
    }
    let Some(text) = text else {
        return Ok(());
    };
    tracing::info!(chat = %info.source.chat, "whatsapp message");
    if !is_group {
        return bot.message(&chat, &text).await;
    }
    let replying = base
        .extended_text_message
        .as_option()
        .and_then(|text| text.context_info.as_option())
        .and_then(|context| context.stanza_id.as_deref())
        .is_some_and(|quoted| bot.replies.lock().unwrap().is_reply(&chat.key(), quoted));
    match addressed(&text, replying) {
        Some(text) => bot.message(&chat, &text).await,
        None => bot.command_message(&chat, &text).await,
    }
}

enum Switch {
    On,
    Off,
}

/// "hoi" lets a chat use the bot, "/stop" ends that; any case, nothing else in the message.
fn switch(text: &str) -> Option<Switch> {
    let text = text.trim().to_lowercase();
    match text.as_str() {
        "hoi" => Some(Switch::On),
        "/stop" => Some(Switch::Off),
        _ => None,
    }
}

fn greeting(is_group: bool) -> &'static str {
    if is_group {
        "Hoi zäme! /text <message> puts it on the display. /help lists the commands, /stop sends me away."
    } else {
        "Hoi! Whatever you write here goes on the display. /help lists the commands, /stop sends me away."
    }
}

const FAREWELL: &str = "Tschüss! I'm quiet here until someone writes hoi.";

/// The name that makes the bot listen to a group message, as in "Spades, Hoi".
const NAME: &str = "spades";

/// What a group message asks the bot for when it replies to the bot or names
/// it: the text without the name, `/help` when nothing else is left. `None`
/// when the message is not meant for the bot.
fn addressed(text: &str, replying: bool) -> Option<String> {
    let named = words(text).find(|&(_, word)| word.trim_start_matches('@').to_lowercase().starts_with(NAME));
    let rest = match named {
        Some((start, word)) => {
            let before = text[..start].trim_end();
            let after = text[start + word.len()..]
                .trim_start_matches([',', ':', '!', '.', ';'])
                .trim_start();
            if after.starts_with('/') || before.is_empty() {
                after.to_owned()
            } else {
                format!("{before} {after}")
            }
        }
        None if replying => text.to_owned(),
        None => return None,
    };
    let rest = rest.trim();
    Some(if rest.is_empty() {
        "/help".to_owned()
    } else {
        rest.to_owned()
    })
}

/// The whitespace-separated words of `text` with their byte offsets.
fn words(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.split_whitespace()
        .map(move |word| (word.as_ptr() as usize - text.as_ptr() as usize, word))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn naming_the_bot_or_replying_to_it_addresses_it() {
        assert_eq!(addressed("Spades, Hoi zäme", false).as_deref(), Some("Hoi zäme"));
        assert_eq!(addressed("hey @spadesvision /ring", false).as_deref(), Some("/ring"));
        assert_eq!(addressed("Hoi SPADES!", false).as_deref(), Some("Hoi"));
        assert_eq!(addressed("spades", false).as_deref(), Some("/help"));
        assert_eq!(addressed("Hoi zäme", true).as_deref(), Some("Hoi zäme"));
        assert_eq!(addressed("Hoi zäme", false), None);
        assert_eq!(addressed("/text Hoi", false), None);
    }

    #[test]
    fn the_groups_file_lists_one_jid_per_line() {
        let groups = parse_groups("# the display\n120363000000000001@g.us  # Familie\n\n  120363000000000002@g.us\n");
        assert_eq!(groups.len(), 2);
        assert!(groups.contains("120363000000000001@g.us"));
        assert!(groups.contains("120363000000000002@g.us"));
    }

    #[test]
    fn only_listed_chats_draw() {
        let dir = std::env::temp_dir().join(format!("epaper-groups-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(GROUPS_FILE);
        std::fs::remove_file(&path).ok();
        let mut groups = Groups::new(path.clone());
        assert!(!groups.admits("1@g.us", true));
        std::fs::write(&path, "1@g.us\n").unwrap();
        assert!(groups.admits("1@g.us", true));
        assert!(!groups.admits("2@g.us", true));
        assert!(!groups.admits("41000@s.whatsapp.net", false));
        assert!(groups.ignored.contains("2@g.us"));
        assert!(!groups.ignored.contains("41000@s.whatsapp.net"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn hoi_adds_a_chat_and_stop_removes_it() {
        let dir = std::env::temp_dir().join(format!("epaper-hoi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(GROUPS_FILE);
        std::fs::write(&path, "# the display\n1@g.us").unwrap();
        let mut groups = Groups::new(path.clone());
        assert!(groups.add("41000@s.whatsapp.net").unwrap());
        assert!(!groups.add("41000@s.whatsapp.net").unwrap());
        assert!(groups.admits("41000@s.whatsapp.net", false));
        assert!(groups.admits("1@g.us", true));
        assert!(groups.remove("1@g.us").unwrap());
        assert!(!groups.remove("1@g.us").unwrap());
        assert!(!groups.admits("1@g.us", true));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# the display\n41000@s.whatsapp.net\n"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn only_hoi_and_stop_switch_the_bot() {
        assert!(matches!(switch(" Hoi "), Some(Switch::On)));
        assert!(matches!(switch("HOI"), Some(Switch::On)));
        assert!(matches!(switch("/Stop"), Some(Switch::Off)));
        assert!(switch("hoi zäme").is_none());
        assert!(switch("/stop now").is_none());
    }

    #[test]
    fn the_phone_file_takes_digits_only() {
        assert_eq!(phone_number("+41 76 123 45 67\n").as_deref(), Some("41761234567"));
        assert_eq!(phone_number("41761234567").as_deref(), Some("41761234567"));
        assert_eq!(phone_number("  \n"), None);
        assert_eq!(phone_number("+41 76"), None);
    }
}
