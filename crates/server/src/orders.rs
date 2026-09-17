//! Manual activity, the order ticket, reading orders back from Wealthsimple,
//! and the bracket engine: `bagholder.py` from `_manual_from_fields` to
//! `adjust_bracket`.
//!
//! Without live orders (`BAGHOLDER_DRY_ORDERS=1`) nothing that places, cancels
//! or modifies an order is ever sent: the ticket is recorded as `dry`, and the
//! GraphQL wrapper here refuses the three order mutations outright as a second
//! line of defence.

use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use rusqlite::Connection;
use serde_json::{json, Map, Value};

use bagholder_store::orders as so;
use bagholder_ws::session::{identity_from, CallError, Client};

use crate::app::{app, f, log, now_iso, now_unix, num, qty_text, s, truthy, uuid4};

/// Test seams: a fake Wealthsimple, the live switch, the session, and threads.
/// Under `cfg(test)` nothing reaches the network: without a fake every call fails.
#[cfg(test)]
pub mod seam {
    use super::*;
    use std::sync::Arc;
    pub type Gql = Arc<dyn Fn(&str, &Value) -> Result<Value, CallError> + Send + Sync>;
    pub static GQL: Mutex<Option<Gql>> = Mutex::new(None);
    pub static LIVE: Mutex<Option<bool>> = Mutex::new(None);
    pub static SESSION: Mutex<Option<Option<Value>>> = Mutex::new(None);
    /// 0: a spawned thread never runs (Python's patched `threading.Thread`); 1: it runs inline.
    pub static SPAWN_INLINE: AtomicBool = AtomicBool::new(false);
    pub fn reset() {
        *GQL.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *LIVE.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *SESSION.lock().unwrap_or_else(|e| e.into_inner()) = None;
        SPAWN_INLINE.store(false, Ordering::SeqCst);
    }
}

fn spawn<F: FnOnce() + Send + 'static>(name: &str, f: F) {
    #[cfg(test)]
    {
        let _ = name;
        if seam::SPAWN_INLINE.load(Ordering::SeqCst) {
            f();
        }
        return;
    }
    #[cfg(not(test))]
    crate::app::spawn(name, f)
}
use crate::notify;
use crate::session::{ensure_fresh_token, load_session};

// ---------------------------------------------------------------------------
// small tools
// ---------------------------------------------------------------------------

/// `bagholder.ORDERS_LIVE`.
pub fn orders_live() -> bool {
    #[cfg(test)]
    if let Some(v) = *seam::LIVE.lock().unwrap_or_else(|e| e.into_inner()) {
        return v;
    }
    static LIVE: OnceLock<bool> = OnceLock::new();
    *LIVE.get_or_init(|| std::env::var("BAGHOLDER_DRY_ORDERS").map(|v| v.trim() != "1").unwrap_or(true))
}

fn db() -> Connection {
    app().open().expect("bagholder orders: the store could not be opened")
}

fn must<T>(r: rusqlite::Result<T>) -> T {
    r.unwrap_or_else(|e| panic!("bagholder orders: store: {}", e))
}

fn tr(v: &Value, k: &str) -> bool {
    truthy(v.get(k))
}

fn on(v: &Value, k: &str) -> Option<f64> {
    num(v.get(k), None)
}

/// `x.get(k) or 0.0`.
fn or0(v: &Value, k: &str) -> f64 {
    on(v, k).unwrap_or(0.0)
}

/// Python's `a or b` over optional floats.
fn or_f(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match a {
        Some(x) if x != 0.0 => Some(x),
        _ => b,
    }
}

/// Python's `a or b` over JSON values.
fn or_v<'a>(a: Option<&'a Value>, b: Option<&'a Value>) -> Option<&'a Value> {
    if truthy(a) {
        a
    } else {
        b
    }
}

fn gv(v: &Value, k: &str) -> Value {
    v.get(k).cloned().unwrap_or(Value::Null)
}

fn jo(v: Option<f64>) -> Value {
    match v {
        Some(x) if x.is_finite() => json!(x),
        _ => Value::Null,
    }
}

fn set(v: &mut Value, k: &str, x: Value) {
    if let Value::Object(m) = v {
        m.insert(k.to_string(), x);
    }
}

/// Python's `round(x, n)`.
fn py_round(x: f64, n: usize) -> f64 {
    format!("{:.*}", n, x).parse().unwrap_or(x)
}

/// `str(float)`, or `None`.
fn rp(v: Option<f64>) -> String {
    match v {
        Some(x) => bagholder_model::value::num_repr(x),
        None => "None".into(),
    }
}

