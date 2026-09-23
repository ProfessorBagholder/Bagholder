//! Taking rows in: the Wealthsimple sync, and the CSV or typed-in merge.
//!
//! A Wealthsimple row is known by its canonical id, and a known row is
//! replaced only when Wealthsimple itself has revised it. Bagholder never
//! edits a row of the broker's on its own. An imported row that matches one
//! stored fill exactly is linked to it rather than duplicated; two matches are
//! not guessed between.

use rusqlite::{Connection, Result};
use std::collections::{HashMap, HashSet};

use crate::activities::{
    activity_by_id, all_activities, by_canonical_id, canonical_from_row, canonical_ids, field_match_key,
    insert_activity, insert_local, is_real_account, link_match_key, looks_like_homemade_id, unlinked, ActivityRow,
    Revisable,
};

/// `_REVISABLE_COLUMNS`: what Wealthsimple revises on a row of its own.
///
/// A dividend announced as a placeholder on the record date -- no cash, dated
/// that day -- becomes the paid dividend on pay day under the same canonical
/// id. The identity, the account and the stored id stay.
pub const REVISABLE_COLUMNS: [&str; 18] = [
    "occurred_at", "transaction_date", "settlement_date", "activity_type", "activity_sub_type",
    "description", "direction", "symbol", "name", "currency", "quantity", "unit_price", "commission",
    "net_cash_amount", "category", "raw_type", "aft_type", "counter_symbol",
];

/// `find_link_candidates`: unlinked local rows matching symbol, side,
/// quantity, price and date -- and the account when it is a real one.
pub fn find_link_candidates(conn: &Connection, act: &ActivityRow) -> Result<Vec<ActivityRow>> {
    let include_account = is_real_account(&act.account_id);
    let target = link_match_key(act, include_account);
    let mut out = Vec::new();
    for mapped in unlinked(conn)? {
        if link_match_key(&mapped, include_account) == target {
            out.push(mapped);
        }
    }
    Ok(out)
}

/// `stamp_canonical_id`: mark a local row as the broker's, but only when
/// it has no broker id already.
///
/// The answer is whether this statement changed a row -- not the pooled
/// connection's running tally of changes, which is all but always positive.
/// `apply_wealthsimple_mapped` takes a true here as "the imported row is now
/// the broker's" and skips the insert, so a stamp that matched nothing must
/// not read as one that did.
pub fn stamp_canonical_id(conn: &Connection, activity_id: &str, canonical_id: &str) -> Result<bool> {
    let cid = canonical_id.trim();
    if cid.is_empty() || looks_like_homemade_id(cid) {
        return Ok(false);
    }
    let n = conn.execute(
        "UPDATE activities SET canonical_id = ? WHERE id = ? AND (canonical_id IS NULL OR canonical_id = '')",
        rusqlite::params![cid, activity_id],
    )?;
    Ok(n > 0)
}

/// One field of a `Revisable`, as an update parameter -- the incoming value
/// for every column except `security_id`, which is taken from the incoming
/// row's own (normalized) security id rather than `Revisable`'s copy of it.
fn revisable_param(r: &Revisable, security_id: &Option<String>, col: &str) -> Box<dyn rusqlite::ToSql> {
    match col {
        "occurred_at" => Box::new(r.occurred_at.clone()),
        "transaction_date" => Box::new(r.transaction_date.clone()),
        "settlement_date" => Box::new(r.settlement_date.clone()),
        "activity_type" => Box::new(r.activity_type.clone()),
        "activity_sub_type" => Box::new(r.activity_sub_type.clone()),
        "description" => Box::new(r.description.clone()),
        "direction" => Box::new(r.direction.clone()),
        "symbol" => Box::new(r.symbol.clone()),
        "name" => Box::new(r.name.clone()),
        "currency" => Box::new(r.currency.clone()),
        "quantity" => Box::new(r.quantity),
        "unit_price" => Box::new(r.unit_price),
        "commission" => Box::new(r.commission),
        "net_cash_amount" => Box::new(r.net_cash_amount),
        "category" => Box::new(r.category.clone()),
        "raw_type" => Box::new(r.raw_type.clone()),
        "aft_type" => Box::new(r.aft_type.clone()),
        "counter_symbol" => Box::new(r.counter_symbol.clone()),
        "security_id" => Box::new(security_id.clone()),
        _ => Box::new(None::<String>),
    }
}

