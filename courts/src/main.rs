#![recursion_limit = "512"]

mod auth;
mod db;
mod discover;
mod eversports;
mod feedback;
mod filters;
mod gotcourts;
mod layout;
mod notify;
mod pages;
mod poll;
mod push;
mod share;
mod slots;
mod source;
mod tariffs;
mod time;

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use topcoat::asset::{AssetBundle, RouterBuilderAssetExt};
use topcoat::cookie::RouterBuilderCookieExt;
use topcoat::router::{Router, RouterBuilderDiscoverExt};
use topcoat::runtime::RouterBuilderRuntimeExt;
use topcoat_asset::{Bundler, BundlerConfig};

const USAGE: &str = "usage: courts [bundle DIR | toasty ARGS…]

Serves the court finder on HOST:PORT (default 127.0.0.1:3000) and asks
GotCourts every minute about the clubs its map lists inside the city of
Zurich, and the padel and squash within COURTS_AREA (LAT,LNG,KM; default
Zurich HB, 16 km), plus COURTS_CLUBS and minus COURTS_EXCLUDE (club ids).
COURTS_EVERSPORTS lists venues booked on Eversports instead, by slug
(www.eversports.ch/s/SLUG), comma-separated.
COURTS_DB is the database (default ./courts.db).
COURTS_KEY_FILE holds the key to sign in with (default: made up, beside the database).
COURTS_ORIGIN is where the site is on the web, for push services.
COURTS_HOME is LAT,LNG that distances count from (default Zurich HB).
COURTS_DAYS is how many days ahead to look (default 14).
COURTS_PER_MINUTE is how many later club-days to ask about each minute (default 20).
COURTS_PRICE_GAP is how many seconds to wait between price questions (default 10).
COURTS_ASSETS is the asset bundle; without it the app bundles itself.";

pub use time::now;

fn bundle(dir: &PathBuf) -> Result<(), String> {
    let exe = env::current_exe().map_err(|error| error.to_string())?;
    let binary = std::fs::read(&exe).map_err(|error| format!("{}: {error}", exe.display()))?;
    Bundler::new(&BundlerConfig::new().cache_dir(dir.with_extension("cache")))
        .bundle(&binary, dir)
        .map_err(|error| format!("bundling assets into {}: {error}", dir.display()))?;
    Ok(())
}

fn assets() -> Result<AssetBundle, String> {
    let dir = match env::var_os("COURTS_ASSETS").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => {
            let dir = env::temp_dir().join(format!("courts-assets-{}", std::process::id()));
            bundle(&dir)?;
            dir
        }
    };
    AssetBundle::load_dir(&dir).map_err(|error| format!("{}: {error}", dir.display()))
}

fn var(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn database() -> PathBuf {
    var("COURTS_DB").map_or_else(|| PathBuf::from("courts.db"), PathBuf::from)
}

/// The key in COURTS_KEY_FILE, or else the one kept beside the database,
/// made up on the first start.
fn key() -> Result<String, String> {
    let file = var("COURTS_KEY_FILE").map_or_else(|| database().with_file_name("key"), PathBuf::from);
    if !file.exists() {
        use base64::Engine;
        use ring::rand::SecureRandom;
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut bytes = [0u8; 24];
        ring::rand::SystemRandom::new()
            .fill(&mut bytes)
            .map_err(|_| "no randomness for a key".to_string())?;
        let made = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&file)
            .and_then(|mut out| out.write_all(made.as_bytes()))
            .map_err(|error| format!("{}: {error}", file.display()))?;
        eprintln!("made a key to sign in with: it is in {}", file.display());
    }
    let key = std::fs::read_to_string(&file)
        .map_err(|error| format!("{}: {error}", file.display()))?
        .trim()
        .to_string();
    if key.len() < 16 {
        return Err(format!(
            "the key in {} is too short: use at least 16 characters",
            file.display()
        ));
    }
    Ok(key)
}

async fn serve() -> Result<(), String> {
    let assets = assets()?;
    let db = db::Db::open(&database())
        .await
        .map_err(|error| format!("database: {error}"))?;
    let key = key()?;
    let ids = |name: &str| -> Vec<i64> {
        var(name)
            .unwrap_or_default()
            .split(',')
            .filter_map(|id| id.trim().parse().ok())
            .collect()
    };
    let area = var("COURTS_AREA")
        .and_then(|area| discover::Area::parse(&area))
        .unwrap_or(discover::Area {
            latitude: 47.3779,
            longitude: 8.5403,
            km: 16.0,
        });
    let home = var("COURTS_HOME")
        .and_then(|home| {
            let (lat, lng) = home.split_once(',')?;
            Some((lat.trim().parse().ok()?, lng.trim().parse().ok()?))
        })
        .unwrap_or((47.3779, 8.5403));
    let days = var("COURTS_DAYS")
        .and_then(|d| d.parse().ok())
        .unwrap_or(14i64)
        .clamp(1, 60);
    let per_minute = var("COURTS_PER_MINUTE").and_then(|d| d.parse().ok()).unwrap_or(20usize);
    let origin = var("COURTS_ORIGIN").unwrap_or_else(|| "https://localhost".into());
    let push = push::Push::new(db.clone(), origin.clone());
    let clubs = discover::Clubs::default();
    let finder = std::sync::Arc::new(discover::Finder {
        db: db.clone(),
        gotcourts: gotcourts::GotCourts::default(),
        clubs: clubs.clone(),
        area,
        extra: ids("COURTS_CLUBS"),
        exclude: ids("COURTS_EXCLUDE"),
    });
    let venues = eversports::Venues::new(
        var("COURTS_EVERSPORTS")
            .unwrap_or_default()
            .split(',')
            .map(|slug| slug.trim().to_string())
            .filter(|slug| !slug.is_empty())
            .collect(),
    );
    let poller = poll::Poller {
        db: db.clone(),
        gotcourts: gotcourts::GotCourts::default(),
        push: push.clone(),
        clubs: clubs.clone(),
        finder,
        venues,
        days,
        per_minute,
    };
    tokio::spawn(poller.clone().run());
    tokio::spawn(
        tariffs::Prober {
            db: db.clone(),
            gotcourts: gotcourts::GotCourts::default(),
            clubs: clubs.clone(),
            gap: std::time::Duration::from_secs(var("COURTS_PRICE_GAP").and_then(|s| s.parse().ok()).unwrap_or(10)),
        }
        .run(),
    );
    let router: Router = Router::builder()
        .discover()
        .cookies()
        .layer(auth::SignedIn)
        .app_context(db)
        .app_context(push)
        .app_context(poller)
        .app_context(auth::Key(key))
        .app_context(auth::Origin(origin))
        .app_context(pages::Site { home, days })
        .app_context(clubs)
        .app_context(share::Prices::default())
        .assets(assets)
        .runtime()
        .build();
    topcoat::start(router).await.map_err(|error| error.to_string())
}

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    let runtime = || tokio::runtime::Runtime::new().map_err(|error| error.to_string());
    let result = match arguments.as_slice() {
        [] => runtime().and_then(|runtime| runtime.block_on(serve())),
        [mode, dir] if mode == "bundle" => bundle(&PathBuf::from(dir)),
        [mode, rest @ ..] if mode == "toasty" => runtime().and_then(|runtime| {
            runtime
                .block_on(db::toasty_cli(&database(), rest.to_vec()))
                .map_err(|error| error.to_string())
        }),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