/// Python's `"%g" % x`.
fn py_g(x: f64) -> String {
    if x == 0.0 {
        return if x.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    if !x.is_finite() {
        return if x.is_nan() { "nan".into() } else if x > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let e_form = format!("{:.5e}", x);
    let (mant, exp) = e_form.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap_or(0);
    if exp < -4 || exp >= 6 {
        let m = if mant.contains('.') { mant.trim_end_matches('0').trim_end_matches('.') } else { mant };
        format!("{}e{}{:02}", m, if exp < 0 { '-' } else { '+' }, exp.abs())
    } else {
        let t = format!("{:.*}", (5 - exp).max(0) as usize, x);
        if t.contains('.') {
            t.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            t
        }
    }
}

fn upper(v: Option<&Value>) -> String {
    s(v).trim().to_uppercase()
}

/// `bagholder._date_only`.
fn date_only(v: Option<&Value>) -> String {
    let t = s(v);
    let t = t.trim();
    if t.is_empty() {
        return String::new();
    }
    let t = t.split('T').next().unwrap_or("");
    t.chars().take(10).collect()
}

fn two(b: &[u8], i: usize) -> Option<i64> {
    let a = (b[i] as char).to_digit(10)?;
    let c = (b[i + 1] as char).to_digit(10)?;
    Some((a * 10 + c) as i64)
}

/// `%Y-%m-%dT%H:%M:%S` as unix seconds; exact.
fn parse_ymdhms(t: &str) -> Option<i64> {
    let b = t.as_bytes();
    if b.len() != 19 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let y: i64 = t[..4].parse().ok()?;
    let (mo, d, h, mi, se) = (two(b, 5)?, two(b, 8)?, two(b, 11)?, two(b, 14)?, two(b, 17)?);
    if !(1..=12).contains(&mo) || d < 1 || d > bagholder_model::dates::days_in_month(y, mo as u32) as i64 || h > 23 || mi > 59 || se > 61 {
        return None;
    }
    Some(bagholder_model::dates::to_days(y, mo as u32, d as u32) * 86400 + h * 3600 + mi * 60 + se)
}

/// `datetime.strptime(t, "%Y-%m-%dT%H:%M:%SZ")`.
fn parse_z(t: &str) -> Option<i64> {
    t.strip_suffix('Z').and_then(parse_ymdhms)
}

/// `bagholder._parse_utc`.
fn parse_utc(v: Option<&Value>) -> Option<i64> {
    let t = s(v);
    let t = t.trim();
    if t.is_empty() {
        return None;
    }
    parse_ymdhms(&t.chars().take(19).collect::<String>())
}

fn gql(sess: &Value, op: &str, vars: Value) -> Result<Value, CallError> {
    if !orders_live() && matches!(op, "SoOrdersOrderCreate" | "SoOrdersOrderCancel" | "SoOrdersOrderModify") {
        return Err(CallError::Failed("orders are off (BAGHOLDER_DRY_ORDERS)".into()));
    }
    #[cfg(test)]
    {
        let _ = sess;
        let g = seam::GQL.lock().unwrap_or_else(|e| e.into_inner()).clone();
        return match g {
            Some(g) => g(op, &vars),
            None => Err(CallError::Failed(format!("{}: no network in tests", op))),
        };
    }
    #[cfg(not(test))]
    {
        let home = app().ws_home();
        Client { home: &home }.graphql(sess, op, &vars, None)
    }
}

fn err_text(e: &CallError) -> String {
    let t = e.to_string();
    if t.is_empty() {
        "RuntimeError".into()
    } else {
        t
    }
}

fn first_error(errs: &Value) -> Option<String> {
    let a = errs.as_array()?;
    let first = a.first()?;
    Some(if first.is_object() {
        s(or_v(first.get("message"), first.get("code")))
    } else {
        s(Some(first))
    })
}

fn list_orders() -> Vec<Value> {
    must(so::list_orders(&db(), 200))
}

fn get_order(id: &str) -> Option<Value> {
    must(so::get_order(&db(), id))
}

fn insert_order(row: &Value) {
    must(so::insert_order(&db(), row, &now_iso()))
}

fn update_order(id: &str, patch: Value) {
    must(so::update_order(&db(), id, &patch, &now_iso()))
}

fn brackets(statuses: &[&str]) -> Vec<Value> {
    let st: Vec<String> = statuses.iter().map(|x| x.to_string()).collect();
    must(so::list_brackets(&db(), &st))
}

fn get_bracket(id: &str) -> Option<Value> {
    must(so::get_bracket(&db(), id))
}

fn update_bracket(id: &str, patch: Value) {
    must(so::update_bracket(&db(), id, &patch, &now_iso()))
}

fn emit(kind: &str, key: &str, title: &str, body: &str) {
    notify::emit(&db(), kind, key, title, body, None);
}

fn snapshot() -> Value {
    must(bagholder_store::snapshot::snapshot(&db(), false))
}

fn connected_not_syncing() -> bool {
    let st = app().state.lock().unwrap();
    st.connected && !st.syncing
}

// ---------------------------------------------------------------------------
// manual activity
// ---------------------------------------------------------------------------

fn manual_from_fields(body: &Value) -> Value {
    let mut side = upper(or_v(body.get("side"), Some(&json!("BUY"))));
    if side != "BUY" && side != "SELL" {
        side = "BUY".into();
    }
    let pick = |a: &str, b: &str| if body.get(a).map_or(false, |v| !v.is_null()) { body.get(a) } else { body.get(b) };
    let qty = num(pick("qty", "quantity"), Some(0.0)).unwrap_or(0.0).abs();
    let px = num(pick("price", "unitPrice"), Some(0.0)).unwrap_or(0.0).abs();
    let mut date = date_only(or_v(or_v(body.get("date"), body.get("transactionDate")), body.get("occurredAt")));
    if date.is_empty() {
        date = crate::app::today_utc();
    }
    let symbol = upper(body.get("symbol"));
    let mut currency = upper(or_v(body.get("currency"), Some(&json!("CAD"))));
    if currency != "CAD" && currency != "USD" {
        currency = "CAD".into();
    }
    let mut account_id = s(or_v(or_v(body.get("accountId"), body.get("account")), Some(&json!("manual"))));
    if account_id.is_empty() {
        account_id = "manual".into();
    }
    let buy = side == "BUY";
    let signed_qty = if buy { qty } else { -qty };
    let cash = if buy { -(qty * px) } else { qty * px };
    let account_type = s(or_v(body.get("accountType"), Some(&json!(if account_id == "manual" { "Manual" } else { "" }))));
    let desc = format!(
        "{}{}",
        if buy { "Buy" } else { "Sell" },
        if symbol.is_empty() { String::new() } else { format!(" {} {} @ {}", py_g(qty), symbol, py_g(px)) }
    );
    json!({
        "id": uuid4(),
        "occurredAt": date,
        "transactionDate": date,
        "settlementDate": date,
        "accountId": account_id,
        "bookId": account_id,
        "accountType": account_type,
        "activityType": "Trade",
        "activitySubType": side,
        "description": desc,
        "direction": if buy { "DEBIT" } else { "CREDIT" },
        "symbol": symbol,
        "name": symbol,
        "currency": currency,
        "quantity": signed_qty,
        "unitPrice": px,
        "commission": num(body.get("commission"), Some(0.0)).unwrap_or(0.0).abs(),
        "netCashAmount": cash,
        "category": "trade",
        "balance": null,
        "source": "manual",
    })
}

fn normalize_local_row(act: &Value) -> Value {
    let mut act: Map<String, Value> = act.as_object().cloned().unwrap_or_default();
    let mut source = s(act.get("source"));
    if source.is_empty() {
        source = "manual".into();
    }
    if source == "wealthsimple" {
        if let Some(cid) = bagholder_store::activities::canonical_from_row(&Value::Object(act.clone()), "wealthsimple") {
            act.insert("canonicalId".into(), json!(cid));
            act.insert("source".into(), json!("wealthsimple"));
            return Value::Object(act);
        }
        source = "manual".into();
    }
    act.insert("source".into(), json!(source));
    act.shift_remove("canonicalId");
    act.shift_remove("canonical_id");
    if !truthy(act.get("accountId")) {
        act.insert("accountId".into(), json!("manual"));
    }
    if !truthy(act.get("bookId")) {
        let a = act.get("accountId").cloned().unwrap_or(Value::Null);
        act.insert("bookId".into(), a);
    }
    if !truthy(act.get("id")) || bagholder_store::activities::looks_like_homemade_id(&s(act.get("id"))) {
        act.insert("id".into(), json!(uuid4()));
    }
    if !truthy(act.get("occurredAt")) {
        let t = act.get("transactionDate").filter(|v| truthy(Some(v))).cloned().unwrap_or(json!(""));
        act.insert("occurredAt".into(), t);
    }
    Value::Object(act)
}

/// `bagholder.append_manual`.
pub fn append_manual(body: &Value) -> Value {
    let rows: Vec<Value> = if let Some(Value::Array(a)) = body.get("activities") {
        a.iter().filter(|r| r.is_object()).cloned().collect()
    } else if truthy(body.get("activity")) && body.get("activity").map_or(false, |a| a.is_object()) {
        vec![body["activity"].clone()]
    } else {
        vec![manual_from_fields(body)]
    };
    let rows: Vec<Value> = rows.iter().map(normalize_local_row).collect();
    let conn = db();
    let result = must(bagholder_store::merge::merge_local_rows(&conn, &rows, &uuid4));
    let mut snap = snapshot();
    if !tr(&snap, "syncedAt") {
        let stamp = now_iso();
        must(bagholder_store::tables::set_meta(&conn, "synced_at", &stamp));
        snap = snapshot();
    }
    {
        let mut st = app().state.lock().unwrap();
        let synced = f(&snap, "syncedAt");
        if !synced.is_empty() {
            st.last_sync = synced;
        }
    }
    let saved = result.activities;
    let mut out = json!({"ok": true, "added": result.added, "duplicates": result.duplicates});
    if saved.len() == 1 {
        set(&mut out, "activity", saved[0].clone());
    } else if !saved.is_empty() {
        set(&mut out, "activities", Value::Array(saved));
    } else if rows.len() == 1 {
        set(&mut out, "activity", rows[0].clone());
    }
    out
}

// ---------------------------------------------------------------------------
// order ticket
// ---------------------------------------------------------------------------

pub const ORDER_EXEC_TYPES: [&str; 4] = ["MARKET", "LIMIT", "STOP", "STOP_LIMIT"];
pub const ORDER_TIFS: [&str; 2] = ["DAY", "UNTIL_CANCEL"];
const ORDER_TRADABLE_TYPES: [&str; 1] = ["SELF_DIRECTED"];
const ORDER_UNTRADABLE_MARKERS: [&str; 3] = ["CRYPTO", "PREDICTIONS", "MANAGED"];

fn ticket_session() -> Option<Value> {
    #[cfg(test)]
    {
        return seam::SESSION.lock().unwrap_or_else(|e| e.into_inner()).clone().flatten();
    }
    #[allow(unreachable_code)]
    let sess = load_session()?;
    if f(&sess, "access_token").is_empty() {
        return None;
    }
    ensure_fresh_token(Some(sess.clone()));
    match load_session() {
        Some(v) if v.as_object().map_or(false, |m| !m.is_empty()) => Some(v),
        _ => Some(sess),
    }
}

/// `bagholder.order_accounts`.
pub fn order_accounts(accounts: Option<&[Value]>) -> Vec<Value> {
    let owned;
    let list: &[Value] = match accounts {
        Some(a) => a,
        None => {
            owned = snapshot().get("accounts").and_then(|a| a.as_array()).cloned().unwrap_or_default();
            &owned
        }
    };
    let mut out = Vec::new();
    for a in list {
        let typ = s(or_v(a.get("unifiedAccountType"), a.get("unified_account_type"))).to_uppercase();
        let status = f(a, "status").to_lowercase();
        if !tr(a, "id") || status == "closed" || !ORDER_TRADABLE_TYPES.iter().any(|p| typ.starts_with(p)) {
            continue;
        }
        if ORDER_UNTRADABLE_MARKERS.iter().any(|m| typ.contains(m)) {
            continue;
        }
        let nick_src = if tr(a, "nickname") { f(a, "nickname") } else { typ.clone() };
        let nick = bagholder_model::value::norm_account_name(&nick_src);
        let margin = typ.contains("MARGIN");
        out.push(json!({
            "id": f(a, "id"), "name": nick, "type": typ, "margin": margin, "currency": f(a, "currency"),
            "marginAccountId": if margin { f(a, "id") } else { f(a, "marginAccountId") },
        }));
    }
    out
}

/// `bagholder.resolve_security`.
pub fn resolve_security(symbol: &str, security_id: &str) -> Option<Value> {
    let rows = must(bagholder_store::admin::list_securities(&db()));
    let sid = security_id.trim();
    if !sid.is_empty() {
        if let Some(r) = rows.iter().find(|r| f(r, "id") == sid) {
            return Some(r.clone());
        }
        return Some(json!({"id": sid, "symbol": symbol.trim().to_uppercase(), "name": "", "primaryExchange": "", "primaryMic": "", "currency": "", "underlyingId": null}));
    }
    let sym = symbol.trim().to_uppercase();
    if sym.is_empty() {
        return None;
    }
    let mut same: Vec<Value> = rows.into_iter().filter(|r| f(r, "symbol").to_uppercase() == sym).collect();
    same.sort_by_key(|r| (if f(r, "id").starts_with("sec-s-") { 0 } else { 1 }, f(r, "id")));
    same.into_iter().next()
}

/// `bagholder.parse_quote`.
pub fn parse_quote(node: &Value) -> Option<Value> {
    if !node.is_object() || !tr(node, "id") {
        return None;
    }
    let empty = json!({});
    let q = node.get("quoteV2").filter(|v| v.is_object()).unwrap_or(&empty);
    let stock = node.get("stock").filter(|v| v.is_object()).unwrap_or(&empty);
    let opt = node.get("optionDetails").filter(|v| v.is_object()).unwrap_or(&empty);
    let last = on(q, "price").or_else(|| on(q, "last"));
    let base = on(q, "previousBaseline").or_else(|| on(q, "referenceClose"));
    let (bid, ask) = (on(q, "bid"), on(q, "ask"));
    let change = match (last, base) {
        (Some(l), Some(b)) => Some(l - b),
        _ => None,
    };
    let mid = if q.get("mid").map_or(false, |v| !v.is_null()) {
        on(q, "mid")
    } else {
        match (bid, ask) {
            (Some(b), Some(a)) => Some((b + a) / 2.0),
            _ => None,
        }
    };
    let change_pct = match (change, base) {
        (Some(c), Some(b)) if b != 0.0 => Some(c / b),
        _ => None,
    };
    let multiplier = if opt.as_object().map_or(false, |m| !m.is_empty()) { on(opt, "multiplier") } else { None };
    Some(json!({
        "securityId": f(node, "id"),
        "symbol": f(stock, "symbol"),
        "name": f(stock, "name"),
        "exchange": f(stock, "primaryExchange"),
        "currency": s(or_v(q.get("currency"), node.get("currency"))).to_uppercase(),
        "securityType": f(node, "securityType"),
        "buyable": tr(node, "buyable"),
        "sellable": tr(node, "sellable"),
        "tradeEligible": tr(node, "wsTradeEligible"),
        "status": f(node, "status"),
        "last": jo(last),
        "bid": jo(bid),
        "ask": jo(ask),
        "bidSize": jo(on(q, "bidSize")),
        "askSize": jo(on(q, "askSize")),
        "mid": jo(mid),
        "change": jo(change),
        "changePct": jo(change_pct),
        "marketStatus": f(q, "marketStatus"),
        "quotedAsOf": f(q, "quotedAsOf"),
        "multiplier": jo(multiplier),
    }))
}

/// `bagholder.parse_market_data`.
pub fn parse_market_data(data: &Value) -> Value {
    let empty = json!({});
    let sec = data.get("security").filter(|v| v.is_object()).unwrap_or(&empty);
    let subtypes: Vec<String> = sec
        .get("allowedOrderSubtypes")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter(|x| truthy(Some(x))).map(|x| s(Some(x)).to_uppercase()).collect())
        .unwrap_or_default();
    let rates = sec.get("marginRates").filter(|v| v.is_object()).unwrap_or(&empty);
    let mut rate = on(rates, "clientMarginRate");
    if let Some(r) = rate {
        if r > 1.0 {
            rate = Some(r / 100.0);
        }
    }
    let types: Vec<&str> = ORDER_EXEC_TYPES.iter().copied().filter(|t| subtypes.iter().any(|x| x == t)).collect();
    json!({"orderTypes": types, "marginRate": jo(rate)})
}

/// `bagholder.parse_buying_power`.
pub fn parse_buying_power(data: &Value) -> Value {
    let empty = json!({});
    let mut view = data;
    for k in ["account", "financials", "current", "tradingBalanceViewV2"] {
        view = view.get(k).filter(|v| v.is_object()).unwrap_or(&empty);
    }
    let bp = view.get("buyingPower").filter(|v| v.is_object()).unwrap_or(&empty);
    let cash = view.get("cash").filter(|v| v.is_object()).unwrap_or(&empty);
    json!({"buyingPower": jo(on(bp, "quantity")), "cash": jo(on(cash, "quantity")), "currency": s(or_v(bp.get("currency"), cash.get("currency")))})
}

/// `bagholder.fetch_quotes`.
pub fn fetch_quotes(sess: &Value, security_ids: &[String]) -> Result<HashMap<String, Value>, CallError> {
    let ids: Vec<String> = security_ids.iter().map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
    let mut out = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let data = gql(sess, "FetchSecuritiesSummary", json!({"ids": ids}))?;
    if let Some(a) = data.get("securities").and_then(|v| v.as_array()) {
        for node in a {
            if let Some(q) = parse_quote(node) {
                out.insert(f(&q, "securityId"), q);
            }
        }
    }
    Ok(out)
}

const LOOKUP_TYPES: [&str; 2] = ["EQUITY", "EXCHANGE_TRADED_FUND"];
const CANADIAN_SUFFIXES: [&str; 4] = [".TO", ".V", ".CN", ".NE"];

fn bare_symbol(sym: &str) -> String {
    let sym = sym.to_uppercase();
    for suf in CANADIAN_SUFFIXES {
        if let Some(b) = sym.strip_suffix(suf) {
            return b.to_string();
        }
    }
    sym
}

/// `bagholder.parse_listing_search`.
pub fn parse_listing_search(data: &Value, symbol: &str, exchange: &str) -> Option<Value> {
    let (want_sym, want_ex) = (bare_symbol(symbol), exchange.trim().to_uppercase());
    let empty = json!({});
    let results = data.get("securitySearch").and_then(|v| v.get("results")).and_then(|v| v.as_array())?;
    for r in results {
        if !r.is_object() || !tr(r, "id") {
            continue;
        }
        let stock = r.get("stock").filter(|v| v.is_object()).unwrap_or(&empty);
        if bare_symbol(&f(stock, "symbol")) != want_sym || f(stock, "primaryExchange").to_uppercase() != want_ex {
            continue;
        }
        if !LOOKUP_TYPES.contains(&f(r, "securityType").to_uppercase().as_str()) {
            continue;
        }
        return Some(json!({"id": f(r, "id"), "symbol": f(stock, "symbol").to_uppercase(), "name": f(stock, "name"), "primaryExchange": f(stock, "primaryExchange"),
            "primaryMic": f(stock, "primaryMic"), "currency": f(r, "currency").to_uppercase(), "underlyingId": null}));
    }
    None
}

/// `bagholder.lookup_listing`.
pub fn lookup_listing(sess: &Value, symbol: &str, exchange: &str) -> Option<Value> {
    let data = match gql(sess, "FetchSecuritySearchResult", json!({"query": symbol.trim()})) {
        Ok(d) => d,
        Err(e) => {
            log(&format!("bagholder ticket: listing search for {} failed: {}", symbol, e));
            return None;
        }
    };
    let sec = parse_listing_search(&data, symbol, exchange);
    if let Some(sec) = &sec {
        must(bagholder_store::admin::upsert_securities(&db(), std::slice::from_ref(sec), &now_iso()));
    }
    sec
}

/// `bagholder.ticket_quote`.
pub fn ticket_quote(symbol: &str, security_id: &str, account_id: &str, exchange: &str) -> Value {
    let name_of = || if symbol.is_empty() { security_id.to_string() } else { symbol.to_string() };
    let mut sec = resolve_security(symbol, security_id);
    if sec.is_none() && exchange.is_empty() {
        return json!({"ok": false, "error": format!("No listing stored for {}.", name_of())});
    }
    let sess = match ticket_session() {
        Some(s) => s,
        None => return json!({"ok": false, "error": "Not connected."}),
    };
    if sec.is_none() {
        sec = lookup_listing(&sess, &symbol.trim().to_uppercase(), exchange);
    }
    let sec = match sec {
        Some(s) => s,
        None => return json!({"ok": false, "error": format!("No listing stored for {}.", name_of())}),
    };
    let sid = f(&sec, "id");
    let mut quotes = match fetch_quotes(&sess, &[sid.clone()]) {
        Ok(q) => q,
        Err(CallError::NotAuthorized) => return json!({"ok": false, "error": "Wealthsimple refused the session. Connect Wealthsimple again."}),
        Err(e) => return json!({"ok": false, "error": format!("Quote failed: {}", err_text(&e))}),
    };
    let mut quote = match quotes.remove(&sid) {
        Some(q) => q,
        None => {
            let label = if tr(&sec, "symbol") { f(&sec, "symbol") } else { sid.clone() };
            return json!({"ok": false, "error": format!("Wealthsimple has no quote for {}.", label)});
        }
    };
    for (qk, sk) in [("symbol", "symbol"), ("name", "name"), ("exchange", "primaryExchange")] {
        if f(&quote, qk).is_empty() {
            set(&mut quote, qk, json!(f(&sec, sk)));
        }
    }
    if f(&quote, "currency").is_empty() {
        set(&mut quote, "currency", json!(f(&sec, "currency").to_uppercase()));
    }
    let mut md = json!({"orderTypes": ORDER_EXEC_TYPES, "marginRate": null});
    match gql(&sess, "FetchSecurityMarketData", json!({"id": sid})) {
        Ok(d) => md = parse_market_data(&d),
        Err(e) => log(&format!("bagholder ticket: market data for {} failed: {}", sid, e)),
    }
    let accounts = order_accounts(None);
    let acct = accounts.iter().find(|a| f(a, "id") == account_id).cloned();
    let mut balance = json!({"buyingPower": null, "cash": null, "currency": ""});
    if let Some(a) = &acct {
        let cur = if f(&quote, "currency").is_empty() { "CAD".to_string() } else { f(&quote, "currency") };
        match gql(&sess, "FetchTradingBalanceBuyingPower", json!({"accountCanonicalId": f(a, "id"), "currency": cur, "securityId": sid})) {
            Ok(d) => balance = parse_buying_power(&d),
            Err(e) => log(&format!("bagholder ticket: buying power for {} failed: {}", f(a, "id"), e)),
        }
    }
    let mut margin_available = Value::Null;
    if let Some(a) = &acct {
        if tr(a, "marginAccountId") {
            let snap = snapshot();
            for m in snap.get("margin").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
                if f(&m, "accountId") == f(a, "marginAccountId") && m.get("buyingPower").map_or(false, |v| !v.is_null()) {
                    margin_available = jo(on(&m, "buyingPower"));
                }
            }
        }
    }
    let fx_map = must(bagholder_store::tables::fx_rates(&db(), "USDCAD"));
    let fx_usd_cad = if fx_map.is_empty() {
        Value::Null
    } else {
        let fx: bagholder_model::fx::Fx = fx_map.iter().filter_map(|(k, v)| v.as_f64().map(|x| (k.clone(), x))).collect();
        json!(bagholder_model::fx::rate_on(&fx, &bagholder_model::clock::today_local()))
    };
    let order_types = match md.get("orderTypes") {
        Some(Value::Array(a)) if !a.is_empty() => Value::Array(a.clone()),
        _ => json!(ORDER_EXEC_TYPES),
    };
    json!({
        "ok": true,
        "quote": quote,
        "orderTypes": order_types,
        "marginRate": gv(&md, "marginRate"),
        "accounts": accounts,
        "account": acct,
        "buyingPower": gv(&balance, "buyingPower"),
        "cash": gv(&balance, "cash"),
        "marginAvailable": margin_available,
        "fxUsdCad": fx_usd_cad,
        "live": orders_live(),
    })
}

