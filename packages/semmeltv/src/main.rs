mod app;
mod artwork;
mod cec;
mod command;
mod library;
mod loader;
mod nav;
mod player;
mod tmdb;

use artwork::Cache;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

slint::include_modules!();

const USAGE: &str = "usage: semmeltv [ui|list|artwork] [--library DIR] [--cache DIR] [--tmdb-token-file FILE]";

struct Args {
    mode: String,
    library: PathBuf,
    cache: PathBuf,
    token_file: Option<PathBuf>,
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
        token_file: env_path("TMDB_TOKEN_FILE"),
    };
    let mut raw = env::args().skip(1);
    while let Some(argument) = raw.next() {
        let mut value = |name: &str| raw.next().ok_or(format!("{name} needs a value"));
        match argument.as_str() {
            "ui" | "list" | "artwork" => args.mode = argument,
            "--library" => args.library = value("--library")?.into(),
            "--cache" => args.cache = value("--cache")?.into(),
            "--tmdb-token-file" => args.token_file = Some(value("--tmdb-token-file")?.into()),
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

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let library = library::scan(&args.library);
    let cache = Cache::new(args.cache.clone());
    match args.mode.as_str() {
        "list" => list(&library),
        "artwork" => {
            let client = tmdb_client(&args)?.ok_or("artwork needs a TMDB token: set TMDB_TOKEN_FILE")?;
            artwork::generate(&library, &args.library, &cache, client).map_err(|error| error.to_string())?;
        }
        _ => app::run(library, cache).map_err(|error| error.to_string())?,
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
