//! Turning a Wealthsimple `ActivityFeedItem` into a ledger row.
//!
//! This is the only place the broker's vocabulary is read. What it decides --
//! which rows are fills, which way they go, what an option's per-share price
//! is, which corporate action replaced a ticker -- is what the whole model
//! then works from, and the stored copy is never rewritten afterwards.

use serde_json::{json, Map, Value};
use std::collections::HashMap;

use bagholder_model::value::{compact, field_s, get, num};

pub const MONTHS: [&str; 12] =
    ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"];

const SKIP_TYPE_MARKERS: [&str; 4] = ["SHARE_LENDING", "SHARELENDING", "STOCK_LENDING", "STOCKLENDING"];

/// Fills that hit the cash book. Everything else -- pending limits, cancelled,
/// submitted, working -- is not a trade.
const KEEP_STATUS: [&str; 11] = [
    "POSTED", "COMPLETED", "SETTLED", "COMPLETE", "FILLED", "EXECUTED", "PROCESSED", "CONFIRMED",
    "BOOKED", "SUCCEEDED", "SUCCESS",
];

const CORP_BLOBS: [&str; 15] = [
    "STKDIS", "STOCKDISTRIBUTION", "STOCKDIV", "SPINOFF", "SPIN", "DIVIDENDINKIND", "INKIND",
    "CORPORATEACTION", "CODECHANGE", "SYMBOLCHANGE", "TICKERCHANGE", "LISTINGSTATUS", "SECURITYSWAP",
    "MANDATORYEXCHANGE", "NAMECHANGE",
];

const CODE_CHANGE_BLOBS: [&str; 7] = [
    "CODECHANGE", "SYMBOLCHANGE", "TICKERCHANGE", "LISTINGSTATUS", "SECURITYSWAP", "MANDATORYEXCHANGE",
    "NAMECHANGE",
];

fn upper(v: &Value, key: &str) -> String {
    field_s(v, key).to_uppercase()
}

fn n(v: &Value, key: &str) -> f64 {
    num(get(v, key), 0.0)
}

fn date_only(occurred: &str) -> String {
    let s = occurred.trim();
    if s.is_empty() {
        return String::new();
    }
    s.split('T').next().unwrap_or("").chars().take(10).collect()
}

/// An `EXCHANGE:` prefix is a venue, not part of
/// the ticker.
pub fn asset_symbol(item: &Value) -> String {
    let raw = field_s(item, "assetSymbol");
    let raw = raw.trim();
    let raw = if raw.to_uppercase().starts_with("EXCHANGE:") {
        raw.splitn(2, ':').nth(1).unwrap_or(raw)
    } else {
        raw
    };
    raw.to_uppercase().trim().to_string()
}

pub fn counter_symbol(item: &Value) -> String {
    let raw = field_s(item, "counterAssetSymbol");
    let raw = raw.trim();
    let raw = if raw.to_uppercase().starts_with("EXCHANGE:") {
        raw.splitn(2, ':').nth(1).unwrap_or(raw)
    } else {
        raw
    };
    raw.to_uppercase().trim().to_string()
}

/// `(type, subType, the four type fields joined)`.
fn type_blob(item: &Value) -> (String, String, String) {
    let typ = upper(item, "type").replace('-', "_");
    let sub = upper(item, "subType").replace('-', "_");
    let parts: Vec<String> = [
        typ.clone(),
        sub.clone(),
        field_s(item, "aftTransactionType"),
        field_s(item, "aftTransactionCategory"),
    ]
    .into_iter()
    .filter(|x| !x.is_empty())
    .map(|x| compact(&x))
    .collect();
    let blob = parts.join("_");
    (typ, sub, blob)
}

/// A corporate action, or a distribution that
/// delivers shares rather than cash.
pub fn is_corp_share_move(item: &Value) -> bool {
    let (typ, _sub, blob) = type_blob(item);
    if CORP_BLOBS.iter().any(|k| blob.contains(k)) {
        return true;
    }
    let qty = n(item, "assetQuantity").abs();
    let cash = n(item, "amount").abs();
    qty != 0.0
        && !asset_symbol(item).is_empty()
        && cash == 0.0
        && (compact(&typ).contains("DIVIDEND") || blob.contains("DISTRIBUT"))
}

