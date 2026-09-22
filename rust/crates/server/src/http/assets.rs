//! The page and its files.
//!
//! The built page (`web/dist`) is embedded in the binary at build time
//! (`build.rs`), so a release serves its own page with nothing beside it. A debug
//! build looks on disk first, so a fresh `npm run build` shows without a Rust
//! rebuild; development proper runs the Vite server, which proxies `/api` here.
//!
//! Files under `/assets/` carry a hash of their contents in their name, so they
//! are `immutable`: a browser never asks for one twice. `index.html` names them
//! and is `no-cache`: always revalidated, so a new build is picked up at once.
//!
//! The legacy page (`ledger.html`, with its chart library) is served from the
//! repository root at `/v2` until the cutover removes it.

use axum::extract::{Path, State};
use axum::http::{header, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

use super::{blocking, ApiError, AppState};

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded_page.rs"));
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(index))
        .route("/index.html", get(index))
        .route("/assets/{*path}", get(asset))
        .route("/favicon.svg", get(|s: State<AppState>| page_file(s, "favicon.svg")))
        .route("/favicon.png", get(|s: State<AppState>| page_file(s, "favicon.png")))
        .route("/favicon.ico", get(|s: State<AppState>| page_file(s, "favicon.png")))
        .route("/v2", get(legacy))
        .route("/v2/", get(legacy))
        .route("/ledger.html", get(legacy))
        .route("/lightweight-charts.js", get(|s: State<AppState>| root_file(s, "lightweight-charts.js")))
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "woff2" => "font/woff2",
        "json" | "map" => "application/json; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// One file of the built page, by its path inside `web/dist`.
fn built(root: &std::path::Path, name: &str) -> Option<Vec<u8>> {
    let embedded = || embedded::FILES.iter().find(|(n, _)| *n == name).map(|(_, data)| data.to_vec());
    let on_disk = || std::fs::read(root.join("web/dist").join(name)).ok();
    if cfg!(debug_assertions) { on_disk().or_else(embedded) } else { embedded().or_else(on_disk) }
}

fn file(name: &str, data: Vec<u8>, cache: &'static str) -> Response {
    ([(header::CONTENT_TYPE, HeaderValue::from_static(content_type(name))), (header::CACHE_CONTROL, HeaderValue::from_static(cache))], data).into_response()
}

async fn index(State(state): State<AppState>) -> Result<Response, ApiError> {
    let root = state.app.root.clone();
    let app = state.app.clone();
    // a checkout where the page has not been built still opens: on the legacy page
    let data = blocking(move || built(&root, "index.html").or_else(|| std::fs::read(crate::feeds::ledger_path(&app)).ok())).await?;
    data.map(|d| file("index.html", d, "no-cache")).ok_or_else(|| ApiError::NotFound("index missing".into()))
}

async fn asset(State(state): State<AppState>, Path(path): Path<String>) -> Result<Response, ApiError> {
    if path.split('/').any(|part| part == ".." || part.is_empty()) {
        return Err(ApiError::NotFound("asset missing".into()));
    }
    let root = state.app.root.clone();
    let name = format!("assets/{}", path);
    let data = blocking({
        let name = name.clone();
        move || built(&root, &name)
    })
    .await?;
    data.map(|d| file(&name, d, "public, max-age=31536000, immutable")).ok_or_else(|| ApiError::NotFound("asset missing".into()))
}

/// A file of the built page that is not hashed (the icons).
async fn page_file(State(state): State<AppState>, name: &'static str) -> Result<Response, ApiError> {
    let root = state.app.root.clone();
    let data = blocking(move || built(&root, name).or_else(|| std::fs::read(root.join(name)).ok())).await?;
    data.map(|d| file(name, d, "no-cache")).ok_or_else(|| ApiError::NotFound(format!("{} missing", name)))
}

/// A file beside the legacy page, at the repository root.
async fn root_file(State(state): State<AppState>, name: &'static str) -> Result<Response, ApiError> {
    let root = state.app.root.clone();
    let data = blocking(move || std::fs::read(root.join(name)).ok()).await?;
    data.map(|d| file(name, d, "no-cache")).ok_or_else(|| ApiError::NotFound(format!("{} missing", name)))
}

async fn legacy(State(state): State<AppState>) -> Result<Response, ApiError> {
    let app = state.app.clone();
    let data = blocking(move || std::fs::read(crate::feeds::ledger_path(&app)).ok()).await?;
    data.map(|d| file("ledger.html", d, "no-cache")).ok_or_else(|| ApiError::NotFound("ledger.html missing".into()))
}
