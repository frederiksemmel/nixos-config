use serde_json::json;
use topcoat::Result;
use topcoat::asset::{Asset, asset, asset_config};
use topcoat::context::Cx;
use topcoat::router::Body;
use topcoat::router::response::{IntoResponse, Response};
use topcoat::router::route;

pub const THEME: &str = "#0c0c10";
pub const WORDMARK: Asset = asset!("../../assets/logo/semmeltv-dark.svg");
pub const FAVICON: Asset = asset!("../../assets/logo/favicon.svg");
pub const FAVICON_PNG: Asset = asset!("../../assets/logo/favicon-32.png");
pub const TOUCH_ICON: Asset = asset!("../../assets/logo/apple-touch-icon.png");
const ICON_192: Asset = asset!("../../assets/logo/icon-192.png");
const ICON_512: Asset = asset!("../../assets/logo/icon-512.png");
const ICON_MASKABLE: Asset = asset!("../../assets/logo/icon-maskable-512.png");

pub struct Manifest;

impl IntoResponse for Manifest {
    fn into_response(self, cx: &Cx) -> Result<Response> {
        let assets = asset_config(cx);
        let icon = |asset: Asset, sizes: &str, kind: &str, purpose: &str| {
            json!({ "src": assets.resolve(asset), "sizes": sizes, "type": kind, "purpose": purpose })
        };
        let body = json!({
            "id": "/",
            "name": "SemmelTV",
            "short_name": "SemmelTV",
            "start_url": "/",
            "scope": "/",
            "display": "standalone",
            "background_color": THEME,
            "theme_color": THEME,
            "icons": [
                icon(FAVICON, "any", "image/svg+xml", "any"),
                icon(ICON_192, "192x192", "image/png", "any"),
                icon(ICON_512, "512x512", "image/png", "any"),
                icon(ICON_MASKABLE, "512x512", "image/png", "maskable"),
            ],
        });
        Ok(Response::builder()
            .header("Content-Type", "application/manifest+json")
            .header("Cache-Control", "no-cache")
            .body(Body::from(body.to_string()))?)
    }
}

#[route(GET "/manifest.webmanifest")]
async fn manifest() -> Result<Manifest> {
    Ok(Manifest)
}
