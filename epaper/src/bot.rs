use std::sync::Arc;

use anyhow::Result;
use embedded_graphics::prelude::Size;
use epd_waveshare::graphics::DisplayRotation;
use rand::seq::IndexedRandom;
use teloxide::{prelude::*, types::UserId, utils::command::BotCommands};
use tokio::sync::Mutex;

use crate::{
    panel::Panel,
    render::{Ink, Scene, canvas_size, render},
};

const RING_REPLIES: [&str; 19] = [
    "ineigluegt",
    "ine g'luegt",
    "ii gluegt",
    "iinegluegt",
    "ine gluegt",
    "ini gluäggt",
    "ina g'luegt",
    "inegluegt",
    "ine gluet",
    "ine glueget",
    "ine glügät",
    "ine glüget",
    "ine g'lüegt",
    "ine g'lüäget",
    "ine glüegt",
    "ine g'lüaget",
    "ine g'luget",
    "ine glüäget",
    "ine glüeget",
];

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
}

#[derive(Clone)]
struct Display {
    panel: Arc<Mutex<Box<dyn Panel>>>,
    rotation: DisplayRotation,
    size: Size,
}

#[derive(Debug, PartialEq, Eq)]
enum Shown {
    Done,
    Busy,
}

impl Display {
    async fn show(&self, scene: Scene) -> Result<Shown> {
        let Ok(mut panel) = self.panel.clone().try_lock_owned() else {
            return Ok(Shown::Busy);
        };
        let canvas = render(&scene, self.size);
        let planes = canvas.planes(self.rotation);
        tokio::task::spawn_blocking(move || panel.show(&canvas, &planes)).await??;
        Ok(Shown::Done)
    }
}

#[derive(Clone)]
struct Allowed(Arc<[UserId]>);

impl Allowed {
    fn admits(&self, msg: &Message) -> bool {
        self.0.is_empty() || msg.from.as_ref().is_some_and(|u| self.0.contains(&u.id))
    }
}

pub async fn run(token: String, panel: Box<dyn Panel>, rotation: DisplayRotation, allowed: Vec<u64>) -> Result<()> {
    let bot = Bot::new(token);
    bot.set_my_commands(Command::bot_commands()).await?;
    let display = Display {
        panel: Arc::new(Mutex::new(panel)),
        rotation,
        size: canvas_size(rotation),
    };
    let allowed = Allowed(allowed.into_iter().map(UserId).collect());
    tracing::info!("bot is listening");

    let handler = Update::filter_message()
        .inspect(|msg: Message| {
            let user = msg.from.as_ref().map(|u| (u.id.0, u.full_name()));
            tracing::info!(?user, text = msg.text(), "message");
        })
        .filter(|msg: Message, allowed: Allowed| allowed.admits(&msg))
        .branch(dptree::entry().filter_command::<Command>().endpoint(on_command))
        .branch(dptree::filter_map(|msg: Message| msg.text().map(str::to_owned)).endpoint(on_text));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![display, allowed])
        .default_handler(|_| async {})
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
    Ok(())
}

async fn on_command(bot: Bot, msg: Message, cmd: Command, display: Display) -> ResponseResult<()> {
    let scene = match cmd {
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string())
                .await?;
            return Ok(());
        }
        Command::Ring => Scene::Ring,
        Command::Text(text) => text_scene(text, Ink::Black),
        Command::Red(text) => text_scene(text, Ink::Red),
        Command::Clear => Scene::Clear,
    };
    draw(bot, msg, scene, display).await
}

async fn on_text(bot: Bot, msg: Message, text: String, display: Display) -> ResponseResult<()> {
    draw(bot, msg, text_scene(text, Ink::Black), display).await
}

fn text_scene(text: String, ink: Ink) -> Scene {
    if text.trim().is_empty() {
        Scene::Clear
    } else {
        Scene::Text { text, ink }
    }
}

async fn draw(bot: Bot, msg: Message, scene: Scene, display: Display) -> ResponseResult<()> {
    let chat = msg.chat.id;
    let ring = scene == Scene::Ring;
    if ring {
        let reply = RING_REPLIES.choose(&mut rand::rng()).copied().unwrap_or("ine gluegt");
        bot.send_message(chat, reply).await?;
    }
    let pending = bot.send_message(chat, "Wait for it...").await?;
    let reply = match display.show(scene).await {
        Ok(Shown::Done) if ring => "Look at the display",
        Ok(Shown::Done) => "Written it is",
        Ok(Shown::Busy) => "Daddy chill, I'm still drawing",
        Err(err) => {
            tracing::error!("display failed: {err:#}");
            "The display failed, see the journal"
        }
    };
    bot.edit_message_text(chat, pending.id, reply).await?;
    Ok(())
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
    }

    #[test]
    fn blank_text_clears() {
        assert_eq!(text_scene("  ".into(), Ink::Red), Scene::Clear);
    }

    #[tokio::test]
    async fn a_second_drawing_waits_its_turn() {
        let (release, wait) = mpsc::channel();
        let display = Display {
            panel: Arc::new(Mutex::new(Box::new(Slow(wait)))),
            rotation: DisplayRotation::Rotate180,
            size: canvas_size(DisplayRotation::Rotate180),
        };
        let first = tokio::spawn({
            let display = display.clone();
            async move { display.show(Scene::Clear).await.unwrap() }
        });
        while display.panel.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
        assert_eq!(display.show(Scene::Ring).await.unwrap(), Shown::Busy);
        drop(release);
        assert_eq!(first.await.unwrap(), Shown::Done);
        assert_eq!(display.show(Scene::Clear).await.unwrap(), Shown::Done);
    }
}