/// `bagholder.order_tick`.
pub fn order_tick(price: Option<f64>) -> Option<f64> {
    price.map(|p| py_round(p, if p >= 1.0 { 2 } else { 4 }))
}

/// `bagholder.order_request`: (row, request) or the error.
pub fn order_request(body: &Value) -> Result<(Value, Value), String> {
    let empty = json!({});
    let b = if body.is_object() { body } else { &empty };
    let side = f(b, "side").to_uppercase();
    if side != "BUY" && side != "SELL" {
        return Err("Side must be Buy or Sell.".into());
    }
    let exec_type = f(b, "type").to_uppercase();
    if !ORDER_EXEC_TYPES.contains(&exec_type.as_str()) {
        return Err("Order type must be Market, Limit, Stop or Stop limit.".into());
    }
    let tif = s(or_v(b.get("tif"), Some(&json!("DAY")))).to_uppercase();
    if !ORDER_TIFS.contains(&tif.as_str()) {
        return Err("Time in force must be Day or Good till cancelled.".into());
    }
    let qty = num(b.get("quantity"), Some(0.0)).unwrap_or(0.0);
    if qty == 0.0 || qty <= 0.0 {
        return Err("Quantity must be more than zero.".into());
    }
    let limit_price = order_tick(on(b, "limitPrice"));
    let stop_price = order_tick(on(b, "stopPrice"));
    let positive = |p: Option<f64>| p.map_or(false, |x| x != 0.0 && x > 0.0);
    if (exec_type == "LIMIT" || exec_type == "STOP_LIMIT") && !positive(limit_price) {
        return Err("A limit price is required.".into());
    }
    if (exec_type == "STOP" || exec_type == "STOP_LIMIT") && !positive(stop_price) {
        return Err("A stop price is required.".into());
    }
    let acct = match order_accounts(None).into_iter().find(|a| f(a, "id") == f(b, "accountId")) {
        Some(a) => a,
        None => return Err("Choose an account.".into()),
    };
    let sec = match resolve_security(&f(b, "symbol"), &f(b, "securityId")) {
        Some(s) => s,
        None => return Err(format!("No listing stored for {}.", f(b, "symbol"))),
    };
    let mut sl = b.get("stopLoss").filter(|v| v.is_object()).cloned();
    let mut tp = b.get("takeProfit").filter(|v| v.is_object()).cloned();
    if side == "SELL" {
        sl = None;
        tp = None;
    }
    let sl_row = match sl.filter(|v| truthy(Some(v))) {
        None => Value::Null,
        Some(sl) => {
            let kind = s(or_v(sl.get("kind"), Some(&json!("stop")))).to_lowercase();
            if kind != "stop" && kind != "trail" {
                return Err("Stop loss type must be Stop or Trailing stop.".into());
            }
            if kind == "stop" && !(num(sl.get("price"), Some(0.0)).unwrap_or(0.0) > 0.0) {
                return Err("A stop loss price is required.".into());
            }
            if kind == "trail" && !(num(sl.get("trail"), Some(0.0)).unwrap_or(0.0) > 0.0) {
                return Err("A trail is required.".into());
            }
            json!({"kind": kind, "price": jo(order_tick(on(&sl, "price"))), "trail": jo(on(&sl, "trail")),
                   "trailUnit": if f(&sl, "trailUnit").to_lowercase() == "amt" { "amt" } else { "pct" }})
        }
    };
    let tp_row = match tp.filter(|v| truthy(Some(v))) {
        None => Value::Null,
        Some(tp) => {
            if !(num(tp.get("price"), Some(0.0)).unwrap_or(0.0) > 0.0) {
                return Err("A take profit price is required.".into());
            }
            json!({"price": jo(order_tick(on(&tp, "price")))})
        }
    };
    let oid = format!("order-{}", uuid4());
    let mut req = json!({
        "canonicalAccountId": f(&acct, "id"),
        "externalId": oid,
        "executionType": exec_type,
        "orderType": format!("{}_QUANTITY", side),
        "quantity": qty,
        "securityId": f(&sec, "id"),
        "timeInForce": tif,
    });
    if exec_type == "LIMIT" || exec_type == "STOP_LIMIT" {
        set(&mut req, "limitPrice", jo(limit_price));
    }
    if exec_type == "STOP" || exec_type == "STOP_LIMIT" {
        set(&mut req, "stopPrice", jo(stop_price));
    }
    let row = json!({
        "id": oid,
        "createdAt": now_iso(),
        "accountId": f(&acct, "id"),
        "account": f(&acct, "name"),
        "securityId": f(&sec, "id"),
        "symbol": f(&sec, "symbol"),
        "currency": s(or_v(b.get("currency"), sec.get("currency"))).to_uppercase(),
        "side": side,
        "type": exec_type,
        "quantity": qty,
        "limitPrice": gv(&req, "limitPrice"),
        "stopPrice": gv(&req, "stopPrice"),
        "tif": tif,
        "stopLoss": sl_row,
        "takeProfit": tp_row,
        "status": "",
        "wsOrderId": "",
        "error": "",
        "request": req.clone(),
    });
    Ok((row, req))
}

/// `bagholder.submit_order`.
pub fn submit_order(row: &mut Value, req: &Value) -> Value {
    let id = f(row, "id");
    if !orders_live() {
        set(row, "status", json!("dry"));
        insert_order(row);
        log(&format!("bagholder order (dry run, not sent): {}", bagholder_store::tables::py_json_sorted(req)));
        return json!({"ok": true, "id": id, "status": "dry", "order": row.clone()});
    }
    let sess = match ticket_session() {
        Some(s) => s,
        None => return json!({"ok": false, "error": "Not connected."}),
    };
    set(row, "status", json!("sending"));
    insert_order(row);
    let data = match gql(&sess, "SoOrdersOrderCreate", json!({"input": req})) {
        Ok(d) => d,
        Err(CallError::NotAuthorized) => {
            update_order(&id, json!({"status": "failed", "error": "Wealthsimple refused the session."}));
            return json!({"ok": false, "error": "Wealthsimple refused the session. Connect Wealthsimple again.", "id": id});
        }
        Err(e) => {
            let msg = err_text(&e);
            update_order(&id, json!({"status": "failed", "error": msg}));
            log(&format!("bagholder order: {} failed: {}", id, msg));
            return json!({"ok": false, "error": format!("Order failed: {}", msg), "id": id});
        }
    };
    let empty = json!({});
    let result = data.get("soOrdersCreateOrder").filter(|v| truthy(Some(v))).unwrap_or(&empty);
    if let Some(msg) = result.get("errors").filter(|v| truthy(Some(v))).and_then(first_error) {
        update_order(&id, json!({"status": "rejected", "error": msg}));
        log(&format!("bagholder order: {} rejected: {}", id, msg));
        return json!({"ok": false, "error": format!("Wealthsimple rejected the order: {}", msg), "id": id});
    }
    let order = result.get("order").filter(|v| truthy(Some(v))).unwrap_or(&empty);
    let ws_id = f(order, "orderId");
    update_order(&id, json!({"status": "sent", "wsOrderId": ws_id}));
    log(&format!("bagholder order: {} sent, Wealthsimple order {}", id, ws_id));
    let rid = id.clone();
    spawn("bagholder-order-refresh", move || {
        let _ = catch_unwind(|| refresh_orders(&rid));
    });
    json!({"ok": true, "id": id, "status": "sent", "wsOrderId": ws_id})
}

/// `bagholder.place_order`.
pub fn place_order(body: &Value) -> Value {
    let (mut row, req) = match order_request(body) {
        Ok(x) => x,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    if f(&row, "side") == "SELL" {
        let mut left = or0(&row, "quantity");
        for b in brackets(&BRACKET_LIVE) {
            let st = f(&b, "status");
            if f(&b, "accountId") != f(&row, "accountId") || f(&b, "securityId") != f(&row, "securityId") || st == "waiting" || st == "closing" {
                continue;
            }
            let held = or0(&b, "quantity");
            if left >= held {
                end_bracket(&b, "sold from the ticket", "");
                await_cancels(&b, 8);
                left -= held;
            } else if left > 0.0 {
                release_shares(&b, left);
                await_cancels(&b, 8);
                left = 0.0;
            }
        }
    }
    let mut r = submit_order(&mut row, &req);
    if tr(&r, "ok") && (tr(&row, "stopLoss") || tr(&row, "takeProfit")) {
        let b = create_bracket(&row);
        set(&mut r, "bracketId", gv(&b, "id"));
    }
    r
}

// --- reading orders back ---

const ORDER_BRANCH: &str = "TR";
pub const WS_PENDING: [&str; 8] = ["NEW", "PENDING_SUBMISSION", "PENDING_REVIEW", "PENDING_FUND_TRANSFER", "SUBMITTED", "PLACED", "PARTIALLY_FILLED", "CONTINGENT"];
const WS_CANCELLING: [&str; 1] = ["CANCEL_PENDING"];
pub const LIVE_STATUSES: [&str; 3] = ["sent", "pending", "cancelling"];
pub const ORDERS_REFRESH_SEC: u64 = 30;

fn ws_status_map(s: &str) -> Option<&'static str> {
    Some(match s {
        "FILLED" | "POSTED" => "filled",
        "CANCELLED" | "DELETED" => "cancelled",
        "EXPIRED" => "expired",
        "REJECTED" => "rejected",
        _ => return None,
    })
}

fn is_live(o: &Value) -> bool {
    LIVE_STATUSES.contains(&f(o, "status").as_str())
}

fn qty_words(q: Option<f64>) -> String {
    let q = or_f(q, Some(0.0)).unwrap_or(0.0);
    if q.fract() == 0.0 {
        format!("{}", q as i64)
    } else {
        py_g(q)
    }
}

fn price_words(p: Option<f64>) -> String {
    match p {
        None => "—".into(),
        Some(p) => {
            if p.abs() < 1.0 && py_round(p, 3) != py_round(p, 2) {
                format!("{:.3}", p)
            } else {
                format!("{:.2}", p)
            }
        }
    }
}

