//! The Wealthsimple session: signing in through the streamed browser window,
//! refreshing, syncing, disconnecting; and the app updating itself.

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Map, Value};

use super::{answer, blocking, Api, ApiError, AppState, Body};
use crate::{feeds, login, session, update};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/login/start", post(login_start))
        .route("/api/login/cancel", post(login_cancel))
        .route("/api/login/input", post(login_input))
        .route("/api/login/frame", get(login_frame))
        .route("/api/capture", post(capture))
        .route("/api/refresh", post(refresh))
        .route("/api/sync", post(sync))
        .route("/api/disconnect", post(disconnect))
        .route("/api/update", post(start_update))
}

async fn login_start() -> Api {
    answer(login::start_login_browser).await
}

async fn login_cancel() -> Api {
    answer(login::cancel_login).await
}

/// `POST /api/login/input`: a click, a key or a scroll on the streamed window,
/// passed to the browser as the DevTools event it names.
async fn login_input(Body(event): Body<Map<String, Value>>) -> Api {
    answer(move || login::login_input(&Value::Object(event))).await
}

/// `GET /api/login/frame`: the window's latest frame, for a page that cannot hold the stream.
async fn login_frame() -> Result<Response, ApiError> {
    Ok(match blocking(login::login_frame).await? {
        Some(jpeg) => ([(header::CONTENT_TYPE, "image/jpeg")], jpeg).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    })
}

/// `POST /api/capture`: tokens handed over by hand (the fallback to the window).
async fn capture(Body(tokens): Body<Map<String, Value>>) -> Api {
    answer(move || session::capture_tokens(&Value::Object(tokens))).await
}

async fn refresh() -> Api {
    answer(session::refresh_now).await
}

/// `POST /api/sync`: start a pull; its progress reaches the page as status changes.
async fn sync(State(state): State<AppState>) -> Api {
    let app = state.app;
    answer(move || {
        if session::load_session().is_none() {
            return json!({"ok": false, "error": "not connected"});
        }
        app.state.lock().unwrap().error.clear();
        crate::app::spawn("bagholder-sync", || {
            feeds::sync_then_market();
        });
        json!({"ok": true, "syncing": true})
    })
    .await
}

async fn disconnect() -> Api {
    blocking(session::delete_session).await?;
    Ok(Json(json!({"ok": true})))
}

async fn start_update() -> Api {
    answer(update::start_update).await
}
