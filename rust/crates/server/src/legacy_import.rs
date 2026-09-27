//! Turning the database this build keeps today into a book
//! (`docs/plans/stage-1-foundation.md`, "The import of an existing database").
//!
//! The book's own import reads the rows. What only this crate can do is read the
//! journal's and the groups' keys: they are the old engine's own ids (a round
//! trip's first fill, a group of slices hashed by the page before the journal, a
//! saved group's id), and only the old engine, which lives here until the
//! cutover, can say which trade each one names. `translate` runs that engine over
//! the old database and hands the book every key with the row that opened its
//! trade, the group it names, or why neither could be found. Nothing is dropped:
//! a note that cannot be read or placed is kept, orphaned, with the reason.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use rusqlite::Connection;
use serde_json::Value;

use bagholder_book::import::{ImportedNote, NoteOn, Report, Translated, TranslatedGroup};
use bagholder_book::Book;
use bagholder_core::journal::{Grade, JournalEntry};
use bagholder_model::activity::{Direction, Flag};
use bagholder_model::fifo::Slice;
use bagholder_model::trades::slice_member_key;

/// A saved group as the old store keeps it.
#[derive(Clone, Debug)]
struct SavedGroup {
    id: String,
    locked: bool,
    members: Vec<String>,
}

/// Every key the old journal and groups use, placed by the old engine.
pub fn translate(conn: &Connection, today: &str) -> Result<Translated, String> {
    let rows = bagholder_store::activities::all_raw_activities(conn).map_err(|e| e.to_string())?;
    let row_ids: BTreeSet<String> = rows.iter().map(|r| r.id.clone()).collect();
    let securities = bagholder_store::rows::securities(conn).map_err(|e| e.to_string())?;
    let book = bagholder_model::book::build_book(&rows, bagholder_model::securities::Securities::new(&securities), today);
    let closed = &book.fifo.closed;

    let meta = |key: &str| bagholder_store::tables::get_meta(conn, key, "").map_err(|e| e.to_string());
    let mut unreadable: Vec<ImportedNote> = Vec::new();
    let groups = read_groups(&meta("trade_groups")?, &mut unreadable);
    let trades = old_trades(closed, &groups);
    let by_slice: HashMap<String, &Slice> = closed.iter().map(|s| (slice_member_key(s), s)).collect();
    let group_ids: BTreeSet<&str> = groups.iter().map(|g| g.id.as_str()).collect();

    // the row that opened the trade the old app knew by `key`
    let opening = |key: &str| -> Result<String, String> {
        if let Some(row) = key.strip_prefix("rt:").filter(|r| !r.contains('|') && row_ids.contains(*r)) {
            // a round trip is named for its first fill; where the position went flat
            // before the old trade closed, the book's trade that closes it opened later
            return Ok(trades.get(key).and_then(|slices| reopened_before_close(&rows, slices)).unwrap_or_else(|| row.to_string()));
        }
        match trades.get(key) {
            Some(slices) => Ok(opening_row(slices)),
            None => Err("no trade the earlier app matched has this key (the trade may have changed since the note was written)".into()),
        }
    };
    let subject = |key: &str| -> NoteOn {
        if group_ids.contains(key) {
            NoteOn::Group(key.to_string())
        } else {
            NoteOn::Trade(opening(key))
        }
    };

    let mut journal: Vec<ImportedNote> = Vec::new();
    let current = read_notes(&meta("journal_v2")?, false, &mut unreadable);
    let current_keys: BTreeSet<String> = current.iter().map(|(k, _)| k.clone()).collect();
    for (key, entry) in current {
        journal.push(ImportedNote { on: subject(&key), key, entry });
    }
    // the notes the page kept before the journal, placed by the old app's own
    // migration; where the journal already has a note on the same trade, the
    // journal's stands on the trade and the older one is kept beside it
    let groups_val: Vec<Value> = groups.iter().map(|g| serde_json::json!({"id": g.id, "locked": g.locked, "members": g.members})).collect();
    for (key, entry) in read_notes(&meta("trade_notes")?, true, &mut unreadable) {
        let mut single = serde_json::Map::new();
        single.insert(key.clone(), serde_json::json!({"thesis": entry.thesis, "tag": entry.tags.join(","), "grade": entry.grade.map(|g| g.as_str()).unwrap_or("")}));
        let placed = bagholder_model::symbols_of::migrate_legacy_notes(closed, &groups_val, &single);
        match placed.keys().next() {
            // the journal has a note on this trade already: the older note is kept on
            // its own, with why, rather than dropped or written over the newer one
            Some(trade_key) if current_keys.contains(trade_key) => journal.push(ImportedNote {
                key,
                on: NoteOn::Trade(Err(format!("a note from before the journal, on a trade ({trade_key}) the journal has a newer note on"))),
                entry,
            }),
            Some(trade_key) => journal.push(ImportedNote { key: key.clone(), on: subject(trade_key), entry }),
            None => journal.push(ImportedNote {
                key,
                on: NoteOn::Trade(Err("a note from before the journal whose trade the earlier app can no longer find".into())),
                entry,
            }),
        }
    }
    journal.extend(unreadable);

    let groups = groups
        .iter()
        .map(|g| TranslatedGroup {
            key: g.id.clone(),
            locked: g.locked,
            members: g
                .members
                .iter()
                .map(|m| {
                    let row = match by_slice.get(m) {
                        // the member is the round trip the slice belongs to
                        Some(s) => Ok(match s.rt.as_deref().and_then(|rt| rt.strip_prefix("rt:")).filter(|r| row_ids.contains(*r)) {
                            Some(row) => row.to_string(),
                            None => opening_row(&[s]),
                        }),
                        None => Err("the earlier app no longer matched this piece of the group's trade".into()),
                    };
                    (m.clone(), row)
                })
                .collect(),
        })
        .collect();
    Ok(Translated { journal, groups })
}

/// The row that opened the round trip the old trade `slices` closed in, where the
/// position went flat in between. The earlier app kept one trade running through a
/// moment the position was flat (all of it sold, then bought again); a trade in the
/// book ends there (`SPEC.md`, a trade is one round trip), so a note the person
/// wrote on the old trade belongs on the book's trade that closes as the old one
/// did, the one they saw its figures for. None where the account's rows of the
/// instrument do not replay cleanly (units moved by a row that is not a buy or a
/// sell), or
/// where it never went flat: the old trade's first fill then opens it.
fn reopened_before_close(rows: &[bagholder_model::activity::RawActivity], slices: &[&Slice]) -> Option<String> {
    let last = slices.iter().max_by(|a, b| (&a.exit_when, &a.exit_date).cmp(&(&b.exit_when, &b.exit_date)))?;
    let close = if last.open_direction == Direction::Short { &last.buy_activity_id } else { &last.sell_activity_id };
    let own: Vec<&bagholder_model::activity::RawActivity> = rows.iter().filter(|r| r.account_id == last.account_id && r.security_id == last.security_id && !r.security_id.is_empty()).collect();
    let mut held = 0.0_f64;
    let mut opened: Option<&str> = None;
    for r in own {
        // a distribution names the instrument and moves no units
        if r.quantity == 0.0 {
            continue;
        }
        if r.activity_type != "Trade" {
            return None;
        }
        if held.abs() < 1e-9 {
            opened = Some(&r.id);
        }
        held += r.quantity;
        if &r.id == close {
            return opened.map(str::to_string);
        }
    }
    None
}

