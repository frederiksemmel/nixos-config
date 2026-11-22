use crate::bakery::{SECOND, Scope, Status};
use crate::proxy::Front;
use crate::shop::{Shop, now};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

async fn upstream(name: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut asked = Vec::new();
                let mut buffer = [0; 4096];
                while !asked.windows(4).any(|window| window == b"\r\n\r\n") {
                    let read = stream.read(&mut buffer).await.unwrap();
                    if read == 0 {
                        return;
                    }
                    asked.extend_from_slice(&buffer[..read]);
                }
                let asked = String::from_utf8_lossy(&asked).to_string();
                if asked.to_ascii_lowercase().contains("upgrade: echo") {
                    stream
                        .write_all(b"HTTP/1.1 101 Switching Protocols\r\nConnection: upgrade\r\nUpgrade: echo\r\n\r\n")
                        .await
                        .unwrap();
                    loop {
                        let read = stream.read(&mut buffer).await.unwrap_or(0);
                        if read == 0 {
                            return;
                        }
                        stream.write_all(&buffer[..read]).await.unwrap();
                    }
                }
                let body = format!("{name} got {}", asked.replace("\r\n", " | "));
                let answer = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(answer.as_bytes()).await.unwrap();
            });
        }
    });
    address
}

async fn nobody() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().to_string()
}

async fn front(core: String, web: String) -> (String, Arc<Shop>) {
    let shop = Arc::new(Shop::open(None));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    tokio::spawn(Front::new(shop.clone(), core, web).serve(listener));
    (address, shop)
}

async fn send(address: &str, request: &str) -> String {
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).await.unwrap();
    answer
}

fn get(path: &str, headers: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nHost: semmeltv5.semmeltv.ch\r\nConnection: close\r\n{headers}\r\n")
}

const PAGE: &str = "Accept: text/html,application/xhtml+xml\r\nSec-Fetch-Mode: navigate\r\n";

#[tokio::test]
async fn the_api_goes_to_the_core_and_everything_else_to_the_web_app() {
    let (front, _) = front(upstream("core").await, upstream("web").await).await;
    let api = send(&front, &get("/api/library?q=dune", "")).await;
    assert!(api.contains("core got GET /api/library?q=dune HTTP/1.1"), "{api}");
    let page = send(&front, &get("/movies/dune", PAGE)).await;
    assert!(page.contains("web got GET /movies/dune"), "{page}");
}

#[tokio::test]
async fn the_visitors_host_and_headers_reach_the_upstream_unchanged() {
    let (front, _) = front(upstream("core").await, upstream("web").await).await;
    let answer = send(
        &front,
        &get("/api/playback", "Cookie: session=abc\r\nX-Forwarded-For: 10.0.0.7\r\n"),
    )
    .await;
    assert!(answer.contains("host: semmeltv5.semmeltv.ch"), "{answer}");
    assert!(answer.contains("cookie: session=abc"), "{answer}");
    assert!(answer.contains("x-forwarded-for: 10.0.0.7"), "{answer}");
}

#[tokio::test]
async fn an_upgraded_connection_passes_both_ways() {
    let (front, _) = front(upstream("core").await, upstream("web").await).await;
    let mut stream = TcpStream::connect(&front).await.unwrap();
    stream
        .write_all(b"GET /api/events HTTP/1.1\r\nHost: tv\r\nConnection: Upgrade\r\nUpgrade: echo\r\n\r\n")
        .await
        .unwrap();
    let mut head = vec![0; 256];
    let read = stream.read(&mut head).await.unwrap();
    assert!(String::from_utf8_lossy(&head[..read]).starts_with("HTTP/1.1 101"));
    stream.write_all(b"frische Semmeln").await.unwrap();
    let mut echoed = [0; 15];
    stream.read_exact(&mut echoed).await.unwrap();
    assert_eq!(&echoed, b"frische Semmeln");
}

