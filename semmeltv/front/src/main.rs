mod bakery;
mod command;
mod control;
mod probe;
mod proxy;
mod screen;
mod shop;

#[cfg(test)]
mod tests;

use bakery::Upstream;
use shop::Shop;
use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use tokio::net::TcpListener;

const USAGE: &str = "usage: semmeltv-front
       semmeltv-front maintenance on|off|status|restarting …

Stands in front of the SemmelTV core and web app on SEMMELTV_FRONT_LISTEN
(default 127.0.0.1:8093): /api goes to SEMMELTV_CORE (default
127.0.0.1:8090), the rest to SEMMELTV_WEB (default 127.0.0.1:8091). While
either doesn't answer, or during planned work, pages get the bakery's closed
screen and the API a 503 with a guess of when it is back. The control port
SEMMELTV_FRONT_CONTROL (default 127.0.0.1:8094) is what semmeltv-maintenance
talks to. Outages are remembered in $STATE_DIRECTORY/bakery.json.";

fn setting(name: &str, default: &str) -> String {
    env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

async fn serve() -> Result<(), String> {
    let listen = setting("SEMMELTV_FRONT_LISTEN", "127.0.0.1:8093");
    let control = setting("SEMMELTV_FRONT_CONTROL", "127.0.0.1:8094");
    let core = setting("SEMMELTV_CORE", "127.0.0.1:8090");
    let web = setting("SEMMELTV_WEB", "127.0.0.1:8091");
    let file = env::var_os("STATE_DIRECTORY")
        .filter(|dir| !dir.is_empty())
        .map(|dir| PathBuf::from(dir).join("bakery.json"));
    let shop = Arc::new(Shop::open(file));
    let bind = |address: String| async move {
        TcpListener::bind(&address)
            .await
            .map_err(|error| format!("{address}: {error}"))
    };
    let front_listener = bind(listen.clone()).await?;
    let control_listener = bind(control.clone()).await?;
    tokio::spawn(probe::watch(
        shop.clone(),
        Upstream::Core,
        core.clone(),
        Some("/api/auth/choices"),
    ));
    tokio::spawn(probe::watch(shop.clone(), Upstream::Web, web.clone(), None));
    tokio::spawn(control::serve(shop.clone(), control_listener));
    eprintln!("front on http://{listen}, control on {control}, core {core}, web {web}");
    proxy::Front::new(shop, core, web).serve(front_listener).await;
    Ok(())
}

fn main() -> ExitCode {
    let mut arguments = env::args();
    let name = arguments.next().unwrap_or_default();
    let arguments: Vec<String> = arguments.collect();
    let called = Path::new(&name)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let result = match arguments.as_slice() {
        rest if called == "semmeltv-maintenance" => command::run(rest).map(|said| println!("{said}")),
        [maintenance, rest @ ..] if maintenance == "maintenance" => command::run(rest).map(|said| println!("{said}")),
        [] => tokio::runtime::Runtime::new()
            .map_err(|error| error.to_string())
            .and_then(|runtime| runtime.block_on(serve())),
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