pub fn is_code_change(item: &Value) -> bool {
    let (_, _, blob) = type_blob(item);
    CODE_CHANGE_BLOBS.iter().any(|k| blob.contains(k))
}

/// Whether this row should not be stored.
pub fn skip_activity(item: &Value) -> bool {
    if !item.is_object() {
        return true;
    }
    if field_s(item, "occurredAt").trim().is_empty() {
        return true;
    }
    let status = compact(&field_s(item, "status"));
    let (typ, sub, blob) = type_blob(item);

    // Corporate actions often land as processed or empty, not posted.
    if is_corp_share_move(item) {
        if ["REJECT", "CANCEL", "FAIL", "VOID"].iter().any(|x| status.contains(x)) {
            return true;
        }
    } else if (typ == "DIVIDEND" || typ == "INTEREST_CHARGE") && status.is_empty() {
        // Cash dividends and margin interest charges often arrive with no
        // status at all; both have already hit the cash balance.
    } else if status.is_empty() || !KEEP_STATUS.contains(&status.as_str()) {
        return true;
    }

    // INTEREST / FPL_INTEREST must not be read as a loan.
    if typ == "LOAN" || typ == "RECALL" || sub == "LOAN" || sub == "RECALL" {
        return true;
    }
    if typ.ends_with("_LOAN") || sub.ends_with("_LOAN") || typ.ends_with("_RECALL") || sub.ends_with("_RECALL") {
        return true;
    }
    if SKIP_TYPE_MARKERS.iter().any(|m| blob.contains(m)) {
        return true;
    }
    blob.contains("SHARE_LENDING") || blob.contains("SHARELENDING")
}

/// An OCC-ish display, so an option rolls up into
/// the ticker it is written on.
pub fn option_symbol(item: &Value) -> String {
    let under = asset_symbol(item);
    let contract = field_s(item, "contractType");
    let strike = get(item, "strikePrice");
    let expiry = field_s(item, "expiryDate");
    if contract.is_empty() || strike.is_none() || expiry.is_empty() || under.is_empty() {
        return under;
    }
    let ds = expiry.trim().split('T').next().unwrap_or("").replace('/', "-");
    let ds: String = ds.chars().take(10).collect();
    let parts: Vec<&str> = ds.split('-').collect();
    if parts.len() != 3 {
        return under;
    }
    let (year, month, day) = match (parts[0].parse::<i64>(), parts[1].parse::<usize>(), parts[2].parse::<i64>()) {
        (Ok(y), Ok(m), Ok(d)) if m >= 1 && m <= 12 => (y, m, d),
        _ => return under,
    };
    let mon = MONTHS[month - 1];
    let strike_f = match num(strike, f64::NAN) { v if !v.is_nan() => v, _ => return under };
    let cp = match contract.to_uppercase().as_str() {
        "C" | "CALL" => "CALL",
        "P" | "PUT" => "PUT",
        other => return format!("{} {:02}{}{:02} {:.2} {}", under, day, mon, year % 100, strike_f, other),
    };
    format!("{} {:02}{}{:02} {:.2} {}", under, day, mon, year % 100, strike_f, cp)
}

/// Buys, withdrawals and the source side of a
/// transfer are negative; sells, deposits and income positive.
pub fn signed_cash(item: &Value) -> f64 {
    let amount = n(item, "amount").abs();
    let typ = upper(item, "type").replace('-', "_");
    let sub = upper(item, "subType").replace('-', "_");
    if ["DIY_BUY", "OPTIONS_BUY", "WITHDRAWAL"].contains(&typ.as_str())
        || (typ == "INTERNAL_TRANSFER" && sub.contains("SOURCE"))
    {
        return -amount;
    }
    if ["DIY_SELL", "OPTIONS_SELL", "DEPOSIT", "CONTRIBUTION", "DIVIDEND", "INTEREST"].contains(&typ.as_str())
        || (typ == "INTERNAL_TRANSFER" && sub.contains("DESTINATION"))
    {
        return amount;
    }
    let sign = field_s(item, "amountSign").trim().to_lowercase();
    if ["negative", "debit", "-", "neg"].contains(&sign.as_str()) {
        return -amount;
    }
    if ["positive", "credit", "+", "pos"].contains(&sign.as_str()) {
        return amount;
    }
    match get(item, "amount") {
        None => 0.0,
        Some(Value::String(s)) if s.is_empty() => 0.0,
        Some(v) => num(Some(v), 0.0),
    }
}

