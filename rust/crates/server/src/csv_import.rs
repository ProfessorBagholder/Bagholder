//! A file the person imports, and a folder watched for them (`docs/plans/stage-3c-switch.md`
//! §6; `SPEC.md` §2, "Manual and imported trades"). Each row is read strictly by
//! the file adapter (`bagholder_broker::csv`), placed in the account the person
//! chose or the one the row names, its instrument named as the book knows it,
//! and kept as a record of its own. A row that is a fill a broker already
//! reported, the same account, day, instrument, side, quantity and price, with
//! exactly one such broker row, is linked to it and not counted twice; with more
//! than one, nothing is linked and the report says so. The same is looked for
//! after every pull, for rows imported before the broker's arrived.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use bagholder_book::person::{Contract, Traded, Underlying};
use bagholder_book::records::{Incoming, Outcome};
use bagholder_book::Book;
use bagholder_broker::csv::{self, CsvMapping, Payload};
use bagholder_core::instrument::InstrumentKind;
use bagholder_core::transaction::{Kind, Transaction};
use bagholder_core::{AccountId, Currency, RecordId, Rounding, SourceName};
use bagholder_engine::Engine;

use crate::entries::{contract_of, held_by_symbol, manual_account, Refused};
use crate::figures::Figures;

/// A file the page read from one the person chose.
#[derive(Clone, Debug, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportRequest {
    pub name: String,
    pub text: String,
    /// The account its rows go to; empty is the Manual account. A row naming
    /// an account of its own goes there.
    pub account: String,
}

/// A row the report names: its line in the file and what it says.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct RowNote {
    pub line: u32,
    pub message: String,
}

/// What one file did.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub file: String,
    pub layout: String,
    /// The account its rows went to, by its name.
    pub account: String,
    pub rows: u32,
    /// Rows the book did not hold before.
    pub added: u32,
    /// Rows it held already (the same row in an earlier import).
    pub unchanged: u32,
    /// Rows linked to the broker's own row for the same fill.
    pub linked: u32,
    /// Rows with more than one broker row they could be: not linked.
    pub ambiguous: Vec<RowNote>,
    /// Rows kept with a problem, counted in no figure until it is resolved.
    pub problems: Vec<RowNote>,
}

/// Read a file and keep its rows in `account` (the Manual account when `None`).
pub fn import(f: &Figures, file: &str, text: &str, account: Option<AccountId>, now: bagholder_core::jiff::Timestamp) -> Result<ImportReport, Refused> {
    let read = csv::read_file(text).map_err(Refused::Entry)?;
    let account = match account {
        Some(a) => a,
        None => manual_account(f, now)?,
    };
    let book = f.book().map_err(Refused::Failed)?;
    let fail = |e: bagholder_book::BookError| Refused::Failed(e.to_string());
    let accounts = book.accounts().map_err(fail)?;
    if !accounts.iter().any(|a| a.id == account) {
        return Err(Refused::Entry(format!("no account {account} in the book")));
    }
    // every id a broker states for an account, to place a row that names one
    let mut by_ref: BTreeMap<String, AccountId> = BTreeMap::new();
    for a in &accounts {
        for r in book.account_refs(a.id).map_err(fail)? {
            by_ref.insert(r.value, a.id);
        }
    }
    let mut report = ImportReport {
        file: file.to_string(),
        layout: read.layout.as_str().to_string(),
        account: accounts.iter().find(|a| a.id == account).and_then(|a| a.nickname.clone()).unwrap_or_else(|| account.to_string()),
        rows: read.rows.len() as u32,
        added: 0,
        unchanged: 0,
        linked: 0,
        ambiguous: vec![],
        problems: vec![],
    };
    let mut lines: BTreeMap<RecordId, u32> = BTreeMap::new();
    for row in &read.rows {
        let stated = csv::state(read.layout, &row.cells);
        let (placed, unplaced) = match stated.as_ref().ok().and_then(|s| s.account.as_deref()) {
            None => (account, None),
            Some(named) => match by_ref.get(named) {
                Some(a) => (*a, None),
                None => (account, Some(format!("the row names account {named:?}, which is none of the book's"))),
            },
        };
        let instrument = match &stated {
            Ok(s) => match (&s.instrument, s.currency) {
                (Some((symbol, kind)), Some(currency)) if unplaced.is_none() => match traded(f, symbol, *kind, currency)? {
                    Some(t) => Some(book.name(placed, &t).map_err(|e| match e {
                        bagholder_book::BookError::Refused(why) => Refused::Entry(format!("line {}: {why}", row.line)),
                        other => Refused::Failed(other.to_string()),
                    })?),
                    None => None,
                },
                _ => None,
            },
            Err(_) => None,
        };
        let r = book.account_ref(placed).map_err(fail)?;
        let payload = Payload { layout: read.layout, cells: row.cells.clone(), occurrence: row.occurrence, account: (r.broker.to_string(), r.value), unplaced, instrument };
        let text = serde_json::to_string(&payload).map_err(|e| Refused::Failed(e.to_string()))?;
        let stored = book.store(&CsvMapping, &Incoming { connection: None, source_key: &payload.key(), payload: &text, refs: vec![] }, now).map_err(fail)?;
        match stored.outcome {
            Outcome::Unchanged => report.unchanged += 1,
            Outcome::New | Outcome::Revised(_) => report.added += 1,
        }
        lines.insert(stored.record, row.line as u32);
        for p in book.problems_of(stored.record).map_err(fail)? {
            report.problems.push(RowNote { line: row.line as u32, message: p.detail });
        }
    }
    f.record_changed(now).map_err(Refused::Failed)?;
    let linking = link(f, now).map_err(Refused::Failed)?;
    for (from, _) in &linking.linked {
        if lines.contains_key(from) {
            report.linked += 1;
        }
    }
    for (record, candidates) in &linking.ambiguous {
        if let Some(line) = lines.get(record) {
            report.ambiguous.push(RowNote { line: *line, message: format!("the same fill as {candidates} of the broker's rows: not linked") });
        }
    }
    report.problems.sort_by_key(|n| n.line);
    Ok(report)
}