fn order_words(o: &Value) -> String {
    let side = if f(o, "side") == "BUY" { "Buy" } else { "Sell" };
    let how = match f(o, "type").as_str() {
        "MARKET" => "at market".to_string(),
        "STOP" => format!("stop {}", price_words(on(o, "stopPrice"))),
        "STOP_LIMIT" => format!("stop {} · limit {}", price_words(on(o, "stopPrice")), price_words(on(o, "limitPrice"))),
        _ => format!("at {} limit", price_words(on(o, "limitPrice"))),
    };
    format!("{} {} {}", side, qty_words(on(o, "quantity")), how)
}

/// `bagholder.order_notice`: (kind, key, title, body).
pub fn order_notice(before: &Value, upd: &Value) -> Option<(String, String, String, String)> {
    let (was, now) = (f(before, "status"), f(upd, "status"));
    let sym = if tr(before, "symbol") { f(before, "symbol") } else { "?".into() };
    let role = if tr(before, "role") { f(before, "role") } else { "entry".into() };
    let acct = f(before, "account");
    let tail = if acct.is_empty() { String::new() } else { format!(" · {}", acct) };
    let oid = f(before, "id");
    let qty = or_f(or_f(on(upd, "filledQty"), on(before, "filledQty")), num(before.get("quantity"), Some(0.0)));
    let px = or_f(on(upd, "avgFill"), on(before, "avgFill"));
    let at = match px {
        Some(p) if p != 0.0 => format!(" at {}", price_words(Some(p))),
        _ => String::new(),
    };
    if now == "filled" && was != "filled" {
        let did = if f(before, "side") == "SELL" { "Sold " } else { "Bought " };
        let head = match role.as_str() {
            "stop" => "Stopped out · ",
            "target" => "Target hit · ",
            _ => "Order filled · ",
        };
        return Some(("fills".into(), format!("order:{}:filled", oid), format!("{}{}", head, sym), format!("{}{}{}{}", did, qty_words(qty), at, tail)));
    }
    if (now == "rejected" || now == "failed") && was != "rejected" && was != "failed" {
        let reason = if tr(upd, "error") { f(upd, "error") } else { f(before, "error") };
        let title = format!("{}{}", if now == "rejected" { "Order rejected · " } else { "Order not sent · " }, sym);
        let body = format!("{}{}", order_words(before), if reason.is_empty() { tail.clone() } else { format!(" · {}", reason) });
        return Some(("problems".into(), format!("order:{}:{}", oid, now), title, body));
    }
    if role == "stop" || role == "target" {
        return None;
    }
    if now == "expired" && was != "expired" {
        return Some(("problems".into(), format!("order:{}:expired", oid), format!("Order expired · {}", sym), format!("{}{}", order_words(before), tail)));
    }
    if now == "cancelled" && was != "cancelled" && was != "cancelling" {
        return Some(("problems".into(), format!("order:{}:cancelled", oid), format!("Order cancelled · {}", sym), format!("{}{}", order_words(before), tail)));
    }
    let filled = or_f(num(upd.get("filledQty"), Some(0.0)), Some(0.0)).unwrap_or(0.0);
    let before_filled = or_f(num(before.get("filledQty"), Some(0.0)), Some(0.0)).unwrap_or(0.0);
    let before_qty = or_f(num(before.get("quantity"), Some(0.0)), Some(0.0)).unwrap_or(0.0);
    if LIVE_STATUSES.contains(&now.as_str()) && filled > before_filled && filled < before_qty {
        return Some((
            "fills".into(),
            format!("order:{}:partial:{}", oid, qty_words(Some(filled))),
            format!("Partly filled · {}", sym),
            format!("{} of {}{}{}", qty_words(Some(filled)), qty_words(on(before, "quantity")), at, tail),
        ));
    }
    None
}

/// `bagholder.app_status`.
pub fn app_status(ws_status: &str) -> String {
    let s = ws_status.to_uppercase();
    if WS_PENDING.contains(&s.as_str()) {
        return "pending".into();
    }
    if WS_CANCELLING.contains(&s.as_str()) {
        return "cancelling".into();
    }
    match ws_status_map(&s) {
        Some(x) => x.into(),
        None => if s.is_empty() { String::new() } else { "pending".into() },
    }
}

/// `bagholder.parse_extended_order`.
pub fn parse_extended_order(data: &Value) -> Option<Value> {
    let o = data.get("soOrdersExtendedOrder")?;
    if !o.is_object() || !tr(o, "status") {
        return None;
    }
    Some(json!({
        "wsStatus": f(o, "status").to_uppercase(),
        "status": app_status(&f(o, "status")),
        "filledQty": jo(on(o, "filledQuantity")),
        "avgFill": jo(on(o, "averageFilledPrice")),
        "submittedAt": f(o, "submittedAtUtc"),
        "expiresAt": f(o, "expiredAtUtc"),
        "firstFilledAt": f(o, "firstFilledAtUtc"),
        "lastFilledAt": f(o, "lastFilledAtUtc"),
        "error": s(or_v(o.get("rejectionCause"), o.get("rejectionCode"))),
        "quantity": jo(on(o, "submittedQuantity")),
        "limitPrice": jo(on(o, "limitPrice")),
        "stopPrice": jo(on(o, "stopPrice")),
        "tif": f(o, "timeInForce").to_uppercase(),
        "currency": f(o, "securityCurrency").to_uppercase(),
        "accountId": s(or_v(o.get("canonicalAccountId"), o.get("accountId"))),
        "securityId": f(o, "securityId"),
        "type": f(o, "orderType").to_uppercase(),
    }))
}

fn fetch_extended_order(sess: &Value, external_id: &str) -> Result<Option<Value>, CallError> {
    Ok(parse_extended_order(&gql(sess, "FetchSoOrdersExtendedOrder", json!({"branchId": ORDER_BRANCH, "externalId": external_id}))?))
}

fn fetch_order_feed(sess: &Value, identity: &str) -> Result<Vec<Value>, CallError> {
    let mut out = Vec::new();
    let mut cursor = Value::Null;
    loop {
        let data = gql(sess, "OrderServiceExtendedOrderFeed", json!({"identityId": identity, "statuses": WS_PENDING, "first": 25, "cursor": cursor}))?;
        let empty = json!({});
        let feed = data.get("identity").and_then(|v| v.get("orderServiceExtendedOrderFeed")).filter(|v| v.is_object()).unwrap_or(&empty);
        if let Some(edges) = feed.get("edges").and_then(|v| v.as_array()) {
            for edge in edges {
                if let Some(node) = edge.get("node") {
                    if node.is_object() && tr(node, "id") {
                        out.push(node.clone());
                    }
                }
            }
        }
        let page = feed.get("pageInfo").filter(|v| v.is_object()).unwrap_or(&empty);
        cursor = gv(page, "endCursor");
        if !tr(page, "hasNextPage") || !truthy(Some(&cursor)) {
            break;
        }
    }
    Ok(out)
}

fn feed_order_row(node: &Value) -> Value {
    let empty = json!({});
    let sec = node.get("security").filter(|v| v.is_object()).unwrap_or(&empty);
    let stock = sec.get("stock").filter(|v| v.is_object()).unwrap_or(&empty);
    let acct = order_accounts(None).into_iter().find(|a| f(a, "id") == f(node, "canonicalAccountId"));
    let side = f(node, "side").to_uppercase();
    let sec_id = s(or_v(node.get("securityId"), sec.get("id")));
    let mut symbol = must(so::symbol_for_security(&db(), &sec_id));
    if symbol.is_empty() {
        symbol = s(or_v(node.get("symbol"), stock.get("symbol")));
    }
    let typ = f(node, "executionType").to_uppercase();
    json!({
        "id": f(node, "id"),
        "createdAt": f(node, "createdAtUtc"),
        "accountId": f(node, "canonicalAccountId"),
        "account": acct.map(|a| f(&a, "name")).unwrap_or_default(),
        "securityId": sec_id,
        "symbol": symbol,
        "currency": f(node, "securityCurrency").to_uppercase(),
        "side": if side.starts_with("SELL") { "SELL" } else { "BUY" },
        "type": if typ.is_empty() { "LIMIT".to_string() } else { typ },
        "quantity": num(node.get("submittedQuantity"), Some(0.0)).unwrap_or(0.0),
        "limitPrice": jo(on(node, "limitPrice")),
        "stopPrice": jo(on(node, "stopPrice")),
        "tif": "",
        "stopLoss": null,
        "takeProfit": null,
        "status": app_status(&f(node, "status")),
        "wsStatus": f(node, "status").to_uppercase(),
        "wsOrderId": f(node, "orderId"),
        "avgFill": jo(on(node, "averageFillPrice")),
        "source": "wealthsimple",
    })
}

pub static REFRESHED_AT: Mutex<String> = Mutex::new(String::new());
static REFRESHING: AtomicBool = AtomicBool::new(false);

fn refreshed_at() -> String {
    REFRESHED_AT.lock().unwrap().clone()
}

/// `bagholder.kick_orders_refresh`.
pub fn kick_orders_refresh() -> bool {
    let at = refreshed_at();
    if !at.is_empty() {
        let age = match parse_z(&at) {
            Some(t) => now_unix() - t as f64,
            None => ORDERS_REFRESH_SEC as f64,
        };
        if age < ORDERS_REFRESH_SEC as f64 {
            return false;
        }
    }
    if !connected_not_syncing() {
        return false;
    }
    if REFRESHING.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return false;
    }
    spawn("bagholder-orders-refresh", || {
        let _ = catch_unwind(|| refresh_orders(""));
        REFRESHING.store(false, Ordering::SeqCst);
    });
    true
}

/// `bagholder.book_order_fill`.
pub fn book_order_fill(order: &Value, upd: &Value) -> bool {
    if !order.is_object() {
        return false;
    }
    if f(order, "source") == "wealthsimple" {
        return false;
    }
    let side = f(order, "side").to_uppercase();
    if side != "BUY" && side != "SELL" {
        return false;
    }
    let account_id = f(order, "accountId");
    if !bagholder_store::activities::is_real_account(&account_id) {
        return false;
    }
    if f(order, "securityId").is_empty() {
        return false;
    }
    let symbol = s(or_v(upd.get("symbol"), order.get("symbol"))).trim().to_string();
    if symbol.is_empty() {
        return false;
    }
    let pick = |k: &str| if upd.get(k).map_or(false, |v| !v.is_null()) { upd.get(k) } else { order.get(k) };
    let filled = or_f(num(pick("filledQty"), Some(0.0)), Some(0.0)).unwrap_or(0.0);
    let price = or_f(num(pick("avgFill"), Some(0.0)), Some(0.0)).unwrap_or(0.0);
    if filled <= 0.0 || price <= 0.0 {
        return false;
    }
    let already = or_f(num(order.get("fillBookedQty"), Some(0.0)), Some(0.0)).unwrap_or(0.0);
    if already + 1e-9 >= filled {
        return false;
    }
    let fill_time = or_v(or_v(upd.get("lastFilledAt"), upd.get("firstFilledAt")), order.get("submittedAt"));
    let mut date = date_only(fill_time);
    if date.is_empty() {
        date = crate::app::today_utc();
    }
    let mut currency = upper(or_v(or_v(order.get("currency"), upd.get("currency")), Some(&json!("CAD"))));
    if currency != "CAD" && currency != "USD" {
        currency = "CAD".into();
    }
    let accounts = snapshot().get("accounts").cloned().unwrap_or(json!([]));
    let mult = bagholder_model::symbols::option_multiplier(&symbol);
    let buy = side == "BUY";
    let fifo = bagholder_ws::mapping::fifo_pool_ids(Some(&accounts)).get(&account_id).cloned().unwrap_or_else(|| account_id.clone());
    let act = json!({
        "id": uuid4(),
        "occurredAt": date,
        "transactionDate": date,
        "settlementDate": date,
        "accountId": account_id,
        "bookId": account_id,
        "fifoId": fifo,
        "accountType": bagholder_ws::mapping::account_type(&account_id, Some(&accounts)),
        "activityType": "Trade",
        "activitySubType": side,
        "description": format!("{} {} {} @ {}", if buy { "Buy" } else { "Sell" }, qty_text(filled), symbol, rp(Some(price))),
        "direction": if buy { "DEBIT" } else { "CREDIT" },
        "symbol": symbol,
        "name": symbol,
        "currency": currency,
        "quantity": if buy { filled } else { -filled },
        "unitPrice": price,
        "commission": 0.0,
        "netCashAmount": if buy { -(filled * price * mult) } else { filled * price * mult },
        "category": "trade",
        "balance": null,
        "securityId": f(order, "securityId"),
        "source": "bagholder-fill",
    });
    let conn = db();
    must(bagholder_store::activities::insert_local(&conn, &act, &uuid4));
    must(so::mark_order_fill_booked(&conn, &f(order, "id"), filled, &now_iso()));
    app().invalidate();
    log(&format!("bagholder orders: {} filled {} {} @ {} booked as a local trade until the next sync", f(order, "id"), qty_text(filled), symbol, rp(Some(price))));
    true
}

