mod api;
mod artwork;
mod audio;
mod auth;
mod bus;
mod cache;
mod cec;
mod client_log;
mod command;
mod core;
mod db;
mod deploy;
mod details;
mod display;
mod downloads;
mod feedback;
mod fetch;
mod heatmap;
mod hold;
mod imdb;
mod import;
mod intent;
mod led;
mod library;
mod library_db;
mod loader;
mod manifest;
mod mdblist;
mod nav;
mod offline;
mod overlay;
mod parse;
mod player;
mod profile;
mod progress;
mod push;
mod ready;
mod screen;
mod seek;
mod segments;
mod sidecar;
mod storage;
mod stream;
mod subfetch;
mod subtitles;
mod tags;
mod timing;
mod tmdb;
mod together;
mod tv;
mod ui;
mod up_next;
mod usage;
mod voice;
mod watching;
mod wyoming;

use cache::Cache;
use db::Db;
use profile::ProfileId;
use std::env;
use std::fs;
use std::future::Future;
use std::path::{self, Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

slint::include_modules!();

const USAGE: &str =
    "usage: semmeltv [ui|list|artwork|import|downloads|understand TEXT|hear WAV|invite NAME|stats [DAYS]|toasty tv|library ARGS…|library show|library from-toml FILE|get search|fetch|jobs ARGS…] [--library DIR] [--meta DIR] [--cache DIR] [--db FILE]
                [--vfs-cache DIR] [--tmdb-token-file FILE] [--mdblist-key-file FILE]
                [--manifest FILE] [--listing FILE]
                [--refresh] [--speech HOST:PORT] [--wake HOST:PORT] [--wake-word NAME]
                [--api HOST:PORT]";
const DEFAULT_WAKE_WORD: &str = "stevie_the_tv";

struct Args {
    mode: String,
    library: PathBuf,
    meta: Option<PathBuf>,
    cache: PathBuf,
    db: PathBuf,
    vfs_cache: Option<PathBuf>,
    token_file: Option<PathBuf>,
    mdblist_key_file: Option<PathBuf>,
    manifest: Option<PathBuf>,
    listing: Option<PathBuf>,
    refresh: bool,
    transcript: String,
    toasty: Vec<String>,
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
        db: env_path("SEMMELTV_DB")
            .or_else(|| env_path("XDG_STATE_HOME").map(|dir| dir.join("semmeltv/semmeltv.db")))
            .unwrap_or_else(|| home.join(".local/state/semmeltv/semmeltv.db")),
        vfs_cache: env_path("SEMMELTV_VFS_CACHE"),
        token_file: env_path("TMDB_TOKEN_FILE"),
        mdblist_key_file: env_path("MDBLIST_API_KEY_FILE"),
        manifest: None,
        listing: None,
        refresh: false,
        transcript: String::new(),
        toasty: Vec::new(),
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
            "ui" | "list" | "artwork" | "import" | "downloads" | "progress" => args.mode = argument,
            "toasty" | "library" | "get" | "tv-set" | "stats" => {
                args.toasty = raw.by_ref().collect();
                args.mode = argument;
            }
            "understand" => {
                args.transcript = value("understand")?;
                args.mode = argument;
            }
            "hear" => {
                args.recording = Some(value("hear")?.into());
                args.mode = argument;
            }
            "invite" => {
                args.transcript = value("invite")?;
                args.mode = argument;
            }
            "--speech" => args.speech = Some(value("--speech")?),
            "--wake" => args.wake = Some(value("--wake")?),
            "--wake-word" => args.wake_word = value("--wake-word")?,
            "--api" => args.api = Some(value("--api")?),
            "--library" => args.library = value("--library")?.into(),
            "--meta" => args.meta = Some(value("--meta")?.into()),
            "--cache" => args.cache = value("--cache")?.into(),
            "--db" => args.db = value("--db")?.into(),
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

async fn open_db(args: &Args) -> Result<Db, String> {
    Db::open(&args.db)
        .await
        .map_err(|error| format!("{}: {error}", args.db.display()))
}

fn waiting<T>(work: impl Future<Output = T>) -> Result<T, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    Ok(runtime.block_on(work))
}

fn stored_progress(args: &Args) -> progress::Progress {
    let listed = library::load_listed(&args.library, args.meta()).unwrap_or_default();
    let stored = waiting(async {
        let db = open_db(args).await?;
        db.set_library(&listed);
        db.progress(ProfileId::FIRST).await.map_err(|error| error.to_string())
    });
    stored.and_then(|progress| progress).unwrap_or_else(|error| {
        eprintln!("progress: {error}");
        progress::Progress::default()
    })
}

fn run_ui(args: Args, cache: Cache) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .thread_name("semmeltv-core")
        .enable_all()
        .build()
        .map_err(|error| error.to_string())?;
    let meta = args.meta().to_path_buf();
    let core = runtime.block_on(start_core(args, cache))?;
    let shown = ui::run(core, &meta, runtime.handle()).map_err(|error| error.to_string());
    runtime.shutdown_background();
    shown
}

