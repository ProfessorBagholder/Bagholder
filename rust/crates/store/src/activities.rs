//! Activity rows: the shape the model reads, and the shape the table holds.
//!
//! A Wealthsimple row keeps the broker's own id as its canonical id, so the
//! same fill pulled twice is one row. A typed or imported row never gets one
//! fabricated for it.

use rusqlite::{Connection, Result, Row};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use bagholder_model::lenient;

pub const INVENTED_ACCOUNTS: [&str; 7] = ["", "manual", "legacy", "statement", "canonical", "cad", "usd"];

/// Every column of `activities`, in the order the insert names them.
pub const COLUMNS: [&str; 27] = [
    "id", "canonical_id", "occurred_at", "transaction_date", "settlement_date", "account_id", "book_id",
    "fifo_id", "account_type", "activity_type", "activity_sub_type", "description", "direction", "symbol",
    "name", "currency", "quantity", "unit_price", "commission", "net_cash_amount", "category", "balance",
    "source", "raw_type", "aft_type", "counter_symbol", "security_id",
];

const INSERT_SQL: &str = "INSERT INTO activities (
        id, canonical_id, occurred_at, transaction_date, settlement_date,
        account_id, book_id, fifo_id, account_type, activity_type,
        activity_sub_type, description, direction, symbol, name, currency,
        quantity, unit_price, commission, net_cash_amount, category, balance,
        source, raw_type, aft_type, counter_symbol, security_id
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";

pub(crate) const SELECT_ALL: &str = "SELECT id, canonical_id, occurred_at, transaction_date, settlement_date, account_id, book_id, fifo_id, account_type, activity_type, activity_sub_type, description, direction, symbol, name, currency, quantity, unit_price, commission, net_cash_amount, category, balance, source, raw_type, aft_type, counter_symbol, security_id FROM activities";

/// A text field that is `None` rather than empty when absent, null, or blank
/// -- `canonicalId` and `securityId` are never `Some("")`.
fn opt_text<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Option<String>, D::Error> {
    let v = Value::deserialize(d)?;
    let s = bagholder_model::value::s(Some(&v));
    Ok(if s.is_empty() { None } else { Some(s) })
}

/// One row of `activities`: what every source hands in and what a read gives back.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(default, rename_all = "camelCase")]
pub struct ActivityRow {
    #[serde(deserialize_with = "lenient::text")]
    pub id: String,
    #[serde(deserialize_with = "opt_text", alias = "canonical_id")]
    pub canonical_id: Option<String>,
    #[serde(deserialize_with = "lenient::text", alias = "occurred_at")]
    pub occurred_at: String,
    #[serde(deserialize_with = "lenient::text", alias = "transaction_date")]
    pub transaction_date: String,
    #[serde(deserialize_with = "lenient::text", alias = "settlement_date")]
    pub settlement_date: String,
    #[serde(deserialize_with = "lenient::text", alias = "account_id")]
    pub account_id: String,
    #[serde(deserialize_with = "lenient::text", alias = "book_id")]
    pub book_id: String,
    #[serde(deserialize_with = "lenient::text", alias = "fifo_id")]
    pub fifo_id: String,
    #[serde(deserialize_with = "lenient::text", alias = "account_type")]
    pub account_type: String,
    #[serde(deserialize_with = "lenient::text", alias = "activity_type")]
    pub activity_type: String,
    #[serde(deserialize_with = "lenient::text", alias = "activity_sub_type")]
    pub activity_sub_type: String,
    #[serde(deserialize_with = "lenient::text")]
    pub description: String,
    #[serde(deserialize_with = "lenient::text")]
    pub direction: String,
    #[serde(deserialize_with = "lenient::text")]
    pub symbol: String,
    #[serde(deserialize_with = "lenient::text")]
    pub name: String,
    #[serde(deserialize_with = "lenient::text")]
    pub currency: String,
    #[serde(deserialize_with = "lenient::number")]
    pub quantity: f64,
    #[serde(deserialize_with = "lenient::number", alias = "unit_price")]
    pub unit_price: f64,
    #[serde(deserialize_with = "lenient::number")]
    pub commission: f64,
    #[serde(deserialize_with = "lenient::number", alias = "net_cash_amount")]
    pub net_cash_amount: f64,
    #[serde(deserialize_with = "lenient::text")]
    pub category: String,
    #[serde(deserialize_with = "lenient::maybe_number")]
    pub balance: Option<f64>,
    #[serde(deserialize_with = "lenient::text")]
    pub source: String,
    #[serde(deserialize_with = "lenient::text", alias = "raw_type")]
    pub raw_type: String,
    #[serde(deserialize_with = "lenient::text", alias = "aft_type")]
    pub aft_type: String,
    #[serde(deserialize_with = "lenient::text", alias = "counter_symbol")]
    pub counter_symbol: String,
    #[serde(deserialize_with = "opt_text", alias = "security_id")]
    pub security_id: Option<String>,
}

