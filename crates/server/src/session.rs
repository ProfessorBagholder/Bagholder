//! The Wealthsimple session and the sync: the saved login, the refresh grant,
//! the daily pull of activities, balances, margin and equity history, the
//! listing names, and the Portfolio figures read between syncs.

use serde_json::{json, Value};
use std::time::Duration;

use bagholder_ws::fetch;
use bagholder_ws::session::{identity_from, CallError, Client};

use crate::app::{app, f, log, now_iso, now_unix, spawn, today_utc};

pub const TOKEN_CHECK: Duration = Duration::from_secs(30);
pub const PORTFOLIO_REFRESH_MINUTES: u64 = 5;
/// A sync that has failed this many times in a row is told, once.
pub const SYNC_FAILS_TOLD: i64 = 3;
pub const LOGIN_URL: &str = "https://my.wealthsimple.com/app/login";

pub fn load_session() -> Option<Value> {
    app().ws_home().load_session()
}

pub fn save_session(sess: &Value) {
    let _ = app().ws_home().save_session(sess);
}

fn set_error(msg: &str) {
    app().state.lock().unwrap().error = msg.to_string();
}

fn set_step(msg: &str) {
    app().state.lock().unwrap().sync_step = msg.to_string();
}

/// `bagholder.refresh_session`: the refresh grant, one at a time. A failure
/// says why on the header.
pub fn refresh_session(sess: &mut Value, adopt: bool) -> bool {
    let home = app().ws_home();
    let client = Client { home: &home };
    match client.refresh_session(sess, adopt) {
        Ok(()) => true,
        Err(msg) => {
            set_error(&msg);
            false
        }
    }
}

pub fn token_info(sess: &Value) -> Value {
    let home = app().ws_home();
    Client { home: &home }.token_info(sess)
}

/// `bagholder.apply_token_info_client_id`.
pub fn apply_token_info_client_id(sess: &mut Value, info: Option<&Value>) -> String {
    if f(sess, "access_token").is_empty() {
        return String::new();
    }
    let fetched;
    let info = match info {
        Some(i) => i,
        None => {
            fetched = token_info(sess);
            &fetched
        }
    };
    let cid = bagholder_ws::session::client_id_from_token_info(info);
    if cid.is_empty() {
        return String::new();
    }
    sess["client_id"] = json!(cid);
    app().ws_home().save_client_id(&cid);
    cid
}