fn accounts_list(accounts: Option<&Value>) -> Vec<Value> {
    match accounts {
        Some(Value::Array(a)) => a.iter().filter(|x| x.is_object()).cloned().collect(),
        Some(Value::Object(m)) => m.values().filter(|x| x.is_object()).cloned().collect(),
        _ => vec![],
    }
}

/// The nickname, else what the broker calls it.
pub fn account_type(account_id: &str, accounts: Option<&Value>) -> String {
    let recs = accounts_list(accounts);
    if recs.is_empty() {
        return String::new();
    }
    // a map is keyed by id; a list is searched
    if let Some(Value::Object(m)) = accounts {
        if let Some(rec) = m.get(account_id) {
            return nick_of(rec);
        }
    }
    for a in &recs {
        if field_s(a, "id") == account_id {
            return nick_of(a);
        }
    }
    String::new()
}

fn nick_of(rec: &Value) -> String {
    for k in ["nickname", "unifiedAccountType", "type"] {
        let v = field_s(rec, k);
        if !v.is_empty() {
            return v;
        }
    }
    String::new()
}

/// The nickname a NAV filter names, and the
/// Wealthsimple ids behind it. The CAD and USD sides of one account share a
/// nickname and so share a group.
pub fn nav_account_groups(accounts: Option<&Value>) -> Map<String, Value> {
    let mut groups: Map<String, Value> = Map::new();
    for acc in accounts_list(accounts) {
        let aid = field_s(&acc, "id").trim().to_string();
        if aid.is_empty() {
            continue;
        }
        let nick = nick_of(&acc).trim().to_string();
        if nick.is_empty() {
            continue;
        }
        let bucket = groups.entry(nick).or_insert_with(|| Value::Array(vec![]));
        let arr = bucket.as_array_mut().unwrap();
        if !arr.iter().any(|x| x.as_str() == Some(aid.as_str())) {
            arr.push(json!(aid));
        }
    }
    groups
}

/// The CAD and USD sides of one Wealthsimple
/// account share a single FIFO book.
///
/// A linked pair and a shared nickname both collapse to one root id; distinct
/// nicknames stay separate.
pub fn fifo_pool_ids(accounts: Option<&Value>) -> HashMap<String, String> {
    let recs = accounts_list(accounts);
    let mut parent: HashMap<String, String> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    fn find(parent: &mut HashMap<String, String>, x: &str) -> String {
        parent.entry(x.to_string()).or_insert_with(|| x.to_string());
        let mut cur = x.to_string();
        while parent[&cur] != cur {
            let grand = parent[&parent[&cur]].clone();
            parent.insert(cur.clone(), grand.clone());
            cur = grand;
        }
        cur
    }

    fn union(parent: &mut HashMap<String, String>, a: &str, b: &str) {
        if a.is_empty() || b.is_empty() {
            return;
        }
        let (ra, rb) = (find(parent, a), find(parent, b));
        if ra != rb {
            // the lower id wins
            let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
            parent.insert(hi, lo);
        }
    }

    let mut by_nick: Vec<(String, Vec<String>)> = Vec::new();
    for a in &recs {
        let aid = field_s(a, "id");
        if aid.is_empty() {
            continue;
        }
        if !order.contains(&aid) {
            order.push(aid.clone());
        }
        find(&mut parent, &aid);
        let lid = a.get("linkedAccount").filter(|l| l.is_object()).map(|l| field_s(l, "id")).unwrap_or_default();
        if !lid.is_empty() {
            if !order.contains(&lid) {
                order.push(lid.clone());
            }
            union(&mut parent, &aid, &lid);
        }
        let nick = field_s(a, "nickname").trim().to_string();
        if !nick.is_empty() {
            match by_nick.iter_mut().find(|(k, _)| *k == nick) {
                Some((_, v)) => v.push(aid.clone()),
                None => by_nick.push((nick, vec![aid.clone()])),
            }
        }
    }
    for (_, ids) in &by_nick {
        if let Some((root, rest)) = ids.split_first() {
            for other in rest {
                union(&mut parent, root, other);
            }
        }
    }
    let keys: Vec<String> = parent.keys().cloned().collect();
    keys.iter().map(|k| (k.clone(), find(&mut parent, k))) .collect()
}