#[tokio::test]
async fn an_api_call_to_a_core_that_is_gone_gets_a_503_with_the_guess() {
    let (front, _) = front(nobody().await, upstream("web").await).await;
    let answer = send(&front, &get("/api/library", "")).await;
    assert!(answer.starts_with("HTTP/1.1 503"), "{answer}");
    assert!(answer.contains("retry-after: 60"), "{answer}");
    assert!(answer.contains("x-semmeltv-maintenance: down"), "{answer}");
    assert!(answer.contains("\"eta\":\"Wieder da in etwa 1 Minute\""), "{answer}");
}

#[tokio::test]
async fn a_page_opened_while_the_core_is_gone_gets_the_closed_screen() {
    let (front, _) = front(nobody().await, upstream("web").await).await;
    send(&front, &get("/api/library", "")).await;
    let page = send(&front, &get("/shows/severance", PAGE)).await;
    assert!(page.starts_with("HTTP/1.1 503"), "{page}");
    assert!(page.contains("Wir haben es versemmelt"), "{page}");
    assert!(page.contains("text/html"), "{page}");
}

#[tokio::test]
async fn the_web_apps_scripts_and_styles_still_load_while_the_core_is_gone() {
    let (front, _) = front(nobody().await, upstream("web").await).await;
    send(&front, &get("/api/library", "")).await;
    let script = send(&front, &get("/assets/app.js", "Accept: */*\r\n")).await;
    assert!(script.contains("web got GET /assets/app.js"), "{script}");
}

#[tokio::test]
async fn a_page_tapped_in_the_web_app_gets_the_closed_answer_as_data() {
    let (front, shop) = front(upstream("core").await, upstream("web").await).await;
    shop.change(|bakery| bakery.plan(now(), 600 * SECOND, Some("Tests".to_string())));
    let answer = send(
        &front,
        &get("/movies", "X-Topcoat-Runtime: true\r\nAccept: text/html\r\n"),
    )
    .await;
    assert!(answer.starts_with("HTTP/1.1 503"), "{answer}");
    assert!(answer.contains("\"cause\":\"planned\""), "{answer}");
}

#[tokio::test]
async fn planned_work_closes_pages_and_the_api_until_it_is_switched_off() {
    let (front, shop) = front(upstream("core").await, upstream("web").await).await;
    shop.change(|bakery| bakery.plan(now(), 1200 * SECOND, Some("neue Streaming-Limits".to_string())));
    let page = send(&front, &get("/", PAGE)).await;
    assert!(page.contains("Die Backstube ist kurz zu"), "{page}");
    assert!(page.contains("neue Streaming-Limits"), "{page}");
    assert!(page.contains("in etwa 20 Minuten"), "{page}");
    assert!(
        send(&front, &get("/api/playback", ""))
            .await
            .starts_with("HTTP/1.1 503")
    );
    shop.change(|bakery| bakery.reopen());
    assert!(send(&front, &get("/", PAGE)).await.contains("web got GET /"));
}

#[tokio::test]
async fn a_core_still_answering_after_announcing_its_restart_keeps_serving() {
    let (front, shop) = front(upstream("core").await, upstream("web").await).await;
    shop.change(|bakery| bakery.announce(now()));
    assert!(
        send(&front, &get("/api/playback", ""))
            .await
            .contains("core got GET /api/playback")
    );
    assert!(send(&front, &get("/", PAGE)).await.contains("web got GET /"));
}

#[tokio::test]
async fn the_front_answers_how_the_bakery_is_doing_itself() {
    let (front, shop) = front(nobody().await, nobody().await).await;
    let open = send(&front, &get("/api/maintenance", "")).await;
    assert!(
        open.starts_with("HTTP/1.1 200") && open.contains("{\"state\":\"open\"}"),
        "{open}"
    );
    shop.change(|bakery| bakery.announce(now()));
    let closed = send(&front, &get("/api/maintenance", "")).await;
    assert!(closed.contains("\"cause\":\"restarting\""), "{closed}");
    assert!(matches!(shop.status(Scope::Api), Status::Closed(_)));
}

#[tokio::test]
async fn the_screen_for_an_open_page_tells_its_parent_the_bakery_is_open() {
    let (front, _) = front(upstream("core").await, upstream("web").await).await;
    let screen = send(&front, &get("/api/maintenance/screen", "")).await;
    assert!(screen.contains("postMessage({ semmeltv: \"open\" }"), "{screen}");
}
