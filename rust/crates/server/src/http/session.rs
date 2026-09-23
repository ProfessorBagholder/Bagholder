//! The Wealthsimple session: signing in through the streamed browser window,
//! refreshing, syncing, disconnecting; and the app updating itself.

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use super::{answer, api_routes, blocking, Api, ApiError, AppState, Body, OkOr, Routed};
use crate::{login, session, update};

pub fn routes() -> Routed {
    let mut routed = api_routes! {
        post "/api/login/start" => login_start;
        post "/api/login/cancel" => login_cancel;
        post "/api/login/input" => login_input;
        post "/api/capture" => capture;
        post "/api/refresh" => refresh;
        post "/api/sync" => sync;
        post "/api/disconnect" => disconnect;
        post "/api/update" => start_update;
    };
    routed.router = routed.router.route("/api/login/frame", get(login_frame));
    routed
}

async fn login_start(State(state): State<AppState>) -> Api<login::StartLoginAnswer> {
    answer(move || login::start_login_browser(&state.app)).await
}

async fn login_cancel(State(state): State<AppState>) -> Api<login::CancelLoginAnswer> {
    answer(move || login::cancel_login(&state.app)).await
}

/// `POST /api/login/input`: a click, a key or a scroll on the streamed window,
/// passed to the browser as the DevTools event it names.
async fn login_input(State(state): State<AppState>, Body(event): Body<login::LoginInput>) -> Api<OkOr> {
    answer(move || login::login_input(&state.app, &event)).await
}

/// `GET /api/login/frame`: the window's latest frame, for a page that cannot hold the stream.
async fn login_frame(State(state): State<AppState>) -> Result<Response, ApiError> {
    Ok(match blocking(move || login::login_frame(&state.app)).await? {
        Some(jpeg) => ([(header::CONTENT_TYPE, "image/jpeg")], jpeg).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    })
}

/// `POST /api/capture`: tokens handed over by hand (the fallback to the window).
async fn capture(State(state): State<AppState>, Body(tokens): Body<session::Capture>) -> Api<OkOr> {
    answer(move || session::capture_tokens(&state.app, &tokens)).await
}

async fn refresh(State(state): State<AppState>) -> Api<session::RefreshAnswer> {
    answer(move || session::refresh_now(&state.app)).await
}

/// `POST /api/sync`: start a pull; its progress reaches the page as status changes.
async fn sync(State(state): State<AppState>) -> Api<session::SyncAnswer> {
    answer(move || session::sync_now(&state.app)).await
}

async fn disconnect(State(state): State<AppState>) -> Api<OkOr> {
    blocking(move || session::delete_session(&state.app)).await?;
    Ok(axum::Json(OkOr::ok()))
}

async fn start_update(State(state): State<AppState>) -> Api<OkOr> {
    answer(move || update::start_update(&state.app)).await
}
