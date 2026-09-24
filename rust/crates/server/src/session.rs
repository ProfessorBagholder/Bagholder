//! The Wealthsimple session and the sync: the saved login, the refresh grant,
//! the daily pull of activities, balances, margin and equity history, the
//! listing names, and the Portfolio figures read between syncs.

use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use ts_rs::TS;

use bagholder_ws::fetch;
use bagholder_ws::session::{CallError, Client, Session};

use crate::app::{log, now_iso, now_unix, spawn, today_utc, App};
use crate::http::OkOr;

pub const PORTFOLIO_REFRESH_MINUTES: u64 = 5;
/// A sync that has failed this many times in a row is told, once.
pub const SYNC_FAILS_TOLD: i64 = 3;
pub const LOGIN_URL: &str = "https://my.wealthsimple.com/app/login";

pub fn load_session(app: &Arc<App>) -> Option<Session> {
    app.ws_home().load_session()
}

/// Writes the login; a failure is said in words the header can show.
pub fn save_session(app: &Arc<App>, sess: &Session) -> Result<(), String> {
    app.ws_home().save_session(sess).map_err(|e| format!("Could not save the Wealthsimple login: {}", e))
}

/// The sync's problems, one line: each part that did not answer, named.
fn problems_line(problems: &[String]) -> String {
    problems.join("; ")
}

/// A Wealthsimple row with no usable canonical id: said, not silently dropped.
fn unidentified_line(n: usize) -> String {
    let (noun, was) = if n == 1 { ("row", "was") } else { ("rows", "were") };
    format!("{} Wealthsimple {} had no id and {} not stored", n, noun, was)
}

fn set_error(app: &Arc<App>, msg: &str) {
    app.state.lock().unwrap().error = msg.to_string();
}

fn set_step(app: &Arc<App>, msg: &str) {
    app.state.lock().unwrap().sync_step = msg.to_string();
}

/// The refresh grant, one at a time. A failure
/// says why on the header.
pub fn refresh_session(app: &Arc<App>, sess: &mut Session, adopt: bool) -> bool {
    let home = app.ws_home();
    let client = Client { home: &home };
    match client.refresh_session(sess, adopt) {
        Ok(()) => true,
        Err(msg) => {
            set_error(app, &msg);
            false
        }
    }
}

pub fn token_info(app: &Arc<App>, sess: &Session) -> bagholder_ws::session::TokenInfo {
    let home = app.ws_home();
    Client { home: &home }.token_info(sess)
}

pub fn apply_token_info_client_id(app: &Arc<App>, sess: &mut Session, info: Option<&bagholder_ws::session::TokenInfo>) -> String {
    if sess.access_token.is_empty() {
        return String::new();
    }
    let fetched;
    let info = match info {
        Some(i) => i,
        None => {
            fetched = token_info(app, sess);
            &fetched
        }
    };
    let cid = bagholder_ws::session::client_id_from_token_info(info);
    if cid.is_empty() {
        return String::new();
    }
    sess.client_id = cid.clone();
    app.ws_home().save_client_id(&cid);
    cid
}

