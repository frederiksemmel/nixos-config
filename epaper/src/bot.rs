use std::{
    path::PathBuf,
    sync::{Arc, Mutex as SyncMutex},
};

use anyhow::Result;
use epd_waveshare::graphics::DisplayRotation;
use teloxide::{
    prelude::*,
    types::{MessageId, MessageReactionUpdated, ReactionType, User, UserId},
    utils::command::BotCommands,
};
use tokio::sync::Mutex;

use crate::{
    panel::Panel,
    render::{Ink, Scene, canvas_size, render},
    replies::{Kind, Replies},
};

#[derive(BotCommands, Clone, Debug)]
#[command(
    rename_rule = "lowercase",
    description = "Send any text to put it on the display, or:"
)]
enum Command {
    #[command(description = "show these commands")]
    Help,
    #[command(description = "show the hand")]
    Ring,
    #[command(description = "write text in black")]
    Text(String),
    #[command(description = "write text in red")]
    Red(String),
    #[command(description = "wipe the display")]
    Clear,
    #[command(description = "turn the picture: 0, 90, 180 or 270, or nothing for 90° more")]
    Rotate(String),
}

struct Screen {
    panel: Box<dyn Panel>,
    rotation: DisplayRotation,
    shown: Option<Scene>,
}

impl Screen {
    fn draw(&mut self) -> Result<()> {
        let Some(scene) = &self.shown else {
            return Ok(());
        };
        let canvas = render(scene, canvas_size(self.rotation))?;
        let planes = canvas.planes(self.rotation);
        self.panel.show(&canvas, &planes)
    }
}

#[derive(Clone)]
struct Display(Arc<Mutex<Screen>>);

#[derive(Debug, PartialEq, Eq)]
enum Shown<T> {
    Done(T),
    Busy,
}

impl Display {
    fn new(panel: Box<dyn Panel>, rotation: DisplayRotation) -> Self {
        Display(Arc::new(Mutex::new(Screen {
            panel,
            rotation,
            shown: None,
        })))
    }

    /// Applies `change` to the screen and redraws it, unless a drawing is already running.
    async fn update<T: Send + 'static>(
        &self,
        change: impl FnOnce(&mut Screen) -> T + Send + 'static,
    ) -> Result<Shown<T>> {
        let Ok(mut screen) = self.0.clone().try_lock_owned() else {
            return Ok(Shown::Busy);
        };
        let done = tokio::task::spawn_blocking(move || {
            let done = change(&mut screen);
            screen.draw().map(|()| done)
        })
        .await??;
        Ok(Shown::Done(done))
    }

    async fn show(&self, scene: Scene) -> Result<Shown<()>> {
        self.update(|screen| screen.shown = Some(scene)).await
    }

    async fn rotate(&self, to: Option<DisplayRotation>) -> Result<Shown<DisplayRotation>> {
        self.update(move |screen| {
            screen.rotation = to.unwrap_or_else(|| quarter_turn(screen.rotation));
            screen.rotation
        })
        .await
    }
}

fn quarter_turn(rotation: DisplayRotation) -> DisplayRotation {
    match rotation {
        DisplayRotation::Rotate0 => DisplayRotation::Rotate90,
        DisplayRotation::Rotate90 => DisplayRotation::Rotate180,
        DisplayRotation::Rotate180 => DisplayRotation::Rotate270,
        DisplayRotation::Rotate270 => DisplayRotation::Rotate0,
    }
}

fn degrees(rotation: DisplayRotation) -> u16 {
    match rotation {
        DisplayRotation::Rotate0 => 0,
        DisplayRotation::Rotate90 => 90,
        DisplayRotation::Rotate180 => 180,
        DisplayRotation::Rotate270 => 270,
    }
}

/// `Some(None)` means "a quarter turn further"; `None` means the argument is not an angle.
fn parse_rotation(arg: &str) -> Option<Option<DisplayRotation>> {
    match arg.trim().trim_end_matches('°') {
        "" => Some(None),
        "0" | "360" => Some(Some(DisplayRotation::Rotate0)),
        "90" => Some(Some(DisplayRotation::Rotate90)),
        "180" => Some(Some(DisplayRotation::Rotate180)),
        "270" => Some(Some(DisplayRotation::Rotate270)),
        _ => None,
    }
}

#[derive(Clone)]
struct Allowed(Arc<[UserId]>);

