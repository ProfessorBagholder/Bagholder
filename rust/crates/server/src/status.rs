//! What the header shows: the connection, the sync under way, the update on
//! offer, the counts. It rides with the model on the page's stream, so a change
//! here reaches the page as the field that changed.

use serde::Serialize;
use std::sync::Arc;

use bagholder_diff_derive::Diff;
use ts_rs::TS;

use crate::app::{self, App};
use crate::notify::NotifyStatus;
use crate::{feeds, login, notify, orders, session, update, versions};

/// The header's own data: the connection, the sync under way, the update on
/// offer, the counts. In the order `payload` has always written it.
#[derive(Clone, Debug, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    #[ts(type = "true")]
    pub ok: bool,
    pub connected: bool,
    pub email: String,
    pub last_sync: String,
    pub activity_count: i64,
    pub account_count: i64,
    pub capturing: bool,
    pub syncing: bool,
    pub listings_filling: bool,
    pub sync_step: String,
    pub error: String,
    pub summary_ready: bool,
    pub protocol: String,
    pub started_at: String,
    pub version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub update_url: String,
    pub can_update: bool,
    pub update_by: String,
    /// `true` while `BAGHOLDER_LOGIN_VIEW` asks for the sign-in window shown
    /// as a page rather than streamed frames.
    pub login_view: bool,
    pub orders_live: bool,
    pub open_orders: i64,
    pub updating: String,
    pub update_error: String,
    pub notify: NotifyStatus,
    pub news_reading: Vec<String>,
}

/// `GET /api/status`: `Status` plus the two version strings the legacy
/// polling page reads, which the stream never sends (a page told what moved
/// has no use for them).
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StatusAnswer {
    #[serde(flatten)]
    #[ts(flatten)]
    pub status: Status,
    pub data_version: String,
    pub core_version: String,
}

pub fn status(app: &Arc<App>) -> Status {
    let conn = app.open().ok();
    let (acts, accounts, synced) = conn.as_ref().and_then(|c| versions::status_counts(c).ok()).unwrap_or((0, 0, String::new()));
    let upd = update::update_status(app);
    let sess = session::load_session(app);
    let notify_status = conn.as_ref().and_then(|c| notify::status(c).ok()).unwrap_or_default();
    let open_orders = orders::open_orders_count(app);
    let can_update = update::can_update(app, None);
    let off = update::updates_off();
    let st = app.state.lock().unwrap();
    let connected = st.connected && sess.as_ref().map(|x| !x.access_token.is_empty()).unwrap_or(false);
    let error = [st.error.as_str(), st.portfolio_error.as_str()].into_iter().filter(|e| !e.trim().is_empty()).collect::<Vec<_>>().join("; ");
    let email = if !st.email.is_empty() { st.email.clone() } else { sess.as_ref().map(|x| x.email.clone()).unwrap_or_default() };
    Status {
        ok: true,
        connected,
        email,
        last_sync: if st.last_sync.is_empty() { synced } else { st.last_sync.clone() },
        activity_count: acts,
        account_count: accounts,
        capturing: st.capturing,
        syncing: st.syncing,
        listings_filling: st.listings_filling,
        sync_step: st.sync_step.clone(),
        error,
        summary_ready: bagholder_market::enrich::summary_status() == "ready",
        protocol: app::PROTOCOL.to_string(),
        started_at: app.started_at.clone(),
        version: app::APP_VERSION.to_string(),
        latest_version: upd.latest.clone(),
        update_available: upd.update_available,
        update_url: if off { update::image_page() } else if upd.url.is_empty() { update::repo_url() } else { upd.url.clone() },
        can_update,
        update_by: if off { "image" } else { "app" }.to_string(),
        login_view: login::login_view(),
        orders_live: orders::orders_live(),
        open_orders,
        updating: st.updating.clone(),
        update_error: st.update_error.clone(),
        notify: notify_status,
        news_reading: feeds::news_reading(app),
    }
}

/// `GET /api/status`'s own answer: `status` plus the two version strings a
/// polling page reloads on.
pub fn answer(app: &Arc<App>) -> StatusAnswer {
    let conn = app.open().ok();
    let data_version = conn.as_ref().and_then(|c| versions::data_version(c).ok()).unwrap_or_default();
    let core_version = conn.as_ref().and_then(|c| versions::core_version(c).ok()).unwrap_or_default();
    let today = bagholder_model::clock::today_local();
    StatusAnswer {
        status: status(app),
        data_version: format!("{}|{}", data_version, today),
        // everything the model reads except the quotes: when this is unchanged but the
        // data version moved, only prices ticked (read by the legacy page, which polls)
        core_version: format!("{}|{}", core_version, today),
    }
}