/// The production client id from Wealthsimple's
/// login script, cached.
pub fn scrape_client_id(app: &Arc<App>) -> String {
    let home = app.ws_home();
    let cached = home.cached_client_id();
    if !cached.is_empty() {
        return cached;
    }
    let ua = home.cached_user_agent();
    let hdrs: Vec<(&str, &str)> = if ua.is_empty() { vec![] } else { vec![("User-Agent", ua.as_str())] };
    let get = |url: &str| bagholder_net::client::request("GET", url, &hdrs, None, Duration::from_secs(20)).ok().map(|r| r.text());
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

/// Refresh ahead of the expiry. Connected means
/// this grant produced a new token.
pub fn ensure_fresh_token(app: &Arc<App>, sess: Option<Session>) -> bool {
    let mut sess = match sess.or_else(|| load_session(app)) {
        Some(s) if !s.refresh_token.is_empty() => s,
        _ => {
            let mut st = app.state.lock().unwrap();
            st.connected = false;
            st.error = "missing refresh token".into();
            return false;
        }
    };
    let connected = app.state.lock().unwrap().connected;
    if connected && !bagholder_ws::sync::token_refresh_needed(&sess, now_unix()) {
        return true;
    }
    let ok = refresh_session(app, &mut sess, true);
    let mut st = app.state.lock().unwrap();
    st.connected = ok;
    if ok {
        st.error.clear();
    } else if st.error.trim().is_empty() {
        st.error = "Wealthsimple token refresh failed".into();
    }
    ok
}

/// The login only; stored rows stay.
pub fn delete_session(app: &Arc<App>) {
    app.ws_home().delete_session();
    let mut st = app.state.lock().unwrap();
    st.connected = false;
    st.email.clear();
    st.last_sync.clear();
    st.capturing = false;
    st.error.clear();
    st.portfolio_error.clear();
}

pub fn boot_session(app: &Arc<App>) {
    let conn = match app.open() {
        Ok(c) => c,
        Err(e) => {
            set_error(app, &format!("Could not open the database: {}", e));
            return;
        }
    };
    let mut sess = match load_session(app) {
        Some(s) => s,
        None => {
            let mut st = app.state.lock().unwrap();
            st.connected = false;
            st.last_sync = bagholder_store::tables::get_meta(&conn, "synced_at", "").unwrap_or_default();
            return;
        }
    };
    let mut info_ok = false;
    let mut saved = Ok(());
    if !sess.access_token.is_empty() {
        let info = token_info(app, &sess);
        info_ok = info.is_ok();
        if info_ok {
            apply_token_info_client_id(app, &mut sess, Some(&info));
            let identity = info.identity();
            if !identity.is_empty() && sess.identity().is_empty() {
                sess.ids.identity_canonical_id = identity;
            }
            if !info.email.is_empty() {
                sess.email = info.email.clone();
            }
            saved = save_session(app, &sess);
        }
    }
    let mut ok = info_ok;
    if !ok && !sess.refresh_token.is_empty() {
        ok = refresh_session(app, &mut sess, true);
        sess = load_session(app).unwrap_or(sess);
    }
    let mut st = app.state.lock().unwrap();
    st.connected = ok;
    if ok {
        st.error.clear();
    } else if st.error.trim().is_empty() {
        st.error = if sess.refresh_token.is_empty() { "missing refresh token".into() } else { "Wealthsimple token refresh failed".into() };
    }
    if let Err(e) = saved {
        st.error = e;
    }
    st.email = sess.email.clone();
    match bagholder_store::tables::get_meta(&conn, "synced_at", "") {
        Ok(v) => st.last_sync = v,
        Err(e) => st.error = format!("Could not read the database: {}", e),
    }
}

/// Told once per expiry.
pub fn note_session_expired(app: &Arc<App>) {
    let was = {
        let mut st = app.state.lock().unwrap();
        let was = st.connected;
        st.connected = false;
        st.error = "Session expired. Connect again.".into();
        was
    };
    if was {
        if let Ok(conn) = app.open() {
            crate::notify::emit(app, &conn, "connection", &format!("session:{}", now_iso()), "Sign in needed", "The Wealthsimple session expired. Connect again from the menu.", None);
        }
    }
}

/// The third failure in a row is told.
pub fn note_sync_failed(app: &Arc<App>, reason: &str) {
    let (fails, first) = {
        let mut st = app.state.lock().unwrap();
        st.sync_fails += 1;
        if st.sync_fails == 1 {
            st.sync_first_fail = now_iso();
        }
        (st.sync_fails, st.sync_first_fail.clone())
    };
    if fails == SYNC_FAILS_TOLD {
        if let Ok(conn) = app.open() {
            crate::notify::emit(app, &conn, "connection", &format!("sync:{}", first), "Sync failing", if reason.is_empty() { "Sync failed." } else { reason }, None);
        }
    }
}

/// Per-filter-nickname daily equity: (points, public errors).
fn fetch_nickname_nav_history(app: &Arc<App>, client: &Client, sess: &Session, accounts: &[bagholder_ws::wire::AccountNode], last_by: &std::collections::BTreeMap<String, String>) -> (Vec<bagholder_store::broker::NavPoint>, Vec<String>) {
    let mut points = Vec::new();
    let mut errors = Vec::new();
    let today = today_utc();
    let groups = bagholder_ws::mapping::nav_account_groups(accounts);
    let mut names: Vec<&String> = groups.keys().collect();
    names.sort();
    for nick in names {
        set_step(app, &format!("Fetching equity history for {}…", nick));
        let since = last_by.get(nick).cloned();
        let ids = &groups[nick];
        let mut series = Vec::new();
        let mut failed: Option<String> = None;
        for aid in ids {
            match fetch::fetch_account_nav_history(client, sess, aid, since.as_deref(), &today) {
                Ok(s) => series.push(s),
                Err(e) => {
                    failed = Some(bagholder_ws::sync::public_sync_error(&e.to_string()));
                    break;
                }
            }
        }
        if let Some(public) = failed {
            errors.push(format!("Equity history for {} failed: {}", nick, public));
            continue;
        }
        for mut rec in fetch::merge_nav_points(&series) {
            rec.account_id = nick.clone();
            points.push(rec);
        }
    }
    (points, errors)
}

/// The pull. Inserts new Wealthsimple rows only and never
/// rebuilds the table.
pub fn run_sync(app: &Arc<App>, allow_refresh: bool, force_activity: bool) -> bool {
    {
        let mut st = app.state.lock().unwrap();
        if st.syncing {
            return false;
        }
        st.syncing = true;
        st.error.clear();
        st.sync_step = "Checking session…".into();
    }
    let result = sync_body(app, force_activity);
    let out = match result {
        Ok(v) => v,
        Err(CallError::NotAuthorized) => {
            let refreshed = allow_refresh && {
                let mut s = load_session(app).unwrap_or_default();
                refresh_session(app, &mut s, true)
            };
            if refreshed {
                app.state.lock().unwrap().syncing = false;
                return run_sync(app, false, force_activity);
            }
            note_session_expired(app);
            false
        }
        Err(e) => {
            let public = bagholder_ws::sync::public_sync_error(&e.to_string());
            let line = format!("Sync failed: {}", public);
            log(&line);
            set_error(app, &line);
            note_sync_failed(app, &public);
            false
        }
    };
    let mut st = app.state.lock().unwrap();
    st.syncing = false;
    st.sync_step.clear();
    out
}

fn sync_body(app: &Arc<App>, force_activity: bool) -> Result<bool, CallError> {
    let failed = |e: rusqlite::Error| CallError::Failed(e.to_string());
    let conn = app.open().map_err(failed)?;
    let mut sess = match load_session(app) {
        Some(s) if !s.access_token.is_empty() || !s.refresh_token.is_empty() => s,
        _ => {
            app.state.lock().unwrap().connected = false;
            return Ok(false);
        }
    };
    if sess.access_token.is_empty() {
        app.state.lock().unwrap().connected = false;
        return Ok(false);
    }
    let mut info = bagholder_ws::session::TokenInfo::default();
    let mut identity = sess.identity();
    if identity.is_empty() {
        info = token_info(app, &sess);
        identity = info.identity();
    }
    if identity.is_empty() {
        return Err(CallError::Failed("no identity_canonical_id".into()));
    }
    sess.ids.identity_canonical_id = identity.clone();
    let email = { if !info.email.is_empty() { info.email.clone() } else if !info.username.is_empty() { info.username.clone() } else { sess.email.clone() } };
    if !email.is_empty() {
        sess.email = email.clone();
    }
    let mut problems: Vec<String> = Vec::new();
    if let Err(e) = save_session(app, &sess) {
        problems.push(e);
    }

    if !force_activity && !bagholder_store::admin::activity_pull_due(&conn, now_unix() as i64).map_err(failed)? {
        let synced = bagholder_store::tables::get_meta(&conn, "synced_at", "").unwrap_or_default();
        let mut st = app.state.lock().unwrap();
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

    let home = app.ws_home();
    let client = Client { home: &home };
    set_step(app, "Fetching accounts…");
    let accounts = fetch::fetch_all_accounts(&client, &sess, &identity)?;
    let accts = bagholder_ws::mapping::Accounts::from_nodes(&accounts);
    let (start_date, _full) = bagholder_ws::sync::activity_sync_bounds(&conn).map_err(failed)?;
    let mut mapped: Vec<bagholder_store::activities::ActivityRow> = Vec::new();
    set_step(app, "Syncing transactions");
    let now_i = now_unix() as i64;
    for acc in accounts.iter().filter(|a| !a.id.is_empty()) {
        let items = fetch::fetch_activities_for_account(&client, &sess, &acc.id, start_date.as_deref(), now_i)?;
        for it in &items {
            mapped.extend(bagholder_ws::mapping::map_activity_rows(it, &accts));
        }
    }
    set_step(app, "Fetching balances…");
    let ids: Vec<String> = accounts.iter().map(|a| a.id.clone()).filter(|i| !i.is_empty()).collect();
    let balances = fetch::fetch_balances(&client, &sess, &ids)?;
    let (margin, margin_failed) = fetch::fetch_margin(&client, &sess, &fetch::margin_account_ids(&accounts), &now_iso());
    problems.extend(buying_power_problems(&accts, &margin_failed));
    let margin = keep_unread_margin(&conn, margin, &margin_failed).map_err(failed)?;
    set_step(app, "Fetching equity history…");
    let last_by = bagholder_store::tables::nav_last_dates(&conn).map_err(failed)?;
    let since_all = last_by.get("").cloned();
    let nav_history = match fetch::fetch_nav_history(&client, &sess, &identity, since_all.as_deref(), &today_utc()) {
        Ok(points) => points,
        Err(CallError::NotAuthorized) => return Err(CallError::NotAuthorized),
        Err(e) => {
            problems.push(format!("Equity history failed: {}", bagholder_ws::sync::public_sync_error(&e.to_string())));
            Vec::new()
        }
    };
    let mut combined: Vec<bagholder_store::broker::NavPoint> = nav_history
        .into_iter()
        .map(|mut r| { r.account_id = String::new(); r })
        .collect();
    let (nick_pts, nav_errors) = fetch_nickname_nav_history(app, &client, &sess, &accounts, &last_by);
    problems.extend(nav_errors);
    combined.extend(nick_pts);
    let applied = bagholder_store::merge::apply_wealthsimple_mapped(&conn, &mapped, &crate::app::uuid4).map_err(failed)?;
    if applied.unidentified > 0 {
        problems.push(unidentified_line(applied.unidentified));
    }
    let synced = now_iso();
    set_step(app, "Saving…");
    bagholder_store::tables::replace_accounts(&conn, &bagholder_ws::sync::slim_accounts(&accounts)).map_err(failed)?;
    bagholder_store::tables::replace_balances(&conn, &balances).map_err(failed)?;
    bagholder_store::tables::replace_margin(&conn, &margin, &synced).map_err(failed)?;
    bagholder_store::tables::upsert_nav(&conn, &combined).map_err(failed)?;
    bagholder_store::tables::set_meta(&conn, "synced_at", &synced).map_err(failed)?;
    bagholder_store::admin::mark_activity_pulled(&conn, &synced).map_err(failed)?;
    drop(conn);
    problems.extend(fill_listings(app, &sess, true));
    for p in &problems {
        log(&format!("Sync: {}", p));
    }
    let mut st = app.state.lock().unwrap();
    st.connected = true;
    st.sync_fails = 0;
    st.email = email;
    st.last_sync = synced;
    st.capturing = false;
    st.error = problems_line(&problems);
    st.sync_step.clear();
    Ok(true)
}

/// Stamp missing activity security ids and cache the listings the book
/// names; what could not be read is returned, one line per part.
pub fn fill_listings(app: &Arc<App>, sess: &Session, from_sync: bool) -> Vec<String> {
    if sess.access_token.is_empty() {
        return Vec::new();
    }
    {
        let mut st = app.state.lock().unwrap();
        if st.listings_filling || (st.syncing && !from_sync) {
            return Vec::new();
        }
        st.listings_filling = true;
        st.sync_step = "Attaching listing ids…".into();
    }
    let mut problems: Vec<String> = Vec::new();
    let result = (|| -> Result<(), String> {
        let conn = app.open().map_err(|e| e.to_string())?;
        let home = app.ws_home();
        let client = Client { home: &home };
        if bagholder_store::admin::needs_security_id_backfill(&conn).map_err(|e| e.to_string())? {
            let mut walk_ok = true;
            let mut mapped = Vec::new();
            set_step(app, "Attaching listing ids…");
            let mut ids: Vec<String> = Vec::new();
            for a in bagholder_store::tables::accounts(&conn).map_err(|e| e.to_string())? {
                if !a.id.is_empty() && !ids.contains(&a.id) {
                    ids.push(a.id);
                }
            }
            if ids.is_empty() {
                for id in bagholder_store::book::distinct_activity_account_ids(&conn).map_err(|e| e.to_string())? {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
            }
            for aid in ids {
                let by = bagholder_ws::mapping::Accounts::from_stored(&bagholder_store::tables::accounts(&conn).map_err(|e| e.to_string())?);
                let raw = match fetch::fetch_activities_for_account(&client, sess, &aid, None, now_unix() as i64) {
                    Ok(r) => r,
                    Err(e) => {
                        walk_ok = false;
                        problems.push(format!("Listing ids for {} failed: {}", by.name(&aid), bagholder_ws::sync::public_sync_error(&e.to_string())));
                        continue;
                    }
                };
                for it in &raw {
                    mapped.extend(bagholder_ws::mapping::map_activity_rows(it, &by));
                }
            }
            if !mapped.is_empty() {
                let applied = bagholder_store::merge::apply_wealthsimple_mapped(&conn, &mapped, &crate::app::uuid4).map_err(|e| e.to_string())?;
                if applied.unidentified > 0 {
                    problems.push(unidentified_line(applied.unidentified));
                }
            }
            if walk_ok {
                bagholder_store::tables::set_meta(&conn, "security_id_backfill_done", "1").map_err(|e| e.to_string())?;
            }
        }
        let wanted = bagholder_store::book::distinct_security_ids(&conn).map_err(|e| e.to_string())?;
        let mut pending = bagholder_store::admin::missing_security_ids(&conn, &wanted).map_err(|e| e.to_string())?;
        let mut seen: Vec<String> = Vec::new();
        let mut to_upsert: Vec<bagholder_model::securities::Security> = Vec::new();
        // options point at an underlying security, fetched in a second round
        while !pending.is_empty() {
            set_step(app, &format!("Looking up company names, {} left", pending.len()));
            let batch: Vec<String> = pending.iter().filter(|s| !seen.contains(s)).cloned().collect();
            seen.extend(batch.iter().cloned());
            pending.clear();
            if batch.is_empty() {
                break;
            }
            let (recs, not_read) = fetch::fetch_securities(&client, sess, &batch);
            if let Some(first) = not_read.first() {
                problems.push(format!("Company names for {} {} failed: {}", not_read.len(), if not_read.len() == 1 { "listing" } else { "listings" }, bagholder_ws::sync::public_sync_error(&first.error)));
            }
            let under: Vec<String> = recs.iter().map(|r| r.underlying_id.trim().to_string()).filter(|u| !u.is_empty() && !seen.contains(u)).collect();
            to_upsert.extend(recs);
            if !under.is_empty() {
                pending = bagholder_store::admin::missing_security_ids(&conn, &under).map_err(|e| e.to_string())?;
            }
        }
        if !to_upsert.is_empty() {
            bagholder_store::admin::upsert_securities(&conn, &to_upsert, &now_iso()).map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        problems.push(format!("Listing names failed: {}", bagholder_ws::sync::public_sync_error(&e)));
    }
    let mut st = app.state.lock().unwrap();
    st.listings_filling = false;
    st.sync_step.clear();
    problems
}

/// A line for each margin account whose buying power did not answer.
fn buying_power_problems(accounts: &bagholder_ws::mapping::Accounts, failed: &[fetch::Failed]) -> Vec<String> {
    failed.iter().map(|f| format!("Buying power for {} failed: {}", accounts.name(&f.id), bagholder_ws::sync::public_sync_error(&f.error))).collect()
}

/// The margin rows to store: those just read, and for an account whose read
/// failed, the figures stored before -- a failed read is not a reason to drop
/// what was known, and the failure is said beside it.
fn keep_unread_margin(conn: &rusqlite::Connection, mut read: Vec<bagholder_store::broker::Margin>, failed: &[fetch::Failed]) -> rusqlite::Result<Vec<bagholder_store::broker::Margin>> {
    if failed.is_empty() {
        return Ok(read);
    }
    for kept in bagholder_store::tables::margin(conn)? {
        if failed.iter().any(|f| f.id == kept.account_id) {
            read.push(kept);
        }
    }
    Ok(read)
}

/// Net liquidation values, balances and buying
/// power read again between syncs.
pub fn refresh_portfolio(app: &Arc<App>) {
    {
        let st = app.state.lock().unwrap();
        if st.syncing || !st.connected {
            return;
        }
    }
    let sess = load_session(app);
    let identity = sess.as_ref().map(|s| s.identity()).unwrap_or_default();
    let sess = match sess { Some(s) if !s.access_token.is_empty() && !identity.is_empty() => s, _ => {
        app.state.lock().unwrap().portfolio_error = "Portfolio refresh failed: no Wealthsimple login to read with".into();
        return;
    } };
    let home = app.ws_home();
    let client = Client { home: &home };
    let run = || -> Result<(), String> {
        let conn = app.open().map_err(|e| e.to_string())?;
        let accounts = fetch::fetch_all_accounts(&client, &sess, &identity).map_err(|e| e.to_string())?;
        let ids: Vec<String> = accounts.iter().map(|a| a.id.clone()).filter(|i| !i.is_empty()).collect();
        if ids.is_empty() {
            log("bagholder portfolio: Wealthsimple returned no accounts");
            return Ok(());
        }
        let balances = fetch::fetch_balances(&client, &sess, &ids).map_err(|e| e.to_string())?;
        let now = now_iso();
        let (margin, margin_failed) = fetch::fetch_margin(&client, &sess, &fetch::margin_account_ids(&accounts), &now);
        let names = bagholder_ws::mapping::Accounts::from_nodes(&accounts);
        let problems = buying_power_problems(&names, &margin_failed);
        let margin = keep_unread_margin(&conn, margin, &margin_failed).map_err(|e| e.to_string())?;
        bagholder_store::tables::replace_accounts(&conn, &bagholder_ws::sync::slim_accounts(&accounts)).map_err(|e| e.to_string())?;
        bagholder_store::tables::replace_balances(&conn, &balances).map_err(|e| e.to_string())?;
        bagholder_store::tables::replace_margin(&conn, &margin, &now).map_err(|e| e.to_string())?;
        bagholder_store::tables::set_meta(&conn, "balances_read_at", &now).map_err(|e| e.to_string())?;
        let available = margin.iter().filter(|m| m.buying_power.is_some()).count();
        log(&format!("bagholder portfolio: {} accounts, {} balances, buying power for {} of {} margin accounts", ids.len(), balances.len(), available, margin.len()));
        app.state.lock().unwrap().portfolio_error = problems_line(&problems);
        Ok(())
    };
    if let Err(e) = run() {
        let line = format!("Portfolio refresh failed: {}", bagholder_ws::sync::public_sync_error(&e));
        log(&format!("bagholder portfolio: {}", line));
        app.state.lock().unwrap().portfolio_error = line;
    }
}

pub fn portfolio_loop(app: Arc<App>) {
    // balances, net liquidation values and buying power: read when a page connects and
    // each few minutes while one stays. Nothing else reads them between syncs.
    while app.events.park_until(&app, || app.events.watchers() > 0) {
        refresh_portfolio(&app);
        if app.wait(Duration::from_secs(60 * PORTFOLIO_REFRESH_MINUTES)) {
            return;
        }
    }
}

/// `POST /api/refresh`.
#[derive(Serialize, TS)]
pub struct RefreshAnswer {
    pub ok: bool,
    pub error: String,
    pub connected: bool,
}

/// `POST /api/sync`.
#[derive(Serialize, TS)]
pub struct SyncAnswer {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub syncing: Option<bool>,
}

/// Start a pull if a login is saved; its progress reaches the page as status
/// changes.
pub fn sync_now(app: &Arc<App>) -> SyncAnswer {
    if load_session(app).is_none() {
        return SyncAnswer { ok: false, error: Some("not connected".into()), syncing: None };
    }
    app.state.lock().unwrap().error.clear();
    let a = app.clone();
    spawn("bagholder-sync", move || {
        crate::feeds::sync_then_market(&a);
    });
    SyncAnswer { ok: true, error: None, syncing: Some(true) }
}

/// The grant, always.
pub fn refresh_now(app: &Arc<App>) -> RefreshAnswer {
    let mut sess = match load_session(app) {
        Some(s) if !s.refresh_token.is_empty() => s,
        _ => {
            let mut st = app.state.lock().unwrap();
            st.connected = false;
            st.error = "not connected".into();
            return RefreshAnswer { ok: false, error: "not connected".into(), connected: false };
        }
    };
    let ok = refresh_session(app, &mut sess, true);
    let mut st = app.state.lock().unwrap();
    st.connected = ok;
    if ok {
        st.error.clear();
    }
    RefreshAnswer { ok, error: st.error.trim().to_string(), connected: ok }
}

/// The token kept fresh, and the weekday pull when
/// it is due, backing off on failure.
pub fn auto_sync_loop(app: Arc<App>) {
    // Two things are waited for, and both are known ahead: the token coming up for
    // refresh, and the next pull window opening. The loop sleeps until the nearer --
    // hours, usually -- and wakes early only when the session changes under it (a
    // sign-in, a disconnect). It used to wake every thirty seconds to read the
    // session file and ask the database whether it was time yet. Only a failure is
    // retried on a period, doubling from `RETRY_FIRST` to half an hour.
    const RETRY_FIRST: Duration = Duration::from_secs(30);
    const RETRY_MOST: Duration = Duration::from_secs(1800);
    let has_login = |app: &Arc<App>| load_session(app).map_or(false, |s| !s.refresh_token.is_empty());
    let mut retry: Option<Duration> = None;
    loop {
        let now = now_unix();
        let connected = app.state.lock().unwrap().connected;
        let login = has_login(&app);
        let sleep = match retry {
            Some(d) => d,
            None if !login => Duration::MAX, // nothing to keep fresh until someone signs in
            None => {
                let token = load_session(&app).map_or(0.0, |s| bagholder_ws::sync::seconds_until_token_refresh(&s, now));
                let due = app.open().ok().and_then(|c| bagholder_store::admin::activity_pull_due(&c, now as i64).ok()).unwrap_or(false);
                let pull = if due { 0 } else { bagholder_store::admin::seconds_until_pull_window(now as i64) };
                Duration::from_secs_f64(token.min(pull as f64).max(0.0))
            }
        };
        let was = (connected, login);
        let changed = || (app.state.lock().unwrap().connected, has_login(&app)) != was;
        if sleep == Duration::MAX {
            if !app.events.park_until(&app, changed) {
                return;
            }
        } else if !sleep.is_zero() {
            app.events.park_until_or(&app, sleep, changed);
        }
        if app.stopping() {
            return;
        }
        if !has_login(&app) {
            retry = None;
            continue;
        }
        if !ensure_fresh_token(&app, None) {
            retry = Some(retry.map_or(RETRY_FIRST, |d| (d * 2).min(RETRY_MOST)));
            continue;
        }
        let syncing = app.state.lock().unwrap().syncing;
        let due = app.open().ok().and_then(|c| bagholder_store::admin::activity_pull_due(&c, now_unix() as i64).ok()).unwrap_or(false);
        if due && !syncing {
            let ok = run_sync(&app, true, true);
            crate::feeds::refresh_market_data(&app);
            if !ok {
                retry = Some(retry.map_or(RETRY_FIRST, |d| (d * 2).min(RETRY_MOST)));
                continue;
            }
        } else if due {
            // a pull the user started is running; it marks the window done when it ends
            app.events.park_until(&app, || !app.state.lock().unwrap().syncing);
        }
        retry = None;
    }
}

/// The captured login's fields; any other key the body carries is kept as it
/// was, exactly as a saved `Session` keeps what it does not read.
#[derive(Clone, Debug, Default, serde::Deserialize, TS)]
#[serde(default)]
pub struct Capture {
    #[serde(deserialize_with = "bagholder_model::lenient::text")]
    pub access_token: String,
    #[serde(deserialize_with = "bagholder_model::lenient::text")]
    pub refresh_token: String,
    #[serde(deserialize_with = "bagholder_model::lenient::text")]
    pub client_id: String,
    #[serde(deserialize_with = "bagholder_model::lenient::text")]
    pub wssdi: String,
    #[serde(deserialize_with = "bagholder_model::lenient::text")]
    pub session_id: String,
    #[serde(deserialize_with = "bagholder_model::lenient::text")]
    pub user_agent: String,
    pub expires_at: Option<bagholder_ws::session::Expiry>,
    #[serde(flatten)]
    pub ids: bagholder_ws::session::IdentityKeys,
}

/// Keep the captured login and take it over.
pub fn capture_tokens(app: &Arc<App>, capture: &Capture) -> OkOr {
    let capture = capture.clone();
    if capture.access_token.is_empty() {
        return OkOr::err("missing access_token");
    }
    let mut sess = load_session(app).unwrap_or_default();
    sess.access_token = capture.access_token;
    if !capture.refresh_token.is_empty() {
        sess.refresh_token = capture.refresh_token;
    }
    // a captured `expires_at` of 0 or "" is not truthy and does not overwrite
    let expiry_truthy = match &capture.expires_at {
        Some(bagholder_ws::session::Expiry::Unix(n)) => *n != 0.0,
        Some(bagholder_ws::session::Expiry::Text(t)) => !t.is_empty(),
        None => false,
    };
    if expiry_truthy {
        sess.expires_at = capture.expires_at;
    }
    if !capture.wssdi.is_empty() {
        sess.wssdi = capture.wssdi;
    }
    if !capture.client_id.is_empty() {
        sess.client_id = capture.client_id;
    }
    if !capture.session_id.is_empty() {
        sess.session_id = capture.session_id;
    }
    if !capture.user_agent.is_empty() {
        sess.user_agent = capture.user_agent;
    }

    let mut ident = capture.ids.identity();
    if ident.is_empty() {
        ident = sess.identity();
    }
    let info = if sess.access_token.is_empty() { bagholder_ws::session::TokenInfo::default() } else { token_info(app, &sess) };
    if ident.is_empty() {
        ident = info.identity();
    }
    if !ident.is_empty() {
        sess.ids.identity_canonical_id = ident;
    }
    if sess.session_id.is_empty() {
        sess.session_id = crate::app::uuid4();
    }
    apply_token_info_client_id(app, &mut sess, Some(&info));
    if sess.client_id.is_empty() {
        let cid = scrape_client_id(app);
        if !cid.is_empty() {
            sess.client_id = cid;
        }
    }
    if sess.user_agent.is_empty() {
        let ua = app.ws_home().cached_user_agent();
        if !ua.is_empty() {
            sess.user_agent = ua;
        }
    }
    // take the login over: rotate its refresh token now, so the copy the
    // browser holds goes stale instead of ours
    if !refresh_session(app, &mut sess, false) {
        let mut st = app.state.lock().unwrap();
        st.connected = false;
        let err = if st.error.is_empty() { "Wealthsimple refused the captured login".to_string() } else { st.error.clone() };
        return OkOr::err(err);
    }
    if let Err(e) = save_session(app, &sess) {
        app.state.lock().unwrap().error = e.clone();
        return OkOr::err(e);
    }
    {
        let mut st = app.state.lock().unwrap();
        st.connected = true;
        st.capturing = false;
        st.error.clear();
    }
    let a = app.clone();
    spawn("bagholder-sync", move || {
        run_sync(&a, true, true);
    });
    OkOr::ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bagholder_store::broker::Margin;
    use bagholder_ws::standin::{fixture, graphql, graphql_errors};
    use serde_json::json;
    /// A Wealthsimple that answers everything but buying power and the
    /// account-wide equity history.
    fn half_answering() -> bagholder_ws::standin::Fixture {
        fixture(Box::new(|req| match req.body["operationName"].as_str().unwrap_or("") {
            "FetchAllAccountFinancials" => graphql(json!({"identity": {"accounts": {"edges": [{"node": {
                "id": "m1", "nickname": "Margin", "unifiedAccountType": "SELF_DIRECTED_MARGIN", "status": "open", "currency": "CAD"
            }}], "pageInfo": {"hasNextPage": false}}}})),
            "FetchAccountsWithBalance" => graphql(json!({"accounts": []})),
            "FetchActivityFeedItems" => graphql(json!({"activityFeedItems": {"edges": [], "pageInfo": {"hasNextPage": false}}})),
            "FetchAccountHistoricalFinancials" => graphql(json!({"account": {"financials": {"historicalDaily": {"edges": [], "pageInfo": {}}}}})),
            "FetchSecurities" => graphql(json!({"securities": []})),
            "FetchAccountCurrentMarginBuyingPowerV2" => graphql_errors(json!([{"message": "margin service down"}])),
            "IdentityHistoricalFinancialsQuery" => graphql_errors(json!([{"message": "history service down"}])),
            other => panic!("unexpected operation {}", other),
        }))
    }

    fn app() -> Arc<App> {
        let app = crate::tests_common::app();
        bagholder_store::schema::init_schema(&app.open().unwrap()).unwrap();
        let mut sess = Session { access_token: "tok".into(), refresh_token: "r".into(), ..Default::default() };
        sess.ids.identity_canonical_id = "ident-1".into();
        save_session(&app, &sess).unwrap();
        {
            let mut st = app.state.lock().unwrap();
            st.connected = true;
            st.error.clear();
            st.portfolio_error.clear();
        }
        bagholder_store::tables::replace_margin(&app.open().unwrap(), &[Margin { account_id: "m1".into(), buying_power: Some(500.0), currency: "CAD".into(), ..Default::default() }], "2026-09-22T10:00:00Z").unwrap();
        app
    }

    fn error_line(app: &Arc<App>) -> String {
        crate::status::status(app).error
    }

    #[test]
    fn test_a_part_of_the_pull_that_fails_is_said_and_keeps_what_was_known() {
        let _g = crate::tests_common::guard();
        let _ws = half_answering();
        let app = app();

        refresh_portfolio(&app);
        let line = error_line(&app);
        assert!(line.contains("Buying power for Margin failed") && line.contains("margin service down"), "{}", line);
        let kept = bagholder_store::tables::margin(&app.open().unwrap()).unwrap();
        assert_eq!(kept.iter().map(|m| (m.account_id.as_str(), m.buying_power)).collect::<Vec<_>>(), vec![("m1", Some(500.0))], "a failed read does not drop the figure stored before");

        assert!(run_sync(&app, false, true));
        let line = error_line(&app);
        assert!(line.contains("Buying power for Margin failed"), "{}", line);
        assert!(line.contains("Equity history failed") && line.contains("history service down"), "{}", line);
        delete_session(&app);
    }

    #[test]
    fn test_unidentified_line_is_singular_and_plural() {
        assert_eq!(unidentified_line(1), "1 Wealthsimple row had no id and was not stored");
        assert_eq!(unidentified_line(2), "2 Wealthsimple rows had no id and were not stored");
    }
}