impl Allowed {
    fn admits(&self, user: Option<&User>) -> bool {
        self.0.is_empty() || user.is_some_and(|u| self.0.contains(&u.id))
    }
}

type SharedReplies = Arc<SyncMutex<Replies>>;

pub async fn run(
    token: String,
    panel: Box<dyn Panel>,
    rotation: DisplayRotation,
    allowed: Vec<u64>,
    state_dir: Option<PathBuf>,
) -> Result<()> {
    let bot = Bot::new(token);
    bot.set_my_commands(Command::bot_commands()).await?;
    let display = Display::new(panel, rotation);
    let allowed = Allowed(allowed.into_iter().map(UserId).collect());
    let replies: SharedReplies = Arc::new(SyncMutex::new(Replies::open(state_dir.as_deref())?));
    tracing::info!("bot is listening");

    let messages = Update::filter_message()
        .inspect(|msg: Message| {
            let user = msg.from.as_ref().map(|u| (u.id.0, u.full_name()));
            tracing::info!(?user, text = msg.text(), "message");
        })
        .filter(|msg: Message, allowed: Allowed| allowed.admits(msg.from.as_ref()))
        .branch(dptree::entry().filter_command::<Command>().endpoint(on_command))
        .branch(dptree::filter_map(|msg: Message| msg.text().map(str::to_owned)).endpoint(on_text));
    let reactions = Update::filter_message_reaction_updated()
        .filter(|reaction: MessageReactionUpdated, allowed: Allowed| allowed.admits(reaction.user()))
        .endpoint(on_reaction);

    Dispatcher::builder(bot, dptree::entry().branch(messages).branch(reactions))
        .dependencies(dptree::deps![display, allowed, replies])
        .default_handler(|_| async {})
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
    Ok(())
}

async fn on_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    display: Display,
    replies: SharedReplies,
) -> ResponseResult<()> {
    let scene = match cmd {
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string())
                .await?;
            return Ok(());
        }
        Command::Rotate(arg) => return rotate(bot, msg.chat.id, &arg, display, replies).await,
        Command::Ring => Scene::Ring,
        Command::Text(text) => text_scene(text, Ink::Black),
        Command::Red(text) => text_scene(text, Ink::Red),
        Command::Clear => Scene::Clear,
    };
    draw(bot, msg.chat.id, scene, display, replies).await
}

async fn on_text(bot: Bot, msg: Message, text: String, display: Display, replies: SharedReplies) -> ResponseResult<()> {
    draw(bot, msg.chat.id, text_scene(text, Ink::Black), display, replies).await
}

async fn on_reaction(reaction: MessageReactionUpdated, replies: SharedReplies) -> ResponseResult<()> {
    let emoji = |list: &[ReactionType]| list.iter().filter_map(|r| r.emoji().cloned()).collect::<Vec<_>>();
    let user = reaction.user().map_or(0, |u| u.id.0);
    tracing::info!(user, new = ?emoji(&reaction.new_reaction), "reaction");
    replies.lock().unwrap().reacted(
        reaction.chat.id.0,
        reaction.message_id.0,
        user,
        emoji(&reaction.old_reaction),
        emoji(&reaction.new_reaction),
    );
    Ok(())
}

fn text_scene(text: String, ink: Ink) -> Scene {
    if text.trim().is_empty() {
        Scene::Clear
    } else {
        Scene::Text { text, ink }
    }
}

/// A wait line in the chat that becomes the outcome once the display is done.
struct Pending {
    bot: Bot,
    chat: ChatId,
    message: MessageId,
    replies: SharedReplies,
}

impl Pending {
    async fn start(bot: Bot, chat: ChatId, replies: SharedReplies) -> ResponseResult<Self> {
        let line = replies.lock().unwrap().pick(Kind::Wait);
        let sent = bot.send_message(chat, &line).await?;
        replies.lock().unwrap().sent(chat.0, sent.id.0, Kind::Wait, &line);
        Ok(Pending {
            bot,
            chat,
            message: sent.id,
            replies,
        })
    }

    async fn finish(self, kind: Kind, fill: impl Fn(&str) -> String) -> ResponseResult<()> {
        let line = self.replies.lock().unwrap().pick(kind);
        self.bot.edit_message_text(self.chat, self.message, fill(&line)).await?;
        self.replies
            .lock()
            .unwrap()
            .sent(self.chat.0, self.message.0, kind, &line);
        Ok(())
    }
}

