//! FIFO matching per (account, symbol, currency): `match_fifo`.
//!
//! Returns the closed slices, the lots still open, and the fills that could
//! not be matched to anything. Option rolls are the hard part: Wealthsimple
//! posts a roll as one multileg row that closes a contract and never posts the
//! leg it opened, so the quantity is carried forward in `rolled` and closed
//! against later buy-backs.

use std::collections::{HashMap, HashSet};

use crate::activity::{mark, Activity, Direction, Flag, Kind, RawActivity, Side};
use crate::dates::{days_between, option_expiry};
use crate::normalize::{book_key, fifo_account, fold_stkdis, is_close_only, is_multileg, normalize_all, opening_direction, roll_key, BookKey, RollKey};
use crate::symbols::{is_option_symbol, option_multiplier, option_right, underlying_symbol};
use crate::value::{fmt8, EPS};

// --------------------------------------------------------------------------
// the rows the matcher works with
// --------------------------------------------------------------------------

/// Quantity still open from one opening fill.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lot {
    pub qty: f64,
    pub price: f64,
    pub date: String,
    pub when: String,
    pub commission: f64,
    pub direction: Direction,
    pub account_id: String,
    pub account_type: String,
    pub symbol: String,
    pub name: String,
    pub currency: String,
    pub kind: Kind,
    pub activity_id: String,
    pub security_id: String,
    pub rt: Option<String>,
    pub flags: Vec<Flag>,
}

/// One closed piece of a lot against one fill.
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
    pub kind: Kind,
    /// The closing fill's side.
    pub side: Side,
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
    pub open_direction: Direction,
    pub buy_activity_id: String,
    pub sell_activity_id: String,
    pub security_id: String,
    pub flags: Vec<Flag>,
    /// Filled in by `apply_fx`; absent until then.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fees_cad: Option<f64>,
}

/// A fill that closed more than the book held and cannot open anything.
#[derive(Clone, Debug, serde::Serialize, ts_rs::TS, bagholder_diff_derive::Diff)]
#[diff(key = symbol)]
#[serde(rename_all = "camelCase")]
pub struct Unmatched {
    pub symbol: String,
    pub currency: String,
    pub side: Side,
    pub quantity: f64,
    pub price: f64,
    pub date: String,
    pub description: String,
    pub account_id: String,
    pub account: String,
    pub activity_id: String,
}

#[derive(Clone, Debug)]
struct Fill {
    a: Activity,
    side: Side,
    qty: f64,
    roll_direction: Option<Direction>,
    rt_before: Option<String>,
    /// Where this row is in the caller's list, when it is one of them. The
    /// inference rewrites a multileg row's quantity, price and sub-type, and the
    /// fill stands for the very row the activity list holds, so the rewrite has
    /// to land back there too: those rows are what a trade's fills print.
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

/// Stable from the first fill, so a journal entry survives later exits.
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
        t.side.as_str().to_string(),
    ]
    .join("|")
}

/// Which way a row goes, read from the sub-type, then the type, then the sign
/// of the quantity.
pub fn trade_side(a: &Activity) -> Option<Side> {
    side_of(&a.activity_sub_type, &a.activity_type, a.quantity)
}

/// The same reading for a row that is not a working row yet (the store, keying
/// an incoming row).
pub fn side_of(sub_type: &str, activity_type: &str, quantity: f64) -> Option<Side> {
    let sub = crate::value::compact(sub_type);
    if sub.starts_with("BUY") || sub == "BTO" || sub == "BTC" {
        return Some(Side::Buy);
    }
    if sub.starts_with("SELL") || sub == "STO" || sub == "STC" {
        return Some(Side::Sell);
    }
    let kind = crate::value::compact(activity_type);
    if kind.starts_with("BUY") {
        return Some(Side::Buy);
    }
    if kind.starts_with("SELL") {
        return Some(Side::Sell);
    }
    if quantity > 0.0 {
        Some(Side::Buy)
    } else if quantity < 0.0 {
        Some(Side::Sell)
    } else {
        None
    }
}

// --------------------------------------------------------------------------
// fill ordering
// --------------------------------------------------------------------------

/// Within a day, opens come before closes so a same-day round trip matches
/// against its own entry rather than an older lot.
fn fill_rank(f: &Fill) -> u8 {
    let (t, s) = (f.a.type_c(), f.a.sub_type_c());
    let blob = format!("{}{}", t, s);
    if (blob.contains("TOOPEN") || t == "STO" || s == "STO") && f.side == Side::Sell {
        return 0;
    }
    if is_close_only(&f.a) && f.side == Side::Buy {
        return 1;
    }
    if f.side == Side::Buy {
        return 2;
    }
    if blob.contains("TOCLOSE") || t == "STC" || s == "STC" {
        return 3;
    }
    4
}