/// `bagholder.scrape_client_id`: the production client id from Wealthsimple's
/// login script, cached.
pub fn scrape_client_id() -> String {
    let home = app().ws_home();
    let cached = home.cached_client_id();
    if !cached.is_empty() {
        return cached;
    }
    let ua = home.cached_user_agent();
    let hdrs: Vec<(&str, &str)> = if ua.is_empty() { vec![] } else { vec![("User-Agent", ua.as_str())] };
    let get = |url: &str| bagholder_market::client::request("GET", url, &hdrs, None, Duration::from_secs(20)).ok().map(|r| r.text());
    let html = match get(LOGIN_URL) { Some(h) => h, None => return String::new() };
    let script = regex::Regex::new(r#"(?i)<script[^>]+src="([^"]*app-[a-f0-9]+\.js[^"]*)""#).unwrap();
    let mut js_url = match script.captures(&html) { Some(c) => c[1].to_string(), None => return String::new() };
    if js_url.starts_with("//") {
        js_url = format!("https:{}", js_url);
    } else if js_url.starts_with('/') {
        js_url = format!("https://my.wealthsimple.com{}", js_url);
    }
    let js = match get(&js_url) { Some(j) => j, None => return String::new() };
    match regex::Regex::new(r#"(?s)production:.*?clientId:"([a-f0-9]+)""#).unwrap().captures(&js) {
        Some(c) => {
            home.save_client_id(&c[1]);
            c[1].to_string()
        }
        None => String::new(),
    }
}

/// `bagholder.ensure_fresh_token`: refresh ahead of the expiry. Connected means
/// this grant produced a new token.
pub fn ensure_fresh_token(sess: Option<Value>) -> bool {
    let mut sess = match sess.or_else(load_session) {
        Some(s) if !f(&s, "refresh_token").is_empty() => s,
        _ => {
            let mut st = app().state.lock().unwrap();
            st.connected = false;
            st.error = "missing refresh token".into();
            return false;
        }
    };
    let connected = app().state.lock().unwrap().connected;
    if connected && !bagholder_ws::sync::token_refresh_needed(&sess, now_unix()) {
        return true;
    }
    let ok = refresh_session(&mut sess, true);
    let mut st = app().state.lock().unwrap();
    st.connected = ok;
    if ok {
        st.error.clear();
    } else if st.error.trim().is_empty() {
        st.error = "Wealthsimple token refresh failed".into();
    }
    ok
}

/// `bagholder.delete_session_and_book`: the login only; stored rows stay.
pub fn delete_session() {
    app().ws_home().delete_session();
    let mut st = app().state.lock().unwrap();
    st.connected = false;
    st.email.clear();
    st.last_sync.clear();
    st.capturing = false;
    st.error.clear();
}

/// `bagholder.boot_session`.
pub fn boot_session() {
    let conn = match app().open() { Ok(c) => c, Err(_) => return };
    let mut sess = match load_session() {
        Some(s) => s,
        None => {
            let mut st = app().state.lock().unwrap();
            st.connected = false;
            st.last_sync = bagholder_store::tables::get_meta(&conn, "synced_at", "").unwrap_or_default();
            return;
        }
    };
    let mut info_ok = false;
    if !f(&sess, "access_token").is_empty() {
        let info = token_info(&sess);
        info_ok = info.as_object().map(|m| !m.is_empty()).unwrap_or(false) && !crate::app::truthy(info.get("error")) && !crate::app::truthy(info.get("_http_status"));
        if info_ok {
            apply_token_info_client_id(&mut sess, Some(&info));
            if !f(&info, "identity_canonical_id").is_empty() && f(&sess, "identity_canonical_id").is_empty() {
                sess["identity_canonical_id"] = info["identity_canonical_id"].clone();
            }
            if !f(&info, "email").is_empty() {
                sess["email"] = info["email"].clone();
            }
            save_session(&sess);
        }
    }
    let mut ok = info_ok;
    if !ok && !f(&sess, "refresh_token").is_empty() {
        ok = refresh_session(&mut sess, true);
        sess = load_session().unwrap_or(sess);
    }
    let mut st = app().state.lock().unwrap();
    st.connected = ok;
    if ok {
        st.error.clear();
    } else if st.error.trim().is_empty() {
        st.error = if f(&sess, "refresh_token").is_empty() { "missing refresh token".into() } else { "Wealthsimple token refresh failed".into() };
    }
    st.email = f(&sess, "email");
    st.last_sync = bagholder_store::tables::get_meta(&conn, "synced_at", "").unwrap_or_default();
}

/// `bagholder.note_session_expired`: told once per expiry.
pub fn note_session_expired() {
    let was = {
        let mut st = app().state.lock().unwrap();
        let was = st.connected;
        st.connected = false;
        st.error = "Session expired. Connect again.".into();
        was
    };
    if was {
        if let Ok(conn) = app().open() {
            crate::notify::emit(&conn, "connection", &format!("session:{}", now_iso()), "Sign in needed", "The Wealthsimple session expired. Connect again from the menu.", None);
        }
    }
}

/// `bagholder.note_sync_failed`: the third failure in a row is told.
pub fn note_sync_failed(reason: &str) {
    let (fails, first) = {
        let mut st = app().state.lock().unwrap();
        st.sync_fails += 1;
        if st.sync_fails == 1 {
            st.sync_first_fail = now_iso();
        }
        (st.sync_fails, st.sync_first_fail.clone())
    };
    if fails == SYNC_FAILS_TOLD {
        if let Ok(conn) = app().open() {
            crate::notify::emit(&conn, "connection", &format!("sync:{}", first), "Sync failing", if reason.is_empty() { "Sync failed." } else { reason }, None);
        }
    }
}

/// Per-filter-nickname daily equity: (points, public errors).
fn fetch_nickname_nav_history(client: &Client, sess: &Value, accounts: &[Value], conn: &rusqlite::Connection) -> (Vec<Value>, Vec<String>) {
    let mut points = Vec::new();
    let mut errors = Vec::new();
    let last_by = bagholder_store::tables::nav_last_dates(conn).unwrap_or_default();
    let today = today_utc();
    let groups = bagholder_ws::mapping::nav_account_groups(Some(&Value::Array(accounts.to_vec())));
    let mut names: Vec<&String> = groups.keys().collect();
    names.sort();
    for nick in names {
        set_step(&format!("Fetching equity history for {}…", nick));
        let since = last_by.get(nick).and_then(|v| v.as_str()).map(|s| s.to_string());
        let ids: Vec<String> = groups[nick].as_array().cloned().unwrap_or_default().iter().map(|v| crate::app::s(Some(v))).collect();
        let mut series = Vec::new();
        let mut failed: Option<String> = None;
        for aid in ids {
            match fetch::fetch_account_nav_history(client, sess, &aid, since.as_deref(), &today) {
                Ok(s) => series.push(s),
                Err(e) => {
                    failed = Some(bagholder_ws::sync::public_sync_error(&e.to_string()));
                    break;
                }
            }
        }
        if let Some(public) = failed {
            errors.push(format!("{}: {}", nick, public));
            log(&format!("NAV history failed for {}: {}", nick, public));
            continue;
        }
        for rec in fetch::merge_nav_points(&series) {
            let mut m = crate::app::obj(rec);
            m.insert("accountId".into(), json!(nick));
            points.push(Value::Object(m));
        }
    }
    (points, errors)
}

/// `bagholder.run_sync`: the pull. Inserts new Wealthsimple rows only and never
/// rebuilds the table.
pub fn run_sync(allow_refresh: bool, force_activity: bool) -> bool {
    {
        let mut st = app().state.lock().unwrap();
        if st.syncing {
            return false;
        }
        st.syncing = true;
        st.error.clear();
        st.sync_step = "Checking session…".into();
    }
    let result = sync_body(force_activity);
    let out = match result {
        Ok(v) => v,
        Err(CallError::NotAuthorized) => {
            let refreshed = allow_refresh && {
                let mut s = load_session().unwrap_or(json!({}));
                refresh_session(&mut s, true)
            };
            if refreshed {
                app().state.lock().unwrap().syncing = false;
                return run_sync(false, force_activity);
            }
            note_session_expired();
            false
        }
        Err(e) => {
            let public = bagholder_ws::sync::public_sync_error(&e.to_string());
            let line = format!("Sync failed: {}", public);
            log(&line);
            set_error(&line);
            note_sync_failed(&public);
            false
        }
    };
    let mut st = app().state.lock().unwrap();
    st.syncing = false;
    st.sync_step.clear();
    out
}

fn sync_body(force_activity: bool) -> Result<bool, CallError> {
    let failed = |e: rusqlite::Error| CallError::Failed(e.to_string());
    let conn = app().open().map_err(failed)?;
    let mut sess = match load_session() {
        Some(s) if !f(&s, "access_token").is_empty() || !f(&s, "refresh_token").is_empty() => s,
        _ => {
            app().state.lock().unwrap().connected = false;
            return Ok(false);
        }
    };
    if f(&sess, "access_token").is_empty() {
        app().state.lock().unwrap().connected = false;
        return Ok(false);
    }
    let mut info = json!({});
    let mut identity = identity_from(&sess);
    if identity.is_empty() {
        info = token_info(&sess);
        identity = identity_from(&info);
    }
    if identity.is_empty() {
        return Err(CallError::Failed("no identity_canonical_id".into()));
    }
    sess["identity_canonical_id"] = json!(identity);
    let email = { let e = f(&info, "email"); if !e.is_empty() { e } else { let u = f(&info, "username"); if !u.is_empty() { u } else { f(&sess, "email") } } };
    if !email.is_empty() {
        sess["email"] = json!(email);
    }
    save_session(&sess);

    if !force_activity && !bagholder_store::admin::activity_pull_due(&conn, now_unix() as i64).map_err(failed)? {
        let synced = bagholder_store::tables::get_meta(&conn, "synced_at", "").unwrap_or_default();
        let mut st = app().state.lock().unwrap();
        st.connected = true;
        st.email = email;
        if !synced.is_empty() {
            st.last_sync = synced;
        }
        st.capturing = false;
        st.error.clear();
        st.sync_step.clear();
        return Ok(true);
    }

    let home = app().ws_home();
    let client = Client { home: &home };
    set_step("Fetching accounts…");
    let accounts = fetch::fetch_all_accounts(&client, &sess, &identity)?;
    let mut acc_by_id = serde_json::Map::new();
    for a in &accounts {
        let id = f(a, "id");
        if !id.is_empty() {
            acc_by_id.insert(id, a.clone());
        }
    }
    let acc_by_id = Value::Object(acc_by_id);
    let (start_date, _full) = bagholder_ws::sync::activity_sync_bounds(&conn).map_err(failed)?;
    let mut mapped: Vec<Value> = Vec::new();
    set_step("Syncing transactions");
    let now_i = now_unix() as i64;
    for acc in accounts.iter().filter(|a| !f(a, "id").is_empty()) {
        let items = fetch::fetch_activities_for_account(&client, &sess, &f(acc, "id"), start_date.as_deref(), now_i)?;
        for it in &items {
            mapped.extend(bagholder_ws::mapping::map_activity_rows(it, Some(&acc_by_id)));
        }
    }
    let pools = bagholder_ws::mapping::fifo_pool_ids(Some(&Value::Array(accounts.clone())));
    for row in mapped.iter_mut() {
        let aid = f(row, "accountId");
        let pool = pools.get(&aid).cloned().unwrap_or(aid);
        row["fifoId"] = json!(pool);
    }
    set_step("Fetching balances…");
    let ids: Vec<String> = acc_by_id.as_object().unwrap().keys().cloned().collect();
    let balances = fetch::fetch_balances(&client, &sess, &ids)?;
    let margin = fetch::fetch_margin(&client, &sess, &fetch::margin_account_ids(&accounts), &now_iso());
    set_step("Fetching equity history…");
    let last_by = bagholder_store::tables::nav_last_dates(&conn).map_err(failed)?;
    let since_all = last_by.get("").and_then(|v| v.as_str()).map(|s| s.to_string());
    let nav_history = fetch::fetch_nav_history(&client, &sess, &identity, since_all.as_deref(), &today_utc()).unwrap_or_default();
    let mut combined: Vec<Value> = nav_history
        .into_iter()
        .map(|r| {
            let mut m = crate::app::obj(r);
            m.insert("accountId".into(), json!(""));
            Value::Object(m)
        })
        .collect();
    let (nick_pts, nav_errors) = fetch_nickname_nav_history(&client, &sess, &accounts, &conn);
    combined.extend(nick_pts);
    bagholder_store::merge::apply_wealthsimple_mapped(&conn, &mapped, &crate::app::uuid4).map_err(failed)?;
    let synced = now_iso();
    set_step("Saving…");
    bagholder_store::tables::replace_accounts(&conn, &bagholder_ws::sync::slim_accounts(&accounts)).map_err(failed)?;
    bagholder_store::tables::replace_balances(&conn, &balances).map_err(failed)?;
    bagholder_store::tables::replace_margin(&conn, &margin, &synced).map_err(failed)?;
    bagholder_store::tables::upsert_nav(&conn, &combined).map_err(failed)?;
    bagholder_store::tables::set_meta(&conn, "synced_at", &synced).map_err(failed)?;
    bagholder_store::admin::mark_activity_pulled(&conn, &synced).map_err(failed)?;
    drop(conn);
    fill_listings(&sess, true);
    let nav_line = if nav_errors.is_empty() { String::new() } else { format!("NAV history failed for {}", nav_errors.join("; ")) };
    let mut st = app().state.lock().unwrap();
    st.connected = true;
    st.sync_fails = 0;
    st.email = email;
    st.last_sync = synced;
    st.capturing = false;
    st.error = nav_line;
    st.sync_step.clear();
    Ok(true)
}

/// `bagholder.fill_listings`: stamp missing activity security ids and cache
/// the listings the book names.
pub fn fill_listings(sess: &Value, from_sync: bool) -> bool {
    if f(sess, "access_token").is_empty() {
        return false;
    }
    {
        let mut st = app().state.lock().unwrap();
        if st.listings_filling || (st.syncing && !from_sync) {
            return false;
        }
        st.listings_filling = true;
        st.sync_step = "Attaching listing ids…".into();
    }
    let ok = (|| -> Result<(), String> {
        let conn = app().open().map_err(|e| e.to_string())?;
        let home = app().ws_home();
        let client = Client { home: &home };
        if bagholder_store::admin::needs_security_id_backfill(&conn).map_err(|e| e.to_string())? {
            let mut walk_ok = true;
            let mut mapped = Vec::new();
            set_step("Attaching listing ids…");
            let snap = bagholder_store::snapshot::snapshot(&conn, true).map_err(|e| e.to_string())?;
            let mut ids: Vec<String> = Vec::new();
            for a in snap.get("accounts").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
                let id = f(&a, "id").trim().to_string();
                if !id.is_empty() && !ids.contains(&id) {
                    ids.push(id);
                }
            }
            if ids.is_empty() {
                for a in snap.get("activities").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
                    let id = f(&a, "accountId").trim().to_string();
                    if !id.is_empty() && !ids.contains(&id) {
                        ids.push(id);
                    }
                }
            }
            for aid in ids {
                let raw = match fetch::fetch_activities_for_account(&client, sess, &aid, None, now_unix() as i64) {
                    Ok(r) => r,
                    Err(_) => {
                        walk_ok = false;
                        continue;
                    }
                };
                let accts = bagholder_store::tables::accounts(&conn).map_err(|e| e.to_string())?;
                let mut by = serde_json::Map::new();
                for a in accts {
                    let id = f(&a, "id");
                    if !id.is_empty() {
                        by.insert(id, a);
                    }
                }
                let by = Value::Object(by);
                for it in &raw {
                    mapped.extend(bagholder_ws::mapping::map_activity_rows(it, Some(&by)));
                }
            }
            if !mapped.is_empty() {
                bagholder_store::merge::apply_wealthsimple_mapped(&conn, &mapped, &crate::app::uuid4).map_err(|e| e.to_string())?;
            }
            if walk_ok {
                bagholder_store::tables::set_meta(&conn, "security_id_backfill_done", "1").map_err(|e| e.to_string())?;
            }
        }
        let snap = bagholder_store::snapshot::snapshot(&conn, true).map_err(|e| e.to_string())?;
        let mut wanted: Vec<String> = Vec::new();
        for key in ["activities", "balances"] {
            for r in snap.get(key).and_then(|v| v.as_array()).cloned().unwrap_or_default() {
                let sid = f(&r, "securityId").trim().to_string();
                if !sid.is_empty() && !wanted.contains(&sid) {
                    wanted.push(sid);
                }
            }
        }
        let mut pending = bagholder_store::admin::missing_security_ids(&conn, &wanted).map_err(|e| e.to_string())?;
        let mut seen: Vec<String> = Vec::new();
        let mut to_upsert: Vec<Value> = Vec::new();
        // options point at an underlying security, fetched in a second round
        while !pending.is_empty() {
            set_step(&format!("Looking up company names, {} left", pending.len()));
            let batch: Vec<String> = pending.iter().filter(|s| !seen.contains(s)).cloned().collect();
            seen.extend(batch.iter().cloned());
            pending.clear();
            if batch.is_empty() {
                break;
            }
            let recs = fetch::fetch_securities(&client, sess, &batch);
            let under: Vec<String> = recs.iter().map(|r| f(r, "underlyingId").trim().to_string()).filter(|u| !u.is_empty() && !seen.contains(u)).collect();
            to_upsert.extend(recs);
            if !under.is_empty() {
                pending = bagholder_store::admin::missing_security_ids(&conn, &under).map_err(|e| e.to_string())?;
            }
        }
        if !to_upsert.is_empty() {
            bagholder_store::admin::upsert_securities(&conn, &to_upsert, &now_iso()).map_err(|e| e.to_string())?;
        }
        Ok(())
    })()
    .is_ok();
    let mut st = app().state.lock().unwrap();
    st.listings_filling = false;
    st.sync_step.clear();
    ok
}