async fn start_core(args: Args, cache: Cache) -> Result<core::Core, String> {
    let db = open_db(&args).await?;
    let capacity: Option<u64> = env::var("SEMMELTV_VFS_CACHE_BYTES")
        .ok()
        .and_then(|bytes| bytes.parse().ok());
    let storage = match (&args.vfs_cache, env_path("SEMMELTV_VFS_CACHE_META"), capacity) {
        (Some(data), Some(meta), Some(capacity)) => Some(storage::Cache {
            library: args.library.clone(),
            manifest: args.meta().to_path_buf(),
            data: data.clone(),
            meta,
            marks: storage::Marks::new(capacity),
        }),
        _ => None,
    };
    let wake_word = wake_word(&args);
    let meta = args.meta().to_path_buf();
    let kind = env::var("SEMMELTV_TV_KIND").ok().filter(|kind| !kind.is_empty());
    let feedback =
        env_path("SEMMELTV_FEEDBACK").unwrap_or_else(|| args.db.parent().unwrap_or(Path::new(".")).join("feedback"));
    let gate = if args.api.is_some() {
        sign_in_gate(&db).await?
    } else {
        None
    };
    let api = args.api.map(|address| api::Config {
        address,
        library: args.library.clone(),
        meta: meta.clone(),
        phone_microphone: env::var("SEMMELTV_PHONE_MICROPHONE")
            .ok()
            .filter(|device| !device.is_empty()),
        stream: env_path("SEMMELTV_STREAM_DIR").unwrap_or_else(|| PathBuf::from("/dev/shm/semmeltv-stream")),
        gate,
        feedback,
        grim: env_text("SEMMELTV_GRIM"),
    });
    let setup = core::Setup {
        store: offline::Store::new(args.library.clone(), args.vfs_cache, capacity),
        storage,
        meta,
        root: args.library,
        remote: env::var("SEMMELTV_REMOTE").ok().filter(|remote| !remote.is_empty()),
        cache,
        db,
        tv_kind: kind.as_deref().map(tv::Kind::parse).transpose()?,
        tv_renderer: env::var("SEMMELTV_TV_RENDERER").ok().filter(|url| !url.is_empty()),
        handoff: handoff()?,
        wake_word,
        speech: args.speech,
        microphone: env::var("SEMMELTV_MICROPHONE").ok().filter(|device| !device.is_empty()),
        led: env_path("SEMMELTV_LED").map(|dir| led::Led::new(&dir)),
        headphones: env_text("SEMMELTV_HEADPHONES").is_some_and(|value| value == "bluetooth"),
        downloads: env_text("SEMMELTV_DOWNLOADS").map(downloads::remote::Downloads::new),
        push_subject: env_text("SEMMELTV_ORIGINS")
            .and_then(|origins| {
                origins
                    .split(',')
                    .map(str::trim)
                    .find(|origin| !origin.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "https://semmeltv.ch".to_string()),
        ready: env_path("SEMMELTV_READY"),
    };
    let core = core::Core::start(setup).await;
    if let Some(config) = api {
        api::spawn(config, core.clone());
    }
    Ok(core)
}

fn run_toasty(args: &Args) -> Result<(), String> {
    let (which, rest) = args
        .toasty
        .split_first()
        .ok_or("toasty needs tv or library, then a migration command")?;
    let rest = rest.to_vec();
    match which.as_str() {
        "tv" => waiting(db::toasty_cli(&args.db, rest))?,
        "library" => {
            let file = args
                .manifest
                .clone()
                .unwrap_or_else(|| env::temp_dir().join("semmeltv-library-scratch.db"));
            waiting(library_db::toasty_cli(&file, rest))?
        }
        other => return Err(format!("toasty {other}: say tv or library")),
    }
    .map_err(|error| error.to_string())
}

fn run_library(args: &Args) -> Result<(), String> {
    let manifest = args
        .manifest
        .as_ref()
        .ok_or("library needs --manifest FILE, the library.db")?;
    match args.toasty.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["show"] => {
            let library =
                manifest::Manifest::read(manifest).map_err(|error| format!("{}: {error:?}", manifest.display()))?;
            println!("{}", library.render());
            Ok(())
        }
        ["from-toml", toml] => {
            let library = library_db::from_toml(Path::new(toml), manifest)?;
            let episodes: usize = library.show.iter().map(|show| show.episodes.len()).sum();
            println!(
                "{}: {} shows, {episodes} episodes, {} movies",
                manifest.display(),
                library.show.len(),
                library.movie.len()
            );
            Ok(())
        }
        _ => Err("library show | library from-toml FILE.toml, with --manifest library.db".to_string()),
    }
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
        for episode in &summary.episodes_unrated {
            println!("no IMDb episode rating: {episode}");
        }
        println!(
            "{} episodes rated from the IMDb datasets, {} without a rating",
            summary.episodes_rated,
            summary.episodes_unrated.len()
        );
    }
    Ok(())
}

