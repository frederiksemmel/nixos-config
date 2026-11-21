mod flow;
mod panel;
mod render;
mod replies;
mod telegram;
mod typeset;
mod whatsapp;

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use epd_waveshare::graphics::DisplayRotation;

use crate::{
    panel::{Hat, Panel, Preview},
    render::{Scene, canvas_size, render},
    replies::Replies,
};

/// Draw on the Waveshare 2.7" e-Paper HAT (B), by hand or through a Telegram and WhatsApp bot.
#[derive(Parser, Debug)]
#[command(version)]
struct Cli {
    /// Write a PPM image of the frame to this file instead of driving the HAT.
    #[arg(long, env = "EPAPER_PREVIEW")]
    preview: Option<PathBuf>,

    #[arg(long, env = "EPAPER_ROTATION", default_value = "180")]
    rotation: Rotation,

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
    let mut panel: Box<dyn Panel> = match cli.preview {
        Some(path) => Box::new(Preview(path)),
        None => Box::new(Hat::open().context("opening the e-Paper HAT")?),
    };
    let scene = match cli.command {
        Command::Ring => Scene::Ring,
        Command::Clear => Scene::Clear,
        Command::Text { words } => Scene::Text { text: words.join(" ") },
        Command::Bot {
            token_file,
            allow,
            state_dir,
            whatsapp,
        } => {
            let token = std::fs::read_to_string(&token_file).with_context(|| token_file.display().to_string())?;
            let bot = flow::Bot {
                display: flow::Display::new(panel, rotation),
                replies: Arc::new(Mutex::new(Replies::open(state_dir.as_deref())?)),
            };
            return tokio::runtime::Runtime::new()?.block_on(serve(token, bot, allow, state_dir.filter(|_| whatsapp)));
        }
    };
    let canvas = render(&scene, canvas_size(rotation))?;
    panel.show(&canvas, &canvas.planes(rotation))
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
