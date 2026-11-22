use crate::bakery::{SECOND, Scope, Upstream};
use crate::proxy::{Body, json};
use crate::shop::{Shop, now};
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde::Deserialize;
use serde_json::{Value, json as value};
use std::convert::Infallible;
use std::sync::Arc;
use tokio::net::TcpListener;

#[derive(Deserialize)]
pub struct Plan {
    pub seconds: u64,
    pub reason: Option<String>,
}

pub fn report(shop: &Shop) -> Value {
    value!({
        "pages": shop.status(Scope::Pages),
        "api": shop.status(Scope::Api),
        "guess_seconds": {
            "core": shop.guess(Upstream::Core) / SECOND,
            "web": shop.guess(Upstream::Web) / SECOND,
        },
    })
}

async fn answer(shop: &Shop, request: Request<Incoming>) -> Response<Body> {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    match (method, path.as_str()) {
        (Method::GET, "/status") => {}
        (Method::PUT, "/planned") => {
            let Ok(body) = request.into_body().collect().await else {
                return json(StatusCode::BAD_REQUEST, &value!({ "error": "no body" }));
            };
            let plan: Plan = match serde_json::from_slice(&body.to_bytes()) {
                Ok(plan) => plan,
                Err(error) => return json(StatusCode::BAD_REQUEST, &value!({ "error": error.to_string() })),
            };
            shop.change(|bakery| bakery.plan(now(), plan.seconds * SECOND, plan.reason));
            eprintln!("closed for planned work");
        }
        (Method::DELETE, "/planned") => {
            if shop.change(|bakery| bakery.reopen()) {
                eprintln!("open again after planned work");
            }
        }
        (Method::POST, "/restarting") => {
            if shop.change(|bakery| bakery.announce(now())) {
                eprintln!("the core announced a restart");
            }
            shop.wake(Upstream::Core);
        }
        _ => {
            return json(
                StatusCode::NOT_FOUND,
                &value!({ "error": "GET /status, PUT or DELETE /planned, POST /restarting" }),
            );
        }
    }
    json(StatusCode::OK, &report(shop))
}

pub async fn serve(shop: Arc<Shop>, listener: TcpListener) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let shop = shop.clone();
        tokio::spawn(async move {
            let service = hyper::service::service_fn(move |request| {
                let shop = shop.clone();
                async move { Ok::<_, Infallible>(answer(&shop, request).await) }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}
