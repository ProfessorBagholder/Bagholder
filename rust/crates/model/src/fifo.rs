//! FIFO matching per (account, symbol, currency): `match_fifo`.
//!
//! Returns the closed slices, the lots still open, and the fills that could
//! not be matched to anything. Option rolls are the hard part: Wealthsimple
//! posts a roll as one multileg row that closes a contract and never posts the
//! leg it opened, so the quantity is carried forward in `rolled` and closed
//! against later buy-backs.

use serde_json::Value;
use std::collections::{HashMap, HashSet};

use crate::dates::{days_between, option_expiry};
use crate::normalize::{
    book_key, fifo_account, is_close_only, is_multileg, kind_of, normalize_activity,
    opening_direction, roll_key,
};
use crate::symbols::{is_option_symbol, option_multiplier, option_right, underlying_symbol};
use crate::value::{compact, field_num, field_s, fmt8, get, s as vs, EPS};

// --------------------------------------------------------------------------
// the rows the matcher works with
// --------------------------------------------------------------------------

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lot {
    pub qty: f64,
    pub price: f64,
    pub date: String,
    pub when: String,
    pub commission: f64,
    pub direction: String,
    pub account_id: String,
    pub account_type: String,
    pub symbol: String,
    pub name: String,
    pub currency: String,
    pub kind: String,
    pub activity_id: String,
    pub security_id: String,
    pub rt: Option<String>,
    pub flags: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Slice {
    pub id: String,
    pub rt: Option<String>,
    pub account_id: String,
    pub account_type: String,
    pub account: String,
    pub symbol: String,
    pub name: String,
    pub currency: String,
    pub kind: String,
    pub side: String,
    pub quantity: f64,
    pub entry_price: f64,
    pub exit_price: f64,
    pub entry_date: String,
    pub exit_date: String,
    pub entry_when: String,
    pub exit_when: String,
    pub hold_days: i64,
    pub commission: f64,
    pub entry_commission: f64,
    pub exit_commission: f64,
    pub pnl: f64,
    pub pnl_cad: f64,
    pub open_direction: String,
    pub buy_activity_id: String,
    pub sell_activity_id: String,
    pub security_id: String,
    pub flags: Vec<String>,
    /// Filled in by `apply_fx`; absent until then.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fees_cad: Option<f64>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unmatched {
    pub symbol: String,
    pub currency: String,
    pub side: String,
    pub quantity: f64,
    pub price: f64,
    pub date: String,
    pub description: String,
    pub account_id: String,
    pub account: String,
    pub activity_id: String,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Fill {
    a: Value,
    side: String,
    qty: f64,
    roll_direction: Option<String>,
    rt_before: Option<String>,
    /// Where this row came from in the caller's list, when it came from one.
    /// The inference rewrites a multileg row's quantity, price and sub-type,
    /// and the fill stands for the very row the activity list holds, so the
    /// rewrite has to land back there too: `actsById` feeds the fill rows the
    /// page prints under a trade.
    src: Option<usize>,
}

pub struct Matched {
    pub closed: Vec<Slice>,
    pub open: Vec<Lot>,
    pub unmatched: Vec<Unmatched>,
}

// --------------------------------------------------------------------------
// identity
// --------------------------------------------------------------------------

/// `stable_trade_id`: stable from the first fill, so a journal entry
/// survives later exits.
pub fn stable_trade_id(t: &Slice) -> String {
    [
        t.account_id.clone(),
        t.symbol.clone(),
        t.currency.clone(),
        t.entry_date.clone(),
        t.exit_date.clone(),
        fmt8(t.quantity),
        fmt8(t.entry_price),
        fmt8(t.exit_price),
        t.side.clone(),
    ]
    .join("|")
}

/// `trade_side`: which way a row goes, read from the sub-type, then the
/// type, then the sign of the quantity.
pub fn trade_side(a: &Value) -> String {
    let sub = {
        let v = field_s(a, "activitySubType");
        if v.is_empty() { field_s(a, "activity_sub_type") } else { v }
    };
    let c = compact(&sub);
    if matches!(c.as_str(), "BUY" | "BUYTOOPEN" | "BTO" | "BUYTOCLOSE" | "BTC") || c.starts_with("BUY") {
        return "BUY".into();
    }
    if matches!(c.as_str(), "SELL" | "SELLTOOPEN" | "STO" | "SELLTOCLOSE" | "STC") || c.starts_with("SELL") {
        return "SELL".into();
    }
    let typ = {
        let v = field_s(a, "activityType");
        if v.is_empty() { field_s(a, "activity_type") } else { v }
    };
    let t = compact(&typ);
    if t.starts_with("BUY") { return "BUY".into(); }
    if t.starts_with("SELL") { return "SELL".into(); }
    let qty = field_num(a, "quantity");
    if qty > 0.0 { return "BUY".into(); }
    if qty < 0.0 { return "SELL".into(); }
    String::new()
}

fn flags_of(a: &Value) -> Vec<String> {
    match get(a, "flags") {
        Some(Value::Array(xs)) => xs.iter().map(|x| vs(Some(x))).collect(),
        _ => vec![],
    }
}

fn has_flag(a: &Value, f: &str) -> bool { flags_of(a).iter().any(|x| x == f) }

fn set_num(a: &mut Value, key: &str, v: f64) {
    if let Value::Object(m) = a {
        m.insert(key.into(), serde_json::Number::from_f64(v).map(Value::Number).unwrap_or(Value::Null));
    }
}

fn set_str(a: &mut Value, key: &str, v: &str) {
    if let Value::Object(m) = a { m.insert(key.into(), Value::String(v.into())); }
}

// --------------------------------------------------------------------------
// fill ordering
// --------------------------------------------------------------------------

/// `_fill_rank`: within a day, opens come before closes so a same-day
/// round trip matches against its own entry rather than an older lot.
fn fill_rank(f: &Fill) -> u8 {
    let t = compact(&field_s(&f.a, "activityType"));
    let s = compact(&field_s(&f.a, "activitySubType"));
    let blob = format!("{}{}", t, s);
    if (blob.contains("TOOPEN") || t == "STO" || s == "STO") && f.side == "SELL" { return 0; }
    if is_close_only(&f.a) && f.side == "BUY" { return 1; }
    if f.side == "BUY" { return 2; }
    if blob.contains("TOCLOSE") || t == "STC" || s == "STC" { return 3; }
    4
}

fn fill_sort_key(f: &Fill) -> (String, u8, String, String) {
    (field_s(&f.a, "transactionDate"), fill_rank(f), field_s(&f.a, "occurredAt"), field_s(&f.a, "id"))
}

// --------------------------------------------------------------------------
// slices
// --------------------------------------------------------------------------

/// `_make_slice`: one closed piece of a lot against one fill.
fn make_slice(lot: &Lot, fill_qty: f64, side: &str, a: &Value, matched: f64, symbol: Option<&str>) -> Slice {
    let exit_commission = if fill_qty > 0.0 { field_num(a, "commission") * (matched / fill_qty) } else { 0.0 };
    let entry_commission = if lot.qty > 0.0 { lot.commission * (matched / lot.qty) } else { 0.0 };
    let commission = entry_commission + exit_commission;
    let sym = symbol.unwrap_or(&lot.symbol).to_string();
    let mult = option_multiplier(&sym);
    let exit_px = field_num(a, "unitPrice");
    let raw_pnl = if lot.direction == "LONG" {
        (exit_px - lot.price) * matched * mult
    } else {
        (lot.price - exit_px) * matched * mult
    };
    // A carried symbol takes the fill's name; the lot's own name otherwise.
    let name = if symbol.is_some() {
        let n = field_s(a, "name");
        if n.is_empty() { lot.name.clone() } else { n }
    } else {
        lot.name.clone()
    };
    let security_id = if !lot.security_id.is_empty() { lot.security_id.clone() } else { field_s(a, "securityId") };
    let mut flags: Vec<String> = lot.flags.iter().cloned().chain(flags_of(a)).collect();
    flags.sort();
    flags.dedup();
    let mut t = Slice {
        id: String::new(),
        rt: lot.rt.clone(),
        account_id: lot.account_id.clone(),
        account_type: lot.account_type.clone(),
        account: lot.account_type.clone(),
        symbol: sym,
        name,
        currency: lot.currency.clone(),
        kind: lot.kind.clone(),
        side: side.to_string(),
        quantity: matched,
        entry_price: lot.price,
        exit_price: exit_px,
        entry_date: lot.date.clone(),
        exit_date: field_s(a, "transactionDate"),
        entry_when: lot.when.clone(),
        exit_when: field_s(a, "occurredAt"),
        hold_days: days_between(&lot.date, &field_s(a, "transactionDate")),
        commission,
        entry_commission,
        exit_commission,
        pnl: raw_pnl - commission,
        pnl_cad: raw_pnl - commission,
        open_direction: lot.direction.clone(),
        buy_activity_id: lot.activity_id.clone(),
        sell_activity_id: field_s(a, "id"),
        security_id,
        flags,
        fees_cad: None,
    };
    t.id = stable_trade_id(&t);
    t
}

/// `_dust`: a sale that exceeds the lots by a residue is rounding, not a
/// short. Crypto quantities come back net of in-kind fees, so 1% of the fill
/// is tolerated there; elsewhere only float noise or under a cent of value.
fn dust(remaining: f64, fill_qty: f64, a: &Value) -> bool {
    let qty = fill_qty;
    let px = field_num(a, "unitPrice").abs();
    if remaining <= 1e-6 * f64::max(1.0, qty) { return true; }
    let kind = { let k = field_s(a, "kind"); if k.is_empty() { kind_of(a) } else { k } };
    if kind == "Crypto" && remaining <= 0.01 * qty { return true; }
    px > 0.0 && remaining * px * option_multiplier(&field_s(a, "symbol")) < 0.01
}

// --------------------------------------------------------------------------
// option quantity inference (Wealthsimple multileg rows often carry qty 0)
// --------------------------------------------------------------------------

/// `_is_clean_option_qty`: a quantity is plausible when it makes the
/// cash divide into a price quoted in cents (or in hundredths of a cent).
fn is_clean_option_qty(cash: f64, qty: f64) -> bool {
    if !(qty > 0.0) { return false; }
    let px = cash.abs() / (qty * 100.0);
    if px < 0.0 { return false; }
    if (px * 100.0 - (px * 100.0).round()).abs() < 1e-6 { return true; }
    (px * 10000.0 - (px * 10000.0).round()).abs() < 1e-4
}

/// `_infer_standalone_option_qty`: the smallest contract count that
/// gives a clean price, falling back to one.
fn infer_standalone_option_qty(cash: f64) -> f64 {
    let abs_cash = cash.abs();
    if !(abs_cash > 0.0) { return 0.0; }
    let max_qty = i64::min(10000, i64::max(1, abs_cash.round() as i64));
    for q in 1..=max_qty {
        if is_clean_option_qty(abs_cash, q as f64) { return q as f64; }
    }
    1.0
}

#[derive(Default, Clone, Copy)]
struct Dirs { long: f64, short: f64 }

impl Dirs {
    fn get(&self, d: &str) -> f64 { if d == "LONG" { self.long } else { self.short } }
    fn add(&mut self, d: &str, v: f64) { if d == "LONG" { self.long += v } else { self.short += v } }
    fn sub(&mut self, d: &str, v: f64) { self.add(d, -v) }
}

fn set_fill_side(f: &mut Fill, side: &str, sub: &str) {
    f.side = side.into();
    set_str(&mut f.a, "activitySubType", sub);
    let q = field_num(&f.a, "quantity").abs();
    if q > 0.0 {
        set_num(&mut f.a, "quantity", if side == "SELL" { -q } else { q });
    }
}

/// `_resolve_option_fill_side`: an expiry or assignment row does not say
/// which way it goes, so it is read from what the book still holds.
fn resolve_option_fill_side(f: &mut Fill, rem: &Dirs) {
    let raw = format!("{}{}", compact(&field_s(&f.a, "rawType")), compact(&field_s(&f.a, "activityType")));
    let expirish = raw.contains("EXPIR") || raw.contains("ASSIGN") || raw.contains("EXERCISE");
    if expirish {
        if raw.contains("ASSIGN") || raw.contains("SHORTEXPIR") {
            let sub = if raw.contains("ASSIGN") { "BUYTOCLOSE" } else { "BUY" };
            set_fill_side(f, "BUY", sub);
        } else if raw.contains("EXPIR") && !raw.contains("SHORT") {
            set_fill_side(f, "SELL", "SELL");
        } else if f.side == "BUY" && rem.long > EPS && rem.short <= EPS {
            set_fill_side(f, "SELL", "SELL");
        } else if f.side == "SELL" && rem.short > EPS && rem.long <= EPS {
            set_fill_side(f, "BUY", "BUY");
        }
        return;
    }
    if !(raw.contains("MULTILEG") || is_close_only(&f.a)) { return; }
    if f.side == "BUY" {
        set_str(&mut f.a, "activitySubType", if rem.short > EPS { "BUYTOCLOSE" } else { "BUYTOOPEN" });
    } else if f.side == "SELL" {
        set_str(&mut f.a, "activitySubType", if rem.long > EPS { "SELLTOCLOSE" } else { "SELLTOOPEN" });
    }
}

/// `infer_zero_qty_option_fills`: fills in the quantity a multileg row
/// left at zero, and decides which direction a roll is closing, by walking the
/// fills in order and tracking what each book and each roll chain still holds.
fn infer_zero_qty_option_fills(fills: &mut Vec<Fill>) {
    let mut remaining: HashMap<String, Dirs> = HashMap::new();
    let mut pools: HashMap<(String, String, &'static str), Dirs> = HashMap::new();
    let mut zeros_by_book: HashMap<String, Vec<usize>> = HashMap::new();

    for (i, f) in fills.iter().enumerate() {
        let qty = field_num(&f.a, "quantity").abs();
        let cash = field_num(&f.a, "netCashAmount");
        if !is_option_symbol(&field_s(&f.a, "symbol")) || f.side.is_empty() { continue; }
        if qty == 0.0 && cash.abs() > 1e-9 {
            zeros_by_book.entry(book_key(&f.a)).or_default().push(i);
        }
    }

    for i in 0..fills.len() {
        if fills[i].side.is_empty() { continue; }
        let a_snapshot = fills[i].a.clone();
        let key = book_key(&a_snapshot);
        let rkey = roll_key(&a_snapshot);
        let is_opt = is_option_symbol(&field_s(&a_snapshot, "symbol"));
        let rem_now = *remaining.entry(key.clone()).or_default();

        if is_opt && is_multileg(&a_snapshot) {
            // A roll: this row closes what the contract holds (or what an
            // earlier roll carried forward), and the same quantity moves to
            // the next contract, which the broker never posts.
            let pool = *pools.entry(rkey.clone()).or_default();
            let direction: String = if rem_now.short > EPS {
                "SHORT".into()
            } else if rem_now.long > EPS {
                "LONG".into()
            } else if pool.short >= pool.long {
                "SHORT".into()
            } else {
                "LONG".into()
            };
            let open_sz = rem_now.get(&direction) + pool.get(&direction);
            let mut qty = field_num(&a_snapshot, "quantity").abs();
            let cash = field_num(&a_snapshot, "netCashAmount");
            if qty == 0.0 {
                let upcoming = zeros_by_book.get(&key).map_or(0, |v| v.iter().filter(|j| **j > i).count());
                if rem_now.get(&direction) > EPS && upcoming == 0 {
                    qty = rem_now.get(&direction);
                } else if open_sz > EPS {
                    let cap = i64::max(1, (open_sz + 1e-9) as i64);
                    let mut picked = 0.0;
                    for q in 1..=cap {
                        if is_clean_option_qty(cash, q as f64) { picked = q as f64; break; }
                    }
                    qty = if picked > 0.0 { picked } else { infer_standalone_option_qty(cash) };
                    if qty > open_sz { qty = open_sz; }
                } else {
                    qty = infer_standalone_option_qty(cash);
                }
                let px = if qty > 0.0 { cash.abs() / (qty * 100.0) } else { 0.0 };
                set_num(&mut fills[i].a, "unitPrice", px);
            }
            fills[i].side = if direction == "SHORT" { "BUY".into() } else { "SELL".into() };
            let sub = if direction == "SHORT" { "BUYTOCLOSE" } else { "SELLTOCLOSE" };
            set_str(&mut fills[i].a, "activitySubType", sub);
            set_num(&mut fills[i].a, "quantity", if direction == "SHORT" { qty } else { -qty });
            fills[i].qty = qty;
            fills[i].roll_direction = Some(direction.clone());

            let closed = f64::min(qty, rem_now.get(&direction));
            let r = remaining.entry(key.clone()).or_default();
            r.sub(&direction, closed);
            let p = pools.entry(rkey.clone()).or_default();
            p.sub(&direction, f64::min(qty - closed, p.get(&direction)));
            if closed > EPS || qty > EPS { p.add(&direction, qty); }
            continue;
        }

        if is_opt {
            let rem_copy = rem_now;
            resolve_option_fill_side(&mut fills[i], &rem_copy);
        }
        let a_now = fills[i].a.clone();
        let mut qty = field_num(&a_now, "quantity").abs();
        let cash = field_num(&a_now, "netCashAmount");
        if is_opt && qty == 0.0 {
            let closing_dir = if fills[i].side == "BUY" { "SHORT" } else { "LONG" };
            let open_sz = rem_now.get(closing_dir);
            if cash.abs() > 1e-9 {
                let upcoming = zeros_by_book.get(&key).map_or(0, |v| v.iter().filter(|j| **j > i).count());
                if open_sz > 0.0 && upcoming == 0 {
                    qty = open_sz;
                } else if open_sz > 0.0 {
                    let cap = i64::max(1, (open_sz + 1e-9) as i64);
                    let mut picked = 0.0;
                    for q in 1..=cap {
                        if is_clean_option_qty(cash, q as f64) { picked = q as f64; break; }
                    }
                    qty = if picked > 0.0 { picked } else { infer_standalone_option_qty(cash) };
                    if qty > open_sz { qty = open_sz; }
                } else {
                    qty = infer_standalone_option_qty(cash);
                }
                let px = if qty > 0.0 { cash.abs() / (qty * 100.0) } else { 0.0 };
                set_num(&mut fills[i].a, "unitPrice", px);
            } else if open_sz > 0.0 && is_close_only(&a_now) {
                qty = open_sz;
                set_num(&mut fills[i].a, "unitPrice", 0.0);
            }
            if qty > 0.0 {
                let signed = if fills[i].side == "SELL" { -qty } else { qty };
                set_num(&mut fills[i].a, "quantity", signed);
                fills[i].qty = qty;
            }
        }
        if is_opt && (compact(&field_s(&a_now, "rawType")).contains("ASSIGN")
            || compact(&field_s(&a_now, "activityType")).contains("ASSIGN"))
        {
            set_num(&mut fills[i].a, "unitPrice", 0.0);
        }
        if fills[i].qty > 0.0 {
            let closing_dir = if fills[i].side == "BUY" { "SHORT" } else { "LONG" };
            let side = fills[i].side.clone();
            let opening = opening_direction(&fills[i].a, &side);
            let mut left = fills[i].qty;
            let r = remaining.entry(key.clone()).or_default();
            let close_amt = f64::min(left, r.get(closing_dir));
            r.sub(closing_dir, close_amt);
            left -= close_amt;
            if left > EPS && is_opt {
                let p = pools.entry(rkey.clone()).or_default();
                let pooled = f64::min(left, p.get(closing_dir));
                p.sub(closing_dir, pooled);
                left -= pooled;
            }
            if left > EPS {
                if let Some(op) = opening {
                    remaining.entry(key.clone()).or_default().add(op, left);
                }
            }
        }
    }
}

// --------------------------------------------------------------------------
// corporate actions
// --------------------------------------------------------------------------

/// `replacement_index`: per (account, symbol, currency), the first date
/// the ticker was removed and the dates of real trades, so the sale of a
/// renamed holding can find the old book.
struct Replacement {
    removed: HashMap<(String, String, String), String>,
    trades: HashMap<(String, String, String), Vec<String>>,
}

fn removal_marker(raw: &str) -> bool {
    ["CODECHANGE", "SYMBOLCHANGE", "TICKERCHANGE", "LISTINGSTATUS", "SECURITYSWAP"]
        .iter()
        .any(|m| raw.contains(m))
}

fn replacement_index(activities: &[Value]) -> Replacement {
    let mut removed: HashMap<(String, String, String), String> = HashMap::new();
    let mut trades: HashMap<(String, String, String), Vec<String>> = HashMap::new();
    for a in activities {
        let key = (fifo_account(a), field_s(a, "symbol"), field_s(a, "currency"));
        let t = compact(&field_s(a, "activityType"));
        let d = field_s(a, "transactionDate");
        if t == "STKDIS" {
            let sub = compact(&field_s(a, "activitySubType"));
            if sub == "SELL" || field_num(a, "quantity") < 0.0 {
                if !d.is_empty() && removed.get(&key).map_or(true, |o| d < *o) {
                    removed.insert(key.clone(), d.clone());
                }
            }
            continue;
        }
        let raw = format!("{}{}", compact(&field_s(a, "rawType")), compact(&field_s(a, "aftType")));
        if removal_marker(&raw) && !d.is_empty() && removed.get(&key).map_or(true, |o| d < *o) {
            removed.insert(key.clone(), d.clone());
        }
        let cat = field_s(a, "category");
        if (cat == "trade" || cat == "option_event") && !trade_side(a).is_empty() {
            trades.entry(key).or_default().push(d);
        }
    }
    Replacement { removed, trades }
}

/// `ticker_was_replaced`: the ticker went away before this date and
/// nothing has traded in it since.
fn ticker_was_replaced(ix: &Replacement, account: &str, symbol: &str, currency: &str, by_date: &str) -> bool {
    let key = (account.to_string(), symbol.to_string(), currency.to_string());
    let removed_on = match ix.removed.get(&key) { Some(d) if !d.is_empty() => d, _ => return false };
    if *removed_on > by_date.to_string() { return false; }
    !ix.trades.get(&key).map_or(false, |ds| ds.iter().any(|d| d > removed_on))
}

/// `split_markers`: Wealthsimple posts a share split as a quantity-zero
/// corporate action with no ratio, so the ratio is inferred from the median
/// fill price on either side of it. Lot quantities are multiplied by the
/// factor and prices divided by it.
fn split_markers(activities: &[Value]) -> HashMap<(String, String, String), f64> {
    let mut out = HashMap::new();
    let mut by_book: HashMap<(String, String), Vec<&Value>> = HashMap::new();
    for a in activities {
        let cat = field_s(a, "category");
        if (cat != "trade" && cat != "option_event") || field_s(a, "symbol").is_empty() { continue; }
        by_book.entry((fifo_account(a), field_s(a, "symbol"))).or_default().push(a);
    }
    for a in activities {
        if compact(&field_s(a, "activityType")) != "STKDIS" { continue; }
        if compact(&field_s(a, "rawType")) != "CORPORATEACTION" { continue; }
        if field_num(a, "quantity").abs() > EPS { continue; }
        let day = field_s(a, "transactionDate");
        // the marker's currency does not always match the fills'; key on account+symbol
        let key = (fifo_account(a), field_s(a, "symbol"));
        let mut priced: Vec<&&Value> = by_book
            .get(&key)
            .map(|v| v.iter().filter(|x| field_num(x, "unitPrice") > 0.0 && compact(&field_s(x, "activityType")) != "STKDIS").collect())
            .unwrap_or_default();
        priced.sort_by(|x, y| {
            (field_s(x, "transactionDate"), field_s(x, "occurredAt"))
                .cmp(&(field_s(y, "transactionDate"), field_s(y, "occurredAt")))
        });
        let mut before: Vec<f64> = priced.iter().filter(|x| field_s(x, "transactionDate") < day).map(|x| field_num(x, "unitPrice")).collect();
        let mut after: Vec<f64> = priced.iter().filter(|x| field_s(x, "transactionDate") >= day).map(|x| field_num(x, "unitPrice")).collect();
        if before.len() > 3 { before = before.split_off(before.len() - 3); }
        after.truncate(3);
        if before.is_empty() || after.is_empty() { continue; }
        before.sort_by(|p, q| p.partial_cmp(q).unwrap());
        after.sort_by(|p, q| p.partial_cmp(q).unwrap());
        let pre = before[before.len() / 2];
        let post = after[after.len() / 2];
        if !(pre > 0.0) || !(post > 0.0) { continue; }
        let ratio = post / pre;
        let (n, factor) = if ratio >= 1.5 {
            let n = ratio.round();
            (n, 1.0 / n)
        } else if ratio <= 1.0 / 1.5 {
            let n = (1.0 / ratio).round();
            (n, n)
        } else {
            continue;
        };
        if n < 2.0 || (ratio - (1.0 / factor)).abs() / (1.0 / factor) > 0.35 { continue; }
        out.insert((fifo_account(a), field_s(a, "symbol"), day), factor);
    }
    out
}

// --------------------------------------------------------------------------
// the matcher
// --------------------------------------------------------------------------

/// Books in insertion order, because the search for a renamed ticker takes the
/// first book that matches.
#[derive(Default)]
struct Books {
    keys: Vec<String>,
    map: HashMap<String, Vec<Lot>>,
}

impl Books {
    fn ensure(&mut self, key: &str) {
        if !self.map.contains_key(key) {
            self.keys.push(key.to_string());
            self.map.insert(key.to_string(), Vec::new());
        }
    }
    fn get(&self, key: &str) -> &[Lot] { self.map.get(key).map(|v| v.as_slice()).unwrap_or(&[]) }
    fn at(&mut self, key: &str) -> &mut Vec<Lot> { self.ensure(key); self.map.get_mut(key).unwrap() }
}

#[derive(Default)]
struct RollPool {
    long: Vec<Lot>,
    short: Vec<Lot>,
    rt: Option<String>,
}

impl RollPool {
    fn side(&mut self, d: &str) -> &mut Vec<Lot> { if d == "LONG" { &mut self.long } else { &mut self.short } }
    fn is_empty(&self) -> bool { self.long.is_empty() && self.short.is_empty() }
}

fn lot_from(a: &Value, qty: f64, price: f64, direction: &str, commission: f64, kind: &str, rt: Option<String>, flags: Vec<String>) -> Lot {
    Lot {
        qty,
        price,
        date: field_s(a, "transactionDate"),
        when: field_s(a, "occurredAt"),
        commission,
        direction: direction.to_string(),
        account_id: field_s(a, "accountId"),
        account_type: fifo_account(a),
        symbol: field_s(a, "symbol"),
        name: field_s(a, "name"),
        currency: field_s(a, "currency"),
        kind: kind.to_string(),
        activity_id: field_s(a, "id"),
        security_id: field_s(a, "securityId"),
        rt,
        flags,
    }
}

/// `match_fifo`.
pub fn match_fifo(activities: &[Value]) -> Matched {
    let mut owned = activities.to_vec();
    match_fifo_in_place(&mut owned)
}

/// `match_fifo`, writing the inference back into `activities`.
///
/// A row that already carries `flags` is one the caller normalized, and the
/// fill stands for that same row, so what the inference decides about it --
/// the contract count behind a quantity-zero multileg, the price that implies,
/// which way it closes -- is visible to the caller afterwards. A row this
/// function normalizes itself is a fresh copy, and is left alone.
pub fn match_fifo_in_place(activities: &mut Vec<Value>) -> Matched {
    let prepared: Vec<(Option<usize>, Value)> = activities
        .iter()
        .enumerate()
        .map(|(i, a)| if a.get("flags").is_some() { (Some(i), a.clone()) } else { (None, normalize_activity(a)) })
        .filter(|(_, a)| !has_flag(a, "pending-distribution"))
        .collect();
    let normalized: Vec<Value> = prepared.iter().map(|(_, a)| a.clone()).collect();
    let folded = crate::normalize::fold_stkdis_indexed(&prepared);

    let mut fills: Vec<Fill> = Vec::new();
    for (src, a) in &folded {
        let cat = field_s(a, "category");
        if (cat != "trade" && cat != "option_event") || field_s(a, "symbol").is_empty() { continue; }
        let side = trade_side(a);
        if side.is_empty() { continue; }
        fills.push(Fill { a: a.clone(), side, qty: field_num(a, "quantity").abs(), roll_direction: None, rt_before: None, src: *src });
    }
    fills.sort_by(|x, y| fill_sort_key(x).cmp(&fill_sort_key(y)));
    infer_zero_qty_option_fills(&mut fills);
    // What the inference decided stands even for a fill it left at zero.
    for f in &fills {
        if let Some(i) = f.src { activities[i] = f.a.clone(); }
    }
    let usable: Vec<Fill> = fills.into_iter().filter(|f| f.qty > 0.0).collect();
    let mut rewritten: Vec<(usize, Value)> = Vec::new();

    let mut books = Books::default();
    let mut rt_open: HashMap<String, Option<String>> = HashMap::new();
    let mut closed: Vec<Slice> = Vec::new();
    let mut unmatched: Vec<Unmatched> = Vec::new();
    let mut rolled: Vec<((String, String, &'static str), RollPool)> = Vec::new();
    let mut rolled_keys: HashSet<(String, String, &'static str)> = HashSet::new();

    let replaced = replacement_index(&normalized);
    let splits = split_markers(&normalized);
    let mut pending_splits: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    for ((acct, sym, day), factor) in &splits {
        pending_splits.entry(format!("{}::{}", acct, sym)).or_default().push((day.clone(), *factor));
    }

    for fi in 0..usable.len() {
        let mut fill = usable[fi].clone();
        let a_key = book_key(&fill.a);
        books.ensure(&a_key);
        apply_splits(&mut books, &mut pending_splits, &a_key, &field_s(&fill.a, "transactionDate"));

        let sym = field_s(&fill.a, "symbol");
        if is_option_symbol(&sym) && is_multileg(&fill.a) && fill.roll_direction.is_some() {
            // Roll: close this contract (its book, then the legs an earlier
            // roll carried forward) and carry the same quantity to the leg the
            // broker never posted. A debit belongs to the closed leg's exit, a
            // credit to the new leg's entry.
            let direction = fill.roll_direction.clone().unwrap();
            let cash = field_num(&fill.a, "netCashAmount");
            let per = if fill.qty > 0.0 { cash.abs() / (fill.qty * 100.0) } else { 0.0 };
            let debit = cash < 0.0;
            let exit_px = if (direction == "SHORT") == debit { per } else { 0.0 };
            let entry_px = if (direction == "SHORT") != debit { per } else { 0.0 };
            set_num(&mut fill.a, "unitPrice", exit_px);
            let before = closed.len();
            fill.rt_before = if books.get(&a_key).is_empty() { None } else { rt_open.get(&a_key).cloned().flatten() };
            let mut remaining = close_against(&mut books, &mut rt_open, &a_key, &fill, &fill.a.clone(), fill.qty, None, &mut closed);
            remaining = close_rolled(&mut books, &mut rt_open, &mut rolled, &rolled_keys, &fill, &direction, remaining, &mut closed);
            let moved = fill.qty - remaining;
            let rk = roll_key(&fill.a);
            rolled_keys.insert(rk.clone());
            if moved > EPS {
                for s in closed[before..].iter_mut() {
                    if !s.flags.iter().any(|f| f == "rolled") { s.flags.push("rolled".into()); }
                }
                let chain_rt = fill
                    .rt_before
                    .clone()
                    .or_else(|| pool_of(&mut rolled, &rk).rt.clone())
                    .or_else(|| closed.get(before).and_then(|s| s.rt.clone()));
                let pool = pool_of(&mut rolled, &rk);
                pool.rt = chain_rt.clone();
                let mut lot = lot_from(&fill.a, moved, entry_px, &direction, 0.0, "Options", chain_rt, vec!["rolled-in".into()]);
                lot.security_id = String::new();
                pool.side(&direction).push(lot);
            }
            if remaining > EPS {
                // nothing to roll: this multileg simply opened a position
                let opening = if debit { "LONG" } else { "SHORT" };
                set_num(&mut fill.a, "unitPrice", per);
                fill.side = if opening == "LONG" { "BUY".into() } else { "SELL".into() };
                if books.get(&a_key).is_empty() || rt_open.get(&a_key).cloned().flatten().is_none() {
                    rt_open.insert(a_key.clone(), Some(format!("rt:{}", field_s(&fill.a, "id"))));
                }
                let rt = rt_open.get(&a_key).cloned().flatten();
                let lot = lot_from(&fill.a, remaining, per, opening, 0.0, "Options", rt, flags_of(&fill.a));
                books.at(&a_key).push(lot);
            }
            if let Some(i) = fill.src { rewritten.push((i, fill.a.clone())); }
            continue;
        }

        if has_flag(&fill.a, "transfer-out") {
            // Coins sent out of the account leave at cost: off the open lots
            // first in first out, no slice, no P&L, not a fill of the trade.
            let mut remaining = fill.qty;
            let book = books.at(&a_key);
            while remaining > EPS && !book.is_empty() && book[0].direction == "LONG" {
                let matched = f64::min(book[0].qty, remaining);
                if book[0].qty > 0.0 {
                    book[0].commission *= (book[0].qty - matched) / book[0].qty;
                } else {
                    book[0].commission = 0.0;
                }
                book[0].qty -= matched;
                remaining -= matched;
                if book[0].qty <= EPS { book.remove(0); }
            }
            if book.is_empty() { rt_open.insert(a_key.clone(), None); }
            continue;
        }

        fill.rt_before = if books.get(&a_key).is_empty() { None } else { rt_open.get(&a_key).cloned().flatten() };
        let a_clone = fill.a.clone();
        let mut remaining = close_against(&mut books, &mut rt_open, &a_key, &fill, &a_clone, fill.qty, None, &mut closed);
        if remaining > EPS && is_option_symbol(&sym) {
            let dir = if fill.side == "BUY" { "SHORT" } else { "LONG" };
            remaining = close_rolled(&mut books, &mut rt_open, &mut rolled, &rolled_keys, &fill, dir, remaining, &mut closed);
        }
        if remaining > EPS && fill.side == "SELL" {
            // A holding whose ticker was renamed still sells: find the old book.
            let day = field_s(&fill.a, "transactionDate");
            let acct = fifo_account(&fill.a);
            let cur = field_s(&fill.a, "currency");
            let candidates: Vec<String> = books.keys.clone();
            for dk in candidates {
                if dk == a_key || books.get(&dk).is_empty() { continue; }
                let bits: Vec<&str> = dk.split("::").collect();
                if bits.len() < 3 || bits[0] != acct || bits[2] != cur { continue; }
                if !ticker_was_replaced(&replaced, bits[0], bits[1], bits[2], &day) { continue; }
                remaining = close_against(&mut books, &mut rt_open, &dk, &fill, &a_clone, remaining, Some(&sym), &mut closed);
                if remaining <= EPS { break; }
            }
        }
        if remaining > EPS && fill.side == "SELL" && opening_direction(&fill.a, &fill.side).is_none() && dust(remaining, fill.qty, &fill.a) {
            remaining = 0.0;
        }
        if remaining > EPS {
            match opening_direction(&fill.a, &fill.side) {
                Some(opening) => {
                    if books.get(&a_key).is_empty() || rt_open.get(&a_key).cloned().flatten().is_none() {
                        rt_open.insert(a_key.clone(), Some(format!("rt:{}", field_s(&fill.a, "id"))));
                    }
                    let rt = rt_open.get(&a_key).cloned().flatten();
                    let commission = if fill.qty > 0.0 { field_num(&fill.a, "commission") * (remaining / fill.qty) } else { 0.0 };
                    let kind = { let k = field_s(&fill.a, "kind"); if k.is_empty() { kind_of(&fill.a) } else { k } };
                    let lot = lot_from(&fill.a, remaining, field_num(&fill.a, "unitPrice"), opening, commission, &kind, rt, flags_of(&fill.a));
                    books.at(&a_key).push(lot);
                }
                None => unmatched.push(Unmatched {
                    symbol: field_s(&fill.a, "symbol"),
                    currency: field_s(&fill.a, "currency"),
                    side: fill.side.clone(),
                    quantity: remaining,
                    price: field_num(&fill.a, "unitPrice"),
                    date: field_s(&fill.a, "transactionDate"),
                    description: field_s(&fill.a, "description"),
                    account_id: field_s(&fill.a, "accountId"),
                    account: fifo_account(&fill.a),
                    activity_id: field_s(&fill.a, "id"),
                }),
            }
        }
    }

    for (i, a) in rewritten { activities[i] = a; }

    let all_keys: Vec<String> = books.keys.clone();
    for key in &all_keys {
        apply_splits(&mut books, &mut pending_splits, key, "9999-12-31");
    }

    // The closing leg of each carried-forward roll was never posted; the credit
    // (or nothing, for a debit roll) is what it earned.
    for (_, dirs) in rolled.iter_mut() {
        for direction in ["LONG", "SHORT"] {
            for lot in dirs.side(direction).iter_mut() {
                if lot.qty <= EPS { continue; }
                let pseudo = serde_json::json!({
                    "id": format!("roll-out:{}", lot.activity_id),
                    "unitPrice": 0.0, "commission": 0.0,
                    "transactionDate": lot.date, "occurredAt": lot.when,
                    "name": lot.name, "securityId": "", "flags": ["rolled-out"],
                });
                let side = if direction == "SHORT" { "BUY" } else { "SELL" };
                if lot.rt.is_none() { lot.rt = Some(format!("rt:{}", lot.activity_id)); }
                let mut s = make_slice(lot, lot.qty, side, &pseudo, lot.qty, None);
                s.sell_activity_id = String::new();
                s.id = stable_trade_id(&s);
                closed.push(s);
            }
        }
    }

    let mut open_lots: Vec<Lot> = Vec::new();
    for key in &books.keys {
        for lot in books.map.get(key).unwrap() {
            if lot.qty <= 1e-6 { continue; }
            // crypto residue from in-kind fees: a lot worth under a dollar is not a position
            if lot.kind == "Crypto" && !lot.flags.iter().any(|f| f == "reward") && lot.qty * lot.price < 1.0 { continue; }
            open_lots.push(lot.clone());
        }
    }
    closed.sort_by(|x, y| (x.exit_date.clone(), x.id.clone()).cmp(&(y.exit_date.clone(), y.id.clone())));
    crate::fold::fold_option_rolls(&mut closed, &mut open_lots);
    Matched { closed, open: open_lots, unmatched }
}

fn pool_of<'a>(
    rolled: &'a mut Vec<((String, String, &'static str), RollPool)>,
    k: &(String, String, &'static str),
) -> &'a mut RollPool {
    if let Some(i) = rolled.iter().position(|(rk, _)| rk == k) { return &mut rolled[i].1; }
    rolled.push((k.clone(), RollPool::default()));
    let n = rolled.len() - 1;
    &mut rolled[n].1
}

/// `match_fifo.apply_splits`: every split on this book dated on or
/// before the day is applied to the open lots, once.
fn apply_splits(
    books: &mut Books,
    pending: &mut HashMap<String, Vec<(String, f64)>>,
    key: &str,
    day: &str,
) {
    let skey = key.split("::").take(2).collect::<Vec<_>>().join("::");
    let todo = match pending.get(&skey) { Some(v) if !v.is_empty() => v.clone(), _ => return };
    let mut sorted = todo;
    sorted.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    let mut keep = Vec::new();
    for (split_day, factor) in sorted {
        if split_day.as_str() <= day {
            let label = if factor < 1.0 {
                format!("split 1:{}", (1.0 / factor).round() as i64)
            } else {
                format!("split {}:1", factor.round() as i64)
            };
            for lot in books.at(key).iter_mut() {
                lot.qty *= factor;
                lot.price /= factor;
                if !lot.flags.iter().any(|f| *f == label) { lot.flags.push(label.clone()); }
            }
        } else {
            keep.push((split_day, factor));
        }
    }
    if keep.is_empty() { pending.remove(&skey); } else { pending.insert(skey, keep); }
}

/// `match_fifo.close_against`: take quantity off the front of a book.
#[allow(clippy::too_many_arguments)]
fn close_against(
    books: &mut Books,
    rt_open: &mut HashMap<String, Option<String>>,
    key: &str,
    fill: &Fill,
    a: &Value,
    mut remaining: f64,
    symbol_override: Option<&str>,
    closed: &mut Vec<Slice>,
) -> f64 {
    let closing_dir = if fill.side == "BUY" { "SHORT" } else { "LONG" };
    loop {
        let book = books.at(key);
        if !(remaining > EPS) || book.is_empty() || book[0].direction != closing_dir { break; }
        let matched = f64::min(book[0].qty, remaining);
        let lot = book[0].clone();
        closed.push(make_slice(&lot, fill.qty, &fill.side, a, matched, symbol_override));
        let book = books.at(key);
        book[0].commission *= if book[0].qty > 0.0 { (book[0].qty - matched) / book[0].qty } else { 0.0 };
        book[0].qty -= matched;
        remaining -= matched;
        if book[0].qty <= EPS { book.remove(0); }
    }
    if books.get(key).is_empty() { rt_open.insert(key.to_string(), None); }
    remaining
}

/// `match_fifo.close_rolled`: close the legs an earlier roll carried
/// forward; they take this contract's symbol. Once a chain has been rolled, a
/// buy-back beyond the known shorts also closes the chain's older contracts,
/// nearest expiry first -- those are the legs the rolls moved here without
/// ever being posted.
#[allow(clippy::too_many_arguments)]
fn close_rolled(
    books: &mut Books,
    rt_open: &mut HashMap<String, Option<String>>,
    rolled: &mut Vec<((String, String, &'static str), RollPool)>,
    rolled_keys: &HashSet<(String, String, &'static str)>,
    fill: &Fill,
    closing_dir: &str,
    mut remaining: f64,
    closed: &mut Vec<Slice>,
) -> f64 {
    let a = &fill.a;
    let rk = roll_key(a);
    let key = book_key(a);
    let sym = field_s(a, "symbol");

    // A rolled chain is one position from the first short to the last
    // buy-back: everything this fill closes shares one round trip, the
    // contract's own if it has lots, otherwise the chain's.
    let rt = {
        let pool = pool_of(rolled, &rk);
        fill.rt_before
            .clone()
            .or_else(|| pool.rt.clone())
            .or_else(|| {
                pool.side(closing_dir)
                    .first()
                    .map(|l| l.rt.clone().unwrap_or_else(|| format!("rt:{}", l.activity_id)))
            })
    };
    if rt.is_some() { pool_of(rolled, &rk).rt = rt.clone(); }

    loop {
        let pool = pool_of(rolled, &rk);
        let lots = pool.side(closing_dir);
        if !(remaining > EPS) || lots.is_empty() { break; }
        lots[0].symbol = sym.clone();
        let fallback = format!("rt:{}", lots[0].activity_id);
        lots[0].rt = rt.clone().or_else(|| lots[0].rt.clone()).or(Some(fallback));
        let matched = f64::min(lots[0].qty, remaining);
        let lot = lots[0].clone();
        closed.push(make_slice(&lot, fill.qty, &fill.side, a, matched, None));
        let pool = pool_of(rolled, &rk);
        let lots = pool.side(closing_dir);
        lots[0].qty -= matched;
        remaining -= matched;
        if lots[0].qty <= EPS { lots.remove(0); }
    }

    if remaining > EPS && rolled_keys.contains(&rk) {
        let acct = fifo_account(a);
        let cur = field_s(a, "currency");
        let under = underlying_symbol(&sym);
        let right = option_right(&sym);
        let mut others: Vec<(String, String)> = Vec::new();
        for k2 in books.keys.clone() {
            if k2 == key || books.get(&k2).is_empty() { continue; }
            let bits: Vec<&str> = k2.split("::").collect();
            if bits.len() < 3 || bits[0] != acct || bits[2] != cur { continue; }
            if !is_option_symbol(bits[1]) || underlying_symbol(bits[1]) != under || option_right(bits[1]) != right { continue; }
            others.push((option_expiry(bits[1]), k2.clone()));
        }
        others.sort();
        for (_, k2) in others {
            loop {
                let b2 = books.at(&k2);
                if !(remaining > EPS) || b2.is_empty() || b2[0].direction != closing_dir { break; }
                let matched = f64::min(b2[0].qty, remaining);
                let lot = b2[0].clone();
                let mut s = make_slice(&lot, fill.qty, &fill.side, a, matched, Some(&sym));
                if !s.flags.iter().any(|f| f == "rolled-in") { s.flags.push("rolled-in".into()); }
                s.flags.sort();
                s.flags.dedup();
                if rt.is_some() { s.rt = rt.clone(); }
                s.id = stable_trade_id(&s);
                closed.push(s);
                let b2 = books.at(&k2);
                b2[0].qty -= matched;
                remaining -= matched;
                if b2[0].qty <= EPS { b2.remove(0); }
            }
            if books.get(&k2).is_empty() { rt_open.insert(k2.clone(), None); }
        }
    }

    let pool = pool_of(rolled, &rk);
    if pool.is_empty() && books.get(&key).is_empty() { pool_of(rolled, &rk).rt = None; }
    remaining
}