async fn invite(args: &Args) -> Result<(), String> {
    let db = open_db(args).await?;
    let origins =
        env_text("SEMMELTV_ORIGINS").ok_or("invite needs SEMMELTV_ORIGINS, the addresses people sign in on")?;
    let auth = auth::Auth::new(db, &[])?;
    let wanted = args.transcript.trim();
    let person = auth
        .people()
        .await
        .map_err(|refusal| refusal.to_string())?
        .into_iter()
        .find(|person| person.name.eq_ignore_ascii_case(wanted))
        .ok_or_else(|| format!("There is no profile called {wanted}"))?;
    let token = auth
        .invite(None, person.id)
        .await
        .map_err(|refusal| refusal.to_string())?;
    println!(
        "Invite for {} ({}), open once within {} days:",
        person.name,
        person.role.name(),
        auth::INVITE_LIFETIME / 86400
    );
    for origin in origins.split(',').map(str::trim).filter(|origin| !origin.is_empty()) {
        println!("  {}/invite/{token}", origin.trim_end_matches('/'));
    }
    Ok(())
}

async fn stats(args: &Args) -> Result<(), String> {
    let days = match args.toasty.as_slice() {
        [] => 7,
        [days] => days
            .parse::<u64>()
            .ok()
            .filter(|days| *days > 0)
            .ok_or(format!("stats wants a number of days, not {days}"))?,
        _ => return Err("usage: semmeltv stats [DAYS]".to_string()),
    };
    let db = open_db(args).await?;
    let now = watching::now();
    let events = db
        .usage_since(now.saturating_sub(days * 86_400))
        .await
        .map_err(|error| error.to_string())?;
    let names = db
        .profiles()
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|profile| (profile.id, profile.name))
        .collect();
    print!("{}", usage::report(&events, &names, now, days, details::local_offset));
    Ok(())
}

