//! What the bot does with a message, whichever chat it came from.

use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, Mutex as SyncMutex},
    time::{Duration, Instant},
};

use anyhow::Result;
use epd_waveshare::graphics::DisplayRotation;
use image::RgbaImage;
use teloxide::utils::command::BotCommands;
use tokio::sync::Mutex;

use crate::{
    animation,
    panel::Panel,
    picture,
    render::{Scene, canvas_size, render},
    replies::{Message, Replies},
    xkcd::{Fetched, Pick, Xkcd},
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
    #[command(description = "answer in a mode, such as /mode office; /mode swiss is the usual")]
    Mode(String),
    #[command(description = "draw the newest xkcd, /xkcd random or /xkcd <number>")]
    Xkcd(String),
}

/// A conversation the bot answers in.
pub trait Chat: Send + Sync {
    /// Identifies the chat in the reply log, such as `tg:<id>` or `wa:<jid>`.
    fn key(&self) -> String;

    /// Identifies who wrote the message being answered; in a private chat, the chat itself.
    fn author(&self) -> String {
        self.key()
    }

    /// Posts `text` and returns the new message's id.
    fn send(&self, text: &str) -> impl Future<Output = Result<String>> + Send;

    /// Tells how what the bot's `waiting` message announced turned out, and returns the id of the message that tells it.
    fn follow_up(&self, waiting: &str, text: &str) -> impl Future<Output = Result<String>> + Send;
}

/// The answer to a sticker that moves and has no still picture to draw.
pub const STILL_STICKERS_ONLY: &str = "I can only draw still stickers.";

pub type SharedReplies = Arc<SyncMutex<Replies>>;

/// How long after a sticker its text may follow, or after a text its sticker, to share the panel.
pub const PAIRING: Duration = Duration::from_secs(30);

/// Half of what a sticker scene is made of.
#[derive(Clone, Debug)]
pub enum Part {
    Text(String),
    Sticker(Arc<RgbaImage>),
}

impl Part {
    fn scene(self) -> Scene {
        match self {
            Part::Text(text) => Scene::Text { text },
            Part::Sticker(picture) => Scene::Picture(picture),
        }
    }

    /// The scene of both parts, when one is a sticker and the other a text.
    fn with(self, other: Part) -> Option<Scene> {
        match (self, other) {
            (Part::Text(text), Part::Sticker(picture)) | (Part::Sticker(picture), Part::Text(text)) => {
                Some(Scene::Sticker { picture, text })
            }
            _ => None,
        }
    }

    fn is_text(&self) -> bool {
        matches!(self, Part::Text(_))
    }
}

/// The last sticker or text someone drew, which their next text or sticker joins.
struct Unpaired {
    author: String,
    at: Instant,
    part: Part,
}

impl Unpaired {
    fn joins(&self, author: &str, text: bool) -> bool {
        self.author == author && self.at.elapsed() < PAIRING && self.part.is_text() != text
    }
}

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

    /// Plays a clip with fast refreshes, then draws its first picture in four shades, so no ghosts stay.
    fn play(&mut self, clip: &[u8]) -> Result<()> {
        let reel = animation::decode(clip, self.rotation, animation::LIMIT)?;
        self.panel.play(&reel)?;
        self.shown = Some(Scene::Picture(Arc::new(reel.cover)));
        self.draw()
    }
}

#[derive(Clone)]
pub struct Display {
    screen: Arc<Mutex<Screen>>,
    unpaired: Arc<SyncMutex<Option<Unpaired>>>,
    /// Who last asked for a sticker or a text, when, and whether it was a text; a sticker counts before it is fetched.
    heard: Arc<SyncMutex<Option<(String, Instant, bool)>>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Shown<T> {
    Done(T),
    Busy,
}

impl Display {
    /// Starts with the rotation saved in `rotation_file`, or `default` while none is saved.
    pub fn new(panel: Box<dyn Panel>, default: DisplayRotation, rotation_file: Option<PathBuf>) -> Self {
        let rotation = rotation_file.as_deref().and_then(read_rotation).unwrap_or(default);
        Display {
            screen: Arc::new(Mutex::new(Screen {
                panel,
                rotation,
                rotation_file,
                shown: None,
            })),
            unpaired: Arc::default(),
            heard: Arc::default(),
        }
    }

