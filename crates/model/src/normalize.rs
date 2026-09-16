//! Activity normalization: a copy of each raw Wealthsimple row with crypto and
//! option events expressed as trade fills. `model.normalize_activity`.
//!
//! The raw rows are never rewritten in the store; this is a derived copy.

use serde_json::{Map, Value};

use crate::symbols::{is_option_symbol, option_right, underlying_symbol};
use crate::value::{compact, field_num, field_s, norm_account_name, num, EPS};

pub const KINDS: [&str; 4] = ["Shares", "Options", "Crypto", "Futures"];

/// `model.is_crypto_activity`.
pub fn is_crypto_activity(a: &Value) -> bool {
    compact(&field_s(a, "rawType")).starts_with("CRYPTO")
        || compact(&field_s(a, "activityType")).starts_with("CRYPTO")
}

/// `model.kind_of`: an explicit kind wins, then crypto, then the symbol.
pub fn kind_of(a: &Value) -> String {
    let k = field_s(a, "kind");
    if KINDS.contains(&k.as_str()) { return k; }
    if is_crypto_activity(a) { return "Crypto".into(); }
    if is_option_symbol(&field_s(a, "symbol")) { return "Options".into(); }
    "Shares".into()
}

fn type_fields(a: &Value) -> (String, String) {
    (compact(&field_s(a, "activityType")), compact(&field_s(a, "activitySubType")))
}

/// `model.is_intentional_open`: a row that says it opens a position.
pub fn is_intentional_open(a: &Value) -> bool {
    let (at, sub) = type_fields(a);
    at.contains("TOOPEN") || sub.contains("TOOPEN") || at == "STO" || at == "BTO" || sub == "STO" || sub == "BTO"
}

/// `model.is_close_only`: a row that can only reduce a position -- an explicit
/// close, or the expiry/assignment/exercise the broker posts for one.
pub fn is_close_only(a: &Value) -> bool {
    let (at, sub) = type_fields(a);
    for f in [&at, &sub] {
        if f.contains("TOCLOSE") || f == "BTC" || f == "STC" { return true; }
    }
    for f in [&at, &sub] {
        if f.contains("EXPIR") || f.contains("ASSIGN") || f.contains("EXERCISE") { return true; }
    }
    false
}

/// `model.opening_direction`: which way a fill opens, or `None` when it can
/// only close. A bare share sale is never read as a short unless the row says
/// it opened one, so a sale of something bought before the history starts does
/// not invent a short position.
pub fn opening_direction(a: &Value, side: &str) -> Option<&'static str> {
    match side {
        "BUY" => if is_close_only(a) { None } else { Some("LONG") },
        "SELL" => {
            if is_option_symbol(&field_s(a, "symbol")) {
                return if is_close_only(a) { None } else { Some("SHORT") };
            }
            if is_intentional_open(a) { return Some("SHORT"); }
            None
        }
        _ => None,
    }
}

fn set(m: &mut Map<String, Value>, k: &str, v: Value) { m.insert(k.into(), v); }
fn setf(m: &mut Map<String, Value>, k: &str, v: f64) {
    set(m, k, Value::Number(serde_json::Number::from_f64(v).unwrap_or_else(|| 0.into())));
}
fn push_flag(m: &mut Map<String, Value>, flag: &str) {
    if let Some(Value::Array(a)) = m.get_mut("flags") { a.push(Value::String(flag.into())); }
}

