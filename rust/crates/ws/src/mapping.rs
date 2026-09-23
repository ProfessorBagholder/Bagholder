//! Turning a Wealthsimple `ActivityFeedItem` into a ledger row.
//!
//! This is the only place the broker's vocabulary is read. What it decides --
//! which rows are fills, which way they go, what an option's per-share price
//! is, which corporate action replaced a ticker -- is what the whole model
//! then works from, and the stored copy is never rewritten afterwards.

use std::collections::{BTreeMap, HashMap};

use bagholder_model::value::compact;
use bagholder_store::broker::{Account, MappedActivity};

use crate::wire::AccountNode;

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

use crate::wire::ActivityItem;

fn up(s: &str) -> String {
    s.to_uppercase()
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
pub fn asset_symbol(item: &ActivityItem) -> String {
    let raw = item.asset_symbol.trim();
    let raw = if raw.to_uppercase().starts_with("EXCHANGE:") {
        raw.splitn(2, ':').nth(1).unwrap_or(raw)
    } else {
        raw
    };
    raw.to_uppercase().trim().to_string()
}

pub fn counter_symbol(item: &ActivityItem) -> String {
    let raw = item.counter_asset_symbol.trim();
    let raw = if raw.to_uppercase().starts_with("EXCHANGE:") {
        raw.splitn(2, ':').nth(1).unwrap_or(raw)
    } else {
        raw
    };
    raw.to_uppercase().trim().to_string()
}

/// `(type, subType, the four type fields joined)`.
fn type_blob(item: &ActivityItem) -> (String, String, String) {
    let typ = up(&item.kind).replace('-', "_");
    let sub = up(&item.sub_type).replace('-', "_");
    let parts: Vec<String> = [typ.clone(), sub.clone(), item.aft_transaction_type.clone(), item.aft_transaction_category.clone()]
        .into_iter()
        .filter(|x| !x.is_empty())
        .map(|x| compact(&x))
        .collect();
    let blob = parts.join("_");
    (typ, sub, blob)
}

/// A corporate action, or a distribution that
/// delivers shares rather than cash.
pub fn is_corp_share_move(item: &ActivityItem) -> bool {
    let (typ, _sub, blob) = type_blob(item);
    if CORP_BLOBS.iter().any(|k| blob.contains(k)) {
        return true;
    }
    let qty = item.asset_quantity.abs();
    let cash = item.amount.abs();
    qty != 0.0
        && !asset_symbol(item).is_empty()
        && cash == 0.0
        && (compact(&typ).contains("DIVIDEND") || blob.contains("DISTRIBUT"))
}

pub fn is_code_change(item: &ActivityItem) -> bool {
    let (_, _, blob) = type_blob(item);
    CODE_CHANGE_BLOBS.iter().any(|k| blob.contains(k))
}

/// Whether this row should not be stored.
pub fn skip_activity(item: &ActivityItem) -> bool {
    if item.occurred_at.trim().is_empty() {
        return true;
    }
    let status = compact(&item.status);
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
pub fn option_symbol(item: &ActivityItem) -> String {
    let under = asset_symbol(item);
    let contract = item.contract_type.clone();
    let strike = item.strike_price;
    let expiry = item.expiry_date.clone();
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
    let strike_f = match strike { Some(v) => v, None => return under };
    let cp = match contract.to_uppercase().as_str() {
        "C" | "CALL" => "CALL",
        "P" | "PUT" => "PUT",
        other => return format!("{} {:02}{}{:02} {:.2} {}", under, day, mon, year % 100, strike_f, other),
    };
    format!("{} {:02}{}{:02} {:.2} {}", under, day, mon, year % 100, strike_f, cp)
}

/// Buys, withdrawals and the source side of a
/// transfer are negative; sells, deposits and income positive.
pub fn signed_cash(item: &ActivityItem) -> f64 {
    let amount = item.amount.abs();
    let typ = up(&item.kind).replace('-', "_");
    let sub = up(&item.sub_type).replace('-', "_");
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
    let sign = item.amount_sign.trim().to_lowercase();
    if ["negative", "debit", "-", "neg"].contains(&sign.as_str()) {
        return -amount;
    }
    if ["positive", "credit", "+", "pos"].contains(&sign.as_str()) {
        return amount;
    }
    item.amount
}

/// The names and pools every activity row is mapped against: the nickname
/// and broker type each account is known by, and the FIFO pool every account
/// id resolves to, computed once rather than per row.
#[derive(Default)]
pub struct Accounts {
    nick_or_type: HashMap<String, String>,
    pools: HashMap<String, String>,
}

impl Accounts {
    /// Built from the Wealthsimple accounts a sync just fetched.
    pub fn from_nodes(nodes: &[AccountNode]) -> Self {
        let mut nick_or_type = HashMap::new();
        for a in nodes {
            if a.id.is_empty() {
                continue;
            }
            nick_or_type.insert(a.id.clone(), nick_of_node(a));
        }
        Accounts { nick_or_type, pools: fifo_pool_ids_from_nodes(nodes) }
    }

    /// Built from the store's own saved accounts, which carry no
    /// `linkedAccount` -- a shared nickname is still enough to pool them.
    pub fn from_stored(rows: &[Account]) -> Self {
        let mut nick_or_type = HashMap::new();
        let mut by_nick: Vec<(String, Vec<String>)> = Vec::new();
        let mut order: Vec<String> = Vec::new();
        for a in rows {
            if a.id.is_empty() {
                continue;
            }
            nick_or_type.insert(a.id.clone(), nick_of_stored(a));
            if !order.contains(&a.id) {
                order.push(a.id.clone());
            }
            let nick = a.nickname.trim().to_string();
            if !nick.is_empty() {
                match by_nick.iter_mut().find(|(k, _)| *k == nick) {
                    Some((_, v)) => v.push(a.id.clone()),
                    None => by_nick.push((nick, vec![a.id.clone()])),
                }
            }
        }
        let pools = union_pools(&order, &[], &by_nick);
        Accounts { nick_or_type, pools }
    }

    fn account_type(&self, id: &str) -> String {
        self.nick_or_type.get(id).cloned().unwrap_or_default()
    }

    /// What an account is called in a message: its nickname, else its type,
    /// else its id.
    pub fn name(&self, id: &str) -> String {
        self.nick_or_type.get(id).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| id.to_string())
    }

    /// The FIFO book an account id belongs to; an id nothing knows about is
    /// its own pool.
    pub fn pool(&self, id: &str) -> String {
        self.pools.get(id).cloned().unwrap_or_else(|| id.to_string())
    }

    fn is_empty(&self) -> bool {
        self.nick_or_type.is_empty()
    }
}

fn nick_of_node(a: &AccountNode) -> String {
    for v in [&a.nickname, &a.unified_account_type, &a.kind] {
        if !v.is_empty() {
            return v.clone();
        }
    }
    String::new()
}

fn nick_of_stored(a: &Account) -> String {
    for v in [&a.nickname, &a.unified_account_type, &a.kind] {
        if !v.is_empty() {
            return v.clone();
        }
    }
    String::new()
}

/// The nickname, else what the broker calls it.
pub fn account_type(account_id: &str, accounts: &Accounts) -> String {
    if accounts.is_empty() {
        return String::new();
    }
    accounts.account_type(account_id)
}

/// The nickname a NAV filter names, and the
/// Wealthsimple ids behind it. The CAD and USD sides of one account share a
/// nickname and so share a group.
pub fn nav_account_groups(nodes: &[AccountNode]) -> BTreeMap<String, Vec<String>> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for acc in nodes {
        let aid = acc.id.trim().to_string();
        if aid.is_empty() {
            continue;
        }
        let nick = nick_of_node(acc).trim().to_string();
        if nick.is_empty() {
            continue;
        }
        let bucket = groups.entry(nick).or_default();
        if !bucket.iter().any(|x| x == &aid) {
            bucket.push(aid);
        }
    }
    groups
}

