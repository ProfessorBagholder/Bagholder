//! The notification list and its settings. (Its stream is in `stream`.)

use axum::extract::State;
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::{with_store, Api, AppState, Body};
use crate::{app, notify};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/notifications", get(list))
        .route("/api/notifications/settings", post(settings))
        .route("/api/notifications/test", post(test))
        .route("/api/notifications/read", post(read))
        .route("/api/notifications/seen", post(seen))
        .route("/api/notifications/clear", post(clear))
}

async fn list(State(state): State<AppState>) -> Api {
    with_store(&state, |conn| {
        Ok(json!({
            "ok": true,
            "settings": notify::status(conn)?,
            "kinds": notify::KINDS,
            "rows": bagholder_store::feeds::list_notifications(conn, 0, "", false, 50, true)?,
            "unread": bagholder_store::feeds::unread_notifications(conn)?,
        }))
    })
    .await
}

/// `POST /api/notifications/settings`: the switches that changed, by name.
async fn settings(State(state): State<AppState>, Body(patch): Body<Map<String, Value>>) -> Api {
    with_store(&state, move |conn| {
        notify::set_settings(conn, &Value::Object(patch))?;
        Ok(json!({"ok": true, "settings": notify::status(conn)?}))
    })
    .await
}

async fn test(State(state): State<AppState>) -> Api {
    let app = state.app.clone();
    with_store(&state, move |conn| {
        let row = notify::test_notification(&app, conn);
        Ok(json!({"ok": row.is_some(), "id": row.as_ref().and_then(|r| r.get("id").cloned()).unwrap_or(json!(0))}))
    })
    .await
}

/// Notifications by id. For `read`, no ids at all means every one.
#[derive(Deserialize, Default)]
struct Ids {
    ids: Option<Vec<i64>>,
}

async fn read(State(state): State<AppState>, Body(which): Body<Ids>) -> Api {
    with_store(&state, move |conn| Ok(json!({"ok": true, "read": bagholder_store::feeds::mark_notifications_read(conn, which.ids.as_deref(), &app::now_iso())?}))).await
}

async fn seen(State(state): State<AppState>, Body(which): Body<Ids>) -> Api {
    with_store(&state, move |conn| Ok(json!({"ok": true, "seen": bagholder_store::feeds::mark_notifications_seen(conn, &which.ids.unwrap_or_default(), &app::now_iso())?}))).await
}

async fn clear(State(state): State<AppState>) -> Api {
    with_store(&state, |conn| Ok(json!({"ok": true, "cleared": bagholder_store::feeds::clear_notifications(conn)?}))).await
}
