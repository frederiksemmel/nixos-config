use std::env;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::OnceLock;
use topcoat::Result;
use topcoat::context::Cx;
use topcoat::router::response::{IntoResponse, Response};
use topcoat::router::{Body, HeaderValue, Layer, LayerFuture, Next, Path, header, route};

pub fn id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        env::var("SEMMELTV_WEB_BUILD")
            .ok()
            .filter(|id| !id.is_empty())
            .unwrap_or_else(of_executable)
    })
}

fn of_executable() -> String {
    let mut hasher = DefaultHasher::new();
    if let Ok(exe) = env::current_exe() {
        exe.hash(&mut hasher);
        if let Ok(metadata) = exe.metadata() {
            metadata.len().hash(&mut hasher);
            metadata.modified().ok().hash(&mut hasher);
        }
    }
    format!("{:016x}", hasher.finish())
}

pub struct Build;

impl IntoResponse for Build {
    fn into_response(self, _cx: &Cx) -> Result<Response> {
        Ok(Response::builder()
            .header("Content-Type", "text/plain; charset=utf-8")
            .header("Cache-Control", "no-store")
            .body(Body::from(id()))?)
    }
}

#[route(GET "/build")]
async fn build() -> Result<Build> {
    Ok(Build)
}

pub struct Revalidate;

impl Layer for Revalidate {
    fn path(&self) -> Option<&Path> {
        None
    }

    fn handle<'a>(&'a self, cx: &'a Cx, body: Body, next: Next<'a>) -> LayerFuture<'a> {
        Box::pin(async move {
            let mut response = next.run(cx, body).await?;
            let html = response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|kind| kind.to_str().ok())
                .is_some_and(|kind| kind.starts_with("text/html"));
            if html && !response.headers().contains_key(header::CACHE_CONTROL) {
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            }
            Ok(response)
        })
    }
}