impl ActivityRow {
    /// What `_insert_params` computed today: a date read out of the
    /// occurred timestamp when no transaction date is given, a date-only
    /// occurred timestamp kept date-only, a settlement date defaulted to the
    /// transaction date, a book/fifo id defaulted to the account id, and a
    /// security id trimmed to `None` when blank.
    pub fn normalized(&self) -> ActivityRow {
        let mut r = self.clone();
        let mut occurred = r.occurred_at.trim().to_string();
        let mut date = r.transaction_date.trim().to_string();
        if date.is_empty() && !occurred.is_empty() {
            date = occurred.split('T').next().unwrap_or("").chars().take(10).collect();
        }
        if !occurred.is_empty() && !occurred.contains('T') {
            // a date-only source (typed in, or a CSV) stays date-only
            occurred = occurred.chars().take(10).collect();
        }
        let settle = { let s = r.settlement_date.trim().to_string(); if s.is_empty() { date.clone() } else { s } };
        let book_id = if r.book_id.is_empty() { r.account_id.clone() } else { r.book_id.clone() };
        let fifo_id = if r.fifo_id.is_empty() { r.account_id.clone() } else { r.fifo_id.clone() };
        let sid = r.security_id.clone().unwrap_or_default().trim().to_string();
        r.occurred_at = occurred;
        r.transaction_date = date;
        r.settlement_date = settle;
        r.book_id = book_id;
        r.fifo_id = fifo_id;
        r.security_id = if sid.is_empty() { None } else { Some(sid) };
        r
    }
}

/// `looks_like_homemade_id`: an id Bagholder made, not the broker.
pub fn looks_like_homemade_id(aid: &str) -> bool {
    let s = aid.trim();
    s.is_empty() || s.contains('|') || s.to_lowercase().starts_with("manual")
}

/// `is_real_account`: an account Wealthsimple actually has, as opposed
/// to the placeholders an import invents.
pub fn is_real_account(account_id: &str) -> bool {
    let s = account_id.trim();
    if s.is_empty() || s.starts_with('~') {
        return false;
    }
    !INVENTED_ACCOUNTS.contains(&s.to_lowercase().as_str())
}

/// `_round_qty`: eight decimals, which is what a crypto quantity needs
/// and what the match keys compare on.
pub fn round_qty(v: f64) -> f64 {
    (v * 1e8).round() / 1e8
}

/// `trade_side`.
pub fn trade_side(act: &ActivityRow) -> String {
    let side = bagholder_model::fifo::side_of(&act.activity_sub_type, &act.activity_type, act.quantity);
    side.map_or("", |s| s.as_str()).to_string()
}

fn key_date(act: &ActivityRow) -> String {
    let d: String = act.transaction_date.chars().take(10).collect();
    if !d.is_empty() {
        return d;
    }
    act.occurred_at.split('T').next().unwrap_or("").chars().take(10).collect()
}

fn key_account(act: &ActivityRow, include_account: bool) -> String {
    if !include_account {
        return String::new();
    }
    if is_real_account(&act.account_id) { act.account_id.clone() } else { String::new() }
}

/// `field_match_key`: what makes two rows the same fill when neither
/// carries the broker's id.
pub fn field_match_key(act: &ActivityRow, include_account: bool) -> (String, String, String, f64, f64, f64) {
    (
        key_date(act),
        key_account(act, include_account),
        act.symbol.trim().to_uppercase(),
        round_qty(act.quantity),
        round_qty(act.unit_price),
        round_qty(act.net_cash_amount),
    )
}

/// `link_match_key`: the same, ordered for linking an imported row to a
/// broker one.
pub fn link_match_key(act: &ActivityRow, include_account: bool) -> (String, String, f64, f64, String, String) {
    (
        act.symbol.trim().to_uppercase(),
        trade_side(act),
        round_qty(act.quantity),
        round_qty(act.unit_price),
        key_date(act),
        key_account(act, include_account),
    )
}

/// `_canonical_from_row`: the broker's id, never one Bagholder made.
pub fn canonical_from_row(act: &ActivityRow, source: &str) -> Option<String> {
    if source != "wealthsimple" {
        return None;
    }
    let cid = act.canonical_id.clone().unwrap_or_default().trim().to_string();
    if !cid.is_empty() && !looks_like_homemade_id(&cid) {
        return Some(cid);
    }
    let old_id = act.id.trim().to_string();
    if !old_id.is_empty() && !looks_like_homemade_id(&old_id) {
        return Some(old_id);
    }
    None
}

fn text(row: &Row, idx: usize) -> rusqlite::Result<String> {
    Ok(row.get::<_, Option<String>>(idx)?.unwrap_or_default())
}