/// `bagholder.refresh_nav_only`.
pub fn refresh_nav_only(allow_refresh: bool) -> Value {
    let sess = match load_session() { Some(s) if !f(&s, "access_token").is_empty() => s, _ => return json!({"ok": false, "error": "not connected"}) };
    let mut identity = identity_from(&sess);
    if identity.is_empty() {
        identity = identity_from(&token_info(&sess));
    }
    if identity.is_empty() {
        return json!({"ok": false, "error": "no identity"});
    }
    let conn = match app().open() { Ok(c) => c, Err(e) => return json!({"ok": false, "error": e.to_string()}) };
    let accounts = bagholder_store::tables::accounts(&conn).unwrap_or_default();
    if accounts.is_empty() {
        return json!({"ok": false, "error": "no accounts stored"});
    }
    let home = app().ws_home();
    let client = Client { home: &home };
    let last_by = bagholder_store::tables::nav_last_dates(&conn).unwrap_or_default();
    let since = last_by.get("").and_then(|v| v.as_str()).map(|s| s.to_string());
    let history = match fetch::fetch_nav_history(&client, &sess, &identity, since.as_deref(), &today_utc()) {
        Ok(h) => h,
        Err(CallError::NotAuthorized) => {
            let mut s = load_session().unwrap_or(json!({}));
            if allow_refresh && refresh_session(&mut s, true) {
                return refresh_nav_only(false);
            }
            return json!({"ok": false, "error": "Session expired. Connect again."});
        }
        Err(_) => vec![],
    };
    let mut combined: Vec<Value> = history.into_iter().map(|r| { let mut m = crate::app::obj(r); m.insert("accountId".into(), json!("")); Value::Object(m) }).collect();
    let (pts, errors) = fetch_nickname_nav_history(&client, &sess, &accounts, &conn);
    combined.extend(pts);
    let _ = bagholder_store::tables::upsert_nav(&conn, &combined);
    let mut nicks: Vec<String> = combined.iter().map(|p| f(p, "accountId")).filter(|a| !a.is_empty()).collect();
    nicks.sort();
    nicks.dedup();
    json!({"ok": true, "allDays": combined.iter().filter(|p| f(p, "accountId").is_empty()).count(), "accounts": nicks.len(), "errors": errors})
}

