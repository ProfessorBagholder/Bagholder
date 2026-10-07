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

/// A row the report names: its line in the file and what it says.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS, bagholder_diff_derive::Diff)]
pub struct RowNote {
    pub line: u32,
    pub message: String,
}

/// What one file did.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS, bagholder_diff_derive::Diff)]
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
    /// Rows with more than one broker row they could be: not linked. The first
    /// `REPORT_NOTES` by line, and how many there are.
    pub ambiguous: Vec<RowNote>,
    pub ambiguous_rows: u32,
    /// Rows kept with a problem, counted in no figure until it is resolved: the first
    /// `REPORT_NOTES` by line, and how many there are. Every one is in the book, and
    /// the header says them (`status::unread_rows`).
    pub problems: Vec<RowNote>,
    pub problem_rows: u32,
    /// The person stopped it after `rows` of the file's rows: those are kept, and
    /// importing the file again goes on from them (a row kept already is unchanged).
    #[serde(default)]
    pub stopped: bool,
}

/// The import running now, as the import window shows it: the file, how much of it
/// has arrived of the size its request stated, and how many rows have been read.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase")]
pub struct Importing {
    /// The import's own id, the one `POST /api/import` answered with.
    pub id: String,
    pub file: String,
    /// Bytes received of the file, and its size where the request states it.
    pub received: u64,
    pub size: Option<u64>,
    /// Bytes of the file read through once it has arrived, to check every line
    /// reads and to count its rows.
    pub checked: u64,
    /// Rows kept so far, of the file's rows once they are counted.
    pub rows: u64,
    pub total: Option<u64>,
}

/// An import once it has ended, as the status says it until the next one ends:
/// its id, its file, and what it did or why nothing of it was kept. It is the
/// import's answer, carried apart from the request that sent the file, so it
/// reaches the page whatever happened to that request.
#[derive(Clone, Debug, Default, PartialEq, Serialize, TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    pub id: String,
    pub file: String,
    pub report: Option<ImportReport>,
    pub error: Option<String>,
}

/// What `POST /api/import` answers once the file has arrived whole: the import it
/// started, which runs on as a job (`Importing`, then `Imported`).
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ImportAccepted {
    pub id: String,
}

/// How far an import has come, as it tells the one it reports to.
#[derive(Clone, Copy)]
pub enum Step {
    /// The bytes read through so far, checking every line reads and counting rows.
    Checked(u64),
    /// The rows kept so far, of the rows the file holds.
    Kept { rows: u64, total: u64 },
}

/// What an import is told by the one it reports to: go on, or stop.
pub enum Go {
    On,
    Stop,
}

/// The rows a report lists by line, of each kind; the rest are counted. The answer
/// to a file of any size stays the size of a page's list.
pub const REPORT_NOTES: usize = 8;

/// What an import is told when the person stops it: nothing it read is kept.
pub const IMPORT_STOPPED: &str = "Stopped: nothing from this file was kept.";

/// Imports run one at a time in the process, from the page and from the watched
/// folder alike: one whose request was given up on still finishes before the next.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Read a file held in memory and keep its rows in `account` (the Manual account when `None`).
#[cfg(test)]
pub fn import(f: &Figures, file: &str, text: &str, account: Option<AccountId>, now: bagholder_core::jiff::Timestamp) -> Result<ImportReport, Refused> {
    import_from(f, file, &|| Ok(Box::new(std::io::Cursor::new(text.as_bytes().to_vec())) as Box<dyn std::io::BufRead>), account, now, &mut |_| Go::On)
}

