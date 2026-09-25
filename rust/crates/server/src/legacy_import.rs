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
            return Ok(row.to_string()); // a round trip is named for its first fill
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
    let _ = std::fs::remove_file(&copy);
    result
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