/// What a row's symbol trades, as the book knows it: the instrument it holds by
/// that symbol in that currency, else a contract by its terms or a security the
/// book has not met. `None` for a kind neither is (a coin the book does not
/// hold, an option whose name states no terms): the row names nothing.
fn traded(f: &Figures, symbol: &str, kind: InstrumentKind, currency: Currency) -> Result<Option<Traded>, Refused> {
    let symbol = symbol.trim().to_uppercase();
    f.read(|e| {
        if let Some(id) = held_by_symbol(e, &symbol, currency) {
            return Some(Traded::Held(id));
        }
        match (kind, contract_of(&symbol)) {
            (InstrumentKind::OptionContract, Some((under, expiry, strike, right))) => {
                let held = held_by_symbol(e, &under, currency).filter(|u| e.inputs().ledger.instruments[u].instrument.kind != InstrumentKind::OptionContract);
                let underlying = held.map(Underlying::Held).unwrap_or(Underlying::Named(under));
                Some(Traded::Named { symbol, currency, contract: Some(Contract { underlying, expiry, strike, right }) })
            }
            (InstrumentKind::Security, _) => Some(Traded::Named { symbol, currency, contract: None }),
            _ => None,
        }
    })
    .ok_or_else(|| Refused::Failed("the figures are not built yet".into()))
}

/// What a linking pass did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Linking {
    /// Each file's row, and the broker's record it gave way to.
    pub linked: Vec<(RecordId, RecordId)>,
    /// Each file's row with more than one broker record it could be, and how many.
    pub ambiguous: BTreeMap<RecordId, usize>,
}

/// Link each live row of a file to the broker's row for the same fill, where
/// there is exactly one: the same account, day, instrument, side and quantity,
/// and the price the file states (to the places it states it). A broker row that
/// has taken a file row's place takes no other's.
pub fn link(f: &Figures, now: bagholder_core::jiff::Timestamp) -> Result<Linking, String> {
    let book = f.book()?;
    let csv_rows: BTreeSet<RecordId> = book.live_records(&csv::source()).map_err(|e| e.to_string())?.into_iter().collect();
    if csv_rows.is_empty() {
        return Ok(Linking::default());
    }
    let mut not_broker: BTreeSet<RecordId> = book.live_records(&SourceName::person()).map_err(|e| e.to_string())?.into_iter().collect();
    not_broker.extend(csv_rows.iter().copied());
    let taken: BTreeSet<RecordId> = book.superseding(&csv::source()).map_err(|e| e.to_string())?.into_iter().collect();
    let found = f.read(|e| candidates(e, &csv_rows, &not_broker, &taken)).ok_or("the figures are not built yet")?;
    let mut out = Linking::default();
    let mut used = taken;
    for (row, brokers) in found {
        let free: Vec<RecordId> = brokers.into_iter().filter(|b| !used.contains(b)).collect();
        match free.as_slice() {
            [] => {}
            [one] => {
                book.supersede(&[row], &[*one], "the broker's own row for the same fill", now).map_err(|e| e.to_string())?;
                used.insert(*one);
                out.linked.push((row, *one));
            }
            more => {
                out.ambiguous.insert(row, more.len());
            }
        }
    }
    if !out.linked.is_empty() {
        f.record_changed(now)?;
    }
    Ok(out)
}

