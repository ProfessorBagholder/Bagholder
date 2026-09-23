//! The notification list and its settings. (Its stream is in `stream`.)

use axum::extract::State;

use super::{api_routes, with_store, Api, AppState, Body, Routed};
use crate::{app, notify};

pub fn routes() -> Routed {
    api_routes! {
        get "/api/notifications" => list, answer: "NotificationsAnswer";
        post "/api/notifications/settings" => settings, body: "NotifySettingsPatch", answer: "NotifySettingsAnswer";
        post "/api/notifications/test" => test, answer: "NotifyTestAnswer";
        post "/api/notifications/read" => read, body: "NotificationIds", answer: "NotificationsReadAnswer";
        post "/api/notifications/seen" => seen, body: "NotificationIds", answer: "NotificationsSeenAnswer";
        post "/api/notifications/clear" => clear, answer: "NotificationsClearAnswer";
    }
}

async fn list(State(state): State<AppState>) -> Api<notify::NotificationsAnswer> {
    with_store(&state, |conn| {
        Ok(notify::NotificationsAnswer {
            ok: true,
            settings: notify::status(conn)?,
            kinds: notify::KINDS.iter().map(|k| k.to_string()).collect(),
            rows: bagholder_store::feeds::list_notifications(conn, 0, "", false, 50, true)?,
            unread: bagholder_store::feeds::unread_notifications(conn)?,
        })
    })
    .await
}

/// `POST /api/notifications/settings`: the switches that changed, by name.
async fn settings(State(state): State<AppState>, Body(patch): Body<notify::NotifySettingsPatch>) -> Api<notify::NotifySettingsAnswer> {
    with_store(&state, move |conn| {
        notify::set_settings(conn, &patch)?;
        Ok(notify::NotifySettingsAnswer { ok: true, settings: notify::status(conn)? })
    })
    .await
}

async fn test(State(state): State<AppState>) -> Api<notify::NotifyTestAnswer> {
    let app = state.app.clone();
    with_store(&state, move |conn| {
        let row = notify::test_notification(&app, conn);
        Ok(notify::NotifyTestAnswer { ok: row.is_some(), id: row.map(|r| r.id).unwrap_or(0) })
    })
    .await
}

async fn read(State(state): State<AppState>, Body(which): Body<notify::NotificationIds>) -> Api<notify::NotificationsReadAnswer> {
    with_store(&state, move |conn| Ok(notify::NotificationsReadAnswer { ok: true, read: bagholder_store::feeds::mark_notifications_read(conn, which.ids.as_deref(), &app::now_iso())? })).await
}

async fn seen(State(state): State<AppState>, Body(which): Body<notify::NotificationIds>) -> Api<notify::NotificationsSeenAnswer> {
    with_store(&state, move |conn| Ok(notify::NotificationsSeenAnswer { ok: true, seen: bagholder_store::feeds::mark_notifications_seen(conn, &which.ids.unwrap_or_default(), &app::now_iso())? })).await
}

async fn clear(State(state): State<AppState>) -> Api<notify::NotificationsClearAnswer> {
    with_store(&state, |conn| Ok(notify::NotificationsClearAnswer { ok: true, cleared: bagholder_store::feeds::clear_notifications(conn)? })).await
}
