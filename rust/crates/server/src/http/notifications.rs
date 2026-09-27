//! The notification list and its settings. (Its stream is in `stream`.)

use axum::extract::State;

use super::{api_routes, with_notices, Api, AppState, Body, Routed};
use crate::{app, notify};

pub fn routes() -> Routed {
    api_routes! {
        get "/api/notifications" => list;
        post "/api/notifications/settings" => settings;
        post "/api/notifications/test" => test;
        post "/api/notifications/read" => read;
        post "/api/notifications/seen" => seen;
        post "/api/notifications/clear" => clear;
    }
}

async fn list(State(state): State<AppState>) -> Api<notify::NotificationsAnswer> {
    with_notices(&state, |book| {
        Ok(notify::NotificationsAnswer {
            ok: true,
            settings: notify::status(book)?,
            kinds: notify::KINDS.iter().map(|k| k.to_string()).collect(),
            rows: bagholder_store::feeds::list_notifications(book.notices(), 0, "", false, 50, true)?,
            unread: bagholder_store::feeds::unread_notifications(book.notices())?,
        })
    })
    .await
}

/// `POST /api/notifications/settings`: the switches that changed, by name.
async fn settings(State(state): State<AppState>, Body(patch): Body<notify::NotifySettingsPatch>) -> Api<notify::NotifySettingsAnswer> {
    with_notices(&state, move |book| {
        notify::set_settings(book, &patch)?;
        Ok(notify::NotifySettingsAnswer { ok: true, settings: notify::status(book)? })
    })
    .await
}

async fn test(State(state): State<AppState>) -> Api<notify::NotifyTestAnswer> {
    let app = state.app.clone();
    with_notices(&state, move |book| {
        let row = notify::test_notification(&app, book)?;
        Ok(notify::NotifyTestAnswer { ok: row.is_some(), id: row.map(|r| r.id).unwrap_or(0) })
    })
    .await
}

async fn read(State(state): State<AppState>, Body(which): Body<notify::NotificationIds>) -> Api<notify::NotificationsReadAnswer> {
    with_notices(&state, move |book| Ok(notify::NotificationsReadAnswer { ok: true, read: bagholder_store::feeds::mark_notifications_read(book.notices(), which.ids.as_deref(), &app::now_iso())? })).await
}

async fn seen(State(state): State<AppState>, Body(which): Body<notify::NotificationIds>) -> Api<notify::NotificationsSeenAnswer> {
    with_notices(&state, move |book| Ok(notify::NotificationsSeenAnswer { ok: true, seen: bagholder_store::feeds::mark_notifications_seen(book.notices(), &which.ids.unwrap_or_default(), &app::now_iso())? })).await
}

async fn clear(State(state): State<AppState>) -> Api<notify::NotificationsClearAnswer> {
    with_notices(&state, |book| Ok(notify::NotificationsClearAnswer { ok: true, cleared: bagholder_store::feeds::clear_notifications(book.notices())? })).await
}