/// Read a file from `open` and keep its rows in `account` (the Manual account when
/// `None`), in bounded memory whatever its size: read once to check every line reads
/// and to count its rows (a file that does not read keeps nothing), then again to
/// keep each row in turn. `progress` is told the bytes checked after each row of the
/// first reading, and the rows kept of the rows counted after each of the second; a
/// `Go::Stop` from it ends the import there: stopped while checking, it keeps
/// nothing; while keeping, the rows kept stay.
pub fn import_from(
    f: &Figures,
    file: &str,
    open: &dyn Fn() -> std::io::Result<Box<dyn std::io::BufRead>>,
    account: Option<AccountId>,
    now: bagholder_core::jiff::Timestamp,
    progress: &mut dyn FnMut(Step) -> Go,
) -> Result<ImportReport, Refused> {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let unread = |e: std::io::Error| Refused::Failed(format!("{file} could not be read: {e}"));
    let mut total: u64 = 0;
    let mut check = csv::read_rows(open().map_err(unread)?).map_err(Refused::Entry)?;
    while let Some(row) = check.next() {
        row.map_err(Refused::Entry)?;
        total += 1;
        if let Go::Stop = progress(Step::Checked(check.bytes_read())) {
            return Err(Refused::Entry(IMPORT_STOPPED.into()));
        }
    }
    let mut read = csv::read_rows(open().map_err(unread)?).map_err(Refused::Entry)?;
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
        rows: 0,
        added: 0,
        unchanged: 0,
        linked: 0,
        ambiguous: vec![],
        ambiguous_rows: 0,
        problems: vec![],
        problem_rows: 0,
        stopped: false,
    };
    let layout = read.layout;
    let mut lines: BTreeMap<RecordId, u32> = BTreeMap::new();
    // kept a group at a time, each group one transaction ending where the whole
    // percent of the file's rows moves (or at the end, or a stop), so a file of any
    // size costs a hundred commits at most; a failure inside a group keeps none of
    // that group, and every group before it stays
    let mut failed: Option<Refused> = None;
    loop {
        let more = book.atomically(|| {
            let keep = |row: Result<csv::RowRead, String>| -> Result<(bagholder_book::records::Stored, u32, Vec<RowNote>), Refused> {
                // the file changed between the two readings: the groups kept before stay, and why is said
                let row = row.map_err(|e| Refused::Failed(format!("{file} changed while it was imported: {e}")))?;
                let row = &row;
                let stated = csv::state(layout, &row.cells);
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
                let payload = Payload { layout, cells: row.cells.clone(), occurrence: row.occurrence, account: (r.broker.to_string(), r.value), unplaced, instrument };
                let text = serde_json::to_string(&payload).map_err(|e| Refused::Failed(e.to_string()))?;
                let stored = book.store(&CsvMapping, &Incoming { connection: None, source_key: &payload.key(), payload: &text, refs: vec![] }, now).map_err(fail)?;
                let problems = book.problems_of(stored.record).map_err(fail)?.into_iter().map(|p| RowNote { line: row.line as u32, message: p.detail }).collect();
                Ok((stored, row.line as u32, problems))
            };
            while let Some(row) = read.next() {
                let (stored, line, problems) = match keep(row) {
                    Ok(kept) => kept,
                    Err(e) => {
                        failed = Some(e);
                        return Err(bagholder_book::BookError::Refused("the group is not kept".into()));
                    }
                };
                match stored.outcome {
                    Outcome::Unchanged => report.unchanged += 1,
                    Outcome::New | Outcome::Revised(_) => report.added += 1,
                }
                lines.insert(stored.record, line);
                report.problem_rows += problems.len() as u32;
                let room = REPORT_NOTES.saturating_sub(report.problems.len());
                report.problems.extend(problems.into_iter().take(room));
                report.rows += 1;
                let rows = report.rows as u64;
                if let Go::Stop = progress(Step::Kept { rows, total }) {
                    report.stopped = rows < total;
                    return Ok(false);
                }
                if rows * 100 / total.max(1) != (rows - 1) * 100 / total.max(1) {
                    return Ok(true);
                }
            }
            Ok(false)
        });
        match (more, failed.take()) {
            (_, Some(e)) => return Err(e),
            (Err(e), None) => return Err(fail(e)),
            (Ok(true), None) => {}
            (Ok(false), None) => break,
        }
    }
    f.record_changed(now).map_err(Refused::Failed)?;
    let linking = link(f, now).map_err(Refused::Failed)?;
    for (from, _) in &linking.linked {
        if lines.contains_key(from) {
            report.linked += 1;
        }
    }
    let mut ambiguous: Vec<(u32, usize)> = linking.ambiguous.iter().filter_map(|(record, candidates)| lines.get(record).map(|line| (*line, *candidates))).collect();
    ambiguous.sort();
    report.ambiguous_rows = ambiguous.len() as u32;
    report.ambiguous = ambiguous.into_iter().take(REPORT_NOTES).map(|(line, candidates)| RowNote { line, message: format!("the same fill as {candidates} of the broker's rows: not linked") }).collect();
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