/// `bagholder.refresh_orders`.
pub fn refresh_orders(only_id: &str) -> Value {
    let sess = match ticket_session() {
        Some(s) => s,
        None => return json!({"ok": false, "skipped": "no session"}),
    };
    let live: Vec<Value> = list_orders().into_iter().filter(|o| is_live(o) && (only_id.is_empty() || f(o, "id") == only_id)).collect();
    let (mut read, mut failed, mut added) = (0i64, 0i64, 0i64);
    for o in &live {
        let oid = f(o, "id");
        let upd = match fetch_extended_order(&sess, &oid) {
            Ok(u) => u,
            Err(CallError::NotAuthorized) => {
                log("bagholder orders: Wealthsimple refused the session");
                return json!({"ok": false, "skipped": "refused"});
            }
            Err(e) => {
                failed += 1;
                log(&format!("bagholder orders: {} status failed: {}", oid, err_text(&e)));
                continue;
            }
        };
        let upd = match upd {
            Some(u) => u,
            None => continue,
        };
        let mut patch = Map::new();
        for k in ["wsStatus", "status", "filledQty", "avgFill", "submittedAt", "expiresAt"] {
            if upd.get(k).map_or(false, |v| !v.is_null()) {
                patch.insert(k.into(), upd[k].clone());
            }
        }
        if tr(&upd, "error") {
            patch.insert("error".into(), upd["error"].clone());
        }
        let role = f(o, "role");
        if f(o, "source") == "wealthsimple" || role == "stop" || role == "target" {
            for k in ["tif", "quantity", "limitPrice", "stopPrice", "currency"] {
                match upd.get(k) {
                    None | Some(Value::Null) => {}
                    Some(Value::String(t)) if t.is_empty() => {}
                    Some(v) => {
                        patch.insert(k.into(), v.clone());
                    }
                }
            }
            let name = must(so::symbol_for_security(&db(), &f(o, "securityId")));
            if !name.is_empty() && name != f(o, "symbol") {
                patch.insert("symbol".into(), json!(name));
            }
        }
        let notice = order_notice(o, &upd);
        update_order(&oid, Value::Object(patch));
        read += 1;
        if let Some((kind, key, title, body)) = notice {
            emit(&kind, &key, &title, &body);
        }
        if f(&upd, "status") == "filled" {
            let current = get_order(&oid).unwrap_or_else(|| o.clone());
            match catch_unwind(AssertUnwindSafe(|| book_order_fill(&current, &upd))) {
                Ok(_) => {}
                Err(_) => log(&format!("bagholder orders: {} fill not booked locally", oid)),
            }
        }
    }
    if only_id.is_empty() {
        let identity = identity_from(&sess);
        if !identity.is_empty() {
            let rows = list_orders();
            let mut known: HashSet<String> = rows.iter().map(|o| f(o, "id")).collect();
            known.extend(rows.iter().filter(|o| tr(o, "wsOrderId")).map(|o| f(o, "wsOrderId")));
            match fetch_order_feed(&sess, &identity) {
                Ok(nodes) => {
                    for node in nodes {
                        if known.contains(&f(&node, "id")) || known.contains(&f(&node, "orderId")) {
                            continue;
                        }
                        insert_order(&feed_order_row(&node));
                        added += 1;
                    }
                }
                Err(CallError::NotAuthorized) => return json!({"ok": false, "skipped": "refused"}),
                Err(e) => {
                    failed += 1;
                    log(&format!("bagholder orders: pending-order feed failed: {}", err_text(&e)));
                }
            }
        }
        *REFRESHED_AT.lock().unwrap() = now_iso();
    }
    if read != 0 || added != 0 || failed != 0 {
        log(&format!("bagholder orders: {} read, {} found pending at Wealthsimple, {} failed", read, added, failed));
    }
    json!({"ok": failed == 0, "read": read, "added": added, "failed": failed})
}

/// `bagholder.orders_loop`.
pub fn orders_loop() {
    while !app().wait(Duration::from_secs(ORDERS_REFRESH_SEC)) {
        if !connected_not_syncing() {
            continue;
        }
        let r = catch_unwind(|| {
            if !list_orders().iter().any(is_live) && !refreshed_at().is_empty() && (now_unix() / ORDERS_REFRESH_SEC as f64) as i64 % 10 != 0 {
                return;
            }
            refresh_orders("");
        });
        if r.is_err() {
            log("bagholder orders: refresh failed");
        }
    }
}

/// `bagholder.cancel_order`.
pub fn cancel_order(order_id: &str) -> Value {
    #[cfg(test)]
    if let Some(v) = bracket_seam::CANCEL_ORDER.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return v;
    }
    let row = match get_order(order_id) {
        Some(r) => r,
        None => return json!({"ok": false, "error": "No such order."}),
    };
    if !is_live(&row) {
        return json!({"ok": false, "error": "That order is not open."});
    }
    if !orders_live() {
        return json!({"ok": false, "error": "Orders are off (BAGHOLDER_DRY_ORDERS): nothing is sent to Wealthsimple."});
    }
    let sess = match ticket_session() {
        Some(s) => s,
        None => return json!({"ok": false, "error": "Not connected."}),
    };
    let id = f(&row, "id");
    let data = match gql(&sess, "SoOrdersOrderCancel", json!({"cancelOrderRequest": {"externalId": id}})) {
        Ok(d) => d,
        Err(CallError::NotAuthorized) => return json!({"ok": false, "error": "Wealthsimple refused the session. Connect Wealthsimple again."}),
        Err(e) => {
            let msg = err_text(&e);
            log(&format!("bagholder orders: cancel {} failed: {}", id, msg));
            return json!({"ok": false, "error": format!("Cancel failed: {}", msg)});
        }
    };
    if let Some(msg) = data.get("orderServiceCancelOrder").and_then(|r| r.get("errors")).filter(|v| truthy(Some(v))).and_then(first_error) {
        log(&format!("bagholder orders: cancel {} refused: {}", id, msg));
        return json!({"ok": false, "error": format!("Wealthsimple refused the cancel: {}", msg)});
    }
    update_order(&id, json!({"status": "cancelling", "wsStatus": "CANCEL_PENDING"}));
    log(&format!("bagholder orders: cancel {} accepted", id));
    let rid = id.clone();
    spawn("bagholder-order-refresh", move || {
        let _ = catch_unwind(|| refresh_orders(&rid));
    });
    json!({"ok": true, "id": id, "status": "cancelling"})
}

/// `bagholder.orders_payload`.
pub fn orders_payload(kick: bool) -> Value {
    if kick {
        kick_orders_refresh();
    }
    let exchanges: HashMap<String, String> = must(bagholder_store::admin::list_securities(&db())).iter().map(|s| (f(s, "id"), f(s, "primaryExchange"))).collect();
    let mut orders = list_orders();
    for o in orders.iter_mut() {
        let ex = exchanges.get(&f(o, "securityId")).cloned().unwrap_or_default();
        set(o, "exchange", json!(ex));
    }
    json!({"ok": true, "orders": orders, "brackets": brackets(&[]), "live": orders_live(), "refreshedAt": refreshed_at()})
}

/// `bagholder.open_orders_count`.
pub fn open_orders_count() -> i64 {
    let entries = list_orders().iter().filter(|o| is_live(o) && (o.get("role").is_none() || f(o, "role") == "entry")).count();
    let live = brackets(&[]).iter().filter(|b| BRACKET_LIVE.contains(&f(b, "status").as_str()) && f(b, "status") != "waiting").count();
    (entries + live) as i64
}

// ---------------------------------------------------------------------------
// brackets
// ---------------------------------------------------------------------------

pub const BRACKET_POLL_SEC: u64 = 5;
const BRACKET_RETRY_SEC: [i64; 4] = [60, 300, 900, 3600];
const TRAIL_MIN_MOVE: f64 = 0.005;
const TARGET_BACK_OFF: f64 = 0.01;
pub const BRACKET_LIVE: [&str; 6] = ["waiting", "armed", "firing", "target_placed", "stopping", "closing"];
const BRACKET_RESTING: [&str; 2] = ["sent", "pending"];
const BRACKET_INFLIGHT: [&str; 3] = ["sent", "pending", "cancelling"];
const BRACKET_ROLL_SEC: f64 = 7.0 * 86400.0;
const BRACKET_ROLL_LAST_SEC: f64 = 2.0 * 86400.0;
const GTC_DAYS: i64 = 90;
const BRACKET_TIF: &str = "UNTIL_CANCEL";
const BRACKET_ENDED_QUIETLY: [&str; 5] = ["stopped", "target", "cancelled by the user", "both legs removed", "sold from the ticket"];

static BRACKET_LOCK: AtomicBool = AtomicBool::new(false);

pub fn bracket_said() -> &'static Mutex<HashSet<String>> {
    static SAID: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    SAID.get_or_init(|| Mutex::new(HashSet::new()))
}

pub fn stop_allowed_cache() -> &'static Mutex<HashMap<String, bool>> {
    static C: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

fn st_in(o: &Value, set: &[&str]) -> bool {
    set.contains(&f(o, "status").as_str())
}

fn release_shares(b: &Value, sold: f64) {
    let remaining = py_round(or0(b, "quantity") - sold, 6);
    for k in ["slOrderId", "tpOrderId"] {
        let oid = f(b, k);
        let err = cancel_exit(&oid);
        if !err.is_empty() {
            log(&format!("bagholder bracket: {} for {}: cancel of {} refused: {}", f(b, "id"), f(b, "symbol"), if oid.is_empty() { "None".into() } else { oid }, err));
        }
    }
    update_bracket(&f(b, "id"), json!({"quantity": remaining, "slOrderId": "", "tpOrderId": "", "status": "armed", "error": "", "attempts": 0}));
    log(&format!(
        "bagholder bracket: {} for {}: {} of its shares sold from the ticket; the stop is placed again on the {} left",
        f(b, "id"), f(b, "symbol"), qty_text(sold), qty_text(remaining)
    ));
}