fn outcome<T>(shown: Result<Shown<T>>) -> Result<T, Kind> {
    match shown {
        Ok(Shown::Done(done)) => Ok(done),
        Ok(Shown::Busy) => Err(Kind::Busy),
        Err(err) => {
            tracing::error!("display failed: {err:#}");
            Err(Kind::Failed)
        }
    }
}

async fn draw(bot: Bot, chat: ChatId, scene: Scene, display: Display, replies: SharedReplies) -> ResponseResult<()> {
    let done = if scene == Scene::Ring {
        Kind::Ring
    } else {
        Kind::Written
    };
    let pending = Pending::start(bot, chat, replies).await?;
    let kind = outcome(display.show(scene).await).map_or_else(|kind| kind, |()| done);
    pending.finish(kind, str::to_owned).await
}

async fn rotate(bot: Bot, chat: ChatId, arg: &str, display: Display, replies: SharedReplies) -> ResponseResult<()> {
    let Some(to) = parse_rotation(arg) else {
        bot.send_message(chat, "Use /rotate 0, 90, 180 or 270, or just /rotate for 90° more")
            .await?;
        return Ok(());
    };
    let pending = Pending::start(bot, chat, replies).await?;
    match outcome(display.rotate(to).await) {
        Ok(rotation) => {
            let deg = degrees(rotation).to_string();
            pending.finish(Kind::Rotated, |line| line.replace("{deg}", &deg)).await
        }
        Err(kind) => pending.finish(kind, str::to_owned).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{Canvas, Planes};
    use std::sync::mpsc;

    struct Slow(mpsc::Receiver<()>);

    impl Panel for Slow {
        fn show(&mut self, _: &Canvas, _: &Planes) -> Result<()> {
            self.0.recv().ok();
            Ok(())
        }
    }

    #[test]
    fn commands_parse() {
        let parse = |s| Command::parse(s, "epaperbot").unwrap();
        assert!(matches!(parse("/text Hoi zäme"), Command::Text(t) if t == "Hoi zäme"));
        assert!(matches!(parse("/red@epaperbot rot"), Command::Red(t) if t == "rot"));
        assert!(matches!(parse("/ring"), Command::Ring));
        assert!(matches!(parse("/rotate 90"), Command::Rotate(a) if a == "90"));
        assert!(matches!(parse("/rotate"), Command::Rotate(a) if a.is_empty()));
    }

    #[test]
    fn rotation_arguments() {
        let parse = |arg| parse_rotation(arg).map(|to| to.map(degrees));
        assert_eq!(parse(""), Some(None));
        assert_eq!(parse(" 270° "), Some(Some(270)));
        assert_eq!(parse("45"), None);
        assert_eq!(degrees(quarter_turn(DisplayRotation::Rotate270)), 0);
    }

    #[test]
    fn blank_text_clears() {
        assert_eq!(text_scene("  ".into(), Ink::Red), Scene::Clear);
    }

    #[tokio::test]
    async fn a_second_drawing_waits_its_turn() {
        let (release, wait) = mpsc::channel();
        let display = Display::new(Box::new(Slow(wait)), DisplayRotation::Rotate90);
        let first = tokio::spawn({
            let display = display.clone();
            async move { display.show(Scene::Clear).await.unwrap() }
        });
        while display.0.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
        assert_eq!(display.show(Scene::Ring).await.unwrap(), Shown::Busy);
        drop(release);
        assert_eq!(first.await.unwrap(), Shown::Done(()));
        assert_eq!(display.show(Scene::Clear).await.unwrap(), Shown::Done(()));
    }

    #[tokio::test]
    async fn rotate_turns_a_quarter_without_an_angle() {
        let (release, wait) = mpsc::channel();
        drop(release);
        let display = Display::new(Box::new(Slow(wait)), DisplayRotation::Rotate90);
        let turned = |shown: Shown<DisplayRotation>| match shown {
            Shown::Done(rotation) => Some(degrees(rotation)),
            Shown::Busy => None,
        };
        assert_eq!(turned(display.rotate(None).await.unwrap()), Some(180));
        assert_eq!(
            turned(display.rotate(Some(DisplayRotation::Rotate0)).await.unwrap()),
            Some(0)
        );
    }
}