/// Each file row that is a fill, and the broker records holding a fill like it.
fn candidates(e: &Engine, csv_rows: &BTreeSet<RecordId>, not_broker: &BTreeSet<RecordId>, taken: &BTreeSet<RecordId>) -> Vec<(RecordId, Vec<RecordId>)> {
    let ledger = &e.inputs().ledger;
    let fill = |t: &Transaction| matches!(t.kind, Kind::Buy | Kind::Sell) && t.instrument.is_some() && t.quantity.is_some_and(|q| !q.is_zero());
    let price = |t: &Transaction| bagholder_engine::ledger::fill_price(t, t.instrument.and_then(|i| ledger.instruments.get(&i))).ok();
    let mut out = Vec::new();
    for c in ledger.transactions.iter().filter(|t| csv_rows.contains(&t.id.record) && fill(t)) {
        // the price as the file states it, or as its cash over its units where it states none
        let Some(p) = c.price.map(|p| p.amount).or_else(|| price(c)) else { continue };
        let mut records: Vec<RecordId> = ledger
            .transactions
            .iter()
            .filter(|b| !not_broker.contains(&b.id.record) && !taken.contains(&b.id.record) && fill(b))
            .filter(|b| b.account == c.account && b.trade_date == c.trade_date && b.instrument == c.instrument && b.kind == c.kind && b.quantity == c.quantity)
            .filter(|b| price(b).is_some_and(|bp| bp.round(p.places(), Rounding::HalfEven) == p))
            .map(|b| b.id.record)
            .collect();
        records.sort();
        records.dedup();
        if !records.is_empty() {
            out.push((c.id.record, records));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// the watched folder
// ---------------------------------------------------------------------------

const WATCH_FOLDER: &str = "watch.folder";
const WATCH_ACCOUNT: &str = "watch.account";
const WATCH_FILES: &str = "watch.files";
const WATCH_LAST: &str = "watch.last";
const WATCH_ERROR: &str = "watch.error";

/// A file the folder holds, as last read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WatchedFile {
    pub file: String,
    pub size: u64,
    pub modified: String,
    pub scanned_at: String,
    pub read: FileOutcome,
}

/// What reading a watched file did, or why it could not be read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum FileOutcome {
    Imported { report: ImportReport },
    Failed { error: String },
}

/// `GET /api/watch`'s answer.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WatchStatus {
    pub path: String,
    pub watching: bool,
    /// The account its files go to; empty is the Manual account.
    pub account: String,
    pub last_scan: String,
    /// Why the last scan of the folder failed, until one succeeds.
    pub scan_error: String,
    /// The rows the last scan's files added that the book did not hold.
    pub last_scan_added: u32,
    pub files: Vec<WatchedFile>,
}

#[derive(Clone, Debug, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WatchRequest {
    pub path: String,
    pub account: String,
}

fn setting(book: &Book, key: &str) -> Result<String, String> {
    Ok(book.setting(key).map_err(|e| e.to_string())?.unwrap_or_default())
}

/// The folder watched, its account and what each of its files did.
pub fn watch_status(f: &Figures) -> Result<WatchStatus, String> {
    let book = f.book()?;
    let path = setting(&book, WATCH_FOLDER)?;
    let files: BTreeMap<String, WatchedFile> = match book.setting(WATCH_FILES).map_err(|e| e.to_string())? {
        None => BTreeMap::new(),
        Some(text) => serde_json::from_str(&text).map_err(|e| format!("the watched files kept in the book do not read: {e}"))?,
    };
    let last_scan = setting(&book, WATCH_LAST)?;
    let last_scan_added = files
        .values()
        .filter(|f| f.scanned_at == last_scan)
        .map(|f| match &f.read {
            FileOutcome::Imported { report } => report.added,
            FileOutcome::Failed { .. } => 0,
        })
        .sum();
    Ok(WatchStatus { watching: !path.is_empty(), path, account: setting(&book, WATCH_ACCOUNT)?, last_scan, scan_error: setting(&book, WATCH_ERROR)?, last_scan_added, files: files.into_values().collect() })
}

