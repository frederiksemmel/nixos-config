use std::sync::Arc;

use anyhow::Result;
use teloxide::{
    net::Download,
    prelude::*,
    types::{FileId, MessageId, MessageReactionUpdated, ReactionType, Sticker, User, UserId},
    utils::command::BotCommands,
};

use crate::flow::{self, Chat, Command};

#[derive(Clone)]
struct Allowed(Arc<[UserId]>);

impl Allowed {
    fn admits(&self, user: Option<&User>) -> bool {
        self.0.is_empty() || user.is_some_and(|u| self.0.contains(&u.id))
    }
}

/// One Telegram chat.
struct Conversation {
    bot: Bot,
    chat: ChatId,
}

impl Chat for Conversation {
    fn key(&self) -> String {
        format!("tg:{}", self.chat.0)
    }

    async fn send(&self, text: &str) -> Result<String> {
        Ok(self.bot.send_message(self.chat, text).await?.id.0.to_string())
    }

    async fn follow_up(&self, waiting: &str, text: &str) -> Result<String> {
        let id = MessageId(waiting.parse()?);
        self.bot.edit_message_text(self.chat, id, text).await?;
        Ok(waiting.to_owned())
    }
}

pub async fn run(token: String, bot: flow::Bot, allowed: Vec<u64>) -> Result<()> {
    let telegram = Bot::new(token);
    telegram.set_my_commands(Command::bot_commands()).await?;
    let allowed = Allowed(allowed.into_iter().map(UserId).collect());
    tracing::info!("telegram bot is listening");

    let messages = Update::filter_message()
        .inspect(|msg: Message| {
            let user = msg.from.as_ref().map(|u| (u.id.0, u.full_name()));
            tracing::info!(?user, text = msg.text(), "message");
        })
        .filter(|msg: Message, allowed: Allowed| allowed.admits(msg.from.as_ref()))
        .branch(dptree::entry().filter_command::<Command>().endpoint(on_command))
        .branch(dptree::filter_map(|msg: Message| msg.sticker().cloned()).endpoint(on_sticker))
        .branch(dptree::filter_map(|msg: Message| clip(&msg)).endpoint(on_clip))
        .branch(dptree::filter_map(|msg: Message| msg.text().map(str::to_owned)).endpoint(on_text));
    let reactions = Update::filter_message_reaction_updated()
        .filter(|reaction: MessageReactionUpdated, allowed: Allowed| allowed.admits(reaction.user()))
        .endpoint(on_reaction);

    Dispatcher::builder(telegram, dptree::entry().branch(messages).branch(reactions))
        .dependencies(dptree::deps![bot, allowed])
        .default_handler(|_| async {})
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
    Ok(())
}

fn conversation(telegram: Bot, msg: &Message) -> Conversation {
    Conversation {
        bot: telegram,
        chat: msg.chat.id,
    }
}

fn logged(result: Result<()>) -> ResponseResult<()> {
    if let Err(err) = result {
        tracing::warn!("telegram: {err:#}");
    }
    Ok(())
}

async fn on_command(telegram: Bot, msg: Message, cmd: Command, bot: flow::Bot) -> ResponseResult<()> {
    let chat = conversation(telegram, &msg);
    logged(bot.command(&chat, cmd).await)
}

async fn on_text(telegram: Bot, msg: Message, text: String, bot: flow::Bot) -> ResponseResult<()> {
    let chat = conversation(telegram, &msg);
    logged(bot.message(&chat, &text).await)
}

/// Draws a still sticker, or the still thumbnail of one that moves.
async fn on_sticker(telegram: Bot, msg: Message, sticker: Sticker, bot: flow::Bot) -> ResponseResult<()> {
    let chat = conversation(telegram.clone(), &msg);
    let file = if sticker.is_static() {
        Some(sticker.file.id)
    } else {
        sticker.thumbnail.map(|thumbnail| thumbnail.file.id)
    };
    let Some(file) = file else {
        return logged(chat.send(flow::STILL_STICKERS_ONLY).await.map(drop));
    };
    logged(bot.draw_picture(&chat, download(&telegram, file)).await)
}

/// A GIF, which Telegram sends as an MP4 animation, or a GIF sent as a file.
fn clip(msg: &Message) -> Option<FileId> {
    if let Some(animation) = msg.animation() {
        return Some(animation.file.id.clone());
    }
    msg.document()
        .filter(|document| {
            document
                .mime_type
                .as_ref()
                .is_some_and(|mime| mime.essence_str() == "image/gif")
        })
        .map(|document| document.file.id.clone())
}

async fn on_clip(telegram: Bot, msg: Message, file: FileId, bot: flow::Bot) -> ResponseResult<()> {
    let chat = conversation(telegram.clone(), &msg);
    logged(bot.draw_clip(&chat, download(&telegram, file)).await)
}

async fn download(telegram: &Bot, file: FileId) -> Result<Vec<u8>> {
    let file = telegram.get_file(file).await?;
    let mut bytes = Vec::new();
    telegram.download_file(&file.path, &mut bytes).await?;
    Ok(bytes)
}

async fn on_reaction(reaction: MessageReactionUpdated, bot: flow::Bot) -> ResponseResult<()> {
    let emoji = |list: &[ReactionType]| list.iter().filter_map(|r| r.emoji().cloned()).collect::<Vec<_>>();
    let user = reaction.user().map_or(0, |u| u.id.0);
    tracing::info!(user, new = ?emoji(&reaction.new_reaction), "reaction");
    bot.replies.lock().unwrap().reacted(
        &format!("tg:{}", reaction.chat.id.0),
        &reaction.message_id.0.to_string(),
        &format!("tg:{user}"),
        emoji(&reaction.old_reaction),
        emoji(&reaction.new_reaction),
    );
    Ok(())
}