    /// Whether `author` asked for the other kind of part than `text` says within [`PAIRING`], so the two share the panel.
    pub fn pairs(&self, author: &str, text: bool) -> bool {
        self.heard
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|(who, at, was_text)| who == author && at.elapsed() < PAIRING && *was_text != text)
    }

    fn hear(&self, author: &str, text: bool) {
        *self.heard.lock().unwrap() = Some((author.to_owned(), Instant::now(), text));
    }

    /// Draws `part`, together with the other kind of part `author` drew within [`PAIRING`]. That drawing
    /// may still be running, so a pair waits for the panel instead of being busy.
    pub async fn show_part(&self, author: &str, part: Part) -> Result<Shown<()>> {
        let paired = {
            let mut unpaired = self.unpaired.lock().unwrap();
            match unpaired.take() {
                Some(last) if last.joins(author, part.is_text()) => last.part.with(part.clone()),
                _ => {
                    *unpaired = Some(Unpaired {
                        author: author.to_owned(),
                        at: Instant::now(),
                        part: part.clone(),
                    });
                    None
                }
            }
        };
        match paired {
            Some(scene) => {
                let mut screen = self.screen.clone().lock_owned().await;
                tokio::task::spawn_blocking(move || {
                    screen.shown = Some(scene);
                    screen.draw()
                })
                .await??;
                Ok(Shown::Done(()))
            }
            None => self.show(part.scene()).await,
        }
    }