fn is_option(item: &Value) -> bool {
    !field_s(item, "contractType").is_empty()
}

fn is_to_close(sub: &str) -> bool {
    let c = compact(sub);
    c.contains("TOCLOSE") || ["BTC", "STC", "BUYTOCLOSE", "SELLTOCLOSE"].contains(&c.as_str())
}

/// `%g` form, which is what the descriptions are formatted with.
fn g(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let mag = v.abs();
    if mag >= 1e-4 && mag < 1e6 {
        let mut s = format!("{:.6}", v);
        // %g keeps six significant digits, then drops trailing zeros
        let digits_before = format!("{:.0}", mag.trunc()).len();
        let decimals = 6usize.saturating_sub(if mag >= 1.0 { digits_before } else { 0 });
        s = format!("{:.*}", decimals, v);
        while s.contains('.') && (s.ends_with('0') || s.ends_with('.')) {
            s.pop();
        }
        s
    } else {
        let s = format!("{:e}", v);
        s
    }
}

/// What the row is called in the ledger.
fn human_desc(item: &Value, typ: &str, sub: &str, symbol: &str, qty: f64, px: f64, _cash: f64) -> String {
    let t = typ.to_uppercase().replace('-', "_");
    let s = sub.to_uppercase().replace('-', "_");
    let csub = compact(sub);

    if t == "DIY_BUY" || (t == "TRADE" && ["BUY", "BUYTOOPEN", "BUYTOCLOSE"].contains(&csub.as_str())) {
        let verb = if csub.contains("CLOSE") { "Buy to close" } else if csub.contains("OPEN") { "Buy to open" } else { "Buy" };
        if qty != 0.0 && px != 0.0 {
            return format!("{} {} {} @ {}", verb, g(qty), symbol, g(px));
        }
        return format!("{} {}", verb, symbol).trim().to_string();
    }
    if t == "DIY_SELL" || (t == "TRADE" && csub.contains("SELL")) {
        let verb = if csub.contains("CLOSE") { "Sell to close" } else if csub.contains("OPEN") { "Sell to open" } else { "Sell" };
        if qty != 0.0 && px != 0.0 {
            return format!("{} {} {} @ {}", verb, g(qty.abs()), symbol, g(px));
        }
        return format!("{} {}", verb, symbol).trim().to_string();
    }
    if t == "DEPOSIT" || t == "CONTRIBUTION" {
        return "Deposit".into();
    }
    if t == "WITHDRAWAL" {
        return "Withdrawal".into();
    }
    if t == "INTERNAL_TRANSFER" {
        return if s.contains("SOURCE") { "Transfer out".into() } else { "Transfer in".into() };
    }
    if t == "DIVIDEND" {
        return if symbol.is_empty() { "Dividend".into() } else { format!("Dividend: {}", symbol) };
    }
    if t == "INTEREST" {
        return if s.contains("FPL") { "Stock Lending Earnings".into() } else { "Interest".into() };
    }
    if t == "FUNDS_CONVERSION" {
        return "Funds conversion".into();
    }
    if t == "FEE" || t == "REFUND" {
        return if t == "REFUND" { "Fee refund".into() } else { "Fee".into() };
    }
    if ["STOCK_DISTRIBUTION", "STKDIS", "SPIN", "SPINOFF"].contains(&t.as_str()) {
        return if symbol.is_empty() { "Stock distribution".into() } else { format!("Stock distribution: {}", symbol) };
    }
    if ["EXPIR", "EXPIRY", "EXPIRE", "ASSIGN", "ASSIGNMENT", "EXERCISE"].contains(&t.as_str())
        || t.contains("EXPIR")
        || t.contains("ASSIGN")
        || t.contains("EXERCISE")
    {
        let label = if t.contains("ASSIGN") { "Assign" } else if t.contains("EXERCISE") { "Exercise" } else { "Expir" };
        return format!("{} {}", label, symbol).trim().to_string();
    }
    if !symbol.is_empty() {
        return format!("{}: {}", t, symbol);
    }
    let titled = title_case(&t.replace('_', " "));
    if titled.is_empty() { "Activity".into() } else { titled }
}

