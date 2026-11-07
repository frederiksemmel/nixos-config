mod app;
mod artwork;
mod cec;
mod command;
mod display;
mod fetch;
mod import;
mod intent;
mod library;
mod loader;
mod manifest;
mod nav;
mod offline;
mod parse;
mod player;
mod progress;
mod segments;
mod tmdb;
mod watch;

use artwork::Cache;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

slint::include_modules!();

const USAGE: &str = "usage: semmeltv [ui|list|artwork|import|understand TEXT] [--library DIR] [--cache DIR]
                [--vfs-cache DIR] [--tmdb-token-file FILE] [--manifest FILE] [--listing FILE]";

struct Args {
    mode: String,
    library: PathBuf,
    cache: PathBuf,
    vfs_cache: Option<PathBuf>,
    token_file: Option<PathBuf>,
    manifest: Option<PathBuf>,
    listing: Option<PathBuf>,
    transcript: String,
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name).filter(|value| !value.is_empty()).map(PathBuf::from)
}

fn parse_args() -> Result<Args, String> {
    let home = env_path("HOME").unwrap_or_default();
    let mut args = Args {
        mode: "ui".to_string(),
        library: env_path("SEMMELTV_LIBRARY").unwrap_or_else(|| home.join("pcloud")),
        cache: env_path("SEMMELTV_CACHE")
            .or_else(|| env_path("XDG_CACHE_HOME").map(|dir| dir.join("semmeltv")))
            .unwrap_or_else(|| home.join(".cache/semmeltv")),
        vfs_cache: env_path("SEMMELTV_VFS_CACHE"),
        token_file: env_path("TMDB_TOKEN_FILE"),
        manifest: None,
        listing: None,
        transcript: String::new(),
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
            "--library" => args.library = value("--library")?.into(),
            "--cache" => args.cache = value("--cache")?.into(),
            "--vfs-cache" => args.vfs_cache = Some(value("--vfs-cache")?.into()),
            "--tmdb-token-file" => args.token_file = Some(value("--tmdb-token-file")?.into()),
            "--manifest" => args.manifest = Some(value("--manifest")?.into()),
            "--listing" => args.listing = Some(value("--listing")?.into()),
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

fn run_ui(args: Args, cache: Cache) -> Result<(), String> {
    let capacity = env::var("SEMMELTV_VFS_CACHE_BYTES")
        .ok()
        .and_then(|bytes| bytes.parse().ok());
    let setup = app::Setup {
        store: offline::Store::new(args.library.clone(), args.vfs_cache, capacity),
        root: args.library,
        remote: env::var("SEMMELTV_REMOTE").ok().filter(|remote| !remote.is_empty()),
        cache,
    };
    app::run(setup).map_err(|error| error.to_string())
}

fn run_import(args: &Args, cache: &Cache) -> Result<(), String> {
    let manifest = args.manifest.as_ref().ok_or("import needs --manifest FILE")?;
    import::run(import::Sources {
        library: &args.library,
        manifest,
        listing: args.listing.as_deref(),
        matches: cache.load_matches(),
        client: tmdb_client(args)?,
    })
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let cache = Cache::new(args.cache.clone());
    match args.mode.as_str() {
        "list" => list(&library::load(&args.library)?),
        "understand" => println!(
            "{:?}",
            intent::understand(&args.transcript, &library::load(&args.library)?, &cache.load_progress())
        ),
        "artwork" => {
            let client = tmdb_client(&args)?.ok_or("artwork needs a TMDB token: set TMDB_TOKEN_FILE")?;
            let library = library::load(&args.library)?;
            artwork::generate(&library, &args.library, &cache, client).map_err(|error| error.to_string())?;
        }
        "import" => run_import(&args, &cache)?,
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