    /// Does `work` on the screen, unless a drawing is already running.
    async fn locked<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Screen) -> Result<T> + Send + 'static,
    ) -> Result<Shown<T>> {
        let Ok(mut screen) = self.screen.clone().try_lock_owned() else {
            return Ok(Shown::Busy);
        };
        let done = tokio::task::spawn_blocking(move || work(&mut screen)).await??;
        Ok(Shown::Done(done))
    }

    /// Applies `change` to the screen and redraws it, unless a drawing is already running.
    async fn update<T: Send + 'static>(
        &self,
        change: impl FnOnce(&mut Screen) -> T + Send + 'static,
    ) -> Result<Shown<T>> {
        self.locked(|screen| {
            let done = change(screen);
            screen.draw().map(|()| done)
        })
        .await
    }

    /// Plays a GIF or video clip; the panel stays busy until the clip has played and its last drawing is done.
    pub async fn play(&self, clip: Vec<u8>) -> Result<Shown<()>> {
        self.locked(move |screen| screen.play(&clip)).await
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

/// The display, the reply lines and the comic source, shared by every chat.
#[derive(Clone)]
pub struct Bot {
    pub display: Display,
    pub replies: SharedReplies,
    pub xkcd: Xkcd,
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
            Command::Mode(arg) => return self.mode(chat, &arg).await,
            Command::Xkcd(arg) => return self.comic(chat, &arg).await,
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
            Message::Ring
        } else {
            Message::Written
        };
        if matches!(scene, Scene::Text { .. }) {
            self.display.hear(&chat.author(), true);
        }
        let pending = self.start(chat).await?;
        let shown = match scene {
            Scene::Text { text } => self.display.show_part(&chat.author(), Part::Text(text)).await,
            scene => self.display.show(scene).await,
        };
        let message = outcome(shown).map_or_else(|message| message, |()| done);
        self.finish(chat, &pending, message, str::to_owned).await
    }

    /// Says a wait line, fetches a sticker and draws it, beside a text its author sent just before or after; then tells the outcome.
    pub async fn draw_picture<C: Chat>(&self, chat: &C, fetch: impl Future<Output = Result<Vec<u8>>>) -> Result<()> {
        self.display.hear(&chat.author(), false);
        let pending = self.start(chat).await?;
        let message = match fetch.await.and_then(|bytes| picture::decode(&bytes)) {
            Ok(image) => outcome(
                self.display
                    .show_part(&chat.author(), Part::Sticker(Arc::new(image)))
                    .await,
            )
            .map_or_else(|message| message, |()| Message::Written),
            Err(err) => {
                tracing::warn!("no picture to draw: {err:#}");
                Message::Failed
            }
        };
        self.finish(chat, &pending, message, str::to_owned).await
    }

    /// Says a wait line, fetches a GIF or video clip and plays it, then tells the outcome.
    pub async fn draw_clip<C: Chat>(&self, chat: &C, fetch: impl Future<Output = Result<Vec<u8>>>) -> Result<()> {
        let pending = self.start(chat).await?;
        let message = match fetch.await {
            Ok(clip) => outcome(self.display.play(clip).await).map_or_else(|message| message, |()| Message::Written),
            Err(err) => {
                tracing::warn!("no clip to play: {err:#}");
                Message::Failed
            }
        };
        self.finish(chat, &pending, message, str::to_owned).await
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
                self.finish(chat, &pending, Message::Rotated, |line| line.replace("{deg}", &deg))
                    .await
            }
            Err(message) => self.finish(chat, &pending, message, str::to_owned).await,
        }
    }

    /// Draws an xkcd comic, then tells its number, title and punchline after the outcome.
    pub async fn comic<C: Chat>(&self, chat: &C, arg: &str) -> Result<()> {
        let Some(pick) = Pick::parse(arg) else {
            chat.send("Use /xkcd for the newest comic, /xkcd random or /xkcd <number>")
                .await?;
            return Ok(());
        };
        let pending = self.start(chat).await?;
        let refusal = match self.xkcd.fetch(pick).await {
            Ok(Fetched::Drawable { info, picture }) => {
                return match outcome(self.display.show(Scene::Picture(picture)).await) {
                    Ok(()) => {
                        let caption = info.caption();
                        self.finish(chat, &pending, Message::Written, |variant| {
                            format!("{variant}\n\n{caption}")
                        })
                        .await
                    }
                    Err(message) => self.finish(chat, &pending, message, str::to_owned).await,
                };
            }
            Ok(Fetched::Undrawable(info)) => format!(
                "xkcd {} ({}) has no picture I can draw: https://xkcd.com/{}/",
                info.num, info.title, info.num
            ),
            Ok(Fetched::Missing(num)) => format!("There is no xkcd {num}"),
            Err(err) => {
                tracing::warn!("fetching xkcd: {err:#}");
                "xkcd can't be reached right now".to_owned()
            }
        };
        chat.follow_up(&pending, &refusal).await?;
        Ok(())
    }

    /// Names the chat's mode and the others, or switches the chat to one.
    pub async fn mode<C: Chat>(&self, chat: &C, arg: &str) -> Result<()> {
        let key = chat.key();
        let name = arg.trim().to_lowercase();
        let (text, switched) = {
            let mut replies = self.replies.lock().unwrap();
            let modes = replies.modes();
            let list = format!("Modes: {}", modes.join(", "));
            match name.as_str() {
                "" => {
                    let now = replies.mode_of(&key);
                    (
                        format!("This chat is in {now} mode. {list}. /mode <name> switches."),
                        None,
                    )
                }
                mode if modes.iter().any(|m| m == mode) => {
                    replies.set_mode(&key, mode);
                    let variant = replies.pick(&key, Message::Switched);
                    (variant.replace("{mode}", mode), Some(variant))
                }
                mode => (format!("There is no {mode} mode. {list}."), None),
            }
        };
        let id = chat.send(&text).await?;
        if let Some(variant) = switched {
            self.replies
                .lock()
                .unwrap()
                .sent(&key, &id, Message::Switched, &variant);
        }
        Ok(())
    }

    async fn start<C: Chat>(&self, chat: &C) -> Result<String> {
        let variant = self.replies.lock().unwrap().pick(&chat.key(), Message::Wait);
        let id = chat.send(&variant).await?;
        self.replies
            .lock()
            .unwrap()
            .sent(&chat.key(), &id, Message::Wait, &variant);
        Ok(id)
    }

    async fn finish<C: Chat>(
        &self,
        chat: &C,
        waiting: &str,
        message: Message,
        fill: impl Fn(&str) -> String,
    ) -> Result<()> {
        let variant = self.replies.lock().unwrap().pick(&chat.key(), message);
        let id = chat.follow_up(waiting, &fill(&variant)).await?;
        self.replies.lock().unwrap().sent(&chat.key(), &id, message, &variant);
        Ok(())
    }
}