fn union_pools(order: &[String], links: &[(String, String)], by_nick: &[(String, Vec<String>)]) -> HashMap<String, String> {
    let mut parent: HashMap<String, String> = HashMap::new();

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

    for id in order {
        find(&mut parent, id);
    }
    for (a, b) in links {
        if !order.contains(a) {
            find(&mut parent, a);
        }
        if !order.contains(b) {
            find(&mut parent, b);
        }
        union(&mut parent, a, b);
    }
    for (_, ids) in by_nick {
        if let Some((root, rest)) = ids.split_first() {
            for other in rest {
                union(&mut parent, root, other);
            }
        }
    }
    let keys: Vec<String> = parent.keys().cloned().collect();
    keys.iter().map(|k| (k.clone(), find(&mut parent, k))).collect()
}

/// The CAD and USD sides of one Wealthsimple
/// account share a single FIFO book.
///
/// A linked pair and a shared nickname both collapse to one root id; distinct
/// nicknames stay separate.
pub fn fifo_pool_ids_from_nodes(nodes: &[AccountNode]) -> HashMap<String, String> {
    let mut order: Vec<String> = Vec::new();
    let mut links: Vec<(String, String)> = Vec::new();
    let mut by_nick: Vec<(String, Vec<String>)> = Vec::new();
    for a in nodes {
        let aid = a.id.clone();
        if aid.is_empty() {
            continue;
        }
        if !order.contains(&aid) {
            order.push(aid.clone());
        }
        let lid = a.linked_account.as_ref().map(|l| l.id.clone()).unwrap_or_default();
        if !lid.is_empty() {
            if !order.contains(&lid) {
                order.push(lid.clone());
            }
            links.push((aid.clone(), lid));
        }
        let nick = a.nickname.trim().to_string();
        if !nick.is_empty() {
            match by_nick.iter_mut().find(|(k, _)| *k == nick) {
                Some((_, v)) => v.push(aid.clone()),
                None => by_nick.push((nick, vec![aid.clone()])),
            }
        }
    }
    union_pools(&order, &links, &by_nick)
}