/// `model.normalize_activity`.
pub fn normalize_activity(activity: &Value) -> Value {
    let mut a: Map<String, Value> = match activity {
        Value::Object(o) => o.clone(),
        _ => Map::new(),
    };
    let account = norm_account_name(&field_s(activity, "accountType"));
    set(&mut a, "accountType", Value::String(account));
    let rt = compact(&field_s(activity, "rawType"));
    let at = compact(&field_s(activity, "activityType"));
    let cash = field_num(activity, "netCashAmount");
    let qty = field_num(activity, "quantity").abs();
    set(&mut a, "flags", Value::Array(vec![]));

    // ---- crypto: the rows carry their direction in the type, not the sign
    if rt == "CRYPTOBUY" || at == "CRYPTOBUY" {
        set(&mut a, "category", "trade".into());
        set(&mut a, "activityType", "Trade".into());
        set(&mut a, "activitySubType", "BUY".into());
        set(&mut a, "kind", "Crypto".into());
        setf(&mut a, "quantity", qty);
        setf(&mut a, "netCashAmount", -cash.abs());
        return Value::Object(a);
    }
    if rt == "CRYPTOSELL" || at == "CRYPTOSELL" {
        set(&mut a, "category", "trade".into());
        set(&mut a, "activityType", "Trade".into());
        set(&mut a, "activitySubType", "SELL".into());
        set(&mut a, "kind", "Crypto".into());
        setf(&mut a, "quantity", -qty);
        setf(&mut a, "netCashAmount", cash.abs());
        return Value::Object(a);
    }
    if rt == "CRYPTOTRANSFER" || at == "CRYPTOTRANSFER" {
        let sub = compact(&field_s(activity, "activitySubType"));
        set(&mut a, "category", "trade".into());
        set(&mut a, "activityType", "Trade".into());
        set(&mut a, "kind", "Crypto".into());
        push_flag(&mut a, "transfer");
        if sub.contains("OUT") || cash < 0.0 {
            set(&mut a, "activitySubType", "SELL".into());
            push_flag(&mut a, "transfer-out");
            setf(&mut a, "quantity", -qty);
            setf(&mut a, "netCashAmount", cash.abs());
        } else {
            set(&mut a, "activitySubType", "BUY".into());
            setf(&mut a, "quantity", qty);
            setf(&mut a, "netCashAmount", -cash.abs());
        }
        return Value::Object(a);
    }
    if rt == "CRYPTOSTAKINGREWARD" || at == "CRYPTOSTAKINGREWARD" {
        // Units arriving at no cost: they enter the book at zero, so the whole
        // proceeds show as gain when they are sold.
        set(&mut a, "category", "trade".into());
        set(&mut a, "activityType", "Trade".into());
        set(&mut a, "activitySubType", "BUY".into());
        set(&mut a, "kind", "Crypto".into());
        push_flag(&mut a, "reward");
        setf(&mut a, "quantity", qty);
        setf(&mut a, "unitPrice", 0.0);
        setf(&mut a, "netCashAmount", 0.0);
        return Value::Object(a);
    }
    if rt.starts_with("CRYPTO") {
        set(&mut a, "category", "other".into());
        set(&mut a, "kind", "Crypto".into());
        return Value::Object(a);
    }

    // A distribution posted in units with no cash is a pending notice, not a
    // share delivery: Wealthsimple's balance does not grow by it.
    if at == "STKDIS" && rt == "DIVIDEND" && cash.abs() < EPS {
        set(&mut a, "category", "other".into());
        push_flag(&mut a, "pending-distribution");
        return Value::Object(a);
    }

    let raw = format!("{}{}", rt, at);
    if raw.contains("MULTILEG") {
        set(&mut a, "category", "trade".into());
        if cash < 0.0 || compact(&field_s(activity, "direction")) == "DEBIT" {
            set(&mut a, "activityType", "OPTIONS_BUY".into());
            set(&mut a, "activitySubType", "BUYTOCLOSE".into());
        } else {
            set(&mut a, "activityType", "OPTIONS_SELL".into());
            set(&mut a, "activitySubType", "SELLTOOPEN".into());
        }
    } else if raw.contains("EXPIR") || raw.contains("ASSIGN") || raw.contains("EXERCISE") {
        set(&mut a, "category", "option_event".into());
        if raw.contains("ASSIGN") {
            set(&mut a, "activityType", "ASSIGN".into());
            set(&mut a, "activitySubType", "BUYTOCLOSE".into());
            setf(&mut a, "unitPrice", 0.0);
        } else if raw.contains("SHORTEXPIR") {
            set(&mut a, "activityType", "EXPIR".into());
            set(&mut a, "activitySubType", "BUY".into());
        } else if raw.contains("EXPIR") {
            set(&mut a, "activityType", "EXPIR".into());
            set(&mut a, "activitySubType", "SELL".into());
        } else {
            set(&mut a, "activityType", "EXERCISE".into());
            set(&mut a, "activitySubType", "SELL".into());
        }
        if raw.contains("ASSIGN") || cash.abs() < 1e-12 {
            setf(&mut a, "unitPrice", 0.0);
        }
        if qty > 0.0 {
            let sub = a.get("activitySubType").map(|v| v.as_str().unwrap_or("")).unwrap_or("");
            let q = if sub == "SELL" { -qty } else { qty };
            setf(&mut a, "quantity", q);
        }
    }
    let kind = kind_of(&Value::Object(a.clone()));
    set(&mut a, "kind", Value::String(kind));
    Value::Object(a)
}