/// Link each live row of a file to the broker's row for the same movement,
/// where there is exactly one: for a fill, the same account, day, instrument,
/// side and quantity, and the price the file states (to the places it states
/// it); for cash moved, the same account, currency and exact signed amount, on
/// the same day. The broker's activity feed comes first, then a row booked from
/// its statement (`docs/plans/statement-gaps.md`): a row of the feed takes the
/// file row's place where one fits, else a statement row. A broker row that has
/// taken a file row's place takes no other's.
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
    for (row, tiers) in found {
        // the first tier with a free candidate: the feed's, then the statement's
        let free: Vec<RecordId> = tiers.into_iter().map(|t| t.into_iter().filter(|b| !used.contains(b)).collect::<Vec<_>>()).find(|t| !t.is_empty()).unwrap_or_default();
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

/// Each file row that is a fill or moved cash, and the broker records holding
/// the same movement, the feed's before the statement's.
fn candidates(e: &Engine, csv_rows: &BTreeSet<RecordId>, not_broker: &BTreeSet<RecordId>, taken: &BTreeSet<RecordId>) -> Vec<(RecordId, Vec<Vec<RecordId>>)> {
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
            out.push((c.id.record, vec![records]));
        }
    }
    let statement = bagholder_wealthsimple::statement::source();
    // every other movement on an instrument (a dividend, a distribution, an
    // interest payment on a holding, units in or out): the same account, kind,
    // instrument, units and cash, on its trade or settle day, the feed's before
    // the statement's (`docs/plans/stage-money.md`, part F: by movement identity)
    let movement = |t: &Transaction| t.instrument.is_some() && !fill(t) && t.kind != Kind::Unclassified && (t.cash.is_some_and(|c| !c.amount.is_zero()) || t.quantity.is_some_and(|q| !q.is_zero()));
    for c in ledger.transactions.iter().filter(|t| csv_rows.contains(&t.id.record) && movement(t)) {
        let same = |b: &&Transaction| {
            !not_broker.contains(&b.id.record)
                && !taken.contains(&b.id.record)
                && movement(b)
                && b.account == c.account
                && b.kind == c.kind
                && b.instrument == c.instrument
                && b.quantity == c.quantity
                && b.cash == c.cash
                && (b.trade_date == c.trade_date || Some(b.trade_date) == c.settle_date || b.settle_date == Some(c.trade_date))
        };
        let tier = |from_statement: bool| {
            let mut r: Vec<RecordId> = ledger.transactions.iter().filter(same).filter(|b| (b.mapping.source == statement) == from_statement).map(|b| b.id.record).collect();
            r.sort();
            r.dedup();
            r
        };
        let tiers = vec![tier(false), tier(true)];
        if tiers.iter().any(|t| !t.is_empty()) {
            out.push((c.id.record, tiers));
        }
    }
    // cash moved with no instrument: a deposit, a withdrawal, a transfer, a tax
    let moved = |t: &Transaction| t.instrument.is_none() && t.cash.is_some_and(|c| !c.amount.is_zero());
    for c in ledger.transactions.iter().filter(|t| csv_rows.contains(&t.id.record) && moved(t)) {
        let same = |b: &&Transaction| !not_broker.contains(&b.id.record) && !taken.contains(&b.id.record) && moved(b) && b.account == c.account && b.cash == c.cash && (b.trade_date == c.trade_date || Some(b.trade_date) == c.settle_date);
        let tier = |from_statement: bool| {
            let mut r: Vec<RecordId> = ledger.transactions.iter().filter(same).filter(|b| (b.mapping.source == statement) == from_statement).map(|b| b.id.record).collect();
            r.sort();
            r.dedup();
            r
        };
        let tiers = vec![tier(false), tier(true)];
        if tiers.iter().any(|t| !t.is_empty()) {
            out.push((c.id.record, tiers));
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
        // read from the file as it is, a line at a time, whatever its size
        let path = entry.path();
        let open = || std::fs::File::open(&path).map(|f| Box::new(std::io::BufReader::new(f)) as Box<dyn std::io::BufRead>);
        let read = match import_from(f, &name, &open, account, now, &mut |_| Go::On) {
            Ok(report) => FileOutcome::Imported { report },
            Err(Refused::Entry(error)) => FileOutcome::Failed { error },
            Err(Refused::Failed(why)) => return Err(why),
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

    /// A movement of cash alone the broker reported: a deposit, as a row of an
    /// earlier import stands for the broker's own. Its account, record, day and cash.
    fn feed_cash(f: &Figures) -> (AccountId, RecordId, bagholder_core::jiff::civil::Date, bagholder_core::Money) {
        let book = f.book().unwrap();
        let conn = book.connections().unwrap()[0].id;
        let row = OldActivity {
            id: "deposit-1".into(),
            transaction_date: Some("2025-11-04".into()),
            account_id: Some("anon-tfsa-1".into()),
            activity_type: Some("Deposit".into()),
            currency: Some("CAD".into()),
            net_cash_amount: Some("250".into()),
            source: Some("csv".into()),
            ..OldActivity::default()
        };
        let payload = serde_json::to_string(&ImportedRow { row, security: None, underlying: None }).unwrap();
        let record = book.store(&ImportMapping, &Incoming { connection: Some(conn), source_key: "deposit-1", payload: &payload, refs: vec![] }, now()).unwrap().record;
        f.record_changed(now()).unwrap();
        let account = book.account_by_ref(&bagholder_core::account::AccountRef::new(bagholder_core::Broker::named("wealthsimple"), "anon-tfsa-1")).unwrap().unwrap();
        (account, record, "2025-11-04".parse().unwrap(), bagholder_core::Money::new(Dec::parse("250").unwrap(), bagholder_core::Currency::CAD))
    }

    /// A row booked from the broker's statement for this movement, `n` among the month's.
    fn statement_row(f: &Figures, day: bagholder_core::jiff::civil::Date, cash: bagholder_core::Money, n: usize) -> RecordId {
        let book = f.book().unwrap();
        let conn = book.connections().unwrap()[0].id;
        let row = bagholder_broker::StatementRow { day, executed: None, code: "CONT".into(), description: "Contribution".into(), currency: cash.currency, cash: cash.amount, balance: cash.amount };
        let month = day.first_of_month();
        let key = bagholder_wealthsimple::statement::key("anon-tfsa-1", month, n);
        let payload = bagholder_wealthsimple::statement::payload("anon-tfsa-1", month, n, &row).canonical();
        let r = book.store(&bagholder_wealthsimple::statement::StatementMapping, &bagholder_book::records::Incoming { connection: Some(conn), source_key: &key, payload: &payload, refs: vec![] }, now()).unwrap().record;
        f.record_changed(now()).unwrap();
        r
    }

    fn cash_file(day: bagholder_core::jiff::civil::Date, cash: bagholder_core::Money) -> String {
        let action = if cash.amount.is_negative() { "withdrawal" } else { "deposit" };
        format!("Date,Action,Symbol,Quantity,Price,Amount,Currency\n{day},{action},,,,{},{}\n", cash.amount.to_text(), cash.currency)
    }

    #[test]
    fn a_file_s_cash_row_gives_way_to_the_feed_s_row_before_a_statement_s() {
        let (_h, f) = figures();
        let (account, feed, day, cash) = feed_cash(&f);
        let from_statement = statement_row(&f, day, cash, 0);
        let r = import(&f, "statement.csv", &cash_file(day, cash), Some(account), now()).unwrap();
        assert_eq!((r.added, r.linked), (1, 1), "{r:?}");
        let file_row = f.book().unwrap().superseding(&csv::source()).unwrap();
        assert_eq!(file_row, vec![feed], "the feed's row took its place, not the statement's ({from_statement})");
    }

    #[test]
    fn a_file_s_cash_row_only_a_statement_row_holds_gives_way_to_it_and_two_hold_it_links_nothing() {
        let (_h, f) = figures();
        let (account, _, day, _) = feed_cash(&f);
        let cash = bagholder_core::Money::new(Dec::parse("12.34").unwrap(), bagholder_core::Currency::CAD);
        let from_statement = statement_row(&f, day, cash, 0);
        let r = import(&f, "a.csv", &cash_file(day, cash), Some(account), now()).unwrap();
        assert_eq!(r.linked, 1, "{r:?}");
        assert_eq!(f.book().unwrap().superseding(&csv::source()).unwrap(), vec![from_statement]);
        // two statement rows it could be: it gives way to neither and says so
        let other = bagholder_core::Money::new(Dec::parse("56.78").unwrap(), bagholder_core::Currency::CAD);
        statement_row(&f, day, other, 1);
        statement_row(&f, day, other, 2);
        let r = import(&f, "b.csv", &cash_file(day, other), Some(account), now()).unwrap();
        assert_eq!((r.linked, r.ambiguous.len()), (0, 1), "{r:?}");
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

    #[test]
    fn a_dividend_the_broker_already_reported_is_linked_and_counted_once() {
        let (_h, f) = figures();
        let (account, day, symbol, currency, cash) = f
            .read(|e| {
                let l = &e.inputs().ledger;
                l.transactions.iter().filter(|t| t.kind == Kind::Dividend).find_map(|t| {
                    let n = l.instruments.get(&t.instrument?)?.current_name()?;
                    Some((t.account, t.trade_date, n.symbol.clone(), t.cash?.currency, t.cash?.amount))
                })
            })
            .unwrap()
            .expect("a dividend in the pulled book");
        let broker_account = f.book().unwrap().account_ref(account).unwrap().value;
        let count = |f: &Figures| f.read(|e| e.inputs().ledger.transactions.iter().filter(|t| t.kind == Kind::Dividend && t.account == account && t.trade_date == day).count()).unwrap();
        let before = count(&f);
        let file = format!("Date,Action,Symbol,Quantity,Price,Amount,Currency,Account\n{day},Dividend,{symbol},,,{},{},{broker_account}\n", cash.to_text(), currency.as_str());
        let r = import(&f, "dividends.csv", &file, None, now()).unwrap();
        assert_eq!((r.rows, r.linked, r.ambiguous.len()), (1, 1, 0), "{r:?}");
        assert_eq!(count(&f), before, "the dividend counts once: the broker's");
        let again = import(&f, "dividends (1).csv", &file, None, now()).unwrap();
        assert_eq!(count(&f), before, "a second import of it counts nothing more: {again:?}");
    }

    #[test]
    fn an_import_stopped_part_way_keeps_the_rows_read_and_goes_on_when_imported_again() {
        let (_h, f) = figures();
        let mut file = String::from("Date,Action,Symbol,Quantity,Price,Amount,Currency\n");
        for n in 1..=5 {
            file.push_str(&format!("2025-11-0{n},Buy,ZZPART,{n},1.00,-{n},USD\n"));
        }
        let open = || Ok(Box::new(std::io::Cursor::new(file.clone().into_bytes())) as Box<dyn std::io::BufRead>);
        let mut told = vec![];
        let r = import_from(&f, "part.csv", &open, None, now(), &mut |step| match step {
            Step::Checked(_) => Go::On,
            Step::Kept { rows, total } => {
                told.push((rows, total));
                if rows == 2 { Go::Stop } else { Go::On }
            }
        })
        .unwrap();
        assert_eq!((r.rows, r.added, r.stopped), (2, 2, true), "{r:?}");
        assert_eq!(told, vec![(1, 5), (2, 5)], "told each row of the rows counted");
        let again = import_from(&f, "part.csv", &open, None, now(), &mut |_| Go::On).unwrap();
        assert_eq!((again.rows, again.added, again.unchanged, again.stopped), (5, 3, 2, false), "{again:?}");
    }

    #[test]
    fn an_import_stopped_while_its_file_is_checked_keeps_nothing() {
        let (_h, f) = figures();
        let file = "Date,Action,Symbol,Quantity,Price,Amount,Currency\n2025-11-03,Buy,ZZCHK,1,1.00,-1,USD\n2025-11-04,Buy,ZZCHK,2,1.00,-2,USD\n";
        let open = || Ok(Box::new(std::io::Cursor::new(file.as_bytes().to_vec())) as Box<dyn std::io::BufRead>);
        let mut checked = vec![];
        let r = import_from(&f, "chk.csv", &open, None, now(), &mut |step| match step {
            Step::Checked(bytes) => {
                checked.push(bytes);
                Go::Stop
            }
            Step::Kept { .. } => Go::On,
        });
        assert!(matches!(&r, Err(Refused::Entry(why)) if why == IMPORT_STOPPED), "{r:?}");
        // told the bytes read through its first row, the header's included
        assert_eq!(checked, vec![(file.find("\n2025-11-04").unwrap() + 1) as u64]);
        assert!(csv_records(&f).is_empty());
    }

    #[test]
    fn rows_are_kept_a_whole_percent_at_a_time_and_a_failure_keeps_every_group_before_it() {
        let (_h, f) = figures();
        let rows = |broken: Option<usize>| {
            let mut file = String::from("Date,Action,Symbol,Quantity,Price,Amount,Currency\n");
            for n in 1..=200 {
                let symbol = if broken == Some(n) { "\"ZZ\"GROUP".to_string() } else { "ZZGROUP".to_string() };
                file.push_str(&format!("2025-11-03,Buy,{symbol},{n},1.00,-{n},USD\n"));
            }
            file
        };
        // read whole the first time, changed at row 150 by the second reading
        let readings = std::cell::Cell::new(0);
        let open = || {
            readings.set(readings.get() + 1);
            let text = rows(if readings.get() == 1 { None } else { Some(150) });
            Ok(Box::new(std::io::Cursor::new(text.into_bytes())) as Box<dyn std::io::BufRead>)
        };
        match import_from(&f, "group.csv", &open, None, now(), &mut |_| Go::On) {
            Err(Refused::Failed(why)) => assert!(why.starts_with("group.csv changed while it was imported"), "{why}"),
            other => panic!("{other:?}"),
        }
        // a group is one percent of the file's 200 rows: the one holding rows 149
        // and 150 is not kept, every one before it is
        assert_eq!(csv_records(&f).len(), 148);
    }

    #[test]
    fn the_report_lists_the_first_rows_with_a_problem_by_line_and_counts_them_all() {
        let (_h, f) = figures();
        // no currency stated: each row is kept with that problem
        let mut file = String::from("Date,Action,Symbol,Quantity,Price,Amount\n");
        for n in 1..=20 {
            file.push_str(&format!("2025-11-03,Sell,ZZNOTE,{n},1.00,{n}\n"));
        }
        let r = import(&f, "notes.csv", &file, None, now()).unwrap();
        assert_eq!((r.rows, r.problem_rows), (20, 20), "{r:?}");
        assert_eq!(r.problems.iter().map(|n| n.line).collect::<Vec<_>>(), (2..2 + REPORT_NOTES as u32).collect::<Vec<_>>());
        assert_eq!(csv_records(&f).len(), 20, "every row is kept");
    }

    #[test]
    fn a_file_with_a_line_that_does_not_read_keeps_nothing() {
        let (_h, f) = figures();
        let file = "Date,Action,Symbol,Quantity,Price,Amount,Currency\n2025-11-03,Buy,ZZBAD,1,1.00,-1,USD\n2025-11-04,Buy,\"ZZ\"BAD,1,1.00,-1,USD\n";
        let before = csv_records(&f).len();
        assert!(matches!(import(&f, "bad.csv", file, None, now()), Err(Refused::Entry(_))));
        assert_eq!(csv_records(&f).len(), before, "the line before the bad one is not kept either");
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
        // and the header says it, in the mapping's words, until it is placed
        let said = f.read(|e| crate::status::unread_rows(e.inputs())).unwrap();
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].starts_with("A row in ") && said[0].contains("could not be placed and counts in no figure") && said[0].contains("nobody-123"), "{said:?}");
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
        assert!(matches!(&s.files[1].read, FileOutcome::Failed { error } if error == "line 1 is not UTF-8 text"), "{:?}", s.files[1].read);
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

