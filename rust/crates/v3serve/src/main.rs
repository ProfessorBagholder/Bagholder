//! Phase 0b spike: serve the built Svelte SPA at /v3 from Rust (axum + tower-http
//! ServeDir) and reverse-proxy /api to the existing Bagholder backend, so one
//! origin serves both — exactly as the Phase 4 server relay will. This proves the
//! serving layer; it is not yet the real server (no model, no sync).
//!
//! Env: PORT (default 8790), V3_DIST (path to web/dist), BAGHOLDER_API_TARGET
//! (default http://127.0.0.1:8788).

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header::HOST, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{any, get},
    Router,
};
use tower_http::services::{ServeDir, ServeFile};

#[tokio::main]
async fn main() {
    let dist = std::env::var("V3_DIST").unwrap_or_else(|_| "web/dist".into());
    let index = format!("{dist}/index.html");
    // ServeDir serves hashed assets; unknown paths under /v3 fall back to
    // index.html so client-side routing works (an SPA served under a subpath).
    let serve = ServeDir::new(&dist).fallback(ServeFile::new(index));

    let client = reqwest::Client::new();
    let app = Router::new()
        .route("/api/*path", any(proxy))
        .route("/", get(|| async { Redirect::permanent("/v3/") }))
        .nest_service("/v3", serve)
        .with_state(client);

    let port = std::env::var("PORT").unwrap_or_else(|_| "8790".into());
    let target = api_target();
    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .expect("bind");
    println!("v3serve  http://127.0.0.1:{port}/v3/   (dist: {dist}, api -> {target})");
    axum::serve(listener, app).await.expect("serve");
}

fn api_target() -> String {
    std::env::var("BAGHOLDER_API_TARGET").unwrap_or_else(|_| "http://127.0.0.1:8788".into())
}

/// Forward /api/* to the backend, preserving method, path+query, headers and body,
/// and stream the response back. axum 0.7 and reqwest 0.12 share the `http` 1.x
/// types, so method/header values pass through without conversion.
async fn proxy(State(client): State<reqwest::Client>, req: Request) -> Response {
    let path_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let url = format!("{}{}", api_target(), path_query);

    let (parts, body) = req.into_parts();
    let bytes = match axum::body::to_bytes(body, usize::MAX).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::BAD_GATEWAY, "bad request body").into_response(),
    };

    let mut rb = client.request(parts.method, &url).body(bytes.to_vec());
    for (name, value) in parts.headers.iter() {
        if name != HOST {
            rb = rb.header(name, value);
        }
    }

    let upstream = match rb.send().await {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("upstream: {e}")).into_response(),
    };

    let mut builder = Response::builder().status(upstream.status());
    for (name, value) in upstream.headers().iter() {
        builder = builder.header(name, value);
    }
    match upstream.bytes().await {
        Ok(b) => builder.body(Body::from(b)).unwrap(),
        Err(_) => (StatusCode::BAD_GATEWAY, "upstream body").into_response(),
    }
}
