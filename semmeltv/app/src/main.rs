mod api;
mod app;
mod artwork;
mod bus;
mod cache;
mod cec;
mod command;
mod details;
mod display;
mod fetch;
mod import;
mod intent;
mod led;
mod library;
mod loader;
mod manifest;
mod mdblist;
mod nav;
mod offline;
mod overlay;
mod parse;
mod player;
mod progress;
mod seek;
mod segments;
mod tags;
mod tmdb;
mod voice;
mod watch;
mod watching;
mod wyoming;

use cache::Cache;
use std::env;
use std::fs;
use std::path::{self, Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

slint::include_modules!();

const USAGE: &str =
    "usage: semmeltv [ui|list|artwork|import|understand TEXT|hear WAV] [--library DIR] [--meta DIR] [--cache DIR]
                [--vfs-cache DIR] [--tmdb-token-file FILE] [--mdblist-key-file FILE]
                [--manifest FILE] [--listing FILE]
                [--refresh] [--speech HOST:PORT] [--wake HOST:PORT] [--wake-word NAME]
                [--api HOST:PORT]";
const DEFAULT_WAKE_WORD: &str = "hey_jarvis";

struct Args {
    mode: String,
    library: PathBuf,
    meta: Option<PathBuf>,
    cache: PathBuf,
    vfs_cache: Option<PathBuf>,
    token_file: Option<PathBuf>,
    mdblist_key_file: Option<PathBuf>,
    manifest: Option<PathBuf>,
    listing: Option<PathBuf>,
    refresh: bool,
    transcript: String,
    recording: Option<PathBuf>,
    speech: Option<String>,
    wake: Option<String>,
    wake_word: String,
    api: Option<String>,
}

impl Args {
    fn meta(&self) -> &Path {
        self.meta.as_deref().unwrap_or(&self.library)
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name).filter(|value| !value.is_empty()).map(PathBuf::from)
}

fn parse_args() -> Result<Args, String> {
    let home = env_path("HOME").unwrap_or_default();
    let mut args = Args {
        mode: "ui".to_string(),
        library: env_path("SEMMELTV_LIBRARY").unwrap_or_else(|| home.join("pcloud/SemmelTV")),
        meta: env_path("SEMMELTV_META"),
        cache: env_path("SEMMELTV_CACHE")
            .or_else(|| env_path("XDG_CACHE_HOME").map(|dir| dir.join("semmeltv")))
            .unwrap_or_else(|| home.join(".cache/semmeltv")),
        vfs_cache: env_path("SEMMELTV_VFS_CACHE"),
        token_file: env_path("TMDB_TOKEN_FILE"),
        mdblist_key_file: env_path("MDBLIST_API_KEY_FILE"),
        manifest: None,
        listing: None,
        refresh: false,
        transcript: String::new(),
        recording: None,
        speech: env::var("SEMMELTV_SPEECH").ok().filter(|address| !address.is_empty()),
        wake: env::var("SEMMELTV_WAKE").ok().filter(|address| !address.is_empty()),
        wake_word: env::var("SEMMELTV_WAKE_WORD")
            .ok()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| DEFAULT_WAKE_WORD.to_string()),
        api: env::var("SEMMELTV_API").ok().filter(|address| !address.is_empty()),
    };
    let mut raw = env::args().skip(1);
    while let Some(argument) = raw.next() {
        let mut value = |name: &str| raw.next().ok_or(format!("{name} needs a value"));
        match argument.as_str() {
            "ui" | "list" | "artwork" | "import" => args.mode = argument,
            "understand" => {
                args.transcript = value("understand")?;
                args.mode = argument;
            }
            "hear" => {
                args.recording = Some(value("hear")?.into());
                args.mode = argument;
            }
            "--speech" => args.speech = Some(value("--speech")?),
            "--wake" => args.wake = Some(value("--wake")?),
            "--wake-word" => args.wake_word = value("--wake-word")?,
            "--api" => args.api = Some(value("--api")?),
            "--library" => args.library = value("--library")?.into(),
            "--meta" => args.meta = Some(value("--meta")?.into()),
            "--cache" => args.cache = value("--cache")?.into(),
            "--vfs-cache" => args.vfs_cache = Some(value("--vfs-cache")?.into()),
            "--tmdb-token-file" => args.token_file = Some(value("--tmdb-token-file")?.into()),
            "--mdblist-key-file" => args.mdblist_key_file = Some(value("--mdblist-key-file")?.into()),
            "--manifest" => args.manifest = Some(value("--manifest")?.into()),
            "--listing" => args.listing = Some(value("--listing")?.into()),
            "--refresh" => args.refresh = true,
            other => return Err(format!("unknown argument {other}\n{USAGE}")),
        }
    }
    Ok(args)
}