/// The row that opened a trade: the opening fill of its earliest piece.
fn opening_row(slices: &[&Slice]) -> String {
    let first = slices.iter().min_by(|a, b| (&a.entry_when, &a.entry_date, slice_member_key(a)).cmp(&(&b.entry_when, &b.entry_date, slice_member_key(b))));
    match first {
        Some(s) if s.open_direction == Direction::Short => s.sell_activity_id.clone(),
        Some(s) => s.buy_activity_id.clone(),
        None => String::new(),
    }
}

/// The old app's trades and their pieces, keyed as it keyed them: the saved
/// groups first, then each round trip (`rt:<first fill>`, a deposited coin's
/// pieces apart under `|nobasis`), as `bagholder_model::trades::build_trades`.
fn old_trades<'a>(closed: &'a [Slice], groups: &[SavedGroup]) -> BTreeMap<String, Vec<&'a Slice>> {
    let by_key: HashMap<String, &Slice> = closed.iter().map(|s| (slice_member_key(s), s)).collect();
    let mut used: BTreeSet<String> = BTreeSet::new();
    let mut out: BTreeMap<String, Vec<&Slice>> = BTreeMap::new();
    for g in groups {
        let members: Vec<&Slice> = g.members.iter().filter(|k| used.insert((*k).clone())).filter_map(|k| by_key.get(k).copied()).collect();
        if !members.is_empty() {
            out.insert(g.id.clone(), members);
        }
    }
    for s in closed {
        let key = slice_member_key(s);
        if used.contains(&key) {
            continue;
        }
        let mut rt = s.rt.clone().unwrap_or_else(|| format!("rt:{key}"));
        if s.flags.contains(&Flag::BasisUnknown) {
            rt.push_str("|nobasis");
        }
        out.entry(rt).or_default().push(s);
    }
    out
}

/// The saved groups, read strictly: one the import cannot read is reported as
/// a note kept with its text, so nothing the person made disappears unseen.
fn read_groups(raw: &str, unreadable: &mut Vec<ImportedNote>) -> Vec<SavedGroup> {
    if raw.trim().is_empty() {
        return vec![];
    }
    let kept = |why: String, text: String, unreadable: &mut Vec<ImportedNote>| {
        unreadable.push(ImportedNote {
            key: "trade_groups".into(),
            on: NoteOn::Trade(Err(format!("a saved group the import cannot read ({why}); its text is kept as this note"))),
            entry: JournalEntry { thesis: text, grade: None, tags: vec![] },
        })
    };
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(raw) else {
        kept("not a list".into(), raw.to_string(), unreadable);
        return vec![];
    };
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for item in items {
        let id = item.get("id").and_then(Value::as_str).map(str::trim).unwrap_or("");
        let members: Option<Vec<String>> = item.get("members").and_then(Value::as_array).and_then(|m| m.iter().map(|k| k.as_str().map(|s| s.trim().to_string())).collect());
        let locked = matches!(item.get("locked"), Some(Value::Bool(true)));
        match (id, members) {
            (id, Some(members)) if !id.is_empty() && !members.is_empty() && seen.insert(id.to_string()) => {
                let mut uniq = Vec::new();
                for m in members.into_iter().filter(|m| !m.is_empty()) {
                    if !uniq.contains(&m) {
                        uniq.push(m);
                    }
                }
                out.push(SavedGroup { id: id.to_string(), locked, members: uniq });
            }
            _ => kept("no id, no members, or an id used twice".into(), item.to_string(), unreadable),
        }
    }
    out
}

/// The journal (`journal_v2`: thesis, tags, grade) or the notes before it
/// (`trade_notes`: thesis, tag, grade, the tags one comma-joined string), read
/// strictly: an entry the import cannot read is kept with its text as the thesis,
/// on an orphaned trade, never dropped. An entry with nothing in it is no entry.
fn read_notes(raw: &str, older: bool, unreadable: &mut Vec<ImportedNote>) -> Vec<(String, JournalEntry)> {
    if raw.trim().is_empty() {
        return vec![];
    }
    let what = if older { "trade_notes" } else { "journal_v2" };
    let keep = |key: String, why: String, text: String, unreadable: &mut Vec<ImportedNote>| {
        unreadable.push(ImportedNote {
            key,
            on: NoteOn::Trade(Err(format!("the earlier app's note could not be read ({why}); its text is kept as this note"))),
            entry: JournalEntry { thesis: text, grade: None, tags: vec![] },
        })
    };
    let map = match serde_json::from_str::<Value>(raw) {
        Ok(Value::Object(map)) => map,
        _ => {
            keep(what.into(), "not an object".into(), raw.to_string(), unreadable);
            return vec![];
        }
    };
    let mut out = Vec::new();
    for (key, v) in map {
        match note(&v, older) {
            Ok(e) if e.is_empty() => {}
            Ok(e) => out.push((key, e)),
            Err(why) => keep(key, why, v.to_string(), unreadable),
        }
    }
    out
}

fn note(v: &Value, older: bool) -> Result<JournalEntry, String> {
    let obj = v.as_object().ok_or("not an object")?;
    let text = |field: &str| -> Result<String, String> {
        match obj.get(field) {
            None | Some(Value::Null) => Ok(String::new()),
            Some(Value::String(s)) => Ok(s.clone()),
            Some(other) => Err(format!("its {field} is not text: {other}")),
        }
    };
    let thesis = text("thesis")?;
    let grade = match text("grade")?.trim() {
        "" => None,
        g => Some(Grade::parse(&g.to_uppercase()).map_err(|e| e.to_string())?),
    };
    let split = |s: &str| s.split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect::<Vec<_>>();
    let tags = if older {
        split(&text("tag")?)
    } else {
        match obj.get("tags") {
            None | Some(Value::Null) => vec![],
            // a page old enough sent the tags as one comma-joined string
            Some(Value::String(s)) => split(s),
            Some(Value::Array(items)) => items.iter().map(|t| t.as_str().map(|s| s.trim().to_string()).ok_or_else(|| format!("a tag is not text: {t}"))).collect::<Result<Vec<_>, _>>()?.into_iter().filter(|t| !t.is_empty()).collect(),
            Some(other) => return Err(format!("its tags are not a list: {other}")),
        }
    };
    let mut uniq: Vec<String> = Vec::new();
    for t in tags {
        if !uniq.contains(&t) {
            uniq.push(t);
        }
    }
    Ok(JournalEntry { thesis, grade, tags: uniq })
}