pub fn normalize_activities(activities: &[Value]) -> Vec<Value> {
    activities.iter().map(normalize_activity).collect()
}

/// `model.fifo_account`: the nickname when there is one, so two accounts with
/// the same symbol keep separate books; the ids only when there is not.
pub fn fifo_account(a: &Value) -> String {
    let nick = norm_account_name(&field_s(a, "accountType"));
    if !nick.is_empty() { return nick; }
    let fifo = field_s(a, "fifoId");
    if !fifo.is_empty() { return fifo; }
    field_s(a, "accountId")
}

/// `model.book_key`.
pub fn book_key(a: &Value) -> String {
    format!("{}::{}::{}", fifo_account(a), field_s(a, "symbol"), field_s(a, "currency"))
}

/// `model.roll_key`: what an option roll is folded within -- one account, one
/// underlying, one right.
pub fn roll_key(a: &Value) -> (String, String, &'static str) {
    let sym = field_s(a, "symbol");
    (fifo_account(a), underlying_symbol(&sym), option_right(&sym))
}

/// `model.is_multileg`.
pub fn is_multileg(a: &Value) -> bool {
    compact(&field_s(a, "rawType")).contains("MULTILEG")
}

/// `model.fold_stkdis`: net the +N/-N name-change rows posted on one day, and
/// open whatever is left over at $0.
pub fn fold_stkdis(activities: &[Value]) -> Vec<Value> {
    struct G { pos: f64, neg: f64, sample: Value }
    let mut rest: Vec<Value> = Vec::new();
    let mut groups: Vec<(String, G)> = Vec::new();

    for a in activities {
        if compact(&field_s(a, "activityType")) != "STKDIS" {
            rest.push(a.clone());
            continue;
        }
        let k = format!("{}\u{0}{}\u{0}{}", field_s(a, "symbol"), field_s(a, "transactionDate"), field_s(a, "currency"));
        let idx = match groups.iter().position(|(gk, _)| *gk == k) {
            Some(i) => i,
            None => { groups.push((k, G { pos: 0.0, neg: 0.0, sample: a.clone() })); groups.len() - 1 }
        };
        let q = field_num(a, "quantity");
        if field_s(a, "activitySubType") == "SELL" || q < 0.0 {
            groups[idx].1.neg += q.abs();
        } else {
            groups[idx].1.pos += q.abs();
        }
    }
    for (_, g) in groups {
        let net = g.pos - g.neg;
        if net > EPS {
            let mut m = match g.sample { Value::Object(o) => o, _ => Map::new() };
            setf(&mut m, "quantity", net);
            set(&mut m, "activitySubType", "BUY".into());
            setf(&mut m, "unitPrice", 0.0);
            setf(&mut m, "netCashAmount", 0.0);
            set(&mut m, "category", "trade".into());
            rest.push(Value::Object(m));
        }
    }
    rest
}

/// `model.num` re-exported for callers that hold a raw `Value`.
pub fn n(v: Option<&Value>) -> f64 { num(v, 0.0) }
