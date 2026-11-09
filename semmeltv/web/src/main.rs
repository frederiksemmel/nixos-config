mod api;
mod brand;
mod controls;
mod layout;
mod live;
mod pages;
mod playing;
mod public;
mod storage;
mod subtitles;

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use topcoat::asset::{AssetBundle, RouterBuilderAssetExt};
use topcoat::router::{Router, RouterBuilderDiscoverExt};
use topcoat::runtime::RouterBuilderRuntimeExt;
use topcoat_asset::{Bundler, BundlerConfig};

const USAGE: &str = "usage: semmeltv-web [bundle DIR]

Serves the SemmelTV web app on HOST:PORT (default 127.0.0.1:3000).
SEMMELTV_API_URL is the TV's API (default http://127.0.0.1:8090).
SEMMELTV_API_PUBLIC_URL is where browsers reach that API (default: this origin).
SEMMELTV_WEB_ASSETS is the asset bundle; without it the app bundles itself.";
const DEFAULT_API: &str = "http://127.0.0.1:8090";

fn bundle(dir: &PathBuf) -> Result<(), String> {
    let exe = env::current_exe().map_err(|error| error.to_string())?;
    let binary = std::fs::read(&exe).map_err(|error| format!("{}: {error}", exe.display()))?;
    Bundler::new(&BundlerConfig::new().cache_dir(dir.with_extension("cache")))
        .bundle(&binary, dir)
        .map_err(|error| format!("bundling assets into {}: {error}", dir.display()))?;
    Ok(())
}

fn assets() -> Result<AssetBundle, String> {
    let dir = match env::var_os("SEMMELTV_WEB_ASSETS").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => {
            let dir = env::temp_dir().join(format!("semmeltv-web-assets-{}", std::process::id()));
            bundle(&dir)?;
            dir
        }
    };
    AssetBundle::load_dir(&dir).map_err(|error| format!("{}: {error}", dir.display()))
}

fn router(api: api::Api, live: live::Live, public: public::Public, assets: AssetBundle) -> Router {
    Router::builder()
        .discover()
        .app_context(api)
        .app_context(live)
        .app_context(public)
        .assets(assets)
        .runtime()
        .build()
}

async fn serve() -> Result<(), String> {
    let assets = assets()?;
    let base = env::var("SEMMELTV_API_URL").unwrap_or_else(|_| DEFAULT_API.to_string());
    let api = api::Api::new(base);
    let live = live::follow(api.events_url());
    let public = public::Public {
        api: env::var("SEMMELTV_API_PUBLIC_URL").unwrap_or_default(),
    };
    topcoat::start(router(api, live, public, assets))
        .await
        .map_err(|error| error.to_string())
}

fn main() -> ExitCode {
    let arguments: Vec<String> = env::args().skip(1).collect();
    let result = match arguments.as_slice() {
        [] => tokio::runtime::Runtime::new()
            .map_err(|error| error.to_string())
            .and_then(|runtime| runtime.block_on(serve())),
        [mode, dir] if mode == "bundle" => bundle(&PathBuf::from(dir)),
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
