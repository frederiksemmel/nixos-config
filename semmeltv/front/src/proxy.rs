use crate::bakery::{Cause, Closed, Probe, Scope, Status, Upstream};
use crate::probe::probe;
use crate::screen;
use crate::shop::Shop;
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full};
use hyper::body::Incoming;
use hyper::header::{self, HeaderMap, HeaderName, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;

pub type Body = BoxBody<Bytes, hyper::Error>;
type Upstreams = Client<HttpConnector, Body>;

pub const MARK: &str = "x-semmeltv-maintenance";
const HOP_BY_HOP: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "upgrade",
];

#[derive(Clone)]
pub struct Front {
    pub shop: Arc<Shop>,
    pub core: String,
    pub web: String,
    client: Upstreams,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Asking {
    Page,
    Data,
    Anything,
}

pub fn full(bytes: impl Into<Bytes>) -> Body {
    Full::new(bytes.into()).map_err(|never| match never {}).boxed()
}

fn empty() -> Body {
    Empty::new().map_err(|never| match never {}).boxed()
}

pub fn json(status: StatusCode, value: &impl serde::Serialize) -> Response<Body> {
    let mut response = Response::new(full(serde_json::to_vec(value).unwrap_or_default()));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn file(body: &'static str, kind: &'static str) -> Response<Body> {
    let mut response = Response::new(full(body));
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(kind));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=3600"));
    response
}

fn closed(closed: &Closed, asking: Asking) -> Response<Body> {
    let mut response = match asking {
        Asking::Page => {
            let mut response = Response::new(full(screen::page(closed)));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            );
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Asking::Data | Asking::Anything => json(StatusCode::SERVICE_UNAVAILABLE, &Status::Closed(closed.clone())),
    };
    *response.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
    let headers = response.headers_mut();
    headers.insert(header::RETRY_AFTER, HeaderValue::from(closed.retry_after));
    let cause = serde_json::to_value(closed.cause).ok();
    let cause = cause.as_ref().and_then(|cause| cause.as_str()).unwrap_or("closed");
    if let Ok(value) = HeaderValue::from_str(cause) {
        headers.insert(HeaderName::from_static(MARK), value);
    }
    response
}

fn asking(request: &Request<Incoming>, api: bool) -> Asking {
    let headers = request.headers();
    if api || headers.get("x-topcoat-runtime").is_some_and(|value| value == "true") {
        return Asking::Data;
    }
    let reading = matches!(*request.method(), Method::GET | Method::HEAD);
    let navigating = headers.get("sec-fetch-mode").is_some_and(|mode| mode == "navigate")
        || headers
            .get(header::ACCEPT)
            .and_then(|accept| accept.to_str().ok())
            .is_some_and(|accept| accept.contains("text/html"));
    if reading && navigating {
        Asking::Page
    } else {
        Asking::Anything
    }
}

fn is_upgrade(headers: &HeaderMap) -> bool {
    headers.get_all(header::CONNECTION).iter().any(|value| {
        value.to_str().is_ok_and(|value| {
            value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
        })
    })
}

fn strip_hop_by_hop(headers: &mut HeaderMap) {
    let named: Vec<HeaderName> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
        .collect();
    for name in named {
        headers.remove(name);
    }
    for name in HOP_BY_HOP {
        headers.remove(name);
    }
}

impl Front {
    pub fn new(shop: Arc<Shop>, core: String, web: String) -> Self {
        let mut connector = HttpConnector::new();
        connector.set_connect_timeout(Some(Duration::from_secs(2)));
        connector.set_nodelay(true);
        let client = Client::builder(TokioExecutor::new()).build(connector);
        Self {
            shop,
            core,
            web,
            client,
        }
    }