async fn sign_in_gate(db: &Db) -> Result<Option<auth::gate::Gate>, String> {
    let Some(origins) = env_text("SEMMELTV_ORIGINS") else {
        eprintln!("api: SEMMELTV_ORIGINS is unset, so nobody signs in and anyone who reaches the API may use it");
        return Ok(None);
    };
    let origins = origins
        .split(',')
        .map(|origin| origin.trim().to_string())
        .filter(|origin| !origin.is_empty());
    let origins = origins.collect::<Vec<_>>();
    let service = match env_path("SEMMELTV_SERVICE_TOKEN_FILE") {
        Some(file) => Some(
            fs::read_to_string(&file)
                .map_err(|error| format!("{}: {error}", file.display()))?
                .trim()
                .to_string(),
        )
        .filter(|token| !token.is_empty()),
        None => None,
    };
    let auth = auth::Auth::new(db.clone(), &origins)?;
    auth.forget_expired().await;
    Ok(Some(auth::gate::Gate { auth, service }))
}

fn env_text(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.is_empty())
}

/// Plays one file on the TV set's own player, steered from stdin: `p` pauses,
/// `r` resumes, a number seeks to that second, `q` stops.
fn run_tv_set(rest: &[String]) -> Result<(), String> {
    use player::renderer::{self, Session, Start};
    let (file, rest) = rest
        .split_first()
        .ok_or("tv-set needs a file, then where to start in seconds and the audio track (from 1)")?;
    let from = rest
        .first()
        .map_or(Ok(0.0), |from| from.parse())
        .map_err(|error| format!("start: {error}"))?;
    let audio = rest
        .get(1)
        .map(|track| track.parse())
        .transpose()
        .map_err(|error| format!("audio track: {error}"))?;
    let renderer = env_text("SEMMELTV_TV_RENDERER").ok_or("tv-set needs SEMMELTV_TV_RENDERER")?;
    let port = env_text("SEMMELTV_TV_PORT")
        .map_or(Ok(8097), |port| port.parse())
        .map_err(|error| format!("SEMMELTV_TV_PORT: {error}"))?;
    let path = PathBuf::from(file);
    let media = waiting(stream::probe::probe(&path))??;
    let feed = renderer::feed(&media, audio);
    println!("{:?} as {feed:?}", media.video);
    let (done, finished) = std::sync::mpsc::channel();
    let start = Start {
        renderer: tv::upnp::Renderer::new(renderer),
        port,
        name: path.file_stem().unwrap_or_default().to_string_lossy().to_string(),
        path,
        media,
        feed,
        from,
    };
    let session = std::sync::Arc::new(
        Session::start(
            start,
            || {},
            || {},
            move |finish| {
                let _ = done.send(finish);
            },
        )
        .map_err(|error| error.to_string())?,
    );
    let steering = session.clone();
    std::thread::spawn(move || {
        for line in std::io::stdin().lines().map_while(Result::ok) {
            match line.trim() {
                "p" => steering.set_paused(true),
                "r" => steering.set_paused(false),
                "q" => steering.key(command::Command::Back),
                other => match other.parse() {
                    Ok(seconds) => steering.seek_to(seconds),
                    Err(_) => eprintln!("p, r, q or seconds"),
                },
            }
        }
    });
    loop {
        match finished.recv_timeout(std::time::Duration::from_secs(2)) {
            Ok(finish) => {
                println!("{finish:?}");
                return Ok(());
            }
            Err(_) => {
                let seen = session.seen();
                println!(
                    "{:>7.1} / {:.1} {}{}",
                    seen.position,
                    seen.duration,
                    if seen.started { "started" } else { "opening" },
                    if seen.paused { " paused" } else { "" }
                );
            }
        }
    }
}