fn fill_sort_key(f: &Fill) -> (String, u8, String, String) {
    (f.a.transaction_date.clone(), fill_rank(f), f.a.occurred_at.clone(), f.a.id.clone())
}

// --------------------------------------------------------------------------
// slices
// --------------------------------------------------------------------------

/// What a slice takes from the row that closed it.
struct Exit<'a> {
    id: &'a str,
    unit_price: f64,
    commission: f64,
    date: &'a str,
    when: &'a str,
    name: &'a str,
    security_id: &'a str,
    flags: &'a [Flag],
}

impl<'a> From<&'a Activity> for Exit<'a> {
    fn from(a: &'a Activity) -> Exit<'a> {
        Exit { id: &a.id, unit_price: a.unit_price, commission: a.commission, date: &a.transaction_date, when: &a.occurred_at, name: &a.name, security_id: &a.security_id, flags: &a.flags }
    }
}

/// One closed piece of a lot against one fill.
fn make_slice(lot: &Lot, fill_qty: f64, side: Side, exit: &Exit, matched: f64, symbol: Option<&str>) -> Slice {
    let exit_commission = if fill_qty > 0.0 { exit.commission * (matched / fill_qty) } else { 0.0 };
    let entry_commission = if lot.qty > 0.0 { lot.commission * (matched / lot.qty) } else { 0.0 };
    let commission = entry_commission + exit_commission;
    let sym = symbol.unwrap_or(&lot.symbol).to_string();
    let mult = option_multiplier(&sym);
    let raw_pnl = match lot.direction {
        Direction::Long => (exit.unit_price - lot.price) * matched * mult,
        Direction::Short => (lot.price - exit.unit_price) * matched * mult,
    };
    // A carried symbol takes the fill's name; the lot's own name otherwise.
    let name = if symbol.is_some() && !exit.name.is_empty() { exit.name.to_string() } else { lot.name.clone() };
    let security_id = if !lot.security_id.is_empty() { lot.security_id.clone() } else { exit.security_id.to_string() };
    let mut flags: Vec<Flag> = lot.flags.iter().chain(exit.flags).cloned().collect();
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
        kind: lot.kind,
        side,
        quantity: matched,
        entry_price: lot.price,
        exit_price: exit.unit_price,
        entry_date: lot.date.clone(),
        exit_date: exit.date.to_string(),
        entry_when: lot.when.clone(),
        exit_when: exit.when.to_string(),
        hold_days: days_between(&lot.date, exit.date),
        commission,
        entry_commission,
        exit_commission,
        pnl: raw_pnl - commission,
        pnl_cad: raw_pnl - commission,
        open_direction: lot.direction,
        buy_activity_id: lot.activity_id.clone(),
        sell_activity_id: exit.id.to_string(),
        security_id,
        flags,
        fees_cad: None,
    };
    t.id = stable_trade_id(&t);
    t
}