fn is_option(item: &ActivityItem) -> bool {
    !item.contract_type.is_empty()
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
fn human_desc(typ: &str, sub: &str, symbol: &str, qty: f64, px: f64) -> String {
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
pub fn map_activity_rows(item: &ActivityItem, accounts: &Accounts) -> Vec<MappedActivity> {
    let src = asset_symbol(item);
    let dst = counter_symbol(item);
    let qty = item.asset_quantity.abs();
    if !src.is_empty() && !dst.is_empty() && src != dst && qty != 0.0 && is_corp_share_move(item) {
        let cid = { let c = item.canonical_id.trim().to_string(); if c.is_empty() { "swap".to_string() } else { c } };
        let mut rows = Vec::new();
        for (symbol, signed, sign, suffix) in [
            (src.clone(), -qty, "negative", ":out"),
            (dst.clone(), qty, "positive", ":in"),
        ] {
            let mut leg = item.clone();
            leg.asset_symbol = symbol;
            leg.counter_asset_symbol = String::new();
            leg.kind = "STKDIS".into();
            leg.sub_type = "STKDIS".into();
            leg.asset_quantity = signed;
            leg.amount = 0.0;
            leg.amount_sign = sign.to_string();
            leg.canonical_id = format!("{}{}", cid, suffix);
            if let Some(r) = map_activity(&leg, accounts) {
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
pub fn map_activity(item: &ActivityItem, accounts: &Accounts) -> Option<MappedActivity> {
    if skip_activity(item) {
        return None;
    }
    let occurred = item.occurred_at.trim().to_string();
    let transaction_date = date_only(&occurred);
    if transaction_date.is_empty() {
        return None;
    }

    let account_id = item.account_id.clone();
    let typ = up(&item.kind).replace('-', "_");
    let sub = up(&item.sub_type).replace('-', "_");
    let ctyp = compact(&typ);
    let qty_raw = item.asset_quantity;
    let qty_abs = qty_raw.abs();
    let cash = signed_cash(item);
    let amount_abs = item.amount.abs();
    let fees = item.fees.abs();
    let is_opt = is_option(item);
    let mut symbol = if is_opt { option_symbol(item) } else { asset_symbol(item) };

    let mut cur = up(&item.currency);
    if cur != "CAD" && cur != "USD" {
        cur = if is_opt { "USD".into() } else { "CAD".into() };
    }

    let mut unit_price = 0.0;
    if qty_abs != 0.0 {
        // a Wealthsimple option `amount` is full contract cash: per share, times
        // a hundred, times the contracts
        unit_price = if is_opt { amount_abs / (qty_abs * 100.0) } else { amount_abs / qty_abs };
    }

    let activity_type: String;
    let activity_sub: String;
    let category: String;
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
        let sign = item.amount_sign.trim().to_lowercase();
        let mut outgoing = qty_raw < 0.0 || ["negative", "debit", "-", "neg"].contains(&sign.as_str());
        // A lone international code change names the old ticker with a positive
        // quantity. Those shares were replaced, not bought.
        if !outgoing && is_code_change(item) && counter_symbol(item).is_empty() && !compact(&item.kind).contains("STKDIS") {
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
        activity_type = { let t = item.kind.clone(); if t.is_empty() { "Other".into() } else { t } };
        activity_sub = { let s = item.sub_type.clone(); if s.is_empty() { "other".into() } else { s } };
        category = "other".into();
    }

    if ["SELL", "SELLTOOPEN", "SELLTOCLOSE"].contains(&activity_sub.as_str()) && qty_abs != 0.0 {
        quantity = -qty_abs;
    }

    let sign = item.amount_sign.trim().to_lowercase();
    let direction = if ["negative", "debit", "-", "neg"].contains(&sign.as_str()) || cash < 0.0 {
        "DEBIT"
    } else if ["positive", "credit", "+", "pos"].contains(&sign.as_str()) || cash > 0.0 {
        "CREDIT"
    } else {
        ""
    };

    let mut cid = item.canonical_id.trim().to_string();
    if bagholder_store::activities::looks_like_homemade_id(&cid) {
        cid = String::new();
    }

    let desc = human_desc(&typ, &sub, &symbol, quantity, unit_price);
    let name = {
        let a = item.aft_originator_name.clone();
        if !a.is_empty() { a } else {
            let i = item.institution_name.clone();
            if !i.is_empty() { i } else { symbol.clone() }
        }
    };
    let fifo_id = if accounts.is_empty() { account_id.clone() } else { accounts.pool(&account_id) };
    let security_id = item.security_id.trim().to_string();

    Some(MappedActivity {
        canonical_id: if cid.is_empty() { None } else { Some(cid) },
        occurred_at: occurred,
        transaction_date: transaction_date.clone(),
        settlement_date: transaction_date,
        account_id: account_id.clone(),
        book_id: account_id.clone(),
        fifo_id,
        account_type: account_type(&account_id, accounts),
        activity_type,
        activity_sub_type: activity_sub,
        description: desc,
        direction: direction.to_string(),
        symbol,
        name,
        currency: cur,
        quantity,
        unit_price,
        commission: fees,
        net_cash_amount: cash,
        category,
        balance: None,
        source: "wealthsimple".into(),
        raw_type: item.kind.clone(),
        aft_type: item.aft_transaction_type.clone(),
        counter_symbol: counter_symbol(item),
        security_id: if security_id.is_empty() { None } else { Some(security_id) },
    })
}
