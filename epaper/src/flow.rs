//! What the bot does with a message, whichever chat it came from.

use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, Mutex as SyncMutex},
};

use anyhow::Result;
use epd_waveshare::graphics::DisplayRotation;
use teloxide::utils::command::BotCommands;
use tokio::sync::Mutex;

use crate::{
    panel::Panel,
    render::{Scene, canvas_size, render},
    replies::{Kind, Replies},
};

#[derive(BotCommands, Clone, Debug)]
#[command(rename_rule = "lowercase", description = "Commands for the e-Paper display:")]
pub enum Command {
    #[command(description = "show these commands")]
    Help,
    #[command(description = "show the hand")]
    Ring,
    #[command(description = "write text in black")]
    Text(String),
    #[command(description = "wipe the display")]
    Clear,
    #[command(description = "turn the picture: 0, 90, 180 or 270, or nothing for 90° more")]
    Rotate(String),
}

/// A conversation the bot answers in.
pub trait Chat: Send + Sync {
    /// Identifies the chat in the reply log, such as `tg:<id>` or `wa:<jid>`.
    fn key(&self) -> String;

    /// Posts `text` and returns the new message's id.
    fn send(&self, text: &str) -> impl Future<Output = Result<String>> + Send;

    /// Tells how what the bot's `waiting` message announced turned out, and returns the id of the message that tells it.
    fn follow_up(&self, waiting: &str, text: &str) -> impl Future<Output = Result<String>> + Send;
}

pub type SharedReplies = Arc<SyncMutex<Replies>>;

pub struct Screen {
    panel: Box<dyn Panel>,
    rotation: DisplayRotation,
    rotation_file: Option<PathBuf>,
    shown: Option<Scene>,
}

impl Screen {
    fn draw(&mut self) -> Result<()> {
        let Some(scene) = &self.shown else {
            return Ok(());
        };
        let canvas = render(scene, canvas_size(self.rotation))?;
        let planes = canvas.planes(self.rotation);
        self.panel.show(&canvas, &planes, scene.refresh())
    }
}

#[derive(Clone)]
pub struct Display(Arc<Mutex<Screen>>);

#[derive(Debug, PartialEq, Eq)]
pub enum Shown<T> {
    Done(T),
    Busy,
}

