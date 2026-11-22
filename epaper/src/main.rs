mod animation;
mod flow;
mod markdown;
mod panel;
mod picture;
mod render;
mod replies;
mod telegram;
mod typeset;
mod whatsapp;
mod xkcd;

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use epd_waveshare::graphics::DisplayRotation;

use crate::{
    panel::{Hat, Panel, Preview, Refresh},
    render::{Scene, canvas_size, render},
    replies::Replies,
};

const ROTATION_FILE: &str = "rotation";

/// Draw on the Waveshare 2.7" e-Paper HAT (B), by hand or through a Telegram and WhatsApp bot.
#[derive(Parser, Debug)]
#[command(version)]
struct Cli {
    /// Write a PPM image of the frame to this file instead of driving the HAT.
    #[arg(long, env = "EPAPER_PREVIEW")]
    preview: Option<PathBuf>,

    #[arg(long, env = "EPAPER_ROTATION", default_value = "180")]
    rotation: Rotation,

    /// How to change the picture; by default grey, and clean for clearing.
    #[arg(long, value_enum)]
    refresh: Option<Refresh>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Show the hand.
    Ring,
    /// Write text; words are wrapped and the font shrinks to fit.
    Text {
        #[arg(required = true)]
        words: Vec<String>,
    },
    /// Wipe the display to white.
    Clear,
    /// Draw a PNG, JPEG, GIF or WebP picture, fitted and in four shades.
    Picture { path: PathBuf },
    /// Draw an xkcd comic: the newest, `random`, or the one with this number.
    Xkcd {
        #[arg(default_value = "")]
        comic: String,
    },
    /// Play a GIF or short video in black and white, as the bot does, refreshing only the window that changed.
    Gif {
        file: PathBuf,
        /// Show this many pictures in order, looping, none skipped; for measuring the panel.
        #[arg(long)]
        frames: Option<usize>,
        /// Panel frames, at 81.8 per second, the fast refresh drives a changing pixel for.
        #[arg(long, default_value_t = panel::CLIP_LUT_FRAMES)]
        lut_frames: u8,
        /// Send and refresh the whole panel for every picture.
        #[arg(long)]
        full: bool,
    },
    /// Run the Telegram bot, and the WhatsApp one with --whatsapp.
    Bot {
        #[arg(long, env = "EPAPER_TOKEN_FILE")]
        token_file: PathBuf,
        /// Telegram user ids allowed to draw; everyone when none are given.
        #[arg(long = "allow", env = "EPAPER_ALLOW", value_delimiter = ',')]
        allow: Vec<u64>,
        /// Where the bot keeps its reply log; systemd's StateDirectory.
        #[arg(long, env = "STATE_DIRECTORY")]
        state_dir: Option<PathBuf>,
        /// Also answer on WhatsApp, as a linked device whose session lives in the state dir.
        #[arg(long, env = "EPAPER_WHATSAPP", requires = "state_dir")]
        whatsapp: bool,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Rotation {
    #[value(name = "0")]
    R0,
    #[value(name = "90")]
    R90,
    #[value(name = "180")]
    R180,
    #[value(name = "270")]
    R270,
}

impl From<Rotation> for DisplayRotation {
    fn from(rotation: Rotation) -> Self {
        match rotation {
            Rotation::R0 => DisplayRotation::Rotate0,
            Rotation::R90 => DisplayRotation::Rotate90,
            Rotation::R180 => DisplayRotation::Rotate180,
            Rotation::R270 => DisplayRotation::Rotate270,
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn,epaper=info".into()),
        )
        .init();
    let cli = Cli::parse();
    let rotation = cli.rotation.into();
    if let Command::Gif {
        file,
        frames,
        lut_frames,
        full,
    } = &cli.command
    {
        let clip = std::fs::read(file).with_context(|| file.display().to_string())?;
        let reel = animation::decode(&clip, rotation, animation::LIMIT)?;
        let mut hat = Hat::open().context("opening the e-Paper HAT")?;
        let mut playing = hat.animate(&reel.pictures[0].1, *lut_frames, !full)?;
        let played = match frames {
            Some(count) => animation::every(&mut playing, &reel, *count),
            None => animation::in_time(&mut playing, &reel),
        };
        playing.finish()?;
        return played;
    }
    let mut panel: Box<dyn Panel> = match cli.preview {
        Some(path) => Box::new(Preview(path)),
        None => Box::new(Hat::open().context("opening the e-Paper HAT")?),
    };
    let scene = match cli.command {
        Command::Ring => Scene::Ring,
        Command::Clear => Scene::Clear,
        Command::Picture { path } => {
            let bytes = std::fs::read(&path).with_context(|| path.display().to_string())?;
            Scene::Picture(Arc::new(picture::decode(&bytes)?))
        }
        Command::Text { words } => Scene::Text { text: words.join(" ") },
        Command::Xkcd { comic } => {
            let pick = xkcd::Pick::parse(&comic).context("pick nothing for the newest comic, random or a number")?;
            match tokio::runtime::Runtime::new()?.block_on(xkcd::Xkcd::default().fetch(pick))? {
                xkcd::Fetched::Drawable { info, picture } => {
                    println!("{}", info.caption());
                    Scene::Picture(picture)
                }
                xkcd::Fetched::Undrawable(info) => bail!("xkcd {} has no picture that can be drawn", info.num),
                xkcd::Fetched::Missing(num) => bail!("there is no xkcd {num}"),
            }
        }
        Command::Gif { .. } => unreachable!("played above"),
        Command::Bot {
            token_file,
            allow,
            state_dir,
            whatsapp,
        } => {
            let token = std::fs::read_to_string(&token_file).with_context(|| token_file.display().to_string())?;
            let bot = flow::Bot {
                display: flow::Display::new(panel, rotation, state_dir.as_ref().map(|dir| dir.join(ROTATION_FILE))),
                replies: Arc::new(Mutex::new(Replies::open(state_dir.as_deref())?)),
                xkcd: xkcd::Xkcd::default(),
            };
            return tokio::runtime::Runtime::new()?.block_on(serve(token, bot, allow, state_dir.filter(|_| whatsapp)));
        }
    };
    let canvas = render(&scene, canvas_size(rotation))?;
    let refresh = cli.refresh.unwrap_or(scene.refresh());
    panel.show(&canvas, &canvas.planes(rotation), refresh)
}

async fn serve(token: String, bot: flow::Bot, allow: Vec<u64>, whatsapp_dir: Option<PathBuf>) -> Result<()> {
    let telegram = telegram::run(token.trim().to_owned(), bot.clone(), allow);
    let Some(dir) = whatsapp_dir else {
        return telegram.await;
    };
    tokio::select! {
        result = telegram => result,
        () = whatsapp::run(dir, bot) => unreachable!("the WhatsApp client restarts forever"),
    }
}