/// Title case: every run of letters capitalised, the rest kept.
fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut start = true;
    for c in s.chars() {
        if c.is_alphabetic() {
            if start {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
            start = false;
        } else {
            out.push(c);
            start = true;
        }
    }
    out
}

/// One GraphQL row becomes two when a code
/// change names a different ticker -- the old one going out, the new one
/// coming in.
pub fn map_activity_rows(item: &Value, accounts: Option<&Value>) -> Vec<Value> {
    if !item.is_object() {
        return vec![];
    }
    let src = asset_symbol(item);
    let dst = counter_symbol(item);
    let qty = n(item, "assetQuantity").abs();
    if !src.is_empty() && !dst.is_empty() && src != dst && qty != 0.0 && is_corp_share_move(item) {
        let cid = { let c = field_s(item, "canonicalId").trim().to_string(); if c.is_empty() { "swap".to_string() } else { c } };
        let mut rows = Vec::new();
        for (symbol, signed, sign, suffix) in [
            (src.clone(), -qty, "negative", ":out"),
            (dst.clone(), qty, "positive", ":in"),
        ] {
            let mut m = item.as_object().cloned().unwrap_or_default();
            m.insert("assetSymbol".into(), json!(symbol));
            m.insert("counterAssetSymbol".into(), json!(""));
            m.insert("type".into(), json!("STKDIS"));
            m.insert("subType".into(), json!("STKDIS"));
            m.insert("assetQuantity".into(), json!(signed));
            m.insert("amount".into(), json!(0));
            m.insert("amountSign".into(), json!(sign));
            m.insert("canonicalId".into(), json!(format!("{}{}", cid, suffix)));
            if let Some(r) = map_activity(&Value::Object(m), accounts) {
                rows.push(r);
            }
        }
        return rows;
    }
    match map_activity(item, accounts) {
        Some(r) => vec![r],
        None => vec![],
    }
}