fn tmdb_client(args: &Args) -> Result<Option<tmdb::Client>, String> {
    let token = match &args.token_file {
        Some(file) => Some(fs::read_to_string(file).map_err(|error| format!("{}: {error}", file.display()))?),
        None => env::var("TMDB_TOKEN").ok(),
    };
    Ok(token.map(|token| tmdb::Client::new(token.trim().to_string())))
}

fn mdblist_client(args: &Args) -> Result<Option<mdblist::Client>, String> {
    let key = match &args.mdblist_key_file {
        Some(file) => Some(fs::read_to_string(file).map_err(|error| format!("{}: {error}", file.display()))?),
        None => env::var("MDBLIST_API_KEY").ok(),
    };
    Ok(key
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
        .map(mdblist::Client::new))
}

fn list(library: &library::Library) {
    for show in &library.shows {
        println!("{} [{}] {:?}", show.title, show.subtitle(), show.year);
        for season in &show.seasons {
            let labels: Vec<_> = season.episodes.iter().map(library::Episode::label).collect();
            println!("  {}: {}", season.label(), labels.join(" | "));
        }
    }
    for movie in &library.movies {
        println!("{} {:?}", movie.title, movie.year);
    }
}

fn wake_word(args: &Args) -> Option<voice::WakeWord> {
    args.wake.clone().map(|address| voice::WakeWord {
        address,
        name: args.wake_word.clone(),
    })
}

fn run_ui(args: Args, cache: Cache) -> Result<(), String> {
    let capacity = env::var("SEMMELTV_VFS_CACHE_BYTES")
        .ok()
        .and_then(|bytes| bytes.parse().ok());
    let wake_word = wake_word(&args);
    let meta = args.meta().to_path_buf();
    let setup = app::Setup {
        store: offline::Store::new(args.library.clone(), args.vfs_cache, capacity),
        meta,
        root: args.library,
        remote: env::var("SEMMELTV_REMOTE").ok().filter(|remote| !remote.is_empty()),
        cache,
        wake_word,
        speech: args.speech,
        microphone: env::var("SEMMELTV_MICROPHONE").ok().filter(|device| !device.is_empty()),
        led: env_path("SEMMELTV_LED").map(|dir| led::Led::new(&dir)),
        api: args.api.map(|address| api::Config {
            address,
            phone_microphone: env::var("SEMMELTV_PHONE_MICROPHONE")
                .ok()
                .filter(|device| !device.is_empty()),
        }),
    };
    app::run(setup).map_err(|error| error.to_string())
}

fn run_import(args: &Args) -> Result<(), String> {
    let manifest = args.manifest.as_ref().ok_or("import needs --manifest FILE")?;
    import::run(import::Sources {
        library: &args.library,
        manifest,
        listing: args.listing.as_deref(),
        client: tmdb_client(args)?,
    })
}

fn inside(path: &Path, dir: &Path) -> bool {
    let absolute = |path: &Path| path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    absolute(path).starts_with(absolute(dir))
}

