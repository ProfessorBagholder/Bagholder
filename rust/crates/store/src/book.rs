//! The book export (`GET /api/book`, for the phones and for export): the
//! stored rows as they are, typed straight from the columns.

use rusqlite::{Connection, Result};
use serde::Serialize;
use std::collections::BTreeMap;

use bagholder_model::input::TradeGroup;
use bagholder_model::securities::Security;

use crate::activities::ActivityRow;
use crate::broker::Account;
use crate::tables::LegacyNotes;

/// One security position, for the export: a text column that was `NULL` in
/// the database is absent on the wire, not an empty string.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookBalance {
    pub account_id: Option<String>,
    pub custodian_account_id: Option<String>,
    pub security_id: Option<String>,
    pub quantity: Option<f64>,
}

/// One day's net liquidation value, for the export: no account id, because
/// that is the map key it is filed under (or omitted entirely, for the
/// identity-wide series).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookNav {
    pub date: String,
    pub equity: Option<f64>,
    pub currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net_deposits: Option<f64>,
}

/// The stored rows as they are, for the phones and for export.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Book {
    pub ok: bool,
    pub activities: Vec<ActivityRow>,
    pub accounts: Vec<Account>,
    pub balances: Vec<BookBalance>,
    pub nav_history: Vec<BookNav>,
    pub nav_by_account: BTreeMap<String, Vec<BookNav>>,
    pub synced_at: String,
    pub trade_groups: Vec<TradeGroup>,
    pub notes: LegacyNotes,
    pub securities: Vec<Security>,
}

fn balances(conn: &Connection) -> Result<Vec<BookBalance>> {
    // no ORDER BY: the rowid order is the order
    let mut stmt = conn.prepare("SELECT account_id, custodian_account_id, security_id, quantity FROM balances")?;
    let rows = stmt.query_map([], |r| {
        Ok(BookBalance {
            account_id: r.get::<_, Option<String>>(0)?.filter(|s| !s.is_empty()),
            custodian_account_id: r.get::<_, Option<String>>(1)?.filter(|s| !s.is_empty()),
            security_id: r.get::<_, Option<String>>(2)?.filter(|s| !s.is_empty()),
            quantity: r.get(3)?,
        })
    })?;
    rows.collect()
}

/// The NAV history: every account together, and each account's own.
fn nav(conn: &Connection) -> Result<(Vec<BookNav>, BTreeMap<String, Vec<BookNav>>)> {
    let mut together = Vec::new();
    let mut by_account: BTreeMap<String, Vec<BookNav>> = BTreeMap::new();
    let mut stmt = conn.prepare("SELECT account_id, date, equity, currency, net_deposits FROM nav_history ORDER BY account_id, date")?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let account_id: String = r.get::<_, Option<String>>(0)?.unwrap_or_default();
        let currency: String = r.get::<_, Option<String>>(3)?.filter(|c| !c.is_empty()).unwrap_or_else(|| "CAD".into());
        let point = BookNav {
            date: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            equity: r.get(2)?,
            currency,
            net_deposits: r.get(4)?,
        };
        if account_id.is_empty() {
            together.push(point);
        } else {
            by_account.entry(account_id).or_default().push(point);
        }
    }
    Ok((together, by_account))
}

/// The stored rows as they are: for `GET /api/book` and for export.
pub fn book(conn: &Connection) -> Result<Book> {
    let (nav_history, nav_by_account) = nav(conn)?;
    Ok(Book {
        ok: true,
        activities: crate::activities::all_activities(conn)?,
        accounts: crate::tables::accounts(conn)?,
        balances: balances(conn)?,
        nav_history,
        nav_by_account,
        synced_at: crate::tables::get_meta(conn, "synced_at", "")?,
        trade_groups: crate::tables::trade_groups(conn)?,
        notes: crate::tables::trade_notes(conn)?,
        securities: crate::rows::securities(conn)?,
    })
}

/// The distinct account ids an activity was ever posted to.
pub fn distinct_activity_account_ids(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT DISTINCT account_id FROM activities WHERE IFNULL(account_id, '') != ''")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    rows.collect()
}

/// The distinct security ids an activity or a balance names.
pub fn distinct_security_ids(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT security_id FROM activities WHERE IFNULL(security_id, '') != '' \
         UNION SELECT security_id FROM balances WHERE IFNULL(security_id, '') != ''",
    )?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    rows.collect()
}