/// `bagholder.refresh_portfolio`: net liquidation values, balances and buying
/// power read again between syncs.
pub fn refresh_portfolio() -> Value {
    {
        let st = app().state.lock().unwrap();
        if st.syncing {
            return json!({"ok": false, "skipped": "sync running"});
        }
        if !st.connected {
            return json!({"ok": false, "skipped": "not connected"});
        }
    }
    let sess = load_session();
    let identity = sess.as_ref().map(identity_from).unwrap_or_default();
    let sess = match sess { Some(s) if !f(&s, "access_token").is_empty() && !identity.is_empty() => s, _ => {
        log("bagholder portfolio: no session to read with");
        return json!({"ok": false, "skipped": "no session"});
    } };
    let home = app().ws_home();
    let client = Client { home: &home };
    let run = || -> Result<Value, String> {
        let conn = app().open().map_err(|e| e.to_string())?;
        let accounts = fetch::fetch_all_accounts(&client, &sess, &identity).map_err(|e| e.to_string())?;
        let ids: Vec<String> = accounts.iter().map(|a| f(a, "id")).filter(|i| !i.is_empty()).collect();
        if ids.is_empty() {
            log("bagholder portfolio: Wealthsimple returned no accounts");
            return Ok(json!({"ok": false, "skipped": "no accounts"}));
        }
        let balances = fetch::fetch_balances(&client, &sess, &ids).map_err(|e| e.to_string())?;
        let now = now_iso();
        let margin = fetch::fetch_margin(&client, &sess, &fetch::margin_account_ids(&accounts), &now);
        bagholder_store::tables::replace_accounts(&conn, &bagholder_ws::sync::slim_accounts(&accounts)).map_err(|e| e.to_string())?;
        bagholder_store::tables::replace_balances(&conn, &balances).map_err(|e| e.to_string())?;
        bagholder_store::tables::replace_margin(&conn, &margin, &now).map_err(|e| e.to_string())?;
        bagholder_store::tables::set_meta(&conn, "balances_read_at", &now).map_err(|e| e.to_string())?;
        app().invalidate();
        let available = margin.iter().filter(|m| !m.get("buyingPower").map(|v| v.is_null()).unwrap_or(true)).count();
        log(&format!("bagholder portfolio: {} accounts, {} balances, buying power for {} of {} margin accounts", ids.len(), balances.len(), available, margin.len()));
        Ok(json!({"ok": true, "accounts": ids.len(), "balances": balances.len(), "margin": margin.len()}))
    };
    match run() {
        Ok(v) => v,
        Err(e) => {
            log(&format!("bagholder portfolio: failed: {}", e));
            json!({"ok": false, "skipped": "error"})
        }
    }
}