/// Drop the earlier store's figure tables from the live file once the book
/// holds their rows (`docs/plans/stage-3c-switch.md` §8), the file snapshotted
/// first beside the book's own snapshots, where an import or `compare-figures`
/// can still read it: the snapshot's path, or none when there was nothing to do.
/// Refused while the file holds activity the book never imported.
pub fn retire_old_figures(home: &Path, conn: &Connection, book: &Book, at: jiff::Timestamp) -> Result<Option<std::path::PathBuf>, String> {
    let e = |e: rusqlite::Error| e.to_string();
    if bagholder_store::schema::figures_moved(conn).map_err(e)? {
        return Ok(None);
    }
    let rows: i64 = conn.query_row("SELECT COUNT(*) FROM activities", [], |r| r.get(0)).map_err(e)?;
    let imported = book.record_count(&bagholder_book::import::import_source()).map_err(|e| e.to_string())?;
    if rows > 0 && imported == 0 {
        return Err(format!("the earlier store holds {rows} activity rows the book has not imported; its tables are kept"));
    }
    // beside the book's own snapshots
    let dir = home.join("snapshots");
    std::fs::create_dir_all(&dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let snapshot = dir.join(format!("bagholder-before-the-book-{}.db", at.as_millisecond()));
    bagholder_book::import::copy_database(&home.join("bagholder.db"), &snapshot).map_err(|e| e.to_string())?;
    bagholder_store::schema::drop_figure_tables(conn, &at.to_string()).map_err(e)?;
    Ok(Some(snapshot))
}

/// Import the database at `old` into the book in `home`, as of `at`: copy it,
/// translate its keys, import. The database at `old` is only read.
pub fn import(old: &Path, home: &Path, at: jiff::Timestamp) -> Result<Report, String> {
    std::fs::create_dir_all(home).map_err(|e| format!("{}: {e}", home.display()))?;
    let copy = home.join(format!("import-source-{}.db", at.as_millisecond()));
    let result = (|| {
        bagholder_book::import::copy_database(old, &copy).map_err(|e| e.to_string())?;
        let conn = Connection::open_with_flags(&copy, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
        let today = at.to_zoned(jiff::tz::TimeZone::UTC).date().to_string();
        let translated = translate(&conn, &today)?;
        drop(conn);
        let data = bagholder_book::import::old::read(&copy).map_err(|e| e.to_string())?;
        let (book, _) = Book::open_in(home, crate::app::APP_VERSION, at).map_err(|e| e.to_string())?;
        book.import(&data, &translated, at).map_err(|e| e.to_string())
    })();
    // the copy goes whatever the import came to; one that cannot be removed is said with it
    match std::fs::remove_file(&copy) {
        Ok(()) => result,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => result,
        Err(e) => {
            let left = format!("the copy {} could not be removed: {e}", copy.display());
            Err(match result {
                Ok(_) => format!("the book imported it, but {left}"),
                Err(first) => format!("{first}; and {left}"),
            })
        }
    }
}

/// The name the repair of carried notes is recorded under in the book.
const REATTACH: &str = "carried-notes-reattached";

/// The earlier app's database a book in `home` was imported from, where it is
/// still kept whole: `bagholder.db` beside the book until its figure tables are
/// retired, then the snapshot `retire_old_figures` took of it first (the latest,
/// where there are several).
fn original_database(home: &Path) -> Result<Option<std::path::PathBuf>, String> {
    let live = home.join(crate::figures::OLD_FILE);
    if live.is_file() {
        let conn = Connection::open_with_flags(&live, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| format!("{}: {e}", live.display()))?;
        if !bagholder_store::schema::figures_moved(&conn).map_err(|e| format!("{}: {e}", live.display()))? {
            return Ok(Some(live));
        }
    }
    let dir = home.join("snapshots");
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    let mut best: Option<(i64, std::path::PathBuf)> = None;
    for entry in entries {
        let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let Some(ms) = name.strip_prefix("bagholder-before-the-book-").and_then(|r| r.strip_suffix(".db")).and_then(|n| n.parse::<i64>().ok()) else { continue };
        if best.as_ref().is_none_or(|(b, _)| ms > *b) {
            best = Some((ms, path));
        }
    }
    Ok(best.map(|(_, p)| p))
}

/// What the earlier app's database at `old` says each key it kept names, read
/// from a copy in `home` as the import reads it.
fn translated_from(old: &Path, home: &Path, at: jiff::Timestamp) -> Result<Translated, String> {
    let copy = home.join(format!("import-source-{}.db", at.as_millisecond()));
    let result = (|| {
        bagholder_book::import::copy_database(old, &copy).map_err(|e| e.to_string())?;
        let conn = Connection::open_with_flags(&copy, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
        translate(&conn, &at.to_zoned(jiff::tz::TimeZone::UTC).date().to_string())
    })();
    match std::fs::remove_file(&copy) {
        Ok(()) => result,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => result,
        Err(e) => Err(format!("the copy {} could not be removed: {e}", copy.display())),
    }
}

/// Put back on its round trip each note carried from the earlier app whose trade
/// an earlier build orphaned though the round trip still stands (a first pull
/// replaced the imported rows by the broker's, and the build settled the round
/// trips against the anchors it held from before, `figures::settle_trades`).
/// Done once per book, recorded: each orphaned trade with a key of the earlier
/// app and a note goes to the round trip the import places such a note on today
/// (the one that closes as the earlier app's trade did, read again from the
/// database the book was imported from, where it is kept), else to the round trip
/// holding the broker's row for the fill the key names; never onto a round trip
/// whose trade has a note of its own or is in a group (`Book::reattach`), and a
/// trade with neither stays orphaned. Whether any trade moved.
pub fn reattach_carried_notes(home: &Path, book: &Book, engine: &bagholder_engine::Engine, at: jiff::Timestamp) -> Result<bool, String> {
    use bagholder_core::journal::{Anchor, Opening};
    let e = |e: bagholder_book::BookError| e.to_string();
    if book.repaired(REATTACH).map_err(e)? {
        return Ok(false);
    }
    let orphans: Vec<(bagholder_core::TradeId, String)> = book.orphaned_journal().map_err(e)?.into_iter().filter_map(|(t, _)| Some((t.id, t.legacy_key?))).collect();
    let mut moved = false;
    if !orphans.is_empty() {
        // where the import places each key's note today
        // (one that cannot be read leaves each note to the fill its key names)
        let translated = match original_database(home) {
            Ok(Some(old)) => translated_from(&old, home, at).map_err(|why| format!("{}: {why}", old.display())),
            Ok(None) => Ok(Translated::default()),
            Err(why) => Err(why),
        };
        let translated = translated.unwrap_or_else(|why| {
            crate::app::log(&format!("bagholder: the earlier app's database could not be read to place its notes again ({why}); each goes to the round trip of the fill its key names"));
            Translated::default()
        });
        let placed: BTreeMap<String, String> = translated
            .journal
            .into_iter()
            .filter_map(|n| match n.on {
                NoteOn::Trade(Ok(row)) => Some((n.key, row)),
                _ => None,
            })
            .collect();
        let source = bagholder_book::import::import_source();
        let ledger = &engine.inputs().ledger;
        let trips = &engine.figures().matched.trips;
        for (trade, key) in orphans {
            let rows = placed.get(&key).cloned().into_iter().chain(key.strip_prefix("rt:").filter(|r| !r.contains('|')).map(str::to_string));
            let mut to = None;
            for row in rows {
                let standing: BTreeSet<bagholder_core::TransactionId> = book.standing_for(&source, &row).map_err(e)?.into_iter().map(|t| t.id).collect();
                let mut holding = trips.values().filter(|t| !bagholder_engine::identity::managed(ledger, t.account) && t.fills.iter().any(|f| standing.contains(f)));
                if let (Some(trip), None) = (holding.next(), holding.next()) {
                    to = Some(Opening { transaction: trip.key.opening.clone(), instrument: trip.key.instrument });
                    break;
                }
            }
            let Some(to) = to else { continue };
            if !matches!(book.trade(trade).map_err(e)?.anchor, Anchor::Orphaned(_)) {
                continue;
            }
            if book.reattach(trade, &to).map_err(e)? == bagholder_book::trades::Reattached::Attached {
                moved = true;
            }
        }
    }
    book.record_repair(REATTACH, at).map_err(e)?;
    Ok(moved)
}

/// The Python app's data folder, as that app finds it: `.bagholder` in the
/// person's home (`USERPROFILE` on Windows, `HOME` elsewhere), read through `var`.
pub fn python_home(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<std::path::PathBuf> {
    let home = var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).filter(|h| !h.is_empty())?;
    Some(Path::new(&home).join(".bagholder"))
}

/// When a database was last written: the later of the file's and its write-ahead
/// log's modification times, since a running app writes the log first.
fn last_written(db: &Path) -> Result<std::time::SystemTime, String> {
    let when = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).map_err(|e| format!("{}: {e}", p.display()));
    let file = when(db)?;
    let wal = db.with_file_name(format!("{}-wal", db.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
    if wal.is_file() {
        return Ok(file.max(when(&wal)?));
    }
    Ok(file)
}

/// A first start with no book takes the Python app's database: when `home` holds
/// no book and `python` holds a `bagholder.db` written later than any `home`
/// holds, a copy of it is put in `home`, where the start imports it as its own.
/// A `bagholder.db` of `home`'s own that the Python app's is newer than (one an
/// earlier Rust build left, never made into a book) is moved into `snapshots/`
/// first, with its log. `python` is only read. Answers the file copied, if one was.
pub fn adopt_python_database(home: &Path, python: &Path) -> Result<Option<std::path::PathBuf>, String> {
    let from = python.join(crate::figures::OLD_FILE);
    let to = home.join(crate::figures::OLD_FILE);
    if home.join(bagholder_book::BOOK_FILE).exists() || !from.is_file() {
        return Ok(None);
    }
    if to.exists() {
        if last_written(&to)? >= last_written(&from)? {
            return Ok(None);
        }
        let dir = home.join("snapshots");
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let stamp = jiff::Timestamp::now().as_millisecond();
        for ext in ["", "-wal", "-shm"] {
            let own = home.join(format!("{}{ext}", crate::figures::OLD_FILE));
            let kept = dir.join(format!("bagholder-before-the-python-copy-{stamp}.db{ext}"));
            match std::fs::rename(&own, &kept) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && !ext.is_empty() => {}
                Err(e) => return Err(format!("{} could not be moved aside for the Python app's newer database: {e}", own.display())),
            }
        }
    }
    // written under another name and then put in place, so a copy cut short is never taken for the database
    let part = home.join(format!("{}.part", crate::figures::OLD_FILE));
    let beside = |ext: &str| python.join(format!("{}-{ext}", crate::figures::OLD_FILE));
    let copied = if beside("wal").exists() || beside("shm").exists() {
        // the Python app is running, or left changes in its write-ahead log: SQLite's
        // own copy reads them, through the files the Python app already made
        bagholder_book::import::copy_database(&from, &part).map_err(|e| e.to_string())
    } else {
        // everything is in the file: copied as it is, since opening it with SQLite
        // would leave SQLite's own files beside it in the Python app's folder
        std::fs::copy(&from, &part).map(|_| ()).map_err(|e| e.to_string())
    }
    .and_then(|()| std::fs::rename(&part, &to).map_err(|e| e.to_string()));
    match copied {
        Ok(()) => Ok(Some(from)),
        Err(e) => {
            let left = match std::fs::remove_file(&part) {
                Err(r) if r.kind() != std::io::ErrorKind::NotFound => format!("; and {} could not be removed: {r}", part.display()),
                _ => String::new(),
            };
            Err(format!("the Python app's database {} could not be copied into {}: {e}{left}", from.display(), home.display()))
        }
    }
}

/// `bagholder import-book <old database> <data folder>`: import and print the report.
pub fn cli(args: &[String]) -> i32 {
    let [old, home] = args else {
        eprintln!("usage: bagholder import-book <old database> <data folder>");
        return 2;
    };
    match import(Path::new(old), Path::new(home), jiff::Timestamp::now()) {
        Ok(report) => {
            println!("{}", render(&report));
            0
        }
        Err(e) => {
            eprintln!("the import failed: {e}");
            1
        }
    }
}

fn render(r: &Report) -> String {
    let mut out = String::new();
    let mut line = |s: String| {
        out.push_str(&s);
        out.push('\n');
    };
    line(format!("rows read: {}", r.rows_read));
    line(format!("records: {} new, {} revised, {} unchanged", r.records_new, r.records_revised, r.records_unchanged));
    line(format!("accounts made: {}", r.accounts_made));
    for p in &r.account_problems {
        line(format!("  account: {p}"));
    }
    line("transactions by kind:".into());
    for (k, n) in &r.transactions_by_kind {
        line(format!("  {k}: {n}"));
    }
    line("problems by kind:".into());
    for (k, n) in &r.problems_by_code {
        line(format!("  {k}: {n}"));
    }
    line(format!("journal: {} attached, {} orphaned", r.journal_attached, r.journal_orphaned.len()));
    for (k, why) in &r.journal_orphaned {
        line(format!("  {k}: {why}"));
    }
    line(format!("groups: {}, members {} attached, {} orphaned", r.groups, r.group_members_attached, r.group_members_orphaned.len()));
    for (k, why) in &r.group_members_orphaned {
        line(format!("  {k}: {why}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trade(conn: &Connection, id: &str, symbol: &str, sub: &str, qty: f64, when: &str) {
        conn.execute(
            "INSERT INTO activities(id, occurred_at, transaction_date, settlement_date, account_id, book_id, fifo_id, activity_type, activity_sub_type,
                                    symbol, name, currency, quantity, unit_price, commission, net_cash_amount, category, source, raw_type, security_id)
             VALUES (?1, ?2, substr(?2, 1, 10), substr(?2, 1, 10), 'acc', 'acc', 'acc', 'Trade', ?3, ?4, ?4, 'CAD', ?5, 1.0, 0, -?5, 'trade', 'wealthsimple', 'DIY_' || ?3, 'sec-' || ?4)",
            rusqlite::params![id, when, sub, symbol, qty],
        )
        .unwrap();
        conn.execute("INSERT OR IGNORE INTO securities(id, symbol, name, currency) VALUES ('sec-' || ?1, ?1, ?1, 'CAD')", [symbol]).unwrap();
    }

    /// The earlier app ran one trade through a moment the position was flat (all of
    /// it sold, then bought again: the owner's QNC of 2024-12-12 → 2026-06-24). A note
    /// the person wrote on it belongs on the book's trade that closes as it did, not
    /// on the day-long round trip its first fill now opens.
    #[test]
    fn a_note_on_an_old_trade_that_went_flat_goes_to_the_book_s_trade_that_closes_as_it_did() {
        let dir = tempfile::tempdir().unwrap();
        let conn = bagholder_store::open_db(&dir.path().join("bagholder.db")).unwrap();
        bagholder_store::schema::init_schema(&conn).unwrap();
        trade(&conn, "b1", "QNC", "BUY", 1667.0, "2024-12-12T19:58:39+00:00");
        trade(&conn, "b2", "QNC", "BUY", 1852.0, "2024-12-12T20:53:07+00:00");
        trade(&conn, "s1", "QNC", "SELL", -3519.0, "2024-12-13T17:23:54+00:00");
        trade(&conn, "b3", "QNC", "BUY", 5016.0, "2024-12-13T18:35:20+00:00");
        trade(&conn, "s2", "QNC", "SELL", -5016.0, "2026-06-24T15:00:00+00:00");
        set_meta(&conn, "journal_v2", r#"{"rt:b1": {"thesis": "long hold", "tags": ["winners"], "grade": "A"}}"#);
        let t = translate(&conn, "2026-09-26").unwrap();
        let on: Vec<&NoteOn> = t.journal.iter().map(|n| &n.on).collect();
        // the earlier app's trade `rt:b1` runs through the flat moment to s2
        let rows = bagholder_store::activities::all_raw_activities(&conn).unwrap();
        let securities = bagholder_store::rows::securities(&conn).unwrap();
        let book = bagholder_model::book::build_book(&rows, bagholder_model::securities::Securities::new(&securities), "2026-09-26");
        let old = old_trades(&book.fifo.closed, &[]);
        let pairs: Vec<(&str, &str)> = old["rt:b1"].iter().map(|x| (x.buy_activity_id.as_str(), x.sell_activity_id.as_str())).collect();
        assert_eq!(pairs, [("b1", "s1"), ("b2", "s1"), ("b3", "s2")]);
        assert_eq!(on, vec![&NoteOn::Trade(Ok("b3".into()))], "the book's trade that closes on s2");
    }

    /// The earlier app's QNC trade that ran through a flat moment, noted, in the
    /// earlier app's database in a new home.
    fn noted_home() -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        let conn = bagholder_store::open_db(&home.path().join(crate::figures::OLD_FILE)).unwrap();
        bagholder_store::schema::init_schema(&conn).unwrap();
        trade(&conn, "b1", "QNC", "BUY", 1667.0, "2024-12-12T19:58:39+00:00");
        trade(&conn, "b2", "QNC", "BUY", 1852.0, "2024-12-12T20:53:07+00:00");
        trade(&conn, "s1", "QNC", "SELL", -3519.0, "2024-12-13T17:23:54+00:00");
        trade(&conn, "b3", "QNC", "BUY", 5016.0, "2024-12-13T18:35:20+00:00");
        trade(&conn, "s2", "QNC", "SELL", -5016.0, "2026-06-24T15:00:00+00:00");
        set_meta(&conn, "journal_v2", r#"{"rt:b1": {"thesis": "long hold", "tags": ["winners"], "grade": "A"}}"#);
        home
    }

    fn at(s: &str) -> jiff::Timestamp {
        s.parse().unwrap()
    }

    /// The book as an earlier build left it: the carried note's trade orphaned
    /// though its round trip stands, and the repair not yet done.
    fn orphaned_as_before(home: &Path) {
        let c = Connection::open(home.join(bagholder_book::BOOK_FILE)).unwrap();
        c.execute("UPDATE trades SET anchor_record = NULL, anchor_leg = NULL, anchor_instrument = NULL, orphaned_reason = 'the transaction it opened on no longer opens a round trip' WHERE legacy_key = 'rt:b1'", []).unwrap();
        c.execute("DELETE FROM settings WHERE key LIKE 'repair.%'", []).unwrap();
    }

    /// The imported record of the earlier app's row `row`, as the trade's anchor.
    fn on_row(book: &Book, row: &str) -> bagholder_core::RecordId {
        book.standing_for(&bagholder_book::import::import_source(), row).unwrap()[0].id.record
    }

    fn anchored_on(book: &Book, trade: bagholder_core::TradeId) -> Option<bagholder_core::RecordId> {
        match book.trade(trade).unwrap().anchor {
            bagholder_core::journal::Anchor::Opening(o) => Some(o.transaction.record),
            bagholder_core::journal::Anchor::Orphaned(_) => None,
        }
    }

    /// A book imported before the carried notes kept their trades through a sync
    /// is repaired on its next start, once: the orphaned note's trade goes back to
    /// the round trip the import places it on today, read again from the snapshot
    /// of the earlier app's database, its id and note kept, and the trade a build
    /// gave that round trip since gives way to it.
    #[test]
    fn an_orphaned_carried_note_goes_back_to_the_trade_it_was_written_on_at_the_next_start() {
        let home = noted_home();
        let f = crate::figures::Figures::open(home.path(), at("2026-09-26T12:00:00Z")).unwrap();
        f.state_zone("America/Edmonton", at("2026-09-26T12:00:00Z")).unwrap();
        let book = f.book().unwrap();
        let trade = book.trade_by_legacy_key("rt:b1").unwrap().unwrap();
        assert_eq!(anchored_on(&book, trade), Some(on_row(&book, "b3")), "the import places it on the trade that closes as the old one did");
        // the earlier store's figures retired: the database is kept only as the snapshot
        let old = Connection::open(home.path().join(crate::figures::OLD_FILE)).unwrap();
        retire_old_figures(home.path(), &old, &book, at("2026-09-26T12:30:00Z")).unwrap().expect("a snapshot");
        drop(old);
        drop(f);
        orphaned_as_before(home.path());
        // a start made before the repair existed gave the round trip a trade of its own
        let c = Connection::open(home.path().join(bagholder_book::BOOK_FILE)).unwrap();
        c.execute("INSERT INTO settings(key, value, source, set_at) VALUES ('repair.carried-notes-reattached', 'done', 'bagholder', '2026-09-26T12:40:00Z')", []).unwrap();
        crate::figures::Figures::open(home.path(), at("2026-09-26T12:45:00Z")).unwrap();
        let holder = book.trades().unwrap().into_iter().find(|t| anchored_on(&book, t.id) == Some(on_row(&book, "b3"))).expect("a trade of its own").id;
        assert_ne!(holder, trade);
        c.execute("DELETE FROM settings WHERE key LIKE 'repair.%'", []).unwrap();

        let f = crate::figures::Figures::open(home.path(), at("2026-09-26T13:00:00Z")).unwrap();
        assert_eq!(anchored_on(&book, trade), Some(on_row(&book, "b3")));
        assert!(book.trade(holder).is_err(), "the trade given since gives way");
        assert!(book.orphaned_journal().unwrap().is_empty());
        let shown = f.read(|e| e.figures().trades.iter().find(|t| t.trade == Some(trade)).map(|t| t.journal.grade)).unwrap();
        assert_eq!(shown, Some(Some(Grade::A)), "the figures show the trade with its note");
        // once: a later start does nothing again
        let before = book.trades().unwrap();
        crate::figures::Figures::open(home.path(), at("2026-09-26T14:00:00Z")).unwrap();
        assert_eq!(book.trades().unwrap(), before);
    }

    /// With the earlier app's database no longer kept, the note goes to the round
    /// trip holding the fill its key names.
    #[test]
    fn with_no_earlier_database_kept_the_note_goes_to_the_round_trip_of_the_fill_its_key_names() {
        let home = noted_home();
        let f = crate::figures::Figures::open(home.path(), at("2026-09-26T12:00:00Z")).unwrap();
        f.state_zone("America/Edmonton", at("2026-09-26T12:00:00Z")).unwrap();
        drop(f);
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(home.path().join(format!("{}{ext}", crate::figures::OLD_FILE)));
        }
        orphaned_as_before(home.path());
        let book = crate::figures::Figures::open(home.path(), at("2026-09-26T13:00:00Z")).unwrap().book().unwrap();
        let trade = book.trade_by_legacy_key("rt:b1").unwrap().unwrap();
        assert_eq!(anchored_on(&book, trade), Some(on_row(&book, "b1")));
        assert!(book.orphaned_journal().unwrap().is_empty());
    }

    /// Nothing is put back onto a round trip whose trade has a note of its own:
    /// the carried note stays orphaned, saying so.
    #[test]
    fn a_carried_note_is_not_put_on_a_round_trip_whose_trade_has_a_note_of_its_own() {
        let home = noted_home();
        let f = crate::figures::Figures::open(home.path(), at("2026-09-26T12:00:00Z")).unwrap();
        f.state_zone("America/Edmonton", at("2026-09-26T12:00:00Z")).unwrap();
        let book = f.book().unwrap();
        drop(f);
        orphaned_as_before(home.path());
        let c = Connection::open(home.path().join(bagholder_book::BOOK_FILE)).unwrap();
        c.execute("INSERT INTO settings(key, value, source, set_at) VALUES ('repair.carried-notes-reattached', 'done', 'bagholder', '2026-09-26T12:40:00Z')", []).unwrap();
        crate::figures::Figures::open(home.path(), at("2026-09-26T12:45:00Z")).unwrap();
        let holder = book.trades().unwrap().into_iter().find(|t| anchored_on(&book, t.id) == Some(on_row(&book, "b3"))).unwrap().id;
        let theirs = JournalEntry { thesis: "written since".into(), grade: None, tags: vec![] };
        book.set_journal(bagholder_core::journal::JournalSubject::Trade(holder), &theirs, at("2026-09-26T12:50:00Z")).unwrap();
        c.execute("DELETE FROM settings WHERE key LIKE 'repair.%'", []).unwrap();
        crate::figures::Figures::open(home.path(), at("2026-09-26T13:00:00Z")).unwrap();
        let trade = book.trade_by_legacy_key("rt:b1").unwrap().unwrap();
        assert_eq!(anchored_on(&book, trade), None);
        assert!(book.trade(trade).unwrap().orphaned_reason().is_some_and(|why| why.contains("note of its own")));
        assert_eq!(anchored_on(&book, holder), Some(on_row(&book, "b3")));
        assert_eq!(book.orphaned_journal().unwrap().len(), 1, "the carried note is kept");
    }

    fn set_meta(conn: &Connection, key: &str, value: &str) {
        conn.execute("INSERT INTO meta(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value", [key, value]).unwrap();
    }

    #[test]
    fn every_key_the_earlier_app_used_is_placed_or_kept_with_why() {
        let conn = Connection::open_in_memory().unwrap();
        bagholder_store::schema::init_schema(&conn).unwrap();
        trade(&conn, "b1", "QNC", "BUY", 10.0, "2026-01-02T15:00:00+00:00");
        trade(&conn, "s1", "QNC", "SELL", -10.0, "2026-01-05T15:00:00+00:00");
        trade(&conn, "b2", "ABC", "BUY", 3.0, "2026-02-02T15:00:00+00:00");
        trade(&conn, "s2", "ABC", "SELL", -3.0, "2026-02-05T15:00:00+00:00");
        // the page before the journal keyed a note by a hash of its lane's slices
        let lane = bagholder_model::trades::group_id_for_keys(&["b2|s2|3.00000000".to_string()]);
        set_meta(&conn, "trade_notes", &serde_json::json!({ &lane: {"thesis": "old note", "tag": "a, b", "grade": "c"} }).to_string());
        set_meta(&conn, "trade_groups", r#"[{"id": "g_saved", "locked": true, "members": ["b1|s1|10.00000000", "gone|x|1.00000000"]}, {"members": []}]"#);
        set_meta(&conn, "journal_v2", r#"{"rt:b1": {"thesis": "on the round trip", "tags": ["x"], "grade": "A"},
                                          "rt:ghost": {"thesis": "on a trade that no longer is", "tags": [], "grade": ""},
                                          "g_saved": {"thesis": "on the group", "tags": "one, two", "grade": "B"},
                                          "broken": {"thesis": 5},
                                          "empty": {"thesis": "", "tags": [], "grade": ""}}"#);
        let t = translate(&conn, "2026-09-23").unwrap();
        let find = |key: &str| t.journal.iter().find(|n| n.key == key).unwrap_or_else(|| panic!("{key} missing: {:?}", t.journal));
        assert_eq!(find("rt:b1").on, NoteOn::Trade(Ok("b1".into())));
        assert_eq!(find("rt:b1").entry.grade, Some(Grade::A));
        assert!(matches!(&find("rt:ghost").on, NoteOn::Trade(Err(_))));
        assert_eq!(find("g_saved").on, NoteOn::Group("g_saved".into()));
        assert_eq!(find("g_saved").entry.tags, vec!["one", "two"]);
        assert!(matches!(&find("broken").on, NoteOn::Trade(Err(why)) if why.contains("could not be read")));
        assert_eq!(find("broken").entry.thesis, r#"{"thesis":5}"#, "an unreadable note keeps its text");
        assert!(t.journal.iter().all(|n| n.key != "empty"), "an entry with nothing in it is no entry");
        // the note from before the journal, placed by the earlier app's own migration
        let old = find(&lane);
        assert_eq!(old.on, NoteOn::Trade(Ok("b2".into())));
        assert_eq!((old.entry.thesis.as_str(), old.entry.tags.clone(), old.entry.grade), ("old note", vec!["a".to_string(), "b".to_string()], Some(Grade::C)));
        // a saved group the import cannot read is kept, with its text
        assert!(t.journal.iter().any(|n| n.key == "trade_groups" && n.entry.thesis.contains("members")));
        assert_eq!(t.groups.len(), 1);
        assert_eq!(t.groups[0].members[0], ("b1|s1|10.00000000".to_string(), Ok("b1".to_string())));
        assert!(t.groups[0].members[1].1.is_err());
    }

    #[test]
    fn an_older_note_on_a_trade_the_journal_has_a_note_on_is_kept_beside_it() {
        let conn = Connection::open_in_memory().unwrap();
        bagholder_store::schema::init_schema(&conn).unwrap();
        trade(&conn, "b2", "ABC", "BUY", 3.0, "2026-02-02T15:00:00+00:00");
        trade(&conn, "s2", "ABC", "SELL", -3.0, "2026-02-05T15:00:00+00:00");
        let lane = bagholder_model::trades::group_id_for_keys(&["b2|s2|3.00000000".to_string()]);
        set_meta(&conn, "trade_notes", &serde_json::json!({ &lane: {"thesis": "the older words", "tag": "", "grade": ""} }).to_string());
        set_meta(&conn, "journal_v2", r#"{"rt:b2": {"thesis": "the newer words", "tags": [], "grade": ""}}"#);
        let t = translate(&conn, "2026-09-23").unwrap();
        let newer = t.journal.iter().find(|n| n.key == "rt:b2").unwrap();
        assert_eq!(newer.on, NoteOn::Trade(Ok("b2".into())));
        let older = t.journal.iter().find(|n| n.key == lane).unwrap();
        assert_eq!(older.entry.thesis, "the older words");
        assert!(matches!(&older.on, NoteOn::Trade(Err(why)) if why.contains("newer note")), "{:?}", older.on);
    }

    #[test]
    fn the_import_reads_a_copy_and_leaves_the_database_as_it_was() {
        let dir = std::env::temp_dir().join(format!("bh-import-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("bagholder.db");
        {
            let conn = bagholder_store::open_db(&old).unwrap();
            bagholder_store::schema::init_schema(&conn).unwrap();
            trade(&conn, "b1", "QNC", "BUY", 10.0, "2026-01-02T15:00:00+00:00");
            set_meta(&conn, "journal_v2", r#"{"rt:b1": {"thesis": "kept", "tags": [], "grade": ""}}"#);
        }
        let before = std::fs::read(&old).unwrap();
        let home = dir.join("home");
        let at: jiff::Timestamp = "2026-09-23T12:00:00Z".parse().unwrap();
        let report = import(&old, &home, at).unwrap();
        assert_eq!((report.rows_read, report.records_new, report.journal_attached), (1, 1, 1));
        assert_eq!(std::fs::read(&old).unwrap(), before);
        let left: Vec<String> = std::fs::read_dir(&home).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(left.iter().all(|f| f.starts_with("book.db")), "the copy is gone: {left:?}");
        let again = import(&old, &home, at).unwrap();
        assert_eq!((again.records_new, again.records_unchanged), (0, 1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A made-up person's home: the Python app's folder with its database in it.
    fn python_folder(person: &Path) -> std::path::PathBuf {
        let python = person.join(".bagholder");
        std::fs::create_dir_all(&python).unwrap();
        let conn = bagholder_store::open_db(&python.join("bagholder.db")).unwrap();
        bagholder_store::schema::init_schema(&conn).unwrap();
        trade(&conn, "b1", "QNC", "BUY", 10.0, "2026-01-02T15:00:00+00:00");
        set_meta(&conn, "journal_v2", r#"{"rt:b1": {"thesis": "from the Python app", "tags": [], "grade": ""}}"#);
        python
    }

    fn listing(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut out: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read(&p).unwrap())).collect();
        out.sort();
        out
    }

    #[test]
    fn the_python_app_s_folder_is_found_in_the_person_s_home() {
        let person = tempfile::tempdir().unwrap();
        let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let env = |k: &str| (k == key).then(|| person.path().as_os_str().to_owned());
        assert_eq!(python_home(env), Some(person.path().join(".bagholder")));
        assert_eq!(python_home(|_| None), None);
        assert_eq!(python_home(|_| Some(std::ffi::OsString::new())), None, "an empty home is no home");
    }

    #[test]
    fn a_first_start_with_no_book_takes_a_copy_of_the_python_app_s_database() {
        let person = tempfile::tempdir().unwrap();
        let python = python_folder(person.path());
        let before = listing(&python);
        let env = |k: &str| (k == if cfg!(windows) { "USERPROFILE" } else { "HOME" }).then(|| person.path().as_os_str().to_owned());
        let home = person.path().join(".bagholder-rust");
        std::fs::create_dir_all(&home).unwrap();
        let from = adopt_python_database(&home, &python_home(env).unwrap()).unwrap();
        assert_eq!(from, Some(python.join("bagholder.db")));
        // the start imports it as its own: the rows and the journal are the book's
        let at: jiff::Timestamp = "2026-09-26T12:00:00Z".parse().unwrap();
        crate::figures::Figures::open(&home, at).unwrap();
        let (book, _) = Book::open_in(&home, crate::app::APP_VERSION, at).unwrap();
        assert_eq!(book.record_count(&bagholder_book::import::import_source()).unwrap(), 1);
        assert!(listing(&python) == before, "the Python app's folder is only read");
        assert!(!home.join("bagholder.db.part").exists());
        // started again, there is a book: nothing more is taken
        let copied = std::fs::read(home.join("bagholder.db")).unwrap();
        {
            let conn = bagholder_store::open_db(&python.join("bagholder.db")).unwrap();
            trade(&conn, "b2", "ABC", "BUY", 1.0, "2026-02-02T15:00:00+00:00");
        }
        assert_eq!(adopt_python_database(&home, &python).unwrap(), None);
        assert_eq!(std::fs::read(home.join("bagholder.db")).unwrap(), copied);
    }

    #[test]
    fn a_python_app_that_is_running_is_copied_with_what_its_log_holds() {
        let person = tempfile::tempdir().unwrap();
        let python = python_folder(person.path());
        // the Python app open, a row in its write-ahead log and not yet in the file
        let running = bagholder_store::open_db(&python.join("bagholder.db")).unwrap();
        running.execute_batch("PRAGMA wal_autocheckpoint = 0").unwrap();
        trade(&running, "b2", "ABC", "BUY", 1.0, "2026-02-02T15:00:00+00:00");
        assert!(python.join("bagholder.db-wal").metadata().unwrap().len() > 0);
        let names = |d: &Path| { let mut n: Vec<String> = std::fs::read_dir(d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect(); n.sort(); n };
        let before = names(&python);
        let home = tempfile::tempdir().unwrap();
        adopt_python_database(home.path(), &python).unwrap().unwrap();
        let copy = rusqlite::Connection::open(home.path().join("bagholder.db")).unwrap();
        let ids: Vec<String> = copy.prepare("SELECT id FROM activities ORDER BY id").unwrap().query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(ids, ["b1", "b2"], "the row still in the log is copied");
        assert_eq!(names(&python), before, "nothing added to the Python app's folder");
        drop(running);
    }

    #[test]
    fn a_database_of_its_own_older_than_the_python_app_s_is_set_aside_and_the_python_app_s_taken() {
        let person = tempfile::tempdir().unwrap();
        let python = python_folder(person.path());
        // an earlier Rust build's database, never made into a book, last written days before
        let home = tempfile::tempdir().unwrap();
        let own = home.path().join("bagholder.db");
        std::fs::write(&own, b"days old").unwrap();
        std::fs::write(home.path().join("bagholder.db-wal"), b"its log").unwrap();
        let days_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(5 * 86_400);
        for f in ["bagholder.db", "bagholder.db-wal"] {
            std::fs::File::options().write(true).open(home.path().join(f)).unwrap().set_modified(days_ago).unwrap();
        }
        assert_eq!(adopt_python_database(home.path(), &python).unwrap(), Some(python.join("bagholder.db")));
        assert_eq!(std::fs::read(&own).unwrap(), std::fs::read(python.join("bagholder.db")).unwrap(), "the Python app's is the one taken");
        assert!(!home.path().join("bagholder.db-wal").exists(), "its own log went with it");
        let kept: Vec<Vec<u8>> = listing(&home.path().join("snapshots")).into_iter().map(|(_, bytes)| bytes).collect();
        assert_eq!(kept, [b"days old".to_vec(), b"its log".to_vec()], "set aside, not deleted");
    }

    #[test]
    fn a_folder_with_a_database_or_a_book_of_its_own_takes_nothing() {
        let person = tempfile::tempdir().unwrap();
        let python = python_folder(person.path());
        // a database of its own
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("bagholder.db"), b"its own").unwrap();
        assert_eq!(adopt_python_database(home.path(), &python).unwrap(), None);
        assert_eq!(std::fs::read(home.path().join("bagholder.db")).unwrap(), b"its own");
        // a book of its own
        let home = tempfile::tempdir().unwrap();
        crate::tests_common::pulled_book(home.path());
        assert_eq!(adopt_python_database(home.path(), &python).unwrap(), None);
        assert!(!home.path().join("bagholder.db").exists());
        // no Python app here
        let home = tempfile::tempdir().unwrap();
        assert_eq!(adopt_python_database(home.path(), &person.path().join("nowhere")).unwrap(), None);
        assert!(listing(home.path()).is_empty());
    }

    #[test]
    fn an_earlier_store_s_figure_tables_go_once_the_book_holds_them_and_the_file_as_it_was_is_kept() {
        let home = tempfile::tempdir().unwrap();
        let file = home.path().join("bagholder.db");
        {
            let conn = bagholder_store::open_db(&file).unwrap();
            bagholder_store::schema::init_schema(&conn).unwrap();
            trade(&conn, "b1", "QNC", "BUY", 10.0, "2026-01-02T15:00:00+00:00");
            set_meta(&conn, "journal_v2", r#"{"rt:b1": {"thesis": "kept", "tags": [], "grade": ""}}"#);
        }
        let at: jiff::Timestamp = "2026-09-25T12:00:00Z".parse().unwrap();
        let tables = |c: &Connection| -> Vec<String> { c.prepare("SELECT name FROM sqlite_master WHERE type = 'table'").unwrap().query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap() };
        // a book that has not imported it: refused, and nothing dropped
        let empty = tempfile::tempdir().unwrap();
        let (unimported, _) = Book::open_in(empty.path(), "test", at).unwrap();
        let conn = bagholder_store::open_db(&file).unwrap();
        assert!(retire_old_figures(home.path(), &conn, &unimported, at).unwrap_err().contains("1 activity rows"));
        assert!(tables(&conn).contains(&"activities".to_string()));
        drop(conn);
        // the first start: the book imports it, then its figure tables go
        import(&file, home.path(), at).unwrap();
        let (book, _) = Book::open_in(home.path(), "test", at).unwrap();
        let conn = bagholder_store::open_db(&file).unwrap();
        let snapshot = retire_old_figures(home.path(), &conn, &book, at).unwrap().expect("a snapshot");
        let left = tables(&conn);
        for t in bagholder_store::schema::FIGURE_TABLES {
            assert!(!left.contains(&t.to_string()), "{t} is still in the live file");
        }
        assert!(left.contains(&"orders".to_string()) && left.contains(&"meta".to_string()), "what the book did not take over stays");
        // the snapshot is the file as it was: its rows, for an import or the comparison
        let kept = Connection::open(&snapshot).unwrap();
        let n: i64 = kept.query_row("SELECT COUNT(*) FROM activities", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        // the next start: the schema and its repairs make none of them again, and there is nothing more to do
        bagholder_store::relabel::ensure(&conn).unwrap();
        assert!(!tables(&conn).contains(&"activities".to_string()));
        assert_eq!(retire_old_figures(home.path(), &conn, &book, at).unwrap(), None);
    }
}