/// `_revise_wealthsimple_row`: replace the stored copy with the broker's
/// current version when a revisable field changed.
pub fn revise_wealthsimple_row(conn: &Connection, cid: &str, row: &ActivityRow) -> Result<bool> {
    let normalized = row.normalized();
    let incoming = Revisable::of(&normalized);
    let stored_row = match by_canonical_id(conn, cid)? { Some(r) => r, None => return Ok(false) };
    let stored = Revisable::of(&stored_row);

    let mut changed: Vec<&str> = Vec::new();
    macro_rules! diff_str {
        ($f:ident, $name:expr) => {
            if incoming.$f != stored.$f {
                changed.push($name);
            }
        };
    }
    macro_rules! diff_num {
        ($f:ident, $name:expr) => {
            if (incoming.$f - stored.$f).abs() > 1e-9 {
                changed.push($name);
            }
        };
    }
    diff_str!(occurred_at, "occurred_at");
    diff_str!(transaction_date, "transaction_date");
    diff_str!(settlement_date, "settlement_date");
    diff_str!(activity_type, "activity_type");
    diff_str!(activity_sub_type, "activity_sub_type");
    diff_str!(description, "description");
    diff_str!(direction, "direction");
    diff_str!(symbol, "symbol");
    diff_str!(name, "name");
    diff_str!(currency, "currency");
    diff_num!(quantity, "quantity");
    diff_num!(unit_price, "unit_price");
    diff_num!(commission, "commission");
    diff_num!(net_cash_amount, "net_cash_amount");
    diff_str!(category, "category");
    diff_str!(raw_type, "raw_type");
    diff_str!(aft_type, "aft_type");
    diff_str!(counter_symbol, "counter_symbol");
    if changed.is_empty() {
        return Ok(false);
    }
    // a security id the broker now supplies is taken, but never replaced
    let incoming_has = incoming.security_id.as_deref().is_some_and(|s| !s.is_empty());
    let stored_has = stored.security_id.as_deref().is_some_and(|s| !s.is_empty());
    if incoming_has && !stored_has {
        changed.push("security_id");
    }

    let sets = changed.iter().map(|c| format!("{} = ?", c)).collect::<Vec<_>>().join(", ");
    let sql = format!("UPDATE activities SET {} WHERE canonical_id = ?", sets);
    let mut params: Vec<Box<dyn rusqlite::ToSql>> =
        changed.iter().map(|c| revisable_param(&incoming, &normalized.security_id, c)).collect();
    params.push(Box::new(cid.to_string()));
    let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
    conn.execute(&sql, refs.as_slice())?;
    Ok(true)
}

#[derive(Default, Debug, serde::Serialize)]
pub struct Applied {
    pub inserted: usize,
    pub linked: usize,
    pub skipped: usize,
    pub revised: usize,
    /// A row whose canonical id was empty or looked homemade: said, not
    /// silently dropped.
    pub unidentified: usize,
}