pub fn portfolio_loop() {
    refresh_portfolio();
    while !app().wait(Duration::from_secs(60 * PORTFOLIO_REFRESH_MINUTES)) {
        refresh_portfolio();
    }
}

/// `bagholder.refresh_now`: the grant, always.
pub fn refresh_now() -> Value {
    let mut sess = match load_session() {
        Some(s) if !f(&s, "refresh_token").is_empty() => s,
        _ => {
            let mut st = app().state.lock().unwrap();
            st.connected = false;
            st.error = "not connected".into();
            return json!({"ok": false, "error": "not connected", "connected": false});
        }
    };
    let ok = refresh_session(&mut sess, true);
    let mut st = app().state.lock().unwrap();
    st.connected = ok;
    if ok {
        st.error.clear();
    }
    json!({"ok": ok, "error": st.error.trim(), "connected": ok})
}

/// `bagholder.auto_sync_loop`: the token kept fresh, and the weekday pull when
/// it is due, backing off on failure.
pub fn auto_sync_loop() {
    let mut delay = TOKEN_CHECK;
    let mut fail_delay = TOKEN_CHECK;
    while !app().wait(delay) {
        if let Some(s) = load_session() {
            if !f(&s, "refresh_token").is_empty() {
                ensure_fresh_token(Some(s));
            }
        }
        let (connected, syncing) = { let st = app().state.lock().unwrap(); (st.connected, st.syncing) };
        let due = app().open().ok().and_then(|c| bagholder_store::admin::activity_pull_due(&c, now_unix() as i64).ok()).unwrap_or(false);
        if connected && !syncing && due {
            let ok = run_sync(true, true);
            crate::feeds::refresh_market_data();
            fail_delay = if ok { TOKEN_CHECK } else { (fail_delay.max(TOKEN_CHECK) * 2).min(Duration::from_secs(1800)) };
            delay = fail_delay;
        } else {
            delay = TOKEN_CHECK;
            fail_delay = TOKEN_CHECK;
        }
    }
}

