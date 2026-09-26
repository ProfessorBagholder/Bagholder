//! What a route answers when it cannot answer: one enum, one status code and one
//! body shape (`{"ok": false, "error": …}`) per kind of failure.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::app::log;
use crate::http::OkOr;

#[derive(Debug)]
pub enum ApiError {
    /// The request is missing something or says something that cannot be read. 400.
    BadRequest(String),
    /// There is no such thing. 404.
    NotFound(String),
    /// Not now: something else is under way. 409.
    Conflict(String),
    /// The store failed. 500; the cause is logged, not sent.
    Store(rusqlite::Error),
    /// An outside source failed. 502.
    Upstream(String),
    /// A failure with a message the page shows. 500.
    Failed(String),
    /// The work panicked. 500.
    Internal,
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> ApiError {
        ApiError::Store(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (code, message) = match self {
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            ApiError::NotFound(m) => (StatusCode::NOT_FOUND, m),
            ApiError::Conflict(m) => (StatusCode::CONFLICT, m),
            ApiError::Store(e) => {
                log(&format!("bagholder: {}", e));
                (StatusCode::INTERNAL_SERVER_ERROR, "store failed".to_string())
            }
            ApiError::Upstream(m) => (StatusCode::BAD_GATEWAY, m),
            ApiError::Failed(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
            ApiError::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string()),
        };
        (code, Json(OkOr::err(message))).into_response()
    }
}
