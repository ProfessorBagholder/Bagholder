//! What the header shows: the connection, the sync under way, the update on
//! offer, the counts. It rides with the model on the page's stream, so a change
//! here reaches the page as the field that changed.

use serde_json::{json, Value};
use std::sync::Arc;

use crate::app::{self, f, s, truthy, App};
use crate::{feeds, login, notify, orders, session, update, versions};

pub fn payload(app: &Arc<App>) -> Value {
    let conn = app.open().ok();
    let (acts, accounts, synced) = conn.as_ref().and_then(|c| versions::status_counts(c).ok()).unwrap_or((0, 0, String::new()));
    let data_version = conn.as_ref().and_then(|c| versions::data_version(c).ok()).unwrap_or_default();
    let core_version = conn.as_ref().and_then(|c| versions::core_version(c).ok()).unwrap_or_default();
    let upd = update::update_status(app);
    let sess = session::load_session(app);
    let notify_status = conn.as_ref().and_then(|c| notify::status(c).ok()).unwrap_or(json!({}));
    let open_orders = orders::open_orders_count(app);
    let can_update = update::can_update(app, None);
    let off = update::updates_off();
    let st = app.state.lock().unwrap();
    let connected = st.connected && sess.as_ref().map(|x| truthy(x.get("access_token"))).unwrap_or(false);
    let email = if !st.email.is_empty() { st.email.clone() } else { sess.as_ref().map(|x| f(x, "email")).unwrap_or_default() };
    json!({
        "ok": true,
        "connected": connected,
        "email": email,
        "lastSync": if st.last_sync.is_empty() { synced } else { st.last_sync.clone() },
        "activityCount": acts,
        "accountCount": accounts,
        "capturing": st.capturing,
        "syncing": st.syncing,
        "listingsFilling": st.listings_filling,
        "syncStep": st.sync_step,
        "error": st.error,
        "dataVersion": format!("{}|{}", data_version, bagholder_model::clock::today_local()),
        // everything the model reads except the quotes: when this is unchanged but the
        // data version moved, only prices ticked (read by the legacy page, which polls)
        "coreVersion": format!("{}|{}", core_version, bagholder_model::clock::today_local()),
        "summaryReady": bagholder_market::enrich::summary_status() == "ready",
        "protocol": app::PROTOCOL,
        "startedAt": app.started_at,
        "version": app::APP_VERSION,
        "latestVersion": s(upd.get("latest")),
        "updateAvailable": truthy(upd.get("updateAvailable")),
        "updateUrl": if off { update::image_page() } else { let u = s(upd.get("url")); if u.is_empty() { update::repo_url() } else { u } },
        "canUpdate": can_update,
        "updateBy": if off { "image" } else { "app" },
        "loginView": login::login_view(),
        "ordersLive": orders::orders_live(),
        "openOrders": open_orders,
        "updating": st.updating,
        "updateError": st.update_error,
        "notify": notify_status,
        "newsReading": feeds::news_reading(app),
    })
}