fn real(row: &Row, idx: usize) -> rusqlite::Result<f64> {
    Ok(row.get::<_, Option<f64>>(idx)?.filter(|x| !x.is_nan()).unwrap_or(0.0))
}

/// The row as the model wants it, with these fallbacks -- a missing settlement date is the transaction
/// date, a missing book or fifo id the account's.
pub fn from_row(row: &Row) -> rusqlite::Result<ActivityRow> {
    let canonical = row.get::<_, Option<String>>(1)?.filter(|s| !s.is_empty());
    let transaction_date = text(row, 3)?;
    let account_id = text(row, 5)?;
    let settlement = { let s = text(row, 4)?; if s.is_empty() { transaction_date.clone() } else { s } };
    let book_id = { let s = text(row, 6)?; if s.is_empty() { account_id.clone() } else { s } };
    let fifo_id = { let s = text(row, 7)?; if s.is_empty() { account_id.clone() } else { s } };
    let security_id = row.get::<_, Option<String>>(26)?.filter(|s| !s.is_empty());

    Ok(ActivityRow {
        id: text(row, 0)?,
        canonical_id: canonical,
        occurred_at: text(row, 2)?,
        transaction_date,
        settlement_date: settlement,
        account_id,
        book_id,
        fifo_id,
        account_type: text(row, 8)?,
        activity_type: text(row, 9)?,
        activity_sub_type: text(row, 10)?,
        description: text(row, 11)?,
        direction: text(row, 12)?,
        symbol: text(row, 13)?,
        name: text(row, 14)?,
        currency: text(row, 15)?,
        quantity: real(row, 16)?,
        unit_price: real(row, 17)?,
        commission: real(row, 18)?,
        net_cash_amount: real(row, 19)?,
        category: text(row, 20)?,
        balance: row.get::<_, Option<f64>>(21)?,
        source: text(row, 22)?,
        raw_type: text(row, 23)?,
        aft_type: text(row, 24)?,
        counter_symbol: text(row, 25)?,
        security_id,
    })
}

/// `_all_activities`: every row, oldest first.
pub fn all_activities(conn: &Connection) -> Result<Vec<ActivityRow>> {
    let sql = format!("{SELECT_ALL} ORDER BY COALESCE(occurred_at, transaction_date) ASC, id ASC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], from_row)?;
    rows.collect()
}

/// Rows not yet linked to a broker canonical id -- what an imported row is
/// checked against for a link.
pub fn unlinked(conn: &Connection) -> Result<Vec<ActivityRow>> {
    let sql = format!("{SELECT_ALL} WHERE canonical_id IS NULL OR canonical_id = ''");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], from_row)?;
    rows.collect()
}

/// A stored row as the model reads one: the same columns `from_row` gives, with
/// no JSON between the row and the struct. A number that is not there is zero and a text
/// that is not there is empty, as the model's lenient reading of the JSON row has it.
fn row_to_raw(row: &Row) -> rusqlite::Result<bagholder_model::activity::RawActivity> {
    let number = |idx: usize| -> rusqlite::Result<f64> { Ok(row.get::<_, Option<f64>>(idx)?.filter(|x| !x.is_nan()).unwrap_or(0.0)) };
    let account_id = text(row, 5)?;
    let or_account = |t: String| if t.is_empty() { account_id.clone() } else { t };
    Ok(bagholder_model::activity::RawActivity {
        id: text(row, 0)?,
        occurred_at: text(row, 2)?,
        transaction_date: text(row, 3)?,
        book_id: or_account(text(row, 6)?),
        fifo_id: or_account(text(row, 7)?),
        account_type: text(row, 8)?,
        activity_type: text(row, 9)?,
        activity_sub_type: text(row, 10)?,
        description: text(row, 11)?,
        direction: text(row, 12)?,
        symbol: text(row, 13)?,
        name: text(row, 14)?,
        currency: text(row, 15)?,
        quantity: number(16)?,
        unit_price: number(17)?,
        commission: number(18)?,
        net_cash_amount: number(19)?,
        category: text(row, 20)?,
        raw_type: text(row, 23)?,
        aft_type: text(row, 24)?,
        security_id: text(row, 26)?,
        kind: String::new(), // a stored row never says what it is
        account_id,
    })
}