    pub async fn serve(self, listener: TcpListener) {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(accepted) => accepted,
                Err(error) => {
                    eprintln!("accept: {error}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            let front = self.clone();
            tokio::spawn(async move {
                let service = hyper::service::service_fn(move |request| {
                    let front = front.clone();
                    async move { Ok::<_, Infallible>(front.answer(request).await) }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .with_upgrades()
                    .await;
            });
        }
    }

    pub async fn answer(&self, request: Request<Incoming>) -> Response<Body> {
        let path = request.uri().path();
        match path {
            "/api/maintenance" => return json(StatusCode::OK, &self.shop.status(Scope::Pages)),
            "/api/maintenance/screen" => {
                return match self.shop.status(Scope::Pages) {
                    Status::Closed(status) => {
                        let mut response = closed(&status, Asking::Page);
                        *response.status_mut() = StatusCode::OK;
                        response
                    }
                    Status::Open => {
                        let mut response = Response::new(full(
                            "<!doctype html><script>window.parent.postMessage({ semmeltv: \"open\" }, location.origin)</script>",
                        ));
                        response.headers_mut().insert(
                            header::CONTENT_TYPE,
                            HeaderValue::from_static("text/html; charset=utf-8"),
                        );
                        response
                    }
                };
            }
            "/api/maintenance/roll.svg" => return file(screen::ROLL, "image/svg+xml"),
            "/api/maintenance/wordmark.svg" => return file(screen::WORDMARK, "image/svg+xml"),
            _ => {}
        }
        let api = path == "/api" || path.starts_with("/api/");
        let (upstream, scope) = if api {
            (Upstream::Core, Scope::Api)
        } else {
            (Upstream::Web, Scope::Pages)
        };
        let asking = asking(&request, api);
        if asking != Asking::Anything
            && let Status::Closed(status) = self.shop.status(scope)
            && (status.cause == Cause::Planned || !self.reachable(scope).await)
        {
            return closed(&status, asking);
        }
        let address = match upstream {
            Upstream::Core => &self.core,
            Upstream::Web => &self.web,
        };
        match self.forward(address, request).await {
            Ok(response) => {
                if self.shop.is_down(upstream) {
                    self.shop.wake(upstream);
                }
                response
            }
            Err(error) => {
                if error.is_connect() {
                    self.shop.observe(upstream, Probe::Refused);
                }
                self.shop.wake(upstream);
                match self.shop.status(scope) {
                    Status::Closed(status) => closed(&status, asking),
                    Status::Open => {
                        eprintln!("{upstream:?}: {error}");
                        let mut response = Response::new(empty());
                        *response.status_mut() = StatusCode::BAD_GATEWAY;
                        response
                    }
                }
            }
        }
    }

    async fn reachable(&self, scope: Scope) -> bool {
        let mut needed = vec![(Upstream::Core, &self.core)];
        if scope == Scope::Pages {
            needed.push((Upstream::Web, &self.web));
        }
        for (upstream, address) in needed {
            if self.shop.is_down(upstream) && probe(address, None).await != Probe::Answered {
                return false;
            }
        }
        true
    }

    async fn forward(
        &self,
        address: &str,
        mut request: Request<Incoming>,
    ) -> Result<Response<Body>, hyper_util::client::legacy::Error> {
        let upgrade = is_upgrade(request.headers());
        let target = request.uri().path_and_query().map_or("/", |target| target.as_str());
        *request.uri_mut() = format!("http://{address}{target}")
            .parse()
            .expect("a path from a parsed URI");
        if !upgrade {
            strip_hop_by_hop(request.headers_mut());
        }
        let downstream = upgrade.then(|| hyper::upgrade::on(&mut request));
        let mut response = self.client.request(request.map(BodyExt::boxed)).await?;
        if response.status() == StatusCode::SWITCHING_PROTOCOLS {
            if let Some(downstream) = downstream {
                let upstream = hyper::upgrade::on(&mut response);
                tokio::spawn(async move {
                    if let (Ok(downstream), Ok(upstream)) = (downstream.await, upstream.await) {
                        let _ =
                            tokio::io::copy_bidirectional(&mut TokioIo::new(downstream), &mut TokioIo::new(upstream))
                                .await;
                    }
                });
            }
        } else {
            strip_hop_by_hop(response.headers_mut());
        }
        Ok(response.map(BodyExt::boxed))
    }
}
