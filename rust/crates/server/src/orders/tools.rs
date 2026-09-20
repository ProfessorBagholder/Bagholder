//! Small tools the order code shares: reading a JSON row, words for a price and a
//! quantity, times, the one way to Wealthsimple, and the store.

use super::*;

// ---------------------------------------------------------------------------
// small tools
// ---------------------------------------------------------------------------

pub fn orders_live() -> bool {
    #[cfg(test)]
    if let Some(v) = *seam::LIVE.lock().unwrap_or_else(|e| e.into_inner()) {
        return v;
    }
    static LIVE: OnceLock<bool> = OnceLock::new();
    *LIVE.get_or_init(|| std::env::var("BAGHOLDER_DRY_ORDERS").map(|v| v.trim() != "1").unwrap_or(true))
}

pub(super) fn db() -> bagholder_store::pool::Pooled<'static> {
    app().open().expect("bagholder orders: the store could not be opened")
}

pub(super) fn must<T>(r: rusqlite::Result<T>) -> T {
    r.unwrap_or_else(|e| panic!("bagholder orders: store: {}", e))
}

pub(super) fn tr(v: &Value, k: &str) -> bool {
    truthy(v.get(k))
}

pub(super) fn on(v: &Value, k: &str) -> Option<f64> {
    num(v.get(k), None)
}

/// `x.get(k) or 0.0`.
pub(super) fn or0(v: &Value, k: &str) -> f64 {
    on(v, k).unwrap_or(0.0)
}

/// The first value that is present and non-zero.
pub(super) fn or_f(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match a {
        Some(x) if x != 0.0 => Some(x),
        _ => b,
    }
}

/// The first JSON value that is truthy (not null, false, zero or empty).
pub(super) fn or_v<'a>(a: Option<&'a Value>, b: Option<&'a Value>) -> Option<&'a Value> {
    if truthy(a) {
        a
    } else {
        b
    }
}

pub(super) fn gv(v: &Value, k: &str) -> Value {
    v.get(k).cloned().unwrap_or(Value::Null)
}

pub(super) fn jo(v: Option<f64>) -> Value {
    match v {
        Some(x) if x.is_finite() => json!(x),
        _ => Value::Null,
    }
}

pub(super) fn set(v: &mut Value, k: &str, x: Value) {
    if let Value::Object(m) = v {
        m.insert(k.to_string(), x);
    }
}

/// `x` rounded to `n` decimals, ties to even.
pub(super) fn round_half_even(x: f64, n: usize) -> f64 {
    format!("{:.*}", n, x).parse().unwrap_or(x)
}

/// `str(float)`, or `None`.
pub(super) fn rp(v: Option<f64>) -> String {
    match v {
        Some(x) => bagholder_model::value::num_repr(x),
        None => "None".into(),
    }
}

/// `x` in `%g` form: six significant digits, trailing zeros dropped.
pub(super) fn fmt_g(x: f64) -> String {
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

pub(super) fn upper(v: Option<&Value>) -> String {
    s(v).trim().to_uppercase()
}

pub(super) fn date_only(v: Option<&Value>) -> String {
    let t = s(v);
    let t = t.trim();
    if t.is_empty() {
        return String::new();
    }
    let t = t.split('T').next().unwrap_or("");
    t.chars().take(10).collect()
}

pub(super) fn two(b: &[u8], i: usize) -> Option<i64> {
    let a = (b[i] as char).to_digit(10)?;
    let c = (b[i + 1] as char).to_digit(10)?;
    Some((a * 10 + c) as i64)
}

/// `%Y-%m-%dT%H:%M:%S` as unix seconds; exact.
pub(super) fn parse_ymdhms(t: &str) -> Option<i64> {
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
pub(super) fn parse_z(t: &str) -> Option<i64> {
    t.strip_suffix('Z').and_then(parse_ymdhms)
}

pub(super) fn parse_utc(v: Option<&Value>) -> Option<i64> {
    let t = s(v);
    let t = t.trim();
    if t.is_empty() {
        return None;
    }
    parse_ymdhms(&t.chars().take(19).collect::<String>())
}

pub(super) fn gql(sess: &Value, op: &str, vars: Value) -> Result<Value, CallError> {
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

pub(super) fn err_text(e: &CallError) -> String {
    let t = e.to_string();
    if t.is_empty() {
        "RuntimeError".into()
    } else {
        t
    }
}

pub(super) fn first_error(errs: &Value) -> Option<String> {
    let a = errs.as_array()?;
    let first = a.first()?;
    Some(if first.is_object() {
        s(or_v(first.get("message"), first.get("code")))
    } else {
        s(Some(first))
    })
}

pub(super) fn list_orders() -> Vec<Value> {
    must(so::list_orders(&db(), 200))
}

pub(super) fn get_order(id: &str) -> Option<Value> {
    must(so::get_order(&db(), id))
}

pub(super) fn insert_order(row: &Value) {
    must(so::insert_order(&db(), row, &now_iso()))
}

pub(super) fn update_order(id: &str, patch: Value) {
    must(so::update_order(&db(), id, &patch, &now_iso()))
}

pub(super) fn brackets(statuses: &[&str]) -> Vec<Value> {
    let st: Vec<String> = statuses.iter().map(|x| x.to_string()).collect();
    must(so::list_brackets(&db(), &st))
}

pub(super) fn get_bracket(id: &str) -> Option<Value> {
    must(so::get_bracket(&db(), id))
}

pub(super) fn update_bracket(id: &str, patch: Value) {
    must(so::update_bracket(&db(), id, &patch, &now_iso()))
}

pub(super) fn emit(kind: &str, key: &str, title: &str, body: &str) {
    notify::emit(&db(), kind, key, title, body, None);
}

pub(super) fn snapshot() -> Value {
    must(bagholder_store::snapshot::snapshot(&db(), false))
}

pub(super) fn connected_not_syncing() -> bool {
    let st = app().state.lock().unwrap();
    st.connected && !st.syncing
}
