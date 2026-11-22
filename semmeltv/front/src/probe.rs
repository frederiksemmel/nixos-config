use crate::bakery::{Probe, Upstream};
use crate::shop::Shop;
use std::io::ErrorKind;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout};

const CONNECT: Duration = Duration::from_secs(2);
const ANSWER: Duration = Duration::from_secs(3);
const WHILE_UP: Duration = Duration::from_secs(5);
const WHILE_DOWN: Duration = Duration::from_secs(1);

pub async fn probe(address: &str, path: Option<&str>) -> Probe {
    let mut stream = match timeout(CONNECT, TcpStream::connect(address)).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) if error.kind() == ErrorKind::ConnectionRefused => return Probe::Refused,
        _ => return Probe::NoAnswer,
    };
    let Some(path) = path else { return Probe::Answered };
    let asked = async {
        let request = format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await?;
        let mut start = [0; 7];
        stream.read_exact(&mut start).await?;
        Ok::<_, std::io::Error>(&start == b"HTTP/1.")
    };
    match timeout(ANSWER, asked).await {
        Ok(Ok(true)) => Probe::Answered,
        _ => Probe::NoAnswer,
    }
}

pub async fn watch(shop: Arc<Shop>, upstream: Upstream, address: String, path: Option<&'static str>) {
    loop {
        let probe = probe(&address, path).await;
        shop.observe(upstream, probe);
        let pause = if shop.is_down(upstream) { WHILE_DOWN } else { WHILE_UP };
        let changed = match upstream {
            Upstream::Core => &shop.core_changed,
            Upstream::Web => &shop.web_changed,
        };
        tokio::select! {
            () = sleep(pause) => {}
            () = changed.notified() => {}
        }
    }
}