/// Every row, oldest first, as the model reads them.
pub fn all_raw_activities(conn: &Connection) -> Result<Vec<bagholder_model::activity::RawActivity>> {
    let sql = format!("{SELECT_ALL} ORDER BY COALESCE(occurred_at, transaction_date) ASC, id ASC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_raw)?;
    rows.collect()
}

pub fn activity_by_id(conn: &Connection, id: &str) -> Result<Option<ActivityRow>> {
    let sql = format!("{SELECT_ALL} WHERE id = ?");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([id])?;
    match rows.next()? {
        Some(r) => Ok(Some(from_row(r)?)),
        None => Ok(None),
    }
}

/// The stored row with this canonical id, if any -- what a revision compares
/// the incoming row against.
pub fn by_canonical_id(conn: &Connection, cid: &str) -> Result<Option<ActivityRow>> {
    let sql = format!("{SELECT_ALL} WHERE canonical_id = ?");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([cid])?;
    match rows.next()? {
        Some(r) => Ok(Some(from_row(r)?)),
        None => Ok(None),
    }
}

/// The 18 columns Wealthsimple may revise on a row of its own, plus the
/// security id a later pull may add. Compared field by field: the four
/// numbers within a billionth, the rest as text.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Revisable {
    pub occurred_at: String,
    pub transaction_date: String,
    pub settlement_date: String,
    pub activity_type: String,
    pub activity_sub_type: String,
    pub description: String,
    pub direction: String,
    pub symbol: String,
    pub name: String,
    pub currency: String,
    pub quantity: f64,
    pub unit_price: f64,
    pub commission: f64,
    pub net_cash_amount: f64,
    pub category: String,
    pub raw_type: String,
    pub aft_type: String,
    pub counter_symbol: String,
    pub security_id: Option<String>,
}

impl Revisable {
    pub fn of(row: &ActivityRow) -> Revisable {
        Revisable {
            occurred_at: row.occurred_at.clone(),
            transaction_date: row.transaction_date.clone(),
            settlement_date: row.settlement_date.clone(),
            activity_type: row.activity_type.clone(),
            activity_sub_type: row.activity_sub_type.clone(),
            description: row.description.clone(),
            direction: row.direction.clone(),
            symbol: row.symbol.clone(),
            name: row.name.clone(),
            currency: row.currency.clone(),
            quantity: row.quantity,
            unit_price: row.unit_price,
            commission: row.commission,
            net_cash_amount: row.net_cash_amount,
            category: row.category.clone(),
            raw_type: row.raw_type.clone(),
            aft_type: row.aft_type.clone(),
            counter_symbol: row.counter_symbol.clone(),
            security_id: row.security_id.clone(),
        }
    }
}

/// `insert_activity`: one row. The caller decides the canonical id, and
/// none is ever fabricated.
pub fn insert_activity(
    conn: &Connection,
    act: &ActivityRow,
    canonical_id: Option<&str>,
    assigned_id: Option<&str>,
    new_id: &dyn Fn() -> String,
) -> Result<ActivityRow> {
    let source = if act.source.is_empty() { "wealthsimple".to_string() } else { act.source.clone() };
    let mut canonical: Option<String> = canonical_id.map(|c| c.to_string());
    if canonical.is_none() && source == "wealthsimple" {
        canonical = canonical_from_row(act, &source);
    }
    if source != "wealthsimple" {
        canonical = None;
    }
    if canonical.as_deref() == Some("") {
        canonical = None;
    }
    let mut aid = assigned_id.map(|s| s.to_string()).unwrap_or_else(|| act.id.trim().to_string());
    if aid.is_empty() || looks_like_homemade_id(&aid) {
        aid = new_id();
    }
    let n = act.normalized();
    conn.execute(
        INSERT_SQL,
        rusqlite::params![
            aid,
            canonical,
            n.occurred_at,
            n.transaction_date,
            n.settlement_date,
            n.account_id,
            n.book_id,
            n.fifo_id,
            n.account_type,
            n.activity_type,
            n.activity_sub_type,
            n.description,
            n.direction,
            n.symbol,
            n.name,
            n.currency,
            n.quantity,
            n.unit_price,
            n.commission,
            n.net_cash_amount,
            n.category,
            n.balance,
            act.source.clone().to_string(),
            n.raw_type,
            n.aft_type,
            n.counter_symbol,
            n.security_id,
        ],
    )?;
    Ok(activity_by_id(conn, &aid)?.unwrap_or_default())
}

/// `insert_local`: a typed-in or imported row. It gets a Bagholder id
/// and never a fabricated canonical id.
pub fn insert_local(conn: &Connection, act: &ActivityRow, new_id: &dyn Fn() -> String) -> Result<ActivityRow> {
    let mut row = act.clone();
    let mut source = row.source.clone();
    if source.is_empty() || source == "wealthsimple" {
        source = "manual".into();
    }
    row.source = source;
    row.canonical_id = None;
    insert_activity(conn, &row, None, None, new_id)
}

/// `activity_count`.
pub fn activity_count(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM activities", [], |r| r.get(0))
}

/// `canonical_ids`: every broker id the store already holds.
pub fn canonical_ids(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT canonical_id FROM activities WHERE canonical_id IS NOT NULL AND canonical_id != ''")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    rows.collect()
}