/// `apply_wealthsimple_mapped`.
pub fn apply_wealthsimple_mapped(conn: &Connection, rows: &[ActivityRow], new_id: &dyn Fn() -> String) -> Result<Applied> {
    crate::atomically(conn, || {
        let mut out = Applied::default();
        let mut known: HashSet<String> = canonical_ids(conn)?.into_iter().collect();

        for raw in rows {
            if raw == &ActivityRow::default() {
                continue;
            }
            let mut row = raw.clone();
            row.source = "wealthsimple".into();

            let cid = row.canonical_id.clone().unwrap_or_default().trim().to_string();
            if cid.is_empty() || looks_like_homemade_id(&cid) {
                out.unidentified += 1;
                continue;
            }
            if known.contains(&cid) {
                if revise_wealthsimple_row(conn, &cid, &row)? {
                    out.revised += 1;
                    continue;
                }
                let sid = row.security_id.clone().unwrap_or_default().trim().to_string();
                if !sid.is_empty() {
                    conn.execute(
                        "UPDATE activities SET security_id = ? WHERE canonical_id = ? AND (security_id IS NULL OR security_id = '')",
                        rusqlite::params![sid, cid],
                    )?;
                }
                out.skipped += 1;
                continue;
            }
            let matches = find_link_candidates(conn, &row)?;
            if matches.len() == 1 {
                if stamp_canonical_id(conn, &matches[0].id, &cid)? {
                    known.insert(cid.clone());
                    out.linked += 1;
                    continue;
                }
            }
            insert_activity(conn, &row, Some(&cid), None, new_id)?;
            known.insert(cid);
            out.inserted += 1;
        }
        Ok(out)
    })
}

#[derive(Debug, serde::Serialize)]
pub struct Merged {
    pub ok: bool,
    pub added: usize,
    pub duplicates: usize,
    pub activities: Vec<ActivityRow>,
}

/// `merge_local_rows`: a CSV or typed merge, on date, account, symbol,
/// quantity, price and cash rather than on any id.
pub fn merge_local_rows(conn: &Connection, rows: &[ActivityRow], new_id: &dyn Fn() -> String) -> Result<Merged> {
    crate::atomically(conn, || {
        let mut stored: Vec<ActivityRow> = Vec::new();
        let mut added = 0usize;
        let mut duplicates = 0usize;

        let mut existing: HashMap<Key, usize> = HashMap::new();
        for a in all_activities(conn)? {
            *existing.entry(Key::of(&a)).or_insert(0) += 1;
        }

        let mut incoming_seen: HashMap<Key, usize> = HashMap::new();
        for raw in rows {
            if raw == &ActivityRow::default() {
                continue;
            }
            let mut row = raw.clone();
            let mut source = row.source.clone();
            if source.is_empty() {
                source = "csv".into();
            }
            if source == "wealthsimple" {
                if canonical_from_row(&row, "wealthsimple").is_some() {
                    let result = apply_wealthsimple_mapped(conn, std::slice::from_ref(raw), new_id)?;
                    added += result.inserted + result.linked;
                    if result.skipped > 0 {
                        duplicates += result.skipped;
                    }
                    continue;
                }
                source = "csv".into();
            }
            row.source = source;
            row.canonical_id = None;

            let k = Key::of(&row);
            let n = incoming_seen.entry(k.clone()).or_insert(0);
            *n += 1;
            if *n <= *existing.get(&k).unwrap_or(&0) {
                duplicates += 1;
                continue;
            }
            let saved = insert_local(conn, &row, new_id)?;
            *existing.entry(k).or_insert(0) += 1;
            stored.push(saved);
            added += 1;
        }
        Ok(Merged { ok: true, added, duplicates, activities: stored })
    })
}

/// Re-exported so callers reading a single row back can use it.
pub fn by_id(conn: &Connection, id: &str) -> Result<Option<ActivityRow>> {
    activity_by_id(conn, id)
}


/// A field match key that can be a map key.
///
/// The key's floats compare by value, so the bits are used here with
/// a negative zero folded onto zero -- `0.0 == -0.0` there, and a row whose
/// cash is written either way is the same fill.
#[derive(PartialEq, Eq, Hash, Clone)]
pub struct Key(String, String, String, u64, u64, u64);

impl Key {
    pub fn of(row: &ActivityRow) -> Key {
        let (d, a, s, q, p, c) = field_match_key(row, true);
        Key(d, a, s, bits(q), bits(p), bits(c))
    }
}

fn bits(v: f64) -> u64 {
    (if v == 0.0 { 0.0 } else { v }).to_bits()
}