/// `bagholder.capture_tokens`: keep the captured login and take it over.
pub fn capture_tokens(body: &Value) -> Value {
    if !body.is_object() {
        return json!({"ok": false, "error": "bad body"});
    }
    if f(body, "access_token").is_empty() {
        return json!({"ok": false, "error": "missing access_token"});
    }
    let mut sess = load_session().unwrap_or(json!({}));
    for k in ["access_token", "refresh_token", "identity_canonical_id", "expires_at", "wssdi", "client_id", "session_id", "user_agent"] {
        if crate::app::truthy(body.get(k)) {
            sess[k] = body[k].clone();
        }
    }
    let mut ident = identity_from(body);
    if ident.is_empty() {
        ident = identity_from(&sess);
    }
    let info = if f(&sess, "access_token").is_empty() { json!({}) } else { token_info(&sess) };
    if ident.is_empty() {
        ident = identity_from(&info);
    }
    if !ident.is_empty() {
        sess["identity_canonical_id"] = json!(ident);
    }
    if f(&sess, "session_id").is_empty() {
        sess["session_id"] = json!(crate::app::uuid4());
    }
    apply_token_info_client_id(&mut sess, Some(&info));
    if f(&sess, "client_id").is_empty() {
        let cid = scrape_client_id();
        if !cid.is_empty() {
            sess["client_id"] = json!(cid);
        }
    }
    if f(&sess, "user_agent").is_empty() {
        let ua = app().ws_home().cached_user_agent();
        if !ua.is_empty() {
            sess["user_agent"] = json!(ua);
        }
    }
    // take the login over: rotate its refresh token now, so the copy the
    // browser holds goes stale instead of ours
    if !refresh_session(&mut sess, false) {
        let mut st = app().state.lock().unwrap();
        st.connected = false;
        let err = if st.error.is_empty() { "Wealthsimple refused the captured login".to_string() } else { st.error.clone() };
        return json!({"ok": false, "error": err});
    }
    save_session(&sess);
    {
        let mut st = app().state.lock().unwrap();
        st.connected = true;
        st.capturing = false;
        st.error.clear();
    }
    spawn("bagholder-sync", || {
        run_sync(true, true);
    });
    json!({"ok": true})
}
