//! Typed requests. `Params<T>` reads the query string into `T`; `Body<T>` reads
//! a JSON body into `T`. Both refuse what they cannot read with a 400 in the
//! app's own error shape, rather than axum's plain-text rejections.

use axum::extract::{FromRequest, FromRequestParts, Query, Request};
use axum::http::request::Parts;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};

use super::ApiError;

pub struct Params<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned> FromRequestParts<S> for Params<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        Query::<T>::from_request_parts(parts, state).await.map(|q| Params(q.0)).map_err(|e| ApiError::BadRequest(e.body_text()))
    }
}

/// A JSON body. A request with no body at all is `T::default()`: several of the
/// page's writes say everything in their path.
pub struct Body<T>(pub T);

impl<S: Send + Sync, T: DeserializeOwned + Default> FromRequest<S> for Body<T> {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
        let bytes = axum::body::Bytes::from_request(req, state).await.map_err(|e| ApiError::BadRequest(e.body_text()))?;
        if bytes.iter().all(|b| b.is_ascii_whitespace()) {
            return Ok(Body(T::default()));
        }
        serde_json::from_slice(&bytes).map(Body).map_err(|e| ApiError::BadRequest(format!("the body is not what this route reads: {}", e)))
    }
}

/// A query value with the space around it gone; `None` when nothing is left.
pub fn trimmed<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<String>::deserialize(d)?.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()))
}

/// A query value with the space around it gone; empty when absent.
pub fn text<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(trimmed(d)?.unwrap_or_default())
}

/// A switch in a query: on for `1`, `true` or `yes`.
pub fn flag<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(matches!(trimmed(d)?.as_deref(), Some("1") | Some("true") | Some("yes")))
}