fn home_expanded(p: &str) -> String {
    match p.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => std::env::var("HOME").map(|h| format!("{}{rest}", h.trim_end_matches('/'))).unwrap_or_else(|_| p.to_string()),
        _ => p.to_string(),
    }
}

/// Watch a folder, its files going to `account`: refused when it is not a folder.
pub fn watch(f: &Figures, req: &WatchRequest, now: bagholder_core::jiff::Timestamp) -> Result<(), Refused> {
    let path = home_expanded(req.path.trim());
    if path.is_empty() {
        return Err(Refused::Entry("a folder is required".into()));
    }
    if !std::path::Path::new(&path).is_dir() {
        return Err(Refused::Entry(format!("{path} is not a folder")));
    }
    let account = req.account.trim();
    if !account.is_empty() {
        let id = AccountId::parse(account).map_err(|_| Refused::Entry(format!("{account:?} is not an account")))?;
        f.book().map_err(Refused::Failed)?.account(id).map_err(|_| Refused::Entry(format!("no account {id} in the book")))?;
    }
    let book = f.book().map_err(Refused::Failed)?;
    let fail = |e: bagholder_book::BookError| Refused::Failed(e.to_string());
    let before = setting(&book, WATCH_FOLDER).map_err(Refused::Failed)?;
    book.set_setting(WATCH_FOLDER, Some(&path), now).map_err(fail)?;
    book.set_setting(WATCH_ACCOUNT, Some(account), now).map_err(fail)?;
    if before != path {
        book.set_setting(WATCH_FILES, None, now).map_err(fail)?;
    }
    Ok(())
}

/// Stop watching.
pub fn unwatch(f: &Figures, now: bagholder_core::jiff::Timestamp) -> Result<(), String> {
    let book = f.book()?;
    for key in [WATCH_FOLDER, WATCH_ACCOUNT, WATCH_FILES, WATCH_LAST, WATCH_ERROR] {
        book.set_setting(key, None, now).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Import every CSV at the top of the watched folder that changed since it was
/// last read (every one when `all`). A file that cannot be read says why in its
/// place; the scan itself fails only when the folder or the book cannot be read.
pub fn scan(f: &Figures, all: bool, now: bagholder_core::jiff::Timestamp) -> Result<WatchStatus, String> {
    let scanned = scan_folder(f, all, now);
    let book = f.book()?;
    book.set_setting(WATCH_ERROR, scanned.as_ref().err().map(String::as_str), now).map_err(|e| e.to_string())?;
    scanned?;
    watch_status(f)
}

fn scan_folder(f: &Figures, all: bool, now: bagholder_core::jiff::Timestamp) -> Result<(), String> {
    let status = watch_status(f)?;
    if !status.watching {
        return Err("no folder is watched".into());
    }
    let account = match status.account.as_str() {
        "" => None,
        a => Some(AccountId::parse(a).map_err(|e| format!("the watched folder's account: {e}"))?),
    };
    let mut kept: BTreeMap<String, WatchedFile> = status.files.into_iter().map(|w| (w.file.clone(), w)).collect();
    let entries = std::fs::read_dir(&status.path).map_err(|e| format!("{}: {e}", status.path))?;
    let mut seen = BTreeSet::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", status.path))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                kept.insert(name.clone(), WatchedFile { file: name.clone(), size: 0, modified: String::new(), scanned_at: now.to_string(), read: FileOutcome::Failed { error: e.to_string() } });
                seen.insert(name);
                continue;
            }
        };
        if !meta.is_file() || !name.to_lowercase().ends_with(".csv") || name.starts_with("._") {
            continue;
        }
        seen.insert(name.clone());
        let modified = meta.modified().ok().and_then(|t| bagholder_core::jiff::Timestamp::try_from(t).ok()).map(|t| t.to_string()).unwrap_or_default();
        if !all && kept.get(&name).is_some_and(|w| w.size == meta.len() && w.modified == modified) {
            continue;
        }
        let read = match std::fs::read(entry.path()) {
            Err(e) => FileOutcome::Failed { error: e.to_string() },
            Ok(bytes) => match String::from_utf8(bytes) {
                Err(_) => FileOutcome::Failed { error: "the file is not UTF-8 text".into() },
                Ok(text) => match import(f, &name, &text, account, now) {
                    Ok(report) => FileOutcome::Imported { report },
                    Err(Refused::Entry(error)) => FileOutcome::Failed { error },
                    Err(Refused::Failed(why)) => return Err(why),
                },
            },
        };
        kept.insert(name.clone(), WatchedFile { file: name, size: meta.len(), modified, scanned_at: now.to_string(), read });
    }
    kept.retain(|name, _| seen.contains(name));
    let book = f.book()?;
    let text = serde_json::to_string(&kept).map_err(|e| e.to_string())?;
    book.set_setting(WATCH_FILES, Some(&text), now).map_err(|e| e.to_string())?;
    book.set_setting(WATCH_LAST, Some(&now.to_string()), now).map_err(|e| e.to_string())?;
    Ok(())
}