fn run_artwork(args: &Args) -> Result<(), String> {
    let manifest = args.manifest.as_ref().ok_or("artwork needs --manifest FILE")?;
    if inside(manifest, &args.library) {
        return Err(format!(
            "{} is inside the library; run artwork on a local copy",
            manifest.display()
        ));
    }
    let client = tmdb_client(args)?.ok_or("artwork needs a TMDB token: set TMDB_TOKEN_FILE")?;
    let mdblist = mdblist_client(args)?;
    if mdblist.is_none() {
        println!("no MDBList key: ratings skipped; set MDBLIST_API_KEY_FILE");
    }
    let ratings = mdblist.as_ref().map(|client| client as &dyn artwork::Ratings);
    let summary = artwork::run(manifest, &client, ratings, args.refresh)?;
    for problem in &summary.problems {
        println!("problem: {problem}");
    }
    for title in &summary.without_id {
        println!("no tmdb id: {title}");
    }
    for title in &summary.unrated {
        println!("no ratings: {title}");
    }
    println!(
        "{} images saved, {} titles without a TMDB id, {} problems",
        summary.images,
        summary.without_id.len(),
        summary.problems.len()
    );
    if mdblist.is_some() {
        println!(
            "{} titles rated, {} without ratings, {} left for the next run, {} MDBList requests",
            summary.rated,
            summary.unrated.len(),
            summary.ratings_waiting,
            summary.requests
        );
    }
    Ok(())
}

fn hear(args: &Args, cache: &Cache) -> Result<(), String> {
    let path = args.recording.as_ref().ok_or("hear needs a WAV file")?;
    let speech = args
        .speech
        .as_deref()
        .ok_or("hear needs --speech HOST:PORT or SEMMELTV_SPEECH")?;
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let samples = voice::read_wav(&bytes)?;
    let length = Duration::from_secs_f64(samples.len() as f64 / f64::from(wyoming::RATE));
    let wake_word = wake_word(args);
    match &wake_word {
        Some(wake_word) => println!("recording {:.2}s, waiting for {}", length.as_secs_f64(), wake_word.name),
        None => println!("recording {:.2}s", length.as_secs_f64()),
    }
    let listen = wake_word.is_none();
    let (heard, received) = mpsc::channel();
    let voice = voice::spawn(
        speech.to_string(),
        wake_word,
        voice::Source::Recording(samples),
        move |event| {
            let _ = heard.send(event);
        },
    );
    if listen {
        voice.listen();
    }
    let started = Instant::now();
    let text = loop {
        let event = received
            .recv_timeout(length + Duration::from_secs(10))
            .map_err(|_| "nothing happened: no wake word heard")?;
        println!("{:.2}s {event:?}", started.elapsed().as_secs_f64());
        match event {
            voice::Heard::Listening | voice::Heard::Transcribing => {}
            voice::Heard::Transcript(text) => break text,
            other => return Err(format!("{other:?}")),
        }
    };
    let library = library::load(&args.library, args.meta())?;
    println!("{:?}", intent::understand(&text, &library, &cache.load_progress()));
    Ok(())
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let cache = Cache::new(args.cache.clone());
    match args.mode.as_str() {
        "list" => list(&library::load(&args.library, args.meta())?),
        "understand" => println!(
            "{:?}",
            intent::understand(
                &args.transcript,
                &library::load(&args.library, args.meta())?,
                &cache.load_progress()
            )
        ),
        "artwork" => run_artwork(&args)?,
        "import" => run_import(&args)?,
        "hear" => hear(&args, &cache)?,
        _ => run_ui(args, cache)?,
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_inside_the_library_is_refused() {
        let library = Path::new("/home/me/pcloud/SemmelTV");
        assert!(inside(Path::new("/home/me/pcloud/SemmelTV/library.toml"), library));
        assert!(!inside(Path::new("/tmp/work/library.toml"), library));
        assert!(!inside(
            Path::new("/home/me/pcloud/SemmelTV-copy/library.toml"),
            library
        ));
    }
}
