//! Each security's tax form for a year as this copy read it (`cds_tax`; migration
//! 11): kept on this machine alone.

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use bagholder_core::jiff::civil::Date;
use bagholder_core::Dec;

use crate::cds_tax::{Found, PerUnit};

/// One stored read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    pub year: i16,
    pub found: Found,
    pub read_at: String,
}

fn figures_json(rows: &[PerUnit]) -> String {
    Value::Array(rows.iter().map(|r| json!({"record": r.record.to_string(), "roc": r.return_of_capital.to_text(), "nonCash": r.non_cash.to_text()})).collect()).to_string()
}

fn figures_of(text: &str) -> Vec<PerUnit> {
    let v: Value = serde_json::from_str(text).unwrap_or(Value::Array(vec![]));
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    Some(PerUnit {
                        record: r.get("record")?.as_str()?.parse::<Date>().ok()?,
                        return_of_capital: Dec::parse(r.get("roc")?.as_str()?).ok()?,
                        non_cash: Dec::parse(r.get("nonCash")?.as_str()?).ok()?,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What a read found for a listing and year, in place of what was kept before.
pub fn save(conn: &Connection, listing: &str, year: i16, found: &Found, read_at: &str) -> rusqlite::Result<()> {
    let (outcome, cusip, form, figures, why) = match found {
        Found::Read { cusip, form, per_unit } => ("read", cusip.as_str(), form.as_str(), figures_json(per_unit), ""),
        Found::SetAside { cusip, form, why } => ("set-aside", cusip.as_str(), form.as_str(), "[]".to_string(), why.as_str()),
        Found::None => ("none", "", "", "[]".to_string(), ""),
    };
    conn.execute(
        "INSERT OR REPLACE INTO fund_tax_forms (listing, year, outcome, cusip, form, figures, why, read_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        params![listing, year, outcome, cusip, form, figures, why, read_at],
    )?;
    Ok(())
}

/// Every year kept for a listing, earliest first.
pub fn kept(conn: &Connection, listing: &str) -> rusqlite::Result<Vec<Kept>> {
    let mut st = conn.prepare("SELECT year, outcome, cusip, form, figures, why, read_at FROM fund_tax_forms WHERE listing = ? ORDER BY year")?;
    let rows = st.query_map(params![listing], |r| {
        let outcome: String = r.get(1)?;
        let (cusip, form): (String, String) = (r.get(2)?, r.get(3)?);
        let found = match outcome.as_str() {
            "read" => Found::Read { cusip, form, per_unit: figures_of(&r.get::<_, String>(4)?) },
            "set-aside" => Found::SetAside { cusip, form, why: r.get(5)? },
            _ => Found::None,
        };
        Ok(Kept { year: r.get(0)?, found, read_at: r.get(6)? })
    })?;
    rows.collect()
}

/// The CUSIP a listing's latest form stated, which later years are matched by.
pub fn cusip_of(conn: &Connection, listing: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT cusip FROM fund_tax_forms WHERE listing = ? AND cusip <> '' ORDER BY year DESC LIMIT 1",
        params![listing],
        |r| r.get(0),
    )
    .optional()
}