/// A sale that exceeds the lots by a residue is rounding, not a short. Crypto
/// quantities come back net of in-kind fees, so 1% of the fill is tolerated
/// there; elsewhere only float noise or under a cent of value.
fn dust(remaining: f64, fill_qty: f64, a: &Activity) -> bool {
    if remaining <= 1e-6 * f64::max(1.0, fill_qty) {
        return true;
    }
    if a.kind == Kind::Crypto && remaining <= 0.01 * fill_qty {
        return true;
    }
    let px = a.unit_price.abs();
    px > 0.0 && remaining * px * option_multiplier(&a.symbol) < 0.01
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

/// Quantity by direction.
#[derive(Default, Clone, Copy)]
struct Dirs {
    long: f64,
    short: f64,
}

impl Dirs {
    fn get(&self, d: Direction) -> f64 {
        match d {
            Direction::Long => self.long,
            Direction::Short => self.short,
        }
    }
    fn add(&mut self, d: Direction, v: f64) {
        match d {
            Direction::Long => self.long += v,
            Direction::Short => self.short += v,
        }
    }
    fn sub(&mut self, d: Direction, v: f64) {
        self.add(d, -v)
    }
}

fn set_fill_side(f: &mut Fill, side: Side, sub: &str) {
    f.side = side;
    f.a.activity_sub_type = sub.into();
    let q = f.a.quantity.abs();
    if q > 0.0 {
        f.a.quantity = if side == Side::Sell { -q } else { q };
    }
}

/// An expiry or assignment row does not say which way it goes, so it is read
/// from what the book still holds.
fn resolve_option_fill_side(f: &mut Fill, rem: &Dirs) {
    let raw = format!("{}{}", f.a.raw_type_c(), f.a.type_c());
    if raw.contains("EXPIR") || raw.contains("ASSIGN") || raw.contains("EXERCISE") {
        if raw.contains("ASSIGN") || raw.contains("SHORTEXPIR") {
            let sub = if raw.contains("ASSIGN") { "BUYTOCLOSE" } else { "BUY" };
            set_fill_side(f, Side::Buy, sub);
        } else if raw.contains("EXPIR") && !raw.contains("SHORT") {
            set_fill_side(f, Side::Sell, "SELL");
        } else if f.side == Side::Buy && rem.long > EPS && rem.short <= EPS {
            set_fill_side(f, Side::Sell, "SELL");
        } else if f.side == Side::Sell && rem.short > EPS && rem.long <= EPS {
            set_fill_side(f, Side::Buy, "BUY");
        }
        return;
    }
    if !(raw.contains("MULTILEG") || is_close_only(&f.a)) {
        return;
    }
    f.a.activity_sub_type = match f.side {
        Side::Buy => if rem.short > EPS { "BUYTOCLOSE" } else { "BUYTOOPEN" },
        Side::Sell => if rem.long > EPS { "SELLTOCLOSE" } else { "SELLTOOPEN" },
    }
    .into();
}

/// The contract count behind a quantity-zero row: the smallest count that gives
/// a clean price and fits what is open, or what is open when nothing later
/// needs a share of it.
fn inferred_qty(cash: f64, open_sz: f64, whole_if_alone: f64, upcoming: usize, floor: f64) -> f64 {
    if whole_if_alone > floor && upcoming == 0 {
        return whole_if_alone;
    }
    if open_sz > floor {
        let cap = i64::max(1, (open_sz + 1e-9) as i64);
        let picked = (1..=cap).map(|q| q as f64).find(|q| is_clean_option_qty(cash, *q));
        return f64::min(picked.unwrap_or_else(|| infer_standalone_option_qty(cash)), open_sz);
    }
    infer_standalone_option_qty(cash)
}

/// Fills in the quantity a multileg row left at zero, and decides which
/// direction a roll is closing, by walking the fills in order and tracking what
/// each book and each roll chain still holds.
fn infer_zero_qty_option_fills(fills: &mut [Fill]) {
    let mut remaining: HashMap<BookKey, Dirs> = HashMap::new();
    let mut pools: HashMap<RollKey, Dirs> = HashMap::new();
    let mut zeros_by_book: HashMap<BookKey, Vec<usize>> = HashMap::new();

    for (i, f) in fills.iter().enumerate() {
        if is_option_symbol(&f.a.symbol) && f.a.quantity.abs() == 0.0 && f.a.net_cash_amount.abs() > 1e-9 {
            zeros_by_book.entry(book_key(&f.a)).or_default().push(i);
        }
    }

    for i in 0..fills.len() {
        let key = book_key(&fills[i].a);
        let rkey = roll_key(&fills[i].a);
        let is_opt = is_option_symbol(&fills[i].a.symbol);
        let rem_now = *remaining.entry(key.clone()).or_default();
        let upcoming = zeros_by_book.get(&key).map_or(0, |v| v.iter().filter(|j| **j > i).count());

        if is_opt && is_multileg(&fills[i].a) {
            // A roll: this row closes what the contract holds (or what an
            // earlier roll carried forward), and the same quantity moves to
            // the next contract, which the broker never posts.
            let pool = *pools.entry(rkey.clone()).or_default();
            let direction = if rem_now.short > EPS {
                Direction::Short
            } else if rem_now.long > EPS {
                Direction::Long
            } else if pool.short >= pool.long {
                Direction::Short
            } else {
                Direction::Long
            };
            let open_sz = rem_now.get(direction) + pool.get(direction);
            let mut qty = fills[i].a.quantity.abs();
            let cash = fills[i].a.net_cash_amount;
            if qty == 0.0 {
                qty = inferred_qty(cash, open_sz, rem_now.get(direction), upcoming, EPS);
                fills[i].a.unit_price = if qty > 0.0 { cash.abs() / (qty * 100.0) } else { 0.0 };
            }
            fills[i].side = direction.closed_by();
            fills[i].a.activity_sub_type = if direction == Direction::Short { "BUYTOCLOSE" } else { "SELLTOCLOSE" }.into();
            fills[i].a.quantity = if direction == Direction::Short { qty } else { -qty };
            fills[i].qty = qty;
            fills[i].roll_direction = Some(direction);

            let closed = f64::min(qty, rem_now.get(direction));
            remaining.entry(key.clone()).or_default().sub(direction, closed);
            let p = pools.entry(rkey.clone()).or_default();
            p.sub(direction, f64::min(qty - closed, p.get(direction)));
            if closed > EPS || qty > EPS {
                p.add(direction, qty);
            }
            continue;
        }

        if is_opt {
            resolve_option_fill_side(&mut fills[i], &rem_now);
        }
        let closing = fills[i].side.closes();
        if is_opt && fills[i].a.quantity.abs() == 0.0 {
            let cash = fills[i].a.net_cash_amount;
            let open_sz = rem_now.get(closing);
            let mut qty = 0.0;
            if cash.abs() > 1e-9 {
                qty = inferred_qty(cash, open_sz, open_sz, upcoming, 0.0);
                fills[i].a.unit_price = if qty > 0.0 { cash.abs() / (qty * 100.0) } else { 0.0 };
            } else if open_sz > 0.0 && is_close_only(&fills[i].a) {
                qty = open_sz;
                fills[i].a.unit_price = 0.0;
            }
            if qty > 0.0 {
                fills[i].a.quantity = if fills[i].side == Side::Sell { -qty } else { qty };
                fills[i].qty = qty;
            }
        }
        if is_opt && (fills[i].a.raw_type_c().contains("ASSIGN") || fills[i].a.type_c().contains("ASSIGN")) {
            fills[i].a.unit_price = 0.0;
        }
        if fills[i].qty > 0.0 {
            let closing = fills[i].side.closes();
            let opening = opening_direction(&fills[i].a, fills[i].side);
            let mut left = fills[i].qty;
            let r = remaining.entry(key.clone()).or_default();
            let close_amt = f64::min(left, r.get(closing));
            r.sub(closing, close_amt);
            left -= close_amt;
            if left > EPS && is_opt {
                let p = pools.entry(rkey.clone()).or_default();
                let pooled = f64::min(left, p.get(closing));
                p.sub(closing, pooled);
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

/// Per (account, symbol, currency), the first date the ticker was removed and
/// the dates of real trades, so the sale of a renamed holding can find the old
/// book.
struct Replacement {
    removed: HashMap<BookKey, String>,
    trades: HashMap<BookKey, Vec<String>>,
}

fn removal_marker(raw: &str) -> bool {
    ["CODECHANGE", "SYMBOLCHANGE", "TICKERCHANGE", "LISTINGSTATUS", "SECURITYSWAP"].iter().any(|m| raw.contains(m))
}

fn replacement_index(activities: &[Activity]) -> Replacement {
    let mut removed: HashMap<BookKey, String> = HashMap::new();
    let mut trades: HashMap<BookKey, Vec<String>> = HashMap::new();
    let mut removed_on = |key: &BookKey, day: &str| {
        if !day.is_empty() && removed.get(key).map_or(true, |known| day < known.as_str()) {
            removed.insert(key.clone(), day.to_string());
        }
    };
    for a in activities {
        let key = book_key(a);
        let day = &a.transaction_date;
        if a.type_c() == "STKDIS" {
            if a.sub_type_c() == "SELL" || a.quantity < 0.0 {
                removed_on(&key, day);
            }
            continue;
        }
        if removal_marker(&format!("{}{}", a.raw_type_c(), crate::value::compact(&a.aft_type))) {
            removed_on(&key, day);
        }
        if a.category.is_fill() && trade_side(a).is_some() {
            trades.entry(key).or_default().push(day.clone());
        }
    }
    Replacement { removed, trades }
}

/// The ticker went away before this date and nothing has traded in it since.
fn ticker_was_replaced(ix: &Replacement, book: &BookKey, by_date: &str) -> bool {
    let removed_on = match ix.removed.get(book) {
        Some(d) if !d.is_empty() => d,
        _ => return false,
    };
    if removed_on.as_str() > by_date {
        return false;
    }
    !ix.trades.get(book).map_or(false, |days| days.iter().any(|d| d > removed_on))
}

/// Wealthsimple posts a share split as a quantity-zero corporate action with no
/// ratio, so the ratio is inferred from the median fill price on either side of
/// it. Lot quantities are multiplied by the factor and prices divided by it.
/// Keyed by (account, symbol, day).
fn split_markers(activities: &[Activity]) -> HashMap<(String, String, String), f64> {
    let mut out = HashMap::new();
    let mut by_book: HashMap<(String, String), Vec<&Activity>> = HashMap::new();
    for a in activities {
        if a.category.is_fill() && !a.symbol.is_empty() {
            by_book.entry((fifo_account(a), a.symbol.clone())).or_default().push(a);
        }
    }
    for a in activities {
        if a.type_c() != "STKDIS" || a.raw_type_c() != "CORPORATEACTION" || a.quantity.abs() > EPS {
            continue;
        }
        let day = &a.transaction_date;
        // the marker's currency does not always match the fills'; key on account+symbol
        let key = (fifo_account(a), a.symbol.clone());
        let mut priced: Vec<&Activity> = by_book.get(&key).map(|v| v.iter().copied().filter(|x| x.unit_price > 0.0 && x.type_c() != "STKDIS").collect()).unwrap_or_default();
        priced.sort_by(|x, y| (&x.transaction_date, &x.occurred_at).cmp(&(&y.transaction_date, &y.occurred_at)));
        let mut before: Vec<f64> = priced.iter().filter(|x| x.transaction_date < *day).map(|x| x.unit_price).collect();
        let mut after: Vec<f64> = priced.iter().filter(|x| x.transaction_date >= *day).map(|x| x.unit_price).collect();
        if before.len() > 3 {
            before = before.split_off(before.len() - 3);
        }
        after.truncate(3);
        if before.is_empty() || after.is_empty() {
            continue;
        }
        before.sort_by(|p, q| p.partial_cmp(q).unwrap());
        after.sort_by(|p, q| p.partial_cmp(q).unwrap());
        let pre = before[before.len() / 2];
        let post = after[after.len() / 2];
        if !(pre > 0.0) || !(post > 0.0) {
            continue;
        }
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
        if n < 2.0 || (ratio - (1.0 / factor)).abs() / (1.0 / factor) > 0.35 {
            continue;
        }
        out.insert((key.0, key.1, day.clone()), factor);
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
    keys: Vec<BookKey>,
    map: HashMap<BookKey, Vec<Lot>>,
    /// The round trip each book's open lots belong to.
    rt_open: HashMap<BookKey, Option<String>>,
}

impl Books {
    fn ensure(&mut self, key: &BookKey) {
        if !self.map.contains_key(key) {
            self.keys.push(key.clone());
            self.map.insert(key.clone(), Vec::new());
        }
    }
    fn get(&self, key: &BookKey) -> &[Lot] {
        self.map.get(key).map(|v| v.as_slice()).unwrap_or(&[])
    }
    fn at(&mut self, key: &BookKey) -> &mut Vec<Lot> {
        self.ensure(key);
        self.map.get_mut(key).unwrap()
    }
    fn rt(&self, key: &BookKey) -> Option<String> {
        self.rt_open.get(key).cloned().flatten()
    }
    /// The round trip a lot opened on this book now joins: the one already
    /// open, or a new one named for the fill that starts it.
    fn rt_for_opening(&mut self, key: &BookKey, fill_id: &str) -> Option<String> {
        if self.get(key).is_empty() || self.rt(key).is_none() {
            self.rt_open.insert(key.clone(), Some(format!("rt:{}", fill_id)));
        }
        self.rt(key)
    }
    fn closed_out(&mut self, key: &BookKey) {
        if self.get(key).is_empty() {
            self.rt_open.insert(key.clone(), None);
        }
    }
}

#[derive(Default)]
struct RollPool {
    long: Vec<Lot>,
    short: Vec<Lot>,
    rt: Option<String>,
}

impl RollPool {
    fn side(&mut self, d: Direction) -> &mut Vec<Lot> {
        match d {
            Direction::Long => &mut self.long,
            Direction::Short => &mut self.short,
        }
    }
    fn is_empty(&self) -> bool {
        self.long.is_empty() && self.short.is_empty()
    }
}

type Rolled = Vec<(RollKey, RollPool)>;

fn pool_of<'a>(rolled: &'a mut Rolled, k: &RollKey) -> &'a mut RollPool {
    if let Some(i) = rolled.iter().position(|(rk, _)| rk == k) {
        return &mut rolled[i].1;
    }
    rolled.push((k.clone(), RollPool::default()));
    let n = rolled.len() - 1;
    &mut rolled[n].1
}

#[allow(clippy::too_many_arguments)]
fn lot_from(a: &Activity, qty: f64, price: f64, direction: Direction, commission: f64, kind: Kind, rt: Option<String>, flags: Vec<Flag>) -> Lot {
    Lot {
        qty,
        price,
        date: a.transaction_date.clone(),
        when: a.occurred_at.clone(),
        commission,
        direction,
        account_id: a.account_id.clone(),
        account_type: fifo_account(a),
        symbol: a.symbol.clone(),
        name: a.name.clone(),
        currency: a.currency.clone(),
        kind,
        activity_id: a.id.clone(),
        security_id: a.security_id.clone(),
        rt,
        flags,
    }
}

/// The match over stored rows, each normalized first.
pub fn match_fifo(rows: &[RawActivity]) -> Matched {
    match_fifo_in_place(&mut normalize_all(rows))
}

/// The match over working rows, writing the inference back into them: what it
/// decides about a row -- the contract count behind a quantity-zero multileg,
/// the price that implies, which way it closes -- is what that row says
/// afterwards.
pub fn match_fifo_in_place(activities: &mut Vec<Activity>) -> Matched {
    let live: Vec<(Option<usize>, Activity)> =
        activities.iter().enumerate().filter(|(_, a)| !a.has(&Flag::PendingDistribution)).map(|(i, a)| (Some(i), a.clone())).collect();
    let normalized: Vec<Activity> = live.iter().map(|(_, a)| a.clone()).collect();

    let mut fills: Vec<Fill> = Vec::new();
    for (src, a) in fold_stkdis(live) {
        if !a.category.is_fill() || a.symbol.is_empty() {
            continue;
        }
        let Some(side) = trade_side(&a) else { continue };
        fills.push(Fill { qty: a.quantity.abs(), a, side, roll_direction: None, rt_before: None, src });
    }
    fills.sort_by(|x, y| fill_sort_key(x).cmp(&fill_sort_key(y)));
    infer_zero_qty_option_fills(&mut fills);
    // What the inference decided stands even for a fill it left at zero.
    for f in &fills {
        if let Some(i) = f.src {
            activities[i] = f.a.clone();
        }
    }
    let usable: Vec<Fill> = fills.into_iter().filter(|f| f.qty > 0.0).collect();

    let mut books = Books::default();
    let mut closed: Vec<Slice> = Vec::new();
    let mut unmatched: Vec<Unmatched> = Vec::new();
    let mut rolled: Rolled = Vec::new();
    let mut rolled_keys: HashSet<RollKey> = HashSet::new();

    let replaced = replacement_index(&normalized);
    let mut pending_splits: HashMap<(String, String), Vec<(String, f64)>> = HashMap::new();
    for ((account, symbol, day), factor) in split_markers(&normalized) {
        pending_splits.entry((account, symbol)).or_default().push((day, factor));
    }

    for mut fill in usable {
        let key = book_key(&fill.a);
        books.ensure(&key);
        apply_splits(&mut books, &mut pending_splits, &key, &fill.a.transaction_date);
        let sym = fill.a.symbol.clone();

        if let (true, Some(direction)) = (is_option_symbol(&sym) && is_multileg(&fill.a), fill.roll_direction) {
            // Roll: close this contract (its book, then the legs an earlier
            // roll carried forward) and carry the same quantity to the leg the
            // broker never posted. A debit belongs to the closed leg's exit, a
            // credit to the new leg's entry.
            let cash = fill.a.net_cash_amount;
            let per = if fill.qty > 0.0 { cash.abs() / (fill.qty * 100.0) } else { 0.0 };
            let debit = cash < 0.0;
            let short = direction == Direction::Short;
            let exit_px = if short == debit { per } else { 0.0 };
            let entry_px = if short != debit { per } else { 0.0 };
            fill.a.unit_price = exit_px;
            let before = closed.len();
            fill.rt_before = if books.get(&key).is_empty() { None } else { books.rt(&key) };
            let mut remaining = close_against(&mut books, &key, &fill, fill.qty, None, &mut closed);
            remaining = close_rolled(&mut books, &mut rolled, &rolled_keys, &fill, direction, remaining, &mut closed);
            let moved = fill.qty - remaining;
            let rk = roll_key(&fill.a);
            rolled_keys.insert(rk.clone());
            if moved > EPS {
                for s in closed[before..].iter_mut() {
                    mark(&mut s.flags, Flag::Rolled);
                }
                let chain_rt = fill.rt_before.clone().or_else(|| pool_of(&mut rolled, &rk).rt.clone()).or_else(|| closed.get(before).and_then(|s| s.rt.clone()));
                let pool = pool_of(&mut rolled, &rk);
                pool.rt = chain_rt.clone();
                let mut lot = lot_from(&fill.a, moved, entry_px, direction, 0.0, Kind::Options, chain_rt, vec![Flag::RolledIn]);
                lot.security_id = String::new();
                pool.side(direction).push(lot);
            }
            if remaining > EPS {
                // nothing to roll: this multileg simply opened a position
                let opening = if debit { Direction::Long } else { Direction::Short };
                fill.a.unit_price = per;
                fill.side = opening.opened_by();
                let rt = books.rt_for_opening(&key, &fill.a.id);
                let lot = lot_from(&fill.a, remaining, per, opening, 0.0, Kind::Options, rt, fill.a.flags.clone());
                books.at(&key).push(lot);
            }
            if let Some(i) = fill.src {
                activities[i] = fill.a.clone();
            }
            continue;
        }

        if fill.a.has(&Flag::TransferOut) {
            // Coins sent out of the account leave at cost: off the open lots
            // first in first out, no slice, no P&L, not a fill of the trade.
            let mut remaining = fill.qty;
            let book = books.at(&key);
            while remaining > EPS && !book.is_empty() && book[0].direction == Direction::Long {
                let matched = f64::min(book[0].qty, remaining);
                book[0].commission *= if book[0].qty > 0.0 { (book[0].qty - matched) / book[0].qty } else { 0.0 };
                book[0].qty -= matched;
                remaining -= matched;
                if book[0].qty <= EPS {
                    book.remove(0);
                }
            }
            books.closed_out(&key);
            continue;
        }

        fill.rt_before = if books.get(&key).is_empty() { None } else { books.rt(&key) };
        let mut remaining = close_against(&mut books, &key, &fill, fill.qty, None, &mut closed);
        if remaining > EPS && is_option_symbol(&sym) {
            remaining = close_rolled(&mut books, &mut rolled, &rolled_keys, &fill, fill.side.closes(), remaining, &mut closed);
        }
        if remaining > EPS && fill.side == Side::Sell {
            // A holding whose ticker was renamed still sells: find the old book.
            for old in books.keys.clone() {
                if old == key || books.get(&old).is_empty() || old.account != key.account || old.currency != key.currency {
                    continue;
                }
                if !ticker_was_replaced(&replaced, &old, &fill.a.transaction_date) {
                    continue;
                }
                remaining = close_against(&mut books, &old, &fill, remaining, Some(&sym), &mut closed);
                if remaining <= EPS {
                    break;
                }
            }
        }
        let opening = opening_direction(&fill.a, fill.side);
        if remaining > EPS && fill.side == Side::Sell && opening.is_none() && dust(remaining, fill.qty, &fill.a) {
            remaining = 0.0;
        }
        if remaining > EPS {
            match opening {
                Some(opening) => {
                    let rt = books.rt_for_opening(&key, &fill.a.id);
                    let commission = if fill.qty > 0.0 { fill.a.commission * (remaining / fill.qty) } else { 0.0 };
                    let lot = lot_from(&fill.a, remaining, fill.a.unit_price, opening, commission, fill.a.kind, rt, fill.a.flags.clone());
                    books.at(&key).push(lot);
                }
                None => unmatched.push(Unmatched {
                    symbol: fill.a.symbol.clone(),
                    currency: fill.a.currency.clone(),
                    side: fill.side,
                    quantity: remaining,
                    price: fill.a.unit_price,
                    date: fill.a.transaction_date.clone(),
                    description: fill.a.description.clone(),
                    account_id: fill.a.account_id.clone(),
                    account: fifo_account(&fill.a),
                    activity_id: fill.a.id.clone(),
                }),
            }
        }
    }

    for key in books.keys.clone() {
        apply_splits(&mut books, &mut pending_splits, &key, "9999-12-31");
    }

    // The closing leg of each carried-forward roll was never posted; the credit
    // (or nothing, for a debit roll) is what it earned.
    for (_, pool) in rolled.iter_mut() {
        for direction in Direction::BOTH {
            for lot in pool.side(direction).iter_mut() {
                if lot.qty <= EPS {
                    continue;
                }
                if lot.rt.is_none() {
                    lot.rt = Some(format!("rt:{}", lot.activity_id));
                }
                let id = format!("roll-out:{}", lot.activity_id);
                let never_posted = Exit { id: &id, unit_price: 0.0, commission: 0.0, date: &lot.date, when: &lot.when, name: &lot.name, security_id: "", flags: &[Flag::RolledOut] };
                let mut s = make_slice(lot, lot.qty, direction.closed_by(), &never_posted, lot.qty, None);
                s.sell_activity_id = String::new();
                s.id = stable_trade_id(&s);
                closed.push(s);
            }
        }
    }

    let mut open_lots: Vec<Lot> = Vec::new();
    for key in &books.keys {
        for lot in books.get(key) {
            // crypto residue from in-kind fees: a lot worth under a dollar is not a position
            if lot.qty <= 1e-6 || (lot.kind == Kind::Crypto && lot.qty * lot.price < 1.0) {
                continue;
            }
            open_lots.push(lot.clone());
        }
    }
    closed.sort_by(|x, y| (&x.exit_date, &x.id).cmp(&(&y.exit_date, &y.id)));
    crate::fold::fold_option_rolls(&mut closed, &mut open_lots);
    Matched { closed, open: open_lots, unmatched }
}

/// Every split on this book dated on or before the day is applied to the open
/// lots, once.
fn apply_splits(books: &mut Books, pending: &mut HashMap<(String, String), Vec<(String, f64)>>, key: &BookKey, day: &str) {
    let skey = (key.account.clone(), key.symbol.clone());
    let mut todo = match pending.get(&skey) {
        Some(v) if !v.is_empty() => v.clone(),
        _ => return,
    };
    todo.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    let mut keep = Vec::new();
    for (split_day, factor) in todo {
        if split_day.as_str() > day {
            keep.push((split_day, factor));
            continue;
        }
        let label = if factor < 1.0 { Flag::ReverseSplit((1.0 / factor).round() as i64) } else { Flag::Split(factor.round() as i64) };
        for lot in books.at(key).iter_mut() {
            lot.qty *= factor;
            lot.price /= factor;
            mark(&mut lot.flags, label.clone());
        }
    }
    if keep.is_empty() {
        pending.remove(&skey);
    } else {
        pending.insert(skey, keep);
    }
}

/// Take quantity off the front of a book.
fn close_against(books: &mut Books, key: &BookKey, fill: &Fill, mut remaining: f64, symbol_override: Option<&str>, closed: &mut Vec<Slice>) -> f64 {
    let closing = fill.side.closes();
    loop {
        let book = books.at(key);
        if !(remaining > EPS) || book.is_empty() || book[0].direction != closing {
            break;
        }
        let matched = f64::min(book[0].qty, remaining);
        closed.push(make_slice(&book[0], fill.qty, fill.side, &Exit::from(&fill.a), matched, symbol_override));
        book[0].commission *= if book[0].qty > 0.0 { (book[0].qty - matched) / book[0].qty } else { 0.0 };
        book[0].qty -= matched;
        remaining -= matched;
        if book[0].qty <= EPS {
            book.remove(0);
        }
    }
    books.closed_out(key);
    remaining
}

/// Close the legs an earlier roll carried forward; they take this contract's
/// symbol. Once a chain has been rolled, a buy-back beyond the known shorts also
/// closes the chain's older contracts, nearest expiry first -- those are the
/// legs the rolls moved here without ever being posted.
fn close_rolled(books: &mut Books, rolled: &mut Rolled, rolled_keys: &HashSet<RollKey>, fill: &Fill, closing: Direction, mut remaining: f64, closed: &mut Vec<Slice>) -> f64 {
    let a = &fill.a;
    let rk = roll_key(a);
    let key = book_key(a);

    // A rolled chain is one position from the first short to the last
    // buy-back: everything this fill closes shares one round trip, the
    // contract's own if it has lots, otherwise the chain's.
    let rt = {
        let pool = pool_of(rolled, &rk);
        fill.rt_before.clone().or_else(|| pool.rt.clone()).or_else(|| pool.side(closing).first().map(|l| l.rt.clone().unwrap_or_else(|| format!("rt:{}", l.activity_id))))
    };
    if rt.is_some() {
        pool_of(rolled, &rk).rt = rt.clone();
    }

    loop {
        let lots = pool_of(rolled, &rk).side(closing);
        if !(remaining > EPS) || lots.is_empty() {
            break;
        }
        lots[0].symbol = a.symbol.clone();
        let fallback = format!("rt:{}", lots[0].activity_id);
        lots[0].rt = rt.clone().or_else(|| lots[0].rt.clone()).or(Some(fallback));
        let matched = f64::min(lots[0].qty, remaining);
        closed.push(make_slice(&lots[0], fill.qty, fill.side, &Exit::from(a), matched, None));
        lots[0].qty -= matched;
        remaining -= matched;
        if lots[0].qty <= EPS {
            lots.remove(0);
        }
    }

    if remaining > EPS && rolled_keys.contains(&rk) {
        let under = underlying_symbol(&a.symbol);
        let right = option_right(&a.symbol);
        let mut others: Vec<(String, BookKey)> = books
            .keys
            .iter()
            .filter(|k| **k != key && !books.get(k).is_empty() && k.account == key.account && k.currency == key.currency)
            .filter(|k| is_option_symbol(&k.symbol) && underlying_symbol(&k.symbol) == under && option_right(&k.symbol) == right)
            .map(|k| (option_expiry(&k.symbol), k.clone()))
            .collect();
        others.sort_by(|x, y| (&x.0, &x.1.account, &x.1.symbol, &x.1.currency).cmp(&(&y.0, &y.1.account, &y.1.symbol, &y.1.currency)));
        for (_, other) in others {
            loop {
                let book = books.at(&other);
                if !(remaining > EPS) || book.is_empty() || book[0].direction != closing {
                    break;
                }
                let matched = f64::min(book[0].qty, remaining);
                let mut s = make_slice(&book[0], fill.qty, fill.side, &Exit::from(a), matched, Some(&a.symbol));
                mark(&mut s.flags, Flag::RolledIn);
                s.flags.sort();
                s.flags.dedup();
                if rt.is_some() {
                    s.rt = rt.clone();
                }
                s.id = stable_trade_id(&s);
                closed.push(s);
                book[0].qty -= matched;
                remaining -= matched;
                if book[0].qty <= EPS {
                    book.remove(0);
                }
            }
            books.closed_out(&other);
        }
    }

    if pool_of(rolled, &rk).is_empty() && books.get(&key).is_empty() {
        pool_of(rolled, &rk).rt = None;
    }
    remaining
}