impl Display {
    /// Starts with the rotation saved in `rotation_file`, or `default` while none is saved.
    pub fn new(panel: Box<dyn Panel>, default: DisplayRotation, rotation_file: Option<PathBuf>) -> Self {
        let rotation = rotation_file.as_deref().and_then(read_rotation).unwrap_or(default);
        Display(Arc::new(Mutex::new(Screen {
            panel,
            rotation,
            rotation_file,
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

    pub async fn show(&self, scene: Scene) -> Result<Shown<()>> {
        self.update(|screen| screen.shown = Some(scene)).await
    }

    pub async fn rotate(&self, to: Option<DisplayRotation>) -> Result<Shown<DisplayRotation>> {
        self.update(move |screen| {
            screen.rotation = to.unwrap_or_else(|| quarter_turn(screen.rotation));
            if let Some(file) = &screen.rotation_file
                && let Err(err) = std::fs::write(file, format!("{}\n", degrees(screen.rotation)))
            {
                tracing::warn!("saving the rotation to {}: {err}", file.display());
            }
            screen.rotation
        })
        .await
    }
}

fn read_rotation(file: &Path) -> Option<DisplayRotation> {
    parse_rotation(&std::fs::read_to_string(file).ok()?).flatten()
}

/// The display and the reply lines, shared by every chat.
#[derive(Clone)]
pub struct Bot {
    pub display: Display,
    pub replies: SharedReplies,
}

pub fn quarter_turn(rotation: DisplayRotation) -> DisplayRotation {
    match rotation {
        DisplayRotation::Rotate0 => DisplayRotation::Rotate90,
        DisplayRotation::Rotate90 => DisplayRotation::Rotate180,
        DisplayRotation::Rotate180 => DisplayRotation::Rotate270,
        DisplayRotation::Rotate270 => DisplayRotation::Rotate0,
    }
}

pub fn degrees(rotation: DisplayRotation) -> u16 {
    match rotation {
        DisplayRotation::Rotate0 => 0,
        DisplayRotation::Rotate90 => 90,
        DisplayRotation::Rotate180 => 180,
        DisplayRotation::Rotate270 => 270,
    }
}

/// `Some(None)` means "a quarter turn further"; `None` means the argument is not an angle.
pub fn parse_rotation(arg: &str) -> Option<Option<DisplayRotation>> {
    match arg.trim().trim_end_matches('°') {
        "" => Some(None),
        "0" | "360" => Some(Some(DisplayRotation::Rotate0)),
        "90" => Some(Some(DisplayRotation::Rotate90)),
        "180" => Some(Some(DisplayRotation::Rotate180)),
        "270" => Some(Some(DisplayRotation::Rotate270)),
        _ => None,
    }
}

/// `None` for a message that is not a command; `Some(Err(()))` for an unknown one. The command word may have any case.
pub fn parse_command(text: &str) -> Option<Result<Command, ()>> {
    let text = text.trim_start();
    if !text.starts_with('/') {
        return None;
    }
    let word_end = text.find(char::is_whitespace).unwrap_or(text.len());
    let text = format!("{}{}", text[..word_end].to_lowercase(), &text[word_end..]);
    Some(Command::parse(&text, "").map_err(|_| ()))
}

pub fn text_scene(text: String) -> Scene {
    if text.trim().is_empty() {
        Scene::Clear
    } else {
        Scene::Text { text }
    }
}

impl Bot {
    /// Handles a plain message: a command, or else text to draw.
    pub async fn message<C: Chat>(&self, chat: &C, text: &str) -> Result<()> {
        match parse_command(text) {
            Some(Ok(command)) => self.command(chat, command).await,
            Some(Err(())) => self.command(chat, Command::Help).await,
            None => self.draw(chat, text_scene(text.to_owned())).await,
        }
    }

    /// Handles a message from a shared chat: commands only, everything else is chat.
    pub async fn command_message<C: Chat>(&self, chat: &C, text: &str) -> Result<()> {
        match parse_command(text) {
            Some(Ok(command)) => self.command(chat, command).await,
            Some(Err(())) => self.command(chat, Command::Help).await,
            None => Ok(()),
        }
    }

    pub async fn command<C: Chat>(&self, chat: &C, command: Command) -> Result<()> {
        let scene = match command {
            Command::Help => {
                chat.send(&Command::descriptions().to_string()).await?;
                return Ok(());
            }
            Command::Rotate(arg) => return self.rotate(chat, &arg).await,
            Command::Text(text) if text.trim().is_empty() => {
                chat.send("Use /text <message>").await?;
                return Ok(());
            }
            Command::Ring => Scene::Ring,
            Command::Text(text) => text_scene(text),
            Command::Clear => Scene::Clear,
        };
        self.draw(chat, scene).await
    }

    /// Says a wait line, draws, then tells the outcome.
    pub async fn draw<C: Chat>(&self, chat: &C, scene: Scene) -> Result<()> {
        let done = if scene == Scene::Ring {
            Kind::Ring
        } else {
            Kind::Written
        };
        let pending = self.start(chat).await?;
        let kind = outcome(self.display.show(scene).await).map_or_else(|kind| kind, |()| done);
        self.finish(chat, &pending, kind, str::to_owned).await
    }

    pub async fn rotate<C: Chat>(&self, chat: &C, arg: &str) -> Result<()> {
        let Some(to) = parse_rotation(arg) else {
            chat.send("Use /rotate 0, 90, 180 or 270, or just /rotate for 90° more")
                .await?;
            return Ok(());
        };
        let pending = self.start(chat).await?;
        match outcome(self.display.rotate(to).await) {
            Ok(rotation) => {
                let deg = degrees(rotation).to_string();
                self.finish(chat, &pending, Kind::Rotated, |line| line.replace("{deg}", &deg))
                    .await
            }
            Err(kind) => self.finish(chat, &pending, kind, str::to_owned).await,
        }
    }

    async fn start<C: Chat>(&self, chat: &C) -> Result<String> {
        let line = self.replies.lock().unwrap().pick(Kind::Wait);
        let message = chat.send(&line).await?;
        self.replies
            .lock()
            .unwrap()
            .sent(&chat.key(), &message, Kind::Wait, &line);
        Ok(message)
    }

    async fn finish<C: Chat>(&self, chat: &C, waiting: &str, kind: Kind, fill: impl Fn(&str) -> String) -> Result<()> {
        let line = self.replies.lock().unwrap().pick(kind);
        let message = chat.follow_up(waiting, &fill(&line)).await?;
        self.replies.lock().unwrap().sent(&chat.key(), &message, kind, &line);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        panel::Refresh,
        render::{Canvas, Planes},
    };
    use std::sync::mpsc;

    struct Slow(mpsc::Receiver<()>);

    impl Panel for Slow {
        fn show(&mut self, _: &Canvas, _: &Planes, _: Refresh) -> Result<()> {
            self.0.recv().ok();
            Ok(())
        }
    }

    struct Instant;

    impl Panel for Instant {
        fn show(&mut self, _: &Canvas, _: &Planes, _: Refresh) -> Result<()> {
            Ok(())
        }
    }

    /// A chat that remembers every message as it now reads.
    #[derive(Default)]
    struct Fake(SyncMutex<Vec<String>>);

    impl Chat for Fake {
        fn key(&self) -> String {
            "fake:1".into()
        }

        async fn send(&self, text: &str) -> Result<String> {
            let mut messages = self.0.lock().unwrap();
            messages.push(text.to_owned());
            Ok((messages.len() - 1).to_string())
        }

        async fn follow_up(&self, waiting: &str, text: &str) -> Result<String> {
            self.0.lock().unwrap()[waiting.parse::<usize>()?] = text.to_owned();
            Ok(waiting.to_owned())
        }
    }

    impl Fake {
        fn messages(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }

    fn bot(panel: Box<dyn Panel>) -> Bot {
        Bot {
            display: Display::new(panel, DisplayRotation::Rotate90, None),
            replies: Arc::new(SyncMutex::new(Replies::open(None).unwrap())),
        }
    }

    #[test]
    fn commands_parse() {
        let parse = |s| Command::parse(s, "epaperbot").unwrap();
        assert!(matches!(parse("/text Hoi zäme"), Command::Text(t) if t == "Hoi zäme"));
        assert!(matches!(parse("/text@epaperbot Hoi"), Command::Text(t) if t == "Hoi"));
        assert!(Command::parse("/red rot", "epaperbot").is_err());
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
        assert_eq!(text_scene("  ".into()), Scene::Clear);
    }

    #[tokio::test]
    async fn a_drawing_says_wait_then_becomes_the_outcome() {
        let bot = bot(Box::new(Instant));
        let chat = Fake::default();
        bot.message(&chat, "/clear").await.unwrap();
        assert_eq!(chat.messages(), ["Written it is"]);
        bot.message(&chat, "/ring").await.unwrap();
        assert_eq!(chat.messages(), ["Written it is", "ine gluegt"]);
        bot.message(&chat, "/rotate").await.unwrap();
        assert_eq!(chat.messages()[2], "Rotated to 180°");
    }

    #[tokio::test]
    async fn the_wait_line_is_logged_before_the_outcome() {
        let bot = bot(Box::new(Instant));
        let chat = Fake::default();
        bot.message(&chat, "/clear").await.unwrap();
        let replies = bot.replies.lock().unwrap();
        assert!(replies.is_reply("fake:1", "0"));
    }

    #[tokio::test]
    async fn help_and_bad_angles_answer_without_drawing() {
        let bot = bot(Box::new(Instant));
        let chat = Fake::default();
        bot.message(&chat, "/help").await.unwrap();
        bot.message(&chat, "/rotate 45").await.unwrap();
        let messages = chat.messages();
        assert!(messages[0].contains("/rotate"));
        assert!(messages[1].starts_with("Use /rotate"));
    }

    #[tokio::test]
    async fn shared_chats_only_answer_commands() {
        let bot = bot(Box::new(Instant));
        let chat = Fake::default();
        bot.command_message(&chat, "Hoi zäme").await.unwrap();
        assert!(chat.messages().is_empty());
        bot.command_message(&chat, "/text Hoi").await.unwrap();
        assert_eq!(chat.messages(), ["Written it is"]);
    }

    #[tokio::test]
    async fn a_bare_text_command_explains_instead_of_wiping() {
        let bot = bot(Box::new(Instant));
        let chat = Fake::default();
        bot.message(&chat, "/text").await.unwrap();
        bot.message(&chat, "/text   ").await.unwrap();
        let messages = chat.messages();
        assert!(messages.iter().all(|m| m.starts_with("Use /text")), "{messages:?}");
    }

    #[tokio::test]
    async fn a_second_drawing_waits_its_turn() {
        let (release, wait) = mpsc::channel();
        let display = Display::new(Box::new(Slow(wait)), DisplayRotation::Rotate90, None);
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
        let display = Display::new(Box::new(Slow(wait)), DisplayRotation::Rotate90, None);
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

    #[tokio::test]
    async fn the_rotation_outlasts_a_restart() {
        let file = std::env::temp_dir().join(format!("epaper-rotation-{}", std::process::id()));
        std::fs::remove_file(&file).ok();
        let display = Display::new(Box::new(Instant), DisplayRotation::Rotate90, Some(file.clone()));
        display.rotate(Some(DisplayRotation::Rotate270)).await.unwrap();

        let restarted = Display::new(Box::new(Instant), DisplayRotation::Rotate90, Some(file.clone()));
        let rotation = restarted.0.lock().await.rotation;
        std::fs::remove_file(&file).ok();
        assert_eq!(degrees(rotation), 270);
    }

    #[test]
    fn commands_have_any_case_and_unknown_ones_are_marked() {
        assert!(matches!(parse_command("/Text Hoi zäme"), Some(Ok(Command::Text(t))) if t == "Hoi zäme"));
        assert!(matches!(parse_command(" /CLEAR"), Some(Ok(Command::Clear))));
        assert!(matches!(parse_command("/draw hi"), Some(Err(()))));
        assert!(parse_command("hoi /text").is_none());
    }

    #[tokio::test]
    async fn an_unknown_command_gets_the_help() {
        let chat = Fake::default();
        bot(Box::new(Instant)).command_message(&chat, "/draw hi").await.unwrap();
        assert!(chat.messages()[0].contains("/text"));
    }
}