fn handoff() -> Result<Option<player::renderer::Handoff>, String> {
    let Some(codecs) = env_text("SEMMELTV_TV_DECODES") else {
        return Ok(None);
    };
    let port = env_text("SEMMELTV_TV_PORT")
        .ok_or("SEMMELTV_TV_DECODES needs SEMMELTV_TV_PORT")?
        .parse()
        .map_err(|error| format!("SEMMELTV_TV_PORT: {error}"))?;
    Ok(Some(player::renderer::Handoff {
        codecs: codecs.split(',').map(|codec| codec.trim().to_string()).collect(),
        port,
    }))
}

fn env_number(name: &str, default: u64) -> Result<u64, String> {
    env_text(name).map_or(Ok(default), |value| {
        value.parse().map_err(|_| format!("{name} is not a number: {value}"))
    })
}

fn run_downloads(args: &Args) -> Result<(), String> {
    const GIB: u64 = 1024 * 1024 * 1024;
    let needed = |name: &str| env_text(name).ok_or(format!("downloads needs {name}"));
    let token_file = args.token_file.as_ref().ok_or("downloads needs TMDB_TOKEN_FILE")?;
    let token = fs::read_to_string(token_file).map_err(|error| format!("{}: {error}", token_file.display()))?;
    downloads::run(downloads::Config {
        listen: env_text("SEMMELTV_DOWNLOADS_LISTEN").unwrap_or_else(|| "127.0.0.1:8092".to_string()),
        state: needed("SEMMELTV_DOWNLOADS_STATE")?.into(),
        downloads: needed("SEMMELTV_DOWNLOADS_DIR")?.into(),
        torznab: needed("SEMMELTV_TORZNAB")?
            .split_whitespace()
            .map(str::to_string)
            .collect(),
        torznab_key_file: env_path("SEMMELTV_TORZNAB_KEY_FILE"),
        rqbit: needed("SEMMELTV_RQBIT")?,
        quota: env_number("SEMMELTV_DOWNLOADS_QUOTA_GIB", 30)? * GIB,
        reserve: env_number("SEMMELTV_DOWNLOADS_RESERVE_GIB", 10)? * GIB,
        meta: args.meta().to_path_buf(),
        remote: needed("SEMMELTV_LIBRARY_REMOTE")?,
        mount_rc: needed("SEMMELTV_MOUNT_RC")?.into(),
        sync_unit: env_text("SEMMELTV_SYNC_UNIT").unwrap_or_else(|| "semmeltv-sync.service".to_string()),
        tmdb_token: token.trim().to_string(),
        mdblist: mdblist_client(args)?,
    })
}

fn hear(args: &Args) -> Result<(), String> {
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
    println!("{:?}", intent::understand(&text, &library, &stored_progress(args)));
    Ok(())
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let cache = Cache::new(args.cache.clone());
    match args.mode.as_str() {
        "list" => list(&library::load(&args.library, args.meta())?),
        "understand" => {
            let library = library::load(&args.library, args.meta())?;
            println!(
                "{:?}",
                intent::understand(&args.transcript, &library, &stored_progress(&args))
            );
        }
        "artwork" => run_artwork(&args)?,
        "import" => run_import(&args)?,
        "downloads" => run_downloads(&args)?,
        "get" => downloads::cli::run(&args.toasty)?,
        "toasty" => run_toasty(&args)?,
        "progress" => {
            let mut files: Vec<_> = stored_progress(&args).files.into_iter().collect();
            files.sort_by(|a, b| a.0.cmp(&b.0));
            for (file, watched) in files
                .iter()
                .filter(|(_, watched)| watched.stopped_at().is_some() || watched.finished())
            {
                println!(
                    "{:>8.1} {:>8} {:>8.1}  {}",
                    watched.stopped_at().unwrap_or(0.0),
                    if watched.finished() { "finished" } else { "" },
                    watched.resume_at(),
                    file.strip_prefix(&args.library).unwrap_or(file).display()
                );
            }
        }
        "library" => run_library(&args)?,
        "hear" => hear(&args)?,
        "invite" => waiting(invite(&args))??,
        "stats" => waiting(stats(&args))??,
        "tv-set" => run_tv_set(&args.toasty)?,
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