/// One `ActivityFeedItem` as a ledger row, or
/// nothing when the row is not one the book keeps.
pub fn map_activity(item: &Value, accounts: Option<&Value>) -> Option<Value> {
    if skip_activity(item) {
        return None;
    }
    let occurred = field_s(item, "occurredAt").trim().to_string();
    let transaction_date = date_only(&occurred);
    if transaction_date.is_empty() {
        return None;
    }

    let account_id = field_s(item, "accountId");
    let typ = upper(item, "type").replace('-', "_");
    let sub = upper(item, "subType").replace('-', "_");
    let ctyp = compact(&typ);
    let qty_raw = n(item, "assetQuantity");
    let qty_abs = qty_raw.abs();
    let cash = signed_cash(item);
    let amount_abs = n(item, "amount").abs();
    let fees = n(item, "fees").abs();
    let is_opt = is_option(item);
    let mut symbol = if is_opt { option_symbol(item) } else { asset_symbol(item) };

    let mut cur = upper(item, "currency");
    if cur != "CAD" && cur != "USD" {
        cur = if is_opt { "USD".into() } else { "CAD".into() };
    }

    let mut unit_price = 0.0;
    if qty_abs != 0.0 {
        // a Wealthsimple option `amount` is full contract cash: per share, times
        // a hundred, times the contracts
        unit_price = if is_opt { amount_abs / (qty_abs * 100.0) } else { amount_abs / qty_abs };
    }

    let mut activity_type = "Other".to_string();
    let mut activity_sub = if sub.is_empty() { typ.clone() } else { sub.clone() };
    let mut category = "other".to_string();
    let mut quantity = qty_abs;

    if typ == "DIY_BUY" {
        category = "trade".into();
        activity_type = "Trade".into();
        activity_sub = if is_opt {
            if is_to_close(&sub) { "BUYTOCLOSE".into() } else { "BUYTOOPEN".into() }
        } else {
            "BUY".into()
        };
        quantity = qty_abs;
    } else if typ == "DIY_SELL" {
        category = "trade".into();
        activity_type = "Trade".into();
        activity_sub = if is_opt {
            if is_to_close(&sub) { "SELLTOCLOSE".into() } else { "SELLTOOPEN".into() }
        } else {
            "SELL".into()
        };
        quantity = -qty_abs;
    } else if typ == "OPTIONS_BUY" {
        category = "trade".into();
        activity_type = "OPTIONS_BUY".into();
        activity_sub = if is_to_close(&sub) { "BUYTOCLOSE".into() } else { "BUYTOOPEN".into() };
        quantity = qty_abs;
    } else if typ == "OPTIONS_SELL" {
        category = "trade".into();
        activity_type = "OPTIONS_SELL".into();
        activity_sub = if is_to_close(&sub) { "SELLTOCLOSE".into() } else { "SELLTOOPEN".into() };
        quantity = -qty_abs;
    } else if typ.contains("MULTILEG") {
        // A filled combo or roll leg often has no quantity at all. A credit is
        // covered-call premium -- sell to open a short, not close a long. A
        // debit prefers buy-to-close; the FIFO match opens a long when there is
        // no short to close.
        category = "trade".into();
        if cash < 0.0 {
            activity_type = "OPTIONS_BUY".into();
            activity_sub = "BUYTOCLOSE".into();
            quantity = qty_abs;
        } else {
            activity_type = "OPTIONS_SELL".into();
            activity_sub = "SELLTOOPEN".into();
            quantity = if qty_abs != 0.0 { -qty_abs } else { 0.0 };
        }
    } else if ["EXPIR", "EXPIRY", "EXPIRE", "ASSIGN", "ASSIGNMENT", "EXERCISE"].contains(&typ.as_str())
        || typ.contains("EXPIR")
        || typ.contains("ASSIGN")
        || typ.contains("EXERCISE")
    {
        category = "option_event".into();
        activity_type = if typ.contains("ASSIGN") {
            "ASSIGN".into()
        } else if typ.contains("EXERCISE") {
            "EXERCISE".into()
        } else {
            "EXPIR".into()
        };
        let short_expir = typ.contains("SHORT_EXPIR") || (typ.contains("SHORT") && typ.contains("EXPIR"));
        activity_sub = if typ.contains("ASSIGN") {
            "BUYTOCLOSE".into()
        } else if short_expir {
            "BUY".into()
        } else if typ.contains("EXPIR") {
            "SELL".into()
        } else if compact(&sub).contains("COVER") || is_to_close(&sub) {
            "BUY".into()
        } else {
            "SELL".into()
        };
        quantity = if activity_sub == "SELL" { -qty_abs } else { qty_abs };
        // strike cash on an assignment is share delivery, not an option buyback
        if typ.contains("ASSIGN") || cash.abs() < 1e-12 {
            unit_price = 0.0;
        }
        if !is_opt && symbol.is_empty() {
            symbol = asset_symbol(item);
        }
    } else if typ == "DEPOSIT" || typ == "CONTRIBUTION" {
        activity_type = "Deposit".into();
        activity_sub = "deposit".into();
        category = "deposit".into();
    } else if typ == "WITHDRAWAL" {
        activity_type = "Withdrawal".into();
        activity_sub = "withdrawal".into();
        category = "withdrawal".into();
    } else if typ == "INTERNAL_TRANSFER"
        || ["TRFIN", "TRFOUT", "TRANSFERIN", "TRANSFEROUT", "INTERNALTRANSFER"].contains(&ctyp.as_str())
    {
        // a share TRFIN or TRFOUT is a custody move, not a sale
        activity_type = "Transfer".into();
        activity_sub = "transfer".into();
        category = "transfer".into();
    } else if typ == "DIVIDEND" && !is_corp_share_move(item) {
        activity_type = "Dividend".into();
        activity_sub = "dividend".into();
        category = "dividend".into();
    } else if typ == "INTEREST" || sub.contains("FPL_INTEREST") || ctyp == "FPLINTEREST" {
        activity_type = "Interest".into();
        activity_sub = "interest".into();
        category = "interest".into();
    } else if typ == "FUNDS_CONVERSION" {
        activity_type = "FxExchange".into();
        activity_sub = "fx".into();
        category = "fx".into();
    } else if typ == "FEE" || typ == "REFUND" {
        activity_type = if typ == "REFUND" { "Refund".into() } else { "Fee".into() };
        activity_sub = "fee".into();
        category = "fee".into();
    } else if is_corp_share_move(item)
        || ["STOCK_DISTRIBUTION", "STKDIS", "SPIN", "SPINOFF", "STK_DIS"].contains(&typ.as_str())
        || ctyp.contains("STKDIS")
        || ctyp.contains("STOCKDISTRIBUTION")
        || compact(&sub).contains("STOCKDISTRIBUTION")
    {
        // A name change is -N then +N of the same ticker; fold_stkdis nets the
        // sale against the purchase. An unsigned quantity would open 2N at $0,
        // and whatever is left over opens at $0 so a later sale has lots.
        activity_type = "STKDIS".into();
        category = "trade".into();
        unit_price = 0.0;
        let sign = field_s(item, "amountSign").trim().to_lowercase();
        let mut outgoing = qty_raw < 0.0 || ["negative", "debit", "-", "neg"].contains(&sign.as_str());
        // A lone international code change names the old ticker with a positive
        // quantity. Those shares were replaced, not bought.
        if !outgoing && is_code_change(item) && counter_symbol(item).is_empty() && !compact(&field_s(item, "type")).contains("STKDIS") {
            outgoing = true;
        }
        if outgoing {
            activity_sub = "SELL".into();
            quantity = -qty_abs;
        } else {
            activity_sub = "BUY".into();
            quantity = qty_abs;
        }
    } else {
        activity_type = { let t = field_s(item, "type"); if t.is_empty() { "Other".into() } else { t } };
        activity_sub = { let s = field_s(item, "subType"); if s.is_empty() { "other".into() } else { s } };
        category = "other".into();
    }

    if ["SELL", "SELLTOOPEN", "SELLTOCLOSE"].contains(&activity_sub.as_str()) && qty_abs != 0.0 {
        quantity = -qty_abs;
    }

    let sign = field_s(item, "amountSign").trim().to_lowercase();
    let direction = if ["negative", "debit", "-", "neg"].contains(&sign.as_str()) || cash < 0.0 {
        "DEBIT"
    } else if ["positive", "credit", "+", "pos"].contains(&sign.as_str()) || cash > 0.0 {
        "CREDIT"
    } else {
        ""
    };

    let mut cid = field_s(item, "canonicalId").trim().to_string();
    if bagholder_store::activities::looks_like_homemade_id(&cid) {
        cid = String::new();
    }

    let desc = human_desc(item, &typ, &sub, &symbol, quantity, unit_price, cash);
    let name = {
        let a = field_s(item, "aftOriginatorName");
        if !a.is_empty() { a } else {
            let i = field_s(item, "institutionName");
            if !i.is_empty() { i } else { symbol.clone() }
        }
    };
    let fifo_id = match accounts {
        Some(_) => fifo_pool_ids(accounts).get(&account_id).cloned().unwrap_or_else(|| account_id.clone()),
        None => account_id.clone(),
    };
    let security_id = field_s(item, "securityId").trim().to_string();

    Some(json!({
        "canonicalId": if cid.is_empty() { Value::Null } else { json!(cid) },
        "occurredAt": occurred,
        "transactionDate": transaction_date,
        "settlementDate": transaction_date,
        "accountId": account_id,
        "bookId": account_id,
        "fifoId": fifo_id,
        "accountType": account_type(&account_id, accounts),
        "activityType": activity_type,
        "activitySubType": activity_sub,
        "description": desc,
        "direction": direction,
        "symbol": symbol,
        "name": name,
        "currency": cur,
        "quantity": quantity,
        "unitPrice": unit_price,
        "commission": fees,
        "netCashAmount": cash,
        "category": category,
        "balance": Value::Null,
        "source": "wealthsimple",
        "rawType": field_s(item, "type"),
        "aftType": field_s(item, "aftTransactionType"),
        "counterSymbol": counter_symbol(item),
        "securityId": if security_id.is_empty() { Value::Null } else { json!(security_id) },
    }))
}