/// Whether a folder is watched.
pub fn watching(f: &Figures) -> bool {
    f.book().ok().and_then(|b| b.setting(WATCH_FOLDER).ok().flatten()).is_some_and(|p| !p.is_empty())
}

/// The folder an earlier version watched, watched again, once: the book has
/// never had a folder set and the old store names one.
pub fn adopt(f: &Figures, old_folder: &str, now: bagholder_core::jiff::Timestamp) -> Result<(), String> {
    let book = f.book()?;
    if old_folder.is_empty() || book.setting(WATCH_FOLDER).map_err(|e| e.to_string())?.is_some() {
        return Ok(());
    }
    match watch(f, &WatchRequest { path: old_folder.to_string(), account: String::new() }, now) {
        Ok(()) => Ok(()),
        // the folder is gone: kept as the one watched, so its failure is said
        Err(Refused::Entry(why)) => {
            book.set_setting(WATCH_FOLDER, Some(old_folder), now).map_err(|e| e.to_string())?;
            book.set_setting(WATCH_ERROR, Some(&why), now).map_err(|e| e.to_string())
        }
        Err(Refused::Failed(why)) => Err(why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bagholder_book::import::{ImportMapping, ImportedRow, OldActivity};
    use bagholder_core::record::RecordState;
    use bagholder_core::Dec;

    fn figures() -> (tempfile::TempDir, Figures) {
        let home = tempfile::tempdir().unwrap();
        crate::tests_common::pulled_book(home.path());
        let now: bagholder_core::jiff::Timestamp = "2025-11-19T21:00:00Z".parse().unwrap();
        let f = Figures::open(home.path(), now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        (home, f)
    }

    fn now() -> bagholder_core::jiff::Timestamp {
        "2025-11-19T21:30:00Z".parse().unwrap()
    }

    /// A fill of a share the broker reported: its account and the broker's id
    /// for it, its day, symbol, currency, units and price a unit.
    struct Fill {
        account: AccountId,
        broker_account: String,
        day: String,
        symbol: String,
        currency: String,
        units: Dec,
        price: Dec,
    }

    fn broker_fill(f: &Figures) -> Fill {
        let (account, day, symbol, currency, units, price) = f
            .read(|e| {
                let l = &e.inputs().ledger;
                l.transactions
                    .iter()
                    .filter(|t| t.kind == Kind::Buy)
                    .find_map(|t| {
                        let info = l.instruments.get(&t.instrument?)?;
                        if info.instrument.kind != InstrumentKind::Security {
                            return None;
                        }
                        let price = bagholder_engine::ledger::fill_price(t, Some(info)).ok()?;
                        Some((t.account, t.trade_date.to_string(), info.current_name()?.symbol.clone(), info.instrument.currency.as_str().to_string(), t.quantity?, price))
                    })
                    .unwrap()
            })
            .unwrap();
        let broker_account = f.book().unwrap().account_ref(account).unwrap().value;
        Fill { account, broker_account, day, symbol, currency, units, price }
    }

    fn activities(rows: &[&Fill]) -> String {
        let mut out = String::from("transaction_date,activity_type,activity_sub_type,account_id,symbol,currency,quantity,unit_price,net_cash_amount\n");
        for x in rows {
            out.push_str(&format!("{},Trade,BUY,{},{},{},{},{},-1\n", x.day, x.broker_account, x.symbol, x.currency, x.units.to_text(), x.price.to_text()));
        }
        out
    }

    fn csv_records(f: &Figures) -> Vec<RecordId> {
        f.book().unwrap().live_records(&csv::source()).unwrap()
    }

    fn units_held(f: &Figures, account: AccountId, symbol: &str) -> Dec {
        f.read(|e| {
            let l = &e.inputs().ledger;
            l.transactions
                .iter()
                .filter(|t| t.account == account && t.instrument.and_then(|i| l.instruments[&i].current_name()).is_some_and(|n| n.symbol == symbol))
                .filter_map(|t| t.quantity)
                .fold(Dec::ZERO, |a, q| a.checked_add(q).unwrap())
        })
        .unwrap()
    }

    #[test]
    fn a_row_the_broker_already_reported_is_linked_and_not_counted_twice() {
        let (_h, f) = figures();
        let fill = broker_fill(&f);
        let before = units_held(&f, fill.account, &fill.symbol);
        let r = import(&f, "activity.csv", &activities(&[&fill]), None, now()).unwrap();
        assert_eq!((r.rows, r.added, r.linked, r.ambiguous.len()), (1, 1, 1, 0), "{r:?}");
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert!(csv_records(&f).is_empty(), "the row gave way to the broker's");
        assert_eq!(units_held(&f, fill.account, &fill.symbol), before);
        // the same file again: nothing new, nothing counted
        let again = import(&f, "activity (1).csv", &activities(&[&fill]), None, now()).unwrap();
        assert_eq!((again.added, again.unchanged, again.linked), (0, 1, 0), "{again:?}");
        assert_eq!(units_held(&f, fill.account, &fill.symbol), before);
    }

    #[test]
    fn a_broker_row_takes_the_place_of_one_file_row_only() {
        let (_h, f) = figures();
        let fill = broker_fill(&f);
        let before = units_held(&f, fill.account, &fill.symbol);
        let r = import(&f, "activity.csv", &activities(&[&fill, &fill]), None, now()).unwrap();
        assert_eq!((r.rows, r.added, r.linked), (2, 2, 1), "{r:?}");
        assert_eq!(csv_records(&f).len(), 1);
        assert_eq!(units_held(&f, fill.account, &fill.symbol), before.checked_add(fill.units).unwrap(), "the second fill the file states is its own");
    }

    /// A row of an earlier database's import, a fill of `symbol` in the account:
    /// a record from a source other than the file, standing for the broker's.
    fn imported_fill(f: &Figures, key: &str, account: &str, symbol: &str) -> RecordId {
        let book = f.book().unwrap();
        let conn = book.connections().unwrap()[0].id;
        let row = OldActivity {
            id: key.into(),
            transaction_date: Some("2025-11-03".into()),
            account_id: Some(account.into()),
            activity_type: Some("Trade".into()),
            activity_sub_type: Some("BUY".into()),
            symbol: Some(symbol.into()),
            currency: Some("USD".into()),
            quantity: Some("5".into()),
            unit_price: Some("20".into()),
            net_cash_amount: Some("-100".into()),
            source: Some("csv".into()),
            ..OldActivity::default()
        };
        let payload = serde_json::to_string(&ImportedRow { row, security: None, underlying: None }).unwrap();
        let stored = book.store(&ImportMapping, &Incoming { connection: Some(conn), source_key: key, payload: &payload, refs: vec![] }, now()).unwrap();
        f.record_changed(now()).unwrap();
        stored.record
    }

    fn simple_row(account: &str) -> String {
        format!("Date,Action,Symbol,Quantity,Price,Amount,Currency,Account\n2025-11-03,Buy,ZZQQ,5,20.00,100,USD,{account}\n")
    }

    #[test]
    fn a_row_with_more_than_one_broker_row_it_could_be_links_nothing_and_says_so() {
        let (_h, f) = figures();
        let account = broker_fill(&f).broker_account;
        imported_fill(&f, "a", &account, "ZZQQ");
        imported_fill(&f, "b", &account, "ZZQQ");
        let r = import(&f, "mine.csv", &simple_row(&account), None, now()).unwrap();
        assert_eq!((r.added, r.linked), (1, 0), "{r:?}");
        assert_eq!(r.ambiguous, vec![RowNote { line: 2, message: "the same fill as 2 of the broker's rows: not linked".into() }]);
        assert_eq!(csv_records(&f).len(), 1, "kept, and counted");
    }

    #[test]
    fn a_row_imported_before_the_broker_reported_it_is_linked_when_it_arrives() {
        let (_h, f) = figures();
        let account = broker_fill(&f).broker_account;
        let r = import(&f, "mine.csv", &simple_row(&account), None, now()).unwrap();
        assert_eq!((r.added, r.linked), (1, 0));
        let row = csv_records(&f)[0];
        let broker = imported_fill(&f, "a", &account, "ZZQQ");
        let linking = link(&f, now()).unwrap();
        assert_eq!(linking.linked, vec![(row, broker)]);
        assert_eq!(f.book().unwrap().record(row).unwrap().state, RecordState::Superseded);
    }

    #[test]
    fn a_row_naming_an_account_the_book_does_not_hold_is_kept_and_counts_in_nothing() {
        let (_h, f) = figures();
        let r = import(&f, "mine.csv", &simple_row("nobody-123"), None, now()).unwrap();
        assert_eq!(r.added, 1);
        assert_eq!(r.problems.len(), 1);
        assert!(r.problems[0].message.contains("nobody-123"), "{:?}", r.problems);
        let row = csv_records(&f)[0];
        let cash = f.read(|e| e.inputs().ledger.transactions.iter().filter(|t| t.id.record == row).map(|t| (t.kind, t.cash)).collect::<Vec<_>>()).unwrap();
        assert_eq!(cash, vec![(Kind::Unclassified, None)]);
    }

    #[test]
    fn a_file_no_layout_reads_is_refused_with_why() {
        let (_h, f) = figures();
        match import(&f, "x.csv", "foo,bar\n1,2\n", None, now()) {
            Err(Refused::Entry(why)) => assert!(why.starts_with("its headers (foo, bar) are none of the layouts read"), "{why}"),
            other => panic!("{other:?}"),
        }
        assert!(csv_records(&f).is_empty());
    }

    #[test]
    fn a_watched_folder_reads_each_file_when_it_changes_and_says_what_failed() {
        let (_h, f) = figures();
        let dir = tempfile::tempdir().unwrap();
        let account = broker_fill(&f).broker_account;
        std::fs::write(dir.path().join("a.csv"), simple_row(&account)).unwrap();
        std::fs::write(dir.path().join("b.CSV"), [0xffu8, 0xfe, 0x00]).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "not a csv").unwrap();
        std::fs::write(dir.path().join("._a.csv"), "a Mac's resource fork").unwrap();
        assert!(matches!(watch(&f, &WatchRequest { path: dir.path().join("nope").to_string_lossy().into(), account: String::new() }, now()), Err(Refused::Entry(_))));
        watch(&f, &WatchRequest { path: dir.path().to_string_lossy().into(), account: String::new() }, now()).unwrap();
        let s = scan(&f, false, now()).unwrap();
        assert_eq!(s.files.iter().map(|w| w.file.as_str()).collect::<Vec<_>>(), vec!["a.csv", "b.CSV"]);
        assert!(matches!(&s.files[0].read, FileOutcome::Imported { report } if report.added == 1));
        assert!(matches!(&s.files[1].read, FileOutcome::Failed { error } if error == "the file is not UTF-8 text"));
        // the scan says what it added over every file it read, a file that failed adding none
        assert_eq!(s.last_scan_added, 1);
        // unchanged: not read again, and a scan that read nothing added nothing
        let later: bagholder_core::jiff::Timestamp = "2025-11-19T22:00:00Z".parse().unwrap();
        let s = scan(&f, false, later).unwrap();
        assert_eq!(s.files[0].scanned_at, now().to_string());
        assert_eq!(s.last_scan_added, 0, "the earlier scan's rows are not this one's");
        // every file read again when asked; the row is the same record
        let s = scan(&f, true, later).unwrap();
        assert!(matches!(&s.files[0].read, FileOutcome::Imported { report } if report.unchanged == 1 && report.added == 0));
        assert_eq!(s.last_scan_added, 0, "rows the book already held are not counted");
        // the folder gone: the scan fails, and the status says why until one succeeds
        drop(dir);
        assert!(scan(&f, false, later).is_err());
        assert!(!watch_status(&f).unwrap().scan_error.is_empty());
        unwatch(&f, later).unwrap();
        let s = watch_status(&f).unwrap();
        assert!(!s.watching && s.scan_error.is_empty() && s.files.is_empty());
    }
}