fn outcome<T>(shown: Result<Shown<T>>) -> Result<T, Message> {
    match shown {
        Ok(Shown::Done(done)) => Ok(done),
        Ok(Shown::Busy) => Err(Message::Busy),
        Err(err) => {
            tracing::error!("display failed: {err:#}");
            Err(Message::Failed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        animation::Reel,
        panel::Refresh,
        render::{Canvas, Planes},
        xkcd::tests::{encoded, entry, serve},
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };

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

    /// A panel that counts its drawings.
    struct Counted(Arc<AtomicUsize>);

    impl Panel for Counted {
        fn show(&mut self, _: &Canvas, _: &Planes, _: Refresh) -> Result<()> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// A panel that counts its clips and its drawings, and plays each clip once `release` lets it.
    struct Player {
        clips: Arc<AtomicUsize>,
        drawn: Arc<AtomicUsize>,
        release: mpsc::Receiver<()>,
    }

    impl Panel for Player {
        fn show(&mut self, _: &Canvas, _: &Planes, _: Refresh) -> Result<()> {
            self.drawn.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn play(&mut self, _: &Reel) -> Result<()> {
            self.release.recv().ok();
            self.clips.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn player() -> (Player, Arc<AtomicUsize>, Arc<AtomicUsize>, mpsc::Sender<()>) {
        let (clips, drawn) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (release, wait) = mpsc::channel();
        let panel = Player {
            clips: clips.clone(),
            drawn: drawn.clone(),
            release: wait,
        };
        (panel, clips, drawn, release)
    }

    fn two_pictures() -> Vec<u8> {
        let mut gif = Vec::new();
        let frames = [[0, 0, 0, 255], [255; 4]].map(|color| {
            image::Frame::from_parts(
                image::RgbaImage::from_pixel(8, 8, image::Rgba(color)),
                0,
                0,
                image::Delay::from_numer_denom_ms(100, 1),
            )
        });
        image::codecs::gif::GifEncoder::new(&mut gif)
            .encode_frames(frames)
            .unwrap();
        gif
    }

    #[tokio::test]
    async fn a_clip_plays_then_its_first_picture_is_drawn() {
        let (panel, clips, drawn, release) = player();
        drop(release);
        let bot = bot(Box::new(panel));
        let chat = Fake::default();
        bot.draw_clip(&chat, async { Ok(two_pictures()) }).await.unwrap();
        assert_eq!((clips.load(Ordering::SeqCst), drawn.load(Ordering::SeqCst)), (1, 1));
        bot.draw_clip(&chat, async { Ok(b"GIF89a, but no more".to_vec()) })
            .await
            .unwrap();
        bot.draw_clip(&chat, async { anyhow::bail!("no network") })
            .await
            .unwrap();
        let messages = chat.messages();
        assert_eq!(messages[0], "Written it is");
        assert_ne!(messages[1], messages[0]);
        assert_eq!(messages[1], messages[2]);
        assert_eq!((clips.load(Ordering::SeqCst), drawn.load(Ordering::SeqCst)), (1, 1));
    }

    #[tokio::test]
    async fn nothing_else_draws_while_a_clip_plays() {
        let (panel, clips, drawn, release) = player();
        let display = Display::new(Box::new(panel), DisplayRotation::Rotate90, None);
        let playing = tokio::spawn({
            let display = display.clone();
            async move { display.play(two_pictures()).await.unwrap() }
        });
        while display.screen.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
        assert_eq!(display.show(Scene::Ring).await.unwrap(), Shown::Busy);
        assert_eq!(display.play(two_pictures()).await.unwrap(), Shown::Busy);
        drop(release);
        assert_eq!(playing.await.unwrap(), Shown::Done(()));
        assert_eq!((clips.load(Ordering::SeqCst), drawn.load(Ordering::SeqCst)), (1, 1));
    }

    fn bot(panel: Box<dyn Panel>) -> Bot {
        Bot {
            display: Display::new(panel, DisplayRotation::Rotate90, None),
            replies: Arc::new(SyncMutex::new(Replies::open(None).unwrap())),
            xkcd: Xkcd::default(),
        }
    }

    /// A bot whose comics come from a fake site, and the count of its drawings.
    fn comic_bot(site: &str) -> (Bot, Arc<AtomicUsize>) {
        let drawn = Arc::new(AtomicUsize::new(0));
        let bot = Bot {
            xkcd: Xkcd::at(site),
            ..bot(Box::new(Counted(drawn.clone())))
        };
        (bot, drawn)
    }

    fn comic_site() -> String {
        let picture = image::RgbaImage::from_pixel(30, 20, image::Rgba([0, 0, 0, 255]));
        let png = encoded(&picture, image::ImageFormat::Png);
        serve(|site| {
            vec![
                ("/info.0.json".to_owned(), entry(site, 7, "seven.png")),
                ("/7/info.0.json".to_owned(), entry(site, 7, "seven.png")),
                ("/6/info.0.json".to_owned(), entry(site, 6, "")),
                ("/comics/seven.png".to_owned(), png),
            ]
        })
    }

    #[tokio::test]
    async fn a_comic_is_drawn_and_its_punchline_told() {
        let (bot, drawn) = comic_bot(&comic_site());
        let chat = Fake::default();
        bot.message(&chat, "/xkcd").await.unwrap();
        bot.message(&chat, "/XKCD 7").await.unwrap();
        assert_eq!(chat.messages(), ["Written it is\n\nxkcd 7: Comic 7\nPunchline 7"; 2]);
        assert_eq!(drawn.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn comics_that_cant_be_drawn_leave_the_panel_alone() {
        let (bot, drawn) = comic_bot(&comic_site());
        let chat = Fake::default();
        bot.message(&chat, "/xkcd 6").await.unwrap();
        bot.message(&chat, "/xkcd 404").await.unwrap();
        bot.message(&chat, "/xkcd seven").await.unwrap();
        let messages = chat.messages();
        assert!(messages[0].contains("xkcd 6 (Comic 6) has no picture"), "{messages:?}");
        assert_eq!(messages[1], "There is no xkcd 404");
        assert!(messages[2].starts_with("Use /xkcd"), "{messages:?}");
        assert_eq!(drawn.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn an_unreachable_xkcd_gets_a_short_reply() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let site = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let (bot, drawn) = comic_bot(&site);
        let chat = Fake::default();
        bot.message(&chat, "/xkcd random").await.unwrap();
        assert_eq!(chat.messages(), ["xkcd can't be reached right now"]);
        assert_eq!(drawn.load(Ordering::SeqCst), 0);
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
    async fn a_picture_says_wait_then_written_or_failed() {
        let bot = bot(Box::new(Instant));
        let chat = Fake::default();
        let mut png = Vec::new();
        image::RgbaImage::new(4, 4)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        bot.draw_picture(&chat, async { Ok(png) }).await.unwrap();
        bot.draw_picture(&chat, async { Ok(b"no picture".to_vec()) })
            .await
            .unwrap();
        bot.draw_picture(&chat, async { anyhow::bail!("no network") })
            .await
            .unwrap();
        let messages = chat.messages();
        assert_eq!(messages.len(), 3, "{messages:?}");
        assert_eq!(messages[0], "Written it is");
        assert_eq!(messages[1], messages[2]);
        assert_ne!(messages[1], messages[0]);
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
        while display.screen.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
        assert_eq!(display.show(Scene::Ring).await.unwrap(), Shown::Busy);
        drop(release);
        assert_eq!(first.await.unwrap(), Shown::Done(()));
        assert_eq!(display.show(Scene::Clear).await.unwrap(), Shown::Done(()));
    }

    fn black() -> Arc<RgbaImage> {
        Arc::new(RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 0, 255])))
    }

    async fn shown(display: &Display) -> Option<Scene> {
        display.screen.lock().await.shown.clone()
    }

    #[tokio::test]
    async fn a_text_right_after_a_sticker_waits_and_joins_it() {
        let (release, wait) = mpsc::channel();
        let display = Display::new(Box::new(Slow(wait)), DisplayRotation::Rotate90, None);
        let sticker = tokio::spawn({
            let display = display.clone();
            async move { display.show_part("wa:anna", Part::Sticker(black())).await.unwrap() }
        });
        while display.screen.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
        assert_eq!(display.show(Scene::Ring).await.unwrap(), Shown::Busy);
        let text = tokio::spawn({
            let display = display.clone();
            async move { display.show_part("wa:anna", Part::Text("Hoi".into())).await.unwrap() }
        });
        drop(release);
        assert_eq!(sticker.await.unwrap(), Shown::Done(()));
        assert_eq!(text.await.unwrap(), Shown::Done(()));
        assert_eq!(
            shown(&display).await,
            Some(Scene::Sticker {
                picture: black(),
                text: "Hoi".into()
            })
        );
    }

    #[tokio::test]
    async fn a_sticker_after_a_text_joins_it() {
        let display = Display::new(Box::new(Instant), DisplayRotation::Rotate90, None);
        display.show_part("tg:1", Part::Text("Hoi".into())).await.unwrap();
        display.show_part("tg:1", Part::Sticker(black())).await.unwrap();
        assert!(matches!(shown(&display).await, Some(Scene::Sticker { .. })));
        display.show_part("tg:1", Part::Text("Ciao".into())).await.unwrap();
        assert_eq!(shown(&display).await, Some(Scene::Text { text: "Ciao".into() }));
    }

    #[tokio::test]
    async fn only_a_sticker_and_a_text_by_the_same_author_pair() {
        let display = Display::new(Box::new(Instant), DisplayRotation::Rotate90, None);
        display.show_part("wa:anna", Part::Sticker(black())).await.unwrap();
        display.show_part("wa:beat", Part::Text("Hoi".into())).await.unwrap();
        assert_eq!(shown(&display).await, Some(Scene::Text { text: "Hoi".into() }));
        display.show_part("wa:beat", Part::Text("Ciao".into())).await.unwrap();
        assert_eq!(shown(&display).await, Some(Scene::Text { text: "Ciao".into() }));
        display.show_part("wa:beat", Part::Sticker(black())).await.unwrap();
        assert!(matches!(shown(&display).await, Some(Scene::Sticker { .. })));
    }

    #[tokio::test]
    async fn a_sticker_counts_for_pairing_before_it_is_fetched() {
        let bot = bot(Box::new(Instant));
        let chat = Fake::default();
        assert!(!bot.display.pairs("fake:1", true));
        bot.draw_picture(&chat, async { anyhow::bail!("no network") })
            .await
            .unwrap();
        assert!(bot.display.pairs("fake:1", true));
        assert!(!bot.display.pairs("fake:1", false));
        assert!(!bot.display.pairs("fake:2", true));
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
        let rotation = restarted.screen.lock().await.rotation;
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

    #[tokio::test]
    async fn mode_switches_the_replies_of_the_chat() {
        let dir = std::env::temp_dir().join(format!("epaper-flow-mode-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("modes")).unwrap();
        std::fs::write(
            dir.join("modes/swiss.txt"),
            "[written]\nWritten it is\n[switched]\nBack in {mode}\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("modes/office.txt"),
            "[written]\nThat's what she said\n[switched]\nWelcome to {mode}\n",
        )
        .unwrap();
        let bot = Bot {
            display: Display::new(Box::new(Instant), DisplayRotation::Rotate90, None),
            replies: Arc::new(SyncMutex::new(Replies::open(Some(&dir)).unwrap())),
            xkcd: Xkcd::default(),
        };
        let chat = Fake::default();
        bot.message(&chat, "/mode").await.unwrap();
        bot.message(&chat, "/mode Office").await.unwrap();
        bot.message(&chat, "/clear").await.unwrap();
        bot.message(&chat, "/mode dune").await.unwrap();
        bot.message(&chat, "/mode swiss").await.unwrap();
        bot.message(&chat, "/clear").await.unwrap();
        let messages = chat.messages();
        assert!(
            messages[0].contains("in swiss mode") && messages[0].contains("office, swiss"),
            "{messages:?}"
        );
        assert_eq!(messages[1..3], ["Welcome to office", "That's what she said"]);
        assert!(messages[3].starts_with("There is no dune mode"), "{messages:?}");
        assert_eq!(messages[4..], ["Back in swiss", "Written it is"]);
        std::fs::remove_dir_all(dir).ok();
    }
}