fn await_cancels(b: &Value, seconds: u32) {
    if !orders_live() {
        return;
    }
    for _ in 0..seconds {
        let open: Vec<Value> = own_exit_rows(b).into_iter().filter(|o| st_in(o, &BRACKET_INFLIGHT)).collect();
        if open.is_empty() {
            return;
        }
        for o in &open {
            refresh_orders(&f(o, "id"));
        }
        #[cfg(not(test))]
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// `bagholder.create_bracket`.
pub fn create_bracket(order_row: &Value) -> Value {
    let empty = json!({});
    let sl = order_row.get("stopLoss").filter(|v| truthy(Some(v))).unwrap_or(&empty);
    let tp = order_row.get("takeProfit").filter(|v| truthy(Some(v))).unwrap_or(&empty);
    let id = format!("bracket-{}", uuid4());
    let unit = if tr(sl, "trailUnit") { f(sl, "trailUnit") } else { "pct".into() };
    let b = json!({
        "id": id, "orderId": f(order_row, "id"), "accountId": f(order_row, "accountId"), "securityId": f(order_row, "securityId"),
        "symbol": f(order_row, "symbol"), "currency": f(order_row, "currency"), "quantity": gv(order_row, "quantity"), "tif": BRACKET_TIF,
        "slKind": f(sl, "kind"), "slPrice": gv(sl, "price"), "slTrail": gv(sl, "trail"), "slTrailUnit": unit,
        "tpPrice": gv(tp, "price"), "status": "waiting",
    });
    must(so::insert_bracket(&db(), &b, &now_iso()));
    get_bracket(&id).unwrap_or(b)
}

/// Bracket test seams: `_stop_allowed`, `cancel_order`, the said lines and the caches.
#[cfg(test)]
pub mod bracket_seam {
    use super::*;
    pub static STOP_ALLOWED: Mutex<Option<bool>> = Mutex::new(None);
    pub static CANCEL_ORDER: Mutex<Option<Value>> = Mutex::new(None);
    pub static SAID: Mutex<Vec<String>> = Mutex::new(Vec::new());
    pub fn reset() {
        *STOP_ALLOWED.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *CANCEL_ORDER.lock().unwrap_or_else(|e| e.into_inner()) = None;
        SAID.lock().unwrap_or_else(|e| e.into_inner()).clear();
        bracket_said().lock().unwrap_or_else(|e| e.into_inner()).clear();
        stop_allowed_cache().lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

fn say_once(key: String, line: &str) {
    if !bracket_said().lock().unwrap().insert(key) {
        return;
    }
    #[cfg(test)]
    bracket_seam::SAID.lock().unwrap_or_else(|e| e.into_inner()).push(line.to_string());
    log(line);
}

fn trail_distance(b: &Value, price: f64) -> Option<f64> {
    if f(b, "slKind") != "trail" || !tr(b, "slTrail") {
        return None;
    }
    let t = or0(b, "slTrail");
    Some(if f(b, "slTrailUnit") == "pct" { price * t / 100.0 } else { t })
}

fn exit_body(b: &Value, exec_type: &str, price: Option<f64>, role: &str) -> Result<(Value, Value), String> {
    let mut body = json!({"symbol": gv(b, "symbol"), "securityId": gv(b, "securityId"), "accountId": gv(b, "accountId"), "side": "SELL", "type": exec_type, "tif": BRACKET_TIF,
        "quantity": gv(b, "quantity"), "currency": gv(b, "currency")});
    if exec_type == "LIMIT" {
        set(&mut body, "limitPrice", jo(price));
    }
    if exec_type == "STOP" {
        set(&mut body, "stopPrice", jo(price));
    }
    let (mut row, req) = order_request(&body)?;
    set(&mut row, "role", json!(role));
    set(&mut row, "parentId", gv(b, "orderId"));
    Ok((row, req))
}

/// (order id, error).
fn place_exit(b: &Value, exec_type: &str, price: Option<f64>, role: &str) -> (String, String) {
    let (mut row, req) = match exit_body(b, exec_type, price, role) {
        Ok(x) => x,
        Err(e) => return (String::new(), e),
    };
    if !orders_live() {
        let key = format!("{}|{}|{}", f(b, "id"), role, rp(price.map(|p| py_round(p, 4))));
        say_once(key, &format!("bagholder bracket (orders are off, not placed): {} {} for {}: {}", role, exec_type, f(b, "symbol"), bagholder_store::tables::py_json_sorted(&req)));
        return (String::new(), String::new());
    }
    let r = submit_order(&mut row, &req);
    if !tr(&r, "ok") {
        let e = f(&r, "error");
        return (String::new(), if e.is_empty() { "not sent".into() } else { e });
    }
    (f(&r, "id"), String::new())
}

fn cancel_exit(order_id: &str) -> String {
    if order_id.is_empty() {
        return String::new();
    }
    match get_order(order_id) {
        Some(row) if st_in(&row, &BRACKET_RESTING) => {}
        _ => return String::new(),
    }
    let r = cancel_order(order_id);
    let e = f(&r, "error");
    if tr(&r, "ok") || e.contains("not open") {
        return String::new();
    }
    if e.is_empty() {
        "cancel failed".into()
    } else {
        e
    }
}

fn exit_row(b: &Value, role: &str) -> Option<Value> {
    let held = f(b, if role == "stop" { "slOrderId" } else { "tpOrderId" });
    if !held.is_empty() {
        if let Some(row) = get_order(&held) {
            return Some(row);
        }
    }
    let parent = f(b, "orderId");
    list_orders().into_iter().find(|o| f(o, "parentId") == parent && f(o, "role") == role)
}

fn attempts_of(b: &Value) -> i64 {
    or0(b, "attempts") as i64
}

fn retry_wait(attempts: i64) -> i64 {
    BRACKET_RETRY_SEC[(attempts.min(BRACKET_RETRY_SEC.len() as i64) - 1).max(0) as usize]
}

fn may_retry(b: &Value) -> bool {
    let attempts = attempts_of(b);
    if attempts == 0 {
        return true;
    }
    let wait = retry_wait(attempts);
    match parse_z(&f(b, "updatedAt")) {
        Some(t) => now_unix() - t as f64 >= wait as f64,
        None => true,
    }
}

fn md5_8(msg: &str) -> String {
    openssl::hash::hash(openssl::hash::MessageDigest::md5(), msg.as_bytes())
        .map(|d| d.iter().map(|b| format!("{:02x}", b)).collect::<String>()[..8].to_string())
        .unwrap_or_default()
}

fn capitalized(t: &str) -> String {
    let mut c = t.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn fail(b: &Value, msg: &str) {
    let attempts = attempts_of(b) + 1;
    update_bracket(&f(b, "id"), json!({"error": msg, "attempts": attempts}));
    log(&format!("bagholder bracket: {} for {}: {} (attempt {}; next in {} s)", f(b, "id"), f(b, "symbol"), msg, attempts, retry_wait(attempts)));
    if attempts == 1 {
        let entry = get_order(&f(b, "orderId")).unwrap_or(json!({}));
        let acct = f(&entry, "account");
        emit(
            "problems",
            &format!("bracket:{}:fail:{}", f(b, "id"), md5_8(msg)),
            &format!("Bracket · {}", f(b, "symbol")),
            &format!("{} · trying again in a minute{}", capitalized(msg), if acct.is_empty() { String::new() } else { format!(" · {}", acct) }),
        );
    }
}

fn arm_step(b: &Value, entry: Option<&Value>) {
    let mut b = b.clone();
    let bid = f(&b, "id");
    if f(&b, "status") == "waiting" {
        let entry = match entry {
            None => {
                update_bracket(&bid, json!({"status": "cancelled", "outcome": "entry not found"}));
                return;
            }
            Some(e) => e,
        };
        let est = f(entry, "status");
        if ["pending", "sent", "cancelling", "dry"].contains(&est.as_str()) {
            return;
        }
        let mut filled = or0(entry, "filledQty");
        if est == "filled" && filled == 0.0 {
            filled = or0(entry, "quantity");
        }
        if filled == 0.0 || filled <= 0.0 {
            update_bracket(&bid, json!({"status": "cancelled", "outcome": format!("entry {}", est)}));
            log(&format!("bagholder bracket: {} for {} off: entry {} without a fill", bid, f(&b, "symbol"), est));
            return;
        }
        let armed_at = now_iso();
        set(&mut b, "quantity", json!(filled));
        set(&mut b, "status", json!("armed"));
        set(&mut b, "armedAt", json!(armed_at));
        let mut patch = json!({"quantity": filled, "status": "armed", "armedAt": armed_at});
        if f(&b, "slKind") == "trail" {
            let high = or_f(or_f(on(entry, "avgFill"), on(entry, "limitPrice")), on(&b, "slPrice"));
            if let Some(high) = high.filter(|h| *h != 0.0) {
                let sl_price = py_round(high - trail_distance(&b, high).unwrap_or(0.0), 2);
                set(&mut patch, "highWater", json!(high));
                set(&mut patch, "slPrice", json!(sl_price));
                set(&mut b, "highWater", json!(high));
                set(&mut b, "slPrice", json!(sl_price));
            }
        }
        update_bracket(&bid, patch);
        log(&format!("bagholder bracket: {} armed for {} x {}", bid, qty_text(filled), f(&b, "symbol")));
    }
    if f(&b, "status") != "armed" || !tr(&b, "slKind") || tr(&b, "slOrderId") {
        return;
    }
    if !nothing_resting(&b) {
        return;
    }
    let native = stop_allowed(&f(&b, "securityId"));
    if !native {
        if f(&b, "slMode") != "watched" {
            update_bracket(&bid, json!({"slMode": "watched", "slNative": false}));
            log(&format!("bagholder bracket: {} for {}: Wealthsimple takes no stop order for it; the stop is watched here", bid, f(&b, "symbol")));
        }
        return;
    }
    if !may_retry(&b) {
        return;
    }
    let (oid, err) = place_exit(&b, "STOP", on(&b, "slPrice"), "stop");
    if !err.is_empty() {
        fail(&b, &format!("stop not placed: {}", err));
    } else if !oid.is_empty() {
        update_bracket(&bid, json!({"slOrderId": oid, "slNative": true, "slMode": "native", "error": "", "attempts": 0, "movedAt": now_iso()}));
        log(&format!("bagholder bracket: {} stop placed at {} for {}", bid, rp(on(&b, "slPrice")), f(&b, "symbol")));
    }
}

fn stop_allowed(security_id: &str) -> bool {
    #[cfg(test)]
    if let Some(v) = *bracket_seam::STOP_ALLOWED.lock().unwrap_or_else(|e| e.into_inner()) {
        return v;
    }
    if let Some(v) = stop_allowed_cache().lock().unwrap().get(security_id) {
        return *v;
    }
    let sess = match ticket_session() {
        Some(s) => s,
        None => return false,
    };
    let ok = match gql(&sess, "FetchSecurityMarketData", json!({"id": security_id})) {
        Ok(d) => parse_market_data(&d)["orderTypes"].as_array().map_or(false, |a| a.iter().any(|t| t == "STOP")),
        Err(e) => {
            log(&format!("bagholder bracket: order types for {} unknown: {}", security_id, e));
            return false;
        }
    };
    stop_allowed_cache().lock().unwrap().insert(security_id.to_string(), ok);
    ok
}

fn own_exit_rows(b: &Value) -> Vec<Value> {
    let parent = f(b, "orderId");
    list_orders().into_iter().filter(|o| f(o, "parentId") == parent && (f(o, "role") == "stop" || f(o, "role") == "target")).collect()
}

fn end_bracket(b: &Value, outcome: &str, note: &str) -> String {
    let mut pending = false;
    for o in own_exit_rows(b) {
        if st_in(&o, &BRACKET_RESTING) {
            let err = cancel_exit(&f(&o, "id"));
            if !err.is_empty() {
                log(&format!("bagholder bracket: {} for {}: cancel of {} refused: {}; tried again on the next check", f(b, "id"), f(b, "symbol"), f(&o, "id"), err));
            }
            pending = true;
        } else if f(&o, "status") == "cancelling" {
            pending = true;
        }
    }
    let status = if pending { "closing" } else { "done" };
    update_bracket(&f(b, "id"), json!({"status": status, "outcome": outcome, "error": note, "slOrderId": "", "tpOrderId": ""}));
    log(&format!("bagholder bracket: {} for {}: {}{}", f(b, "id"), f(b, "symbol"), outcome, if pending { "; its resting exit is being cancelled" } else { "" }));
    if !BRACKET_ENDED_QUIETLY.contains(&outcome) {
        let entry = get_order(&f(b, "orderId")).unwrap_or(json!({}));
        let acct = f(&entry, "account");
        emit(
            "problems",
            &format!("bracket:{}:off", f(b, "id")),
            &format!("Bracket off · {}", f(b, "symbol")),
            &format!("{}{}", capitalized(outcome), if acct.is_empty() { String::new() } else { format!(" · {}", acct) }),
        );
    }
    status.to_string()
}

fn closing_step(b: &Value) {
    let open: Vec<Value> = own_exit_rows(b).into_iter().filter(|o| st_in(o, &BRACKET_INFLIGHT)).collect();
    for o in &open {
        if st_in(o, &BRACKET_RESTING) {
            let err = cancel_exit(&f(o, "id"));
            if !err.is_empty() {
                log(&format!("bagholder bracket: {} for {}: cancel of {} refused again: {}", f(b, "id"), f(b, "symbol"), f(o, "id"), err));
            }
        }
    }
    if open.is_empty() {
        update_bracket(&f(b, "id"), json!({"status": "done"}));
        log(&format!("bagholder bracket: {} for {}: nothing rests at Wealthsimple; done", f(b, "id"), f(b, "symbol")));
    }
}

fn sweep_exits() {
    for o in list_orders() {
        let role = f(&o, "role");
        if (role != "stop" && role != "target") || !st_in(&o, &BRACKET_RESTING) {
            continue;
        }
        let b = must(so::bracket_for_order(&db(), &f(&o, "parentId")));
        let held_by = b.as_ref().map_or(false, |b| {
            st_in(b, &BRACKET_LIVE) && (f(b, "status") == "closing" || f(&o, "id") == f(b, "slOrderId") || f(&o, "id") == f(b, "tpOrderId"))
        });
        if held_by {
            continue;
        }
        let err = cancel_exit(&f(&o, "id"));
        say_once(
            format!("{}|orphan", f(&o, "id")),
            &format!(
                "bagholder bracket: {} for {} rests at Wealthsimple with no bracket holding it; cancelled{}\n",
                f(&o, "id"),
                if o.get("symbol").map_or(true, |v| v.is_null()) { "None".into() } else { f(&o, "symbol") },
                if err.is_empty() { String::new() } else { format!(" (refused: {})", err) }
            ),
        );
    }
}

fn nothing_resting(b: &Value) -> bool {
    !own_exit_rows(b).iter().any(|o| st_in(o, &BRACKET_INFLIGHT))
}

fn closed_elsewhere(b: &Value) -> String {
    if !["armed", "firing", "target_placed", "stopping"].contains(&f(b, "status").as_str()) || !tr(b, "armedAt") || !nothing_resting(b) {
        return String::new();
    }
    let conn = db();
    let armed_at = f(b, "armedAt");
    let sold = must(bagholder_store::feeds::sold_since(&conn, &f(b, "accountId"), &f(b, "securityId"), &armed_at, &f(b, "symbol")));
    if sold != 0.0 && sold >= or0(b, "quantity") {
        return format!("sold: {} shares in the activity feed", qty_text(sold));
    }
    let read_at = must(bagholder_store::tables::get_meta(&conn, "balances_read_at", ""));
    if read_at.is_empty() || read_at <= armed_at {
        return String::new();
    }
    let held = must(bagholder_store::feeds::position_quantity(&conn, &f(b, "accountId"), &f(b, "securityId")));
    if let Some(h) = held {
        if h > 0.0 {
            if !tr(b, "seenHeld") || tr(b, "missedAt") {
                update_bracket(&f(b, "id"), json!({"seenHeld": true, "missedAt": ""}));
            }
            return String::new();
        }
    }
    if !tr(b, "seenHeld") {
        return String::new();
    }
    let missed = f(b, "missedAt");
    if missed.is_empty() {
        update_bracket(&f(b, "id"), json!({"missedAt": read_at}));
        log(&format!("bagholder bracket: {} for {}: the balances read at {} does not list the position; a second read decides", f(b, "id"), f(b, "symbol"), read_at));
        return String::new();
    }
    if read_at > missed {
        return format!("position gone: two balance reads without it ({}, {})", missed, read_at);
    }
    String::new()
}

fn expires_in(row: &Value, now: f64) -> Option<f64> {
    let mut exp = parse_utc(row.get("expiresAt"));
    if exp.is_none() && f(row, "tif").to_uppercase() == "UNTIL_CANCEL" {
        let sub = parse_utc(or_v(row.get("submittedAt"), row.get("createdAt")));
        exp = sub.map(|t| t + GTC_DAYS * 86400);
    }
    exp.map(|e| e as f64 - now)
}

fn roll_due(row: Option<&Value>, quote: Option<&Value>, now: f64) -> bool {
    let row = match row {
        Some(r) if st_in(r, &["sent", "pending"]) => r,
        _ => return false,
    };
    let left = match expires_in(row, now) {
        Some(l) if l <= BRACKET_ROLL_SEC => l,
        _ => return false,
    };
    if left <= BRACKET_ROLL_LAST_SEC {
        return true;
    }
    quote.map(|q| f(q, "marketStatus").to_uppercase()).unwrap_or_default() != "OPEN"
}

fn roll_step(b: &Value, quote: Option<&Value>) {
    let now = now_unix().floor();
    let now_s = now_iso();
    let bid = f(b, "id");
    let status = f(b, "status");
    if status == "armed" && f(b, "slMode") == "native" && tr(b, "slOrderId") {
        let row = get_order(&f(b, "slOrderId"));
        if roll_due(row.as_ref(), quote, now) {
            let err = cancel_exit(&f(b, "slOrderId"));
            if !err.is_empty() {
                fail(b, &format!("stop not rolled: {}", err));
                return;
            }
            update_bracket(&bid, json!({"slOrderId": "", "movedAt": now_s, "error": ""}));
            log(&format!("bagholder bracket: {} for {}: stop at {} nears Wealthsimple's ninety days; cancelled, placed again at the same level", bid, f(b, "symbol"), rp(on(b, "slPrice"))));
        }
    } else if status == "target_placed" {
        if tr(b, "tpOrderId") {
            let row = get_order(&f(b, "tpOrderId"));
            if roll_due(row.as_ref(), quote, now) {
                let err = cancel_exit(&f(b, "tpOrderId"));
                if !err.is_empty() {
                    fail(b, &format!("target not rolled: {}", err));
                    return;
                }
                update_bracket(&bid, json!({"tpOrderId": "", "movedAt": now_s, "error": ""}));
                log(&format!("bagholder bracket: {} for {}: target at {} nears Wealthsimple's ninety days; cancelled, placed again", bid, f(b, "symbol"), rp(on(b, "tpPrice"))));
            }
        } else if let Some(tp_row) = exit_row(b, "target") {
            if st_in(&tp_row, &["cancelled", "expired"]) {
                fire_target(b);
            }
        }
    }
}

fn reconcile_step(b: &Value, _entry: Option<&Value>) -> &'static str {
    let mut b = b.clone();
    let bid = f(&b, "id");
    if f(&b, "status") == "closing" {
        closing_step(&b);
        return "done";
    }
    let (stop_row, tp_row) = (exit_row(&b, "stop"), exit_row(&b, "target"));
    if stop_row.as_ref().map_or(false, |r| f(r, "status") == "filled") {
        end_bracket(&b, "stopped", "");
        return "done";
    }
    if tp_row.as_ref().map_or(false, |r| f(r, "status") == "filled") {
        end_bracket(&b, "target", "");
        return "done";
    }
    if let Some(sr) = &stop_row {
        if tr(&b, "slOrderId") && st_in(sr, &BRACKET_RESTING) && tr(sr, "stopPrice") && tr(&b, "slPrice") && (or0(sr, "stopPrice") - or0(&b, "slPrice")).abs() > 0.005 {
            update_bracket(&bid, json!({"slPrice": gv(sr, "stopPrice")}));
            log(&format!("bagholder bracket: {} for {}: stop moved by hand to {}; the bracket follows", bid, f(&b, "symbol"), rp(on(sr, "stopPrice"))));
            set(&mut b, "slPrice", gv(sr, "stopPrice"));
        }
    }
    if let Some(tp) = &tp_row {
        if tr(&b, "tpOrderId") && st_in(tp, &BRACKET_RESTING) && tr(tp, "limitPrice") && tr(&b, "tpPrice") && (or0(tp, "limitPrice") - or0(&b, "tpPrice")).abs() > 0.005 {
            update_bracket(&bid, json!({"tpPrice": gv(tp, "limitPrice")}));
            log(&format!("bagholder bracket: {} for {}: target moved by hand to {}; the bracket follows", bid, f(&b, "symbol"), rp(on(tp, "limitPrice"))));
        }
    }
    let status = f(&b, "status");
    let native_stop = status == "armed" && f(&b, "slMode") == "native" && tr(&b, "slOrderId");
    if native_stop && stop_row.as_ref().map_or(false, |r| f(r, "status") == "expired") {
        update_bracket(&bid, json!({"slOrderId": "", "error": ""}));
        log(&format!("bagholder bracket: {} for {}: stop expired at Wealthsimple; placed again", bid, f(&b, "symbol")));
    } else if let Some(sr) = stop_row.as_ref().filter(|r| native_stop && st_in(r, &["cancelled", "rejected", "failed"])) {
        let why = if f(sr, "status") == "cancelled" {
            "stop cancelled at Wealthsimple by hand".to_string()
        } else {
            format!("stop {} at Wealthsimple{}", f(sr, "status"), if tr(sr, "error") { format!(": {}", f(sr, "error")) } else { String::new() })
        };
        end_bracket(&b, &why, "");
        return "done";
    }
    let tp_placed = status == "target_placed" && tr(&b, "tpOrderId");
    if tp_placed && tp_row.as_ref().map_or(false, |r| f(r, "status") == "expired") {
        update_bracket(&bid, json!({"tpOrderId": "", "error": ""}));
        log(&format!("bagholder bracket: {} for {}: target expired at Wealthsimple; placed again", bid, f(&b, "symbol")));
    } else if let Some(tp) = tp_row.as_ref().filter(|r| tp_placed && st_in(r, &["cancelled", "rejected", "failed"])) {
        let why = if f(tp, "status") == "cancelled" {
            "target cancelled at Wealthsimple by hand".to_string()
        } else {
            format!("target {} at Wealthsimple{}", f(tp, "status"), if tr(tp, "error") { format!(": {}", f(tp, "error")) } else { String::new() })
        };
        end_bracket(&b, &why, "");
        return "done";
    }
    let why = closed_elsewhere(&b);
    if !why.is_empty() {
        end_bracket(&b, &why, "");
        return "done";
    }
    ""
}

fn watch_step(b: &Value, quote: Option<&Value>) {
    let quote = match quote {
        Some(q) if f(q, "marketStatus").to_uppercase() == "OPEN" => q,
        _ => return,
    };
    let (last, bid_px) = (on(quote, "last"), on(quote, "bid"));
    let last = match last {
        Some(l) => l,
        None => return,
    };
    let mut b = b.clone();
    let id = f(&b, "id");
    let sym = f(&b, "symbol");
    let now_s = now_iso();
    let trigger = bid_px.unwrap_or(last);
    let at_target = tr(&b, "tpPrice") && trigger >= or0(&b, "tpPrice");
    let status = f(&b, "status");
    if f(&b, "slKind") == "trail" && (status == "target_placed" || (status == "armed" && !at_target)) {
        let hw = on(&b, "highWater");
        let hw0 = or_f(hw, Some(0.0)).unwrap_or(0.0);
        let high = if last > hw0 { last } else { hw0 };
        if Some(high) != hw {
            update_bracket(&id, json!({"highWater": high}));
        }
        let new_stop = py_round(high - trail_distance(&b, high).unwrap_or(0.0), 2);
        let cur = or0(&b, "slPrice");
        if new_stop > cur + f64::max(0.01, cur * TRAIL_MIN_MOVE) {
            if tr(&b, "slOrderId") {
                let err = cancel_exit(&f(&b, "slOrderId"));
                if !err.is_empty() {
                    fail(&b, &format!("stop not moved: {}", err));
                    return;
                }
            }
            update_bracket(&id, json!({"slPrice": new_stop, "slOrderId": "", "movedAt": now_s}));
            log(&format!("bagholder bracket: {} for {}: stop moves to {} (high {})", id, sym, rp(Some(new_stop)), rp(Some(high))));
            set(&mut b, "slPrice", json!(new_stop));
            set(&mut b, "slOrderId", json!(""));
        }
    }
    if status == "armed" && tr(&b, "slKind") && f(&b, "slMode") == "watched" && !tr(&b, "slOrderId") && tr(&b, "slPrice") {
        if trigger <= or0(&b, "slPrice") {
            if !may_retry(&b) {
                return;
            }
            let (oid, err) = place_exit(&b, "MARKET", on(&b, "slPrice"), "stop");
            if !err.is_empty() {
                fail(&b, &format!("stop not placed: {}", err));
            } else if !oid.is_empty() {
                update_bracket(&id, json!({"slOrderId": oid, "status": "firing", "error": ""}));
                log(&format!("bagholder bracket: {} for {}: stop hit at {}, market sell placed", id, sym, rp(Some(trigger))));
            }
            return;
        }
    }
    if status == "armed" && tr(&b, "tpPrice") && trigger >= or0(&b, "tpPrice") {
        if tr(&b, "slOrderId") {
            let err = cancel_exit(&f(&b, "slOrderId"));
            if !err.is_empty() {
                fail(&b, &format!("stop not cancelled for the target: {}", err));
                return;
            }
            update_bracket(&id, json!({"status": "firing", "error": ""}));
            log(&format!("bagholder bracket: {} for {}: target reached at {}, stop cancel sent", id, sym, rp(Some(trigger))));
            return;
        }
        fire_target(&b);
    }
    if status == "firing" && tr(&b, "tpPrice") && !tr(&b, "tpOrderId") {
        if exit_row(&b, "stop").map_or(false, |r| f(&r, "status") == "cancelled") {
            fire_target(&b);
        }
    }
    if status == "target_placed" && tr(&b, "slKind") && tr(&b, "slPrice") && tr(&b, "tpOrderId") {
        if trigger <= or0(&b, "slPrice") {
            let err = cancel_exit(&f(&b, "tpOrderId"));
            if !err.is_empty() {
                fail(&b, &format!("target not cancelled for the stop: {}", err));
                return;
            }
            update_bracket(&id, json!({"status": "stopping", "tpOrderId": "", "error": "", "attempts": 0}));
            log(&format!(
                "bagholder bracket: {} for {}: stop level {} reached at {} while the limit sell rested; its cancel sent, market sell follows",
                id, sym, rp(on(&b, "slPrice")), rp(Some(trigger))
            ));
            return;
        }
        if tr(&b, "tpPrice") && trigger < or0(&b, "tpPrice") * (1.0 - TARGET_BACK_OFF) {
            let err = cancel_exit(&f(&b, "tpOrderId"));
            if !err.is_empty() {
                fail(&b, &format!("target not cancelled for the stop: {}", err));
                return;
            }
            update_bracket(&id, json!({"status": "armed", "tpOrderId": "", "slOrderId": "", "error": "", "attempts": 0}));
            log(&format!("bagholder bracket: {} for {}: target out of reach at {}; the limit sell's cancel sent, the stop order goes back", id, sym, rp(Some(trigger))));
            return;
        }
    }
    if status == "stopping" {
        if exit_row(&b, "target").map_or(false, |r| st_in(&r, &["cancelled", "expired"])) {
            if !may_retry(&b) || !nothing_resting(&b) {
                return;
            }
            let (oid, err) = place_exit(&b, "MARKET", on(&b, "slPrice"), "stop");
            if !err.is_empty() {
                fail(&b, &format!("stop not placed: {}", err));
            } else if !oid.is_empty() {
                update_bracket(&id, json!({"slOrderId": oid, "status": "firing", "error": "", "attempts": 0}));
                log(&format!("bagholder bracket: {} for {}: market sell placed at the stop", id, sym));
            }
        }
    }
}

fn fire_target(b: &Value) {
    if !may_retry(b) || !nothing_resting(b) {
        return;
    }
    let (oid, err) = place_exit(b, "LIMIT", on(b, "tpPrice"), "target");
    if !err.is_empty() {
        fail(b, &format!("target not placed: {}", err));
    } else if !oid.is_empty() {
        update_bracket(&f(b, "id"), json!({"tpOrderId": oid, "status": "target_placed", "error": "", "attempts": 0}));
        log(&format!("bagholder bracket: {} for {}: limit sell at {} placed", f(b, "id"), f(b, "symbol"), rp(on(b, "tpPrice"))));
    }
}

fn panic_text(e: &(dyn std::any::Any + Send)) -> String {
    e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "panic".into())
}

/// `bagholder.bracket_tick`: quotes by security id, fetched here when None.
pub fn bracket_tick(quotes: Option<HashMap<String, Value>>) -> Value {
    if BRACKET_LOCK.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return json!({"ok": false, "skipped": "running"});
    }
    struct Release;
    impl Drop for Release {
        fn drop(&mut self) {
            BRACKET_LOCK.store(false, Ordering::SeqCst);
        }
    }
    let _release = Release;
    let sweep = || {
        if let Err(e) = catch_unwind(sweep_exits) {
            log(&format!("bagholder bracket: sweep failed: {}", panic_text(&*e)));
        }
    };
    let live = brackets(&BRACKET_LIVE);
    if live.is_empty() {
        sweep();
        return json!({"ok": true, "brackets": 0});
    }
    if orders_live() {
        for b in &live {
            let st = f(b, "status");
            let prev_in_flight = |role: &str| {
                if let Some(prev) = exit_row(b, role) {
                    if st_in(&prev, &["sent", "pending", "cancelling"]) {
                        refresh_orders(&f(&prev, "id"));
                    }
                }
            };
            if st == "waiting" {
                refresh_orders(&f(b, "orderId"));
            } else if st == "firing" && tr(b, "slOrderId") && !tr(b, "tpOrderId") {
                refresh_orders(&f(b, "slOrderId"));
            } else if st == "armed" && tr(b, "slKind") && !tr(b, "slOrderId") {
                prev_in_flight("stop");
            } else if st == "target_placed" && !tr(b, "tpOrderId") {
                prev_in_flight("target");
            } else if st == "stopping" {
                prev_in_flight("target");
            } else if st == "closing" {
                for o in own_exit_rows(b) {
                    if f(&o, "status") == "cancelling" {
                        refresh_orders(&f(&o, "id"));
                    }
                }
            }
        }
    }
    let orders: HashMap<String, Value> = list_orders().into_iter().map(|o| (f(&o, "id"), o)).collect();
    let quotes = match quotes {
        Some(q) => q,
        None => {
            let mut ids: Vec<String> = live.iter().filter(|b| st_in(b, &["armed", "firing", "target_placed", "stopping"])).map(|b| f(b, "securityId")).collect();
            ids.sort();
            ids.dedup();
            let mut q = HashMap::new();
            if !ids.is_empty() {
                if let Some(sess) = ticket_session() {
                    match fetch_quotes(&sess, &ids) {
                        Ok(x) => q = x,
                        Err(e) => log(&format!("bagholder bracket: quotes failed: {}", err_text(&e))),
                    }
                }
            }
            q
        }
    };
    for b in &live {
        let r = catch_unwind(AssertUnwindSafe(|| {
            let bid = f(b, "id");
            let entry = orders.get(&f(b, "orderId"));
            if reconcile_step(b, entry) == "done" {
                return;
            }
            let Some(b) = get_bracket(&bid) else { return };
            roll_step(&b, quotes.get(&f(&b, "securityId")));
            let Some(b) = get_bracket(&bid) else { return };
            arm_step(&b, entry);
            let Some(b) = get_bracket(&bid) else { return };
            if st_in(&b, &["armed", "firing", "target_placed", "stopping"]) {
                watch_step(&b, quotes.get(&f(&b, "securityId")));
            }
        }));
        if let Err(e) = r {
            log(&format!("bagholder bracket: {} tick failed: {}", f(b, "id"), panic_text(&*e)));
        }
    }
    sweep();
    json!({"ok": true, "brackets": live.len()})
}

/// `bagholder.bracket_loop`.
pub fn bracket_loop() {
    while !app().wait(Duration::from_secs(BRACKET_POLL_SEC)) {
        if !connected_not_syncing() {
            continue;
        }
        if let Err(e) = catch_unwind(|| bracket_tick(None)) {
            log(&format!("bagholder bracket: tick failed: {}", panic_text(&*e)));
        }
    }
}

/// `bagholder.cancel_bracket`.
pub fn cancel_bracket(bracket_id: &str) -> Value {
    let b = match get_bracket(bracket_id) {
        Some(b) => b,
        None => return json!({"ok": false, "error": "No such bracket."}),
    };
    if !st_in(&b, &BRACKET_LIVE) {
        return json!({"ok": false, "error": "That bracket is not live."});
    }
    end_bracket(&b, "cancelled by the user", "");
    json!({"ok": true, "id": gv(&b, "id")})
}

/// `bagholder.modify_order`.
pub fn modify_order(order_id: &str, quantity: Option<&Value>, limit_price: Option<&Value>) -> Value {
    let row = match get_order(order_id) {
        Some(r) => r,
        None => return json!({"ok": false, "error": "No such order."}),
    };
    if !st_in(&row, &["sent", "pending"]) {
        return json!({"ok": false, "error": "That order is not open."});
    }
    let typ = f(&row, "type");
    if typ == "STOP" {
        return json!({"ok": false, "error": "A stop order cannot be changed; cancel it and place another."});
    }
    let q = num(quantity, None);
    let lp = order_tick(num(limit_price, None));
    if q.map_or(false, |x| x <= 0.0) {
        return json!({"ok": false, "error": "Shares must be more than zero."});
    }
    if lp.map_or(false, |x| x <= 0.0) {
        return json!({"ok": false, "error": "A limit price must be more than zero."});
    }
    let limit_type = typ == "LIMIT" || typ == "STOP_LIMIT";
    if limit_type && lp.is_none() && q.is_none() {
        return json!({"ok": false, "error": "Nothing to change."});
    }
    let id = f(&row, "id");
    let mut inp = json!({"externalId": id});
    if lp.is_some() && limit_type && lp != on(&row, "limitPrice") {
        set(&mut inp, "newLimitPrice", jo(lp));
    }
    if q.is_some() && q != on(&row, "quantity") {
        set(&mut inp, "newQuantity", jo(q));
    }
    let inp_map = inp.as_object().cloned().unwrap_or_default();
    if inp_map.len() == 1 {
        return json!({"ok": true, "id": id, "unchanged": true});
    }
    if !orders_live() {
        return json!({"ok": false, "error": "Orders are off (BAGHOLDER_DRY_ORDERS): nothing is sent to Wealthsimple."});
    }
    let sess = match ticket_session() {
        Some(s) => s,
        None => return json!({"ok": false, "error": "Not connected."}),
    };
    let data = match gql(&sess, "SoOrdersOrderModify", json!({"input": inp})) {
        Ok(d) => d,
        Err(CallError::NotAuthorized) => return json!({"ok": false, "error": "Wealthsimple refused the session. Connect Wealthsimple again."}),
        Err(e) => {
            let msg = err_text(&e);
            log(&format!("bagholder orders: modify {} failed: {}", id, msg));
            return json!({"ok": false, "error": format!("Change failed: {}", msg)});
        }
    };
    if let Some(msg) = data.get("soOrdersModifyOrder").and_then(|r| r.get("errors")).filter(|v| truthy(Some(v))).and_then(first_error) {
        log(&format!("bagholder orders: modify {} refused: {}", id, msg));
        return json!({"ok": false, "error": format!("Wealthsimple refused the change: {}", msg)});
    }
    let mut patch = Map::new();
    if inp_map.contains_key("newLimitPrice") {
        patch.insert("limitPrice".into(), jo(lp));
    }
    if inp_map.contains_key("newQuantity") {
        patch.insert("quantity".into(), jo(q));
    }
    update_order(&id, Value::Object(patch));
    if let Some(b) = must(so::bracket_for_order(&db(), &id)) {
        if f(&b, "status") == "waiting" && inp_map.contains_key("newQuantity") {
            update_bracket(&f(&b, "id"), json!({"quantity": jo(q)}));
        }
    }
    let shown: Map<String, Value> = inp_map.iter().filter(|(k, _)| *k != "externalId").map(|(k, v)| (k.clone(), v.clone())).collect();
    log(&format!("bagholder orders: modify {} accepted: {}", id, bagholder_store::tables::py_json_sorted(&Value::Object(shown))));
    let rid = id.clone();
    spawn("bagholder-order-refresh", move || {
        let _ = catch_unwind(|| refresh_orders(&rid));
    });
    json!({"ok": true, "id": id})
}

/// `bagholder.adjust_bracket`.
pub fn adjust_bracket(bracket_id: &str, leg: &str, price: Option<&Value>, trail: Option<&Value>, remove: bool) -> Value {
    let b = match get_bracket(bracket_id) {
        Some(b) => b,
        None => return json!({"ok": false, "error": "No such bracket."}),
    };
    if !st_in(&b, &BRACKET_LIVE) {
        return json!({"ok": false, "error": "That bracket is not live."});
    }
    let leg = leg.to_lowercase();
    if leg != "sl" && leg != "tp" {
        return json!({"ok": false, "error": "Which leg?"});
    }
    let id = f(&b, "id");
    let sym = f(&b, "symbol");
    if remove {
        if leg == "sl" {
            let err = cancel_exit(&f(&b, "slOrderId"));
            if !err.is_empty() {
                return json!({"ok": false, "error": err});
            }
            let mut patch = json!({"slKind": "", "slOrderId": "", "slMode": "", "error": ""});
            if !tr(&b, "tpPrice") {
                set(&mut patch, "status", json!("cancelled"));
                set(&mut patch, "outcome", json!("both legs removed"));
            }
            update_bracket(&id, patch);
        } else {
            let err = cancel_exit(&f(&b, "tpOrderId"));
            if !err.is_empty() {
                return json!({"ok": false, "error": err});
            }
            let mut patch = json!({"tpPrice": null, "tpOrderId": "", "error": ""});
            if f(&b, "status") == "target_placed" {
                set(&mut patch, "status", json!("armed"));
            }
            if !tr(&b, "slKind") {
                set(&mut patch, "status", json!("cancelled"));
                set(&mut patch, "outcome", json!("both legs removed"));
            }
            update_bracket(&id, patch);
        }
        log(&format!("bagholder bracket: {} for {}: {} removed by the user", id, sym, if leg == "sl" { "stop loss" } else { "take profit" }));
        return json!({"ok": true, "id": id});
    }
    if leg == "sl" {
        if !tr(&b, "slKind") {
            return json!({"ok": false, "error": "This bracket has no stop loss."});
        }
        let mut patch;
        if f(&b, "slKind") == "trail" {
            let t = match num(trail, None) {
                Some(t) if t != 0.0 && t > 0.0 => t,
                _ => return json!({"ok": false, "error": "A trail is required."}),
            };
            let high = or_f(or_f(on(&b, "highWater"), on(&b, "slPrice")), Some(0.0)).unwrap_or(0.0);
            let mut nb = b.clone();
            set(&mut nb, "slTrail", json!(t));
            let new_price = if high != 0.0 { json!(py_round(high - trail_distance(&nb, high).unwrap_or(0.0), 2)) } else { gv(&b, "slPrice") };
            patch = json!({"slTrail": t, "slPrice": new_price});
        } else {
            let p = match num(price, None) {
                Some(p) if p != 0.0 && p > 0.0 => p,
                _ => return json!({"ok": false, "error": "A stop price is required."}),
            };
            patch = json!({"slPrice": p});
        }
        if tr(&b, "slOrderId") && f(&b, "status") == "armed" {
            let err = cancel_exit(&f(&b, "slOrderId"));
            if !err.is_empty() {
                return json!({"ok": false, "error": err});
            }
            set(&mut patch, "slOrderId", json!(""));
            set(&mut patch, "movedAt", json!(now_iso()));
        }
        set(&mut patch, "error", json!(""));
        let shown = rp(on(&patch, "slPrice"));
        update_bracket(&id, patch);
        log(&format!("bagholder bracket: {} for {}: stop moved to {} by the user", id, sym, shown));
        return json!({"ok": true, "id": id});
    }
    let p = match num(price, None) {
        Some(p) if p != 0.0 && p > 0.0 => p,
        _ => return json!({"ok": false, "error": "A limit price is required."}),
    };
    let mut patch = json!({"tpPrice": p, "error": ""});
    if f(&b, "status") == "target_placed" && tr(&b, "tpOrderId") {
        let err = cancel_exit(&f(&b, "tpOrderId"));
        if !err.is_empty() {
            return json!({"ok": false, "error": err});
        }
        set(&mut patch, "tpOrderId", json!(""));
        set(&mut patch, "status", json!("armed"));
    }
    update_bracket(&id, patch);
    log(&format!("bagholder bracket: {} for {}: target moved to {} by the user", id, sym, rp(Some(p))));
    json!({"ok": true, "id": id})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(py_g(5.0), "5");
        assert_eq!(py_g(1.5), "1.5");
        assert_eq!(py_g(1234567.0), "1.23457e+06");
        assert_eq!(py_g(0.00001), "1e-05");
        assert_eq!(price_words(Some(0.625)), "0.625");
        assert_eq!(price_words(Some(0.54)), "0.54");
        assert_eq!(order_tick(Some(1.005)), Some(1.0));
        assert_eq!(parse_z("2026-09-10T12:00:00Z"), Some(bagholder_model::dates::to_days(2026, 9, 10) * 86400 + 43200));
        assert_eq!(app_status("POSTED"), "filled");
        assert_eq!(app_status("WHATEVER"), "pending");
    }
}
