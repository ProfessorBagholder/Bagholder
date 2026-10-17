//! An instrument's names, kept as rows (`docs/plans/stage-3-a-change-on-a-deep-history.md`,
//! A, book migration 029).
//!
//! A name is each unbroken run of live records' sightings under one symbol and
//! venue, in the order of (day, symbol, sighting), across every instrument read
//! as one with it: the fold `Book::names` was. A sighting that states no venue
//! (a record that names the instrument by its reference: a hand entry, a fill,
//! a row imported with no exchange) says nothing about the venue: it reads as on
//! the venue the last sighting stating one named, so no record erases a venue
//! (brief 43). The book keeps the runs in
//! `instrument_names`; triggers leave a row in `names_due` for every sighting a
//! write changed, and `settle_names`, run before the outermost commit, re-forms
//! only the runs around those sightings, by index reads at their ends. A read
//! takes the kept runs only where nothing is due for any instrument of the
//! group; otherwise it folds, which is always right, only slower.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;

use bagholder_core::instrument::Name;
use bagholder_core::names::SourceName;
use bagholder_core::InstrumentId;
use rusqlite::{params, OptionalExtension};

use crate::identity::group_sql;
use crate::{text, Book, Result};

/// Where a sighting stands in the order names are read in.
type Key = (String, String, i64);

/// One run as it is kept: its first and last sighting, and its name.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Run {
    first: Key,
    last: Key,
    symbol: String,
    venue_mic: Option<String>,
    venue_name: Option<String>,
    name: Option<String>,
    source: String,
}

impl Run {
    fn name(&self) -> Result<Name> {
        Ok(Name {
            symbol: self.symbol.clone(),
            venue_mic: self.venue_mic.clone(),
            venue_name: self.venue_name.clone(),
            name: self.name.clone(),
            first_seen: text::date("instrument_names", "first_day", &self.first.0)?,
            last_seen: text::date("instrument_names", "last_day", &self.last.0)?,
            source: text::parsed("instrument_names", "source", &self.source, SourceName::parse)?,
        })
    }
}

/// What lies between two cut points of a kept run, or a sighting that changed.
#[derive(Debug)]
enum Piece {
    /// Part of a kept run with a live sighting left in it: its label, and its bounds.
    Kept { symbol: String, venue_mic: Option<String>, from: Bound<Key>, to: Bound<Key> },
    /// A changed sighting, live now, with its label; `states` where it states a
    /// venue (one that states none reads as on the piece before it).
    Seen { symbol: String, venue_mic: Option<String>, at: Key, states: bool },
}

impl Piece {
    fn label(&self) -> (&str, Option<&str>) {
        match self {
            Piece::Kept { symbol, venue_mic, .. } | Piece::Seen { symbol, venue_mic, .. } => (symbol, venue_mic.as_deref()),
        }
    }
}

/// The SQL comparing a sighting's place with a bound, and whether it binds a key.
fn bound_sql(b: &Bound<Key>, lower: bool, n: usize) -> String {
    let op = match (b, lower) {
        (Bound::Included(_), true) => ">=",
        (Bound::Excluded(_), true) => ">",
        (Bound::Included(_), false) => "<=",
        (Bound::Excluded(_), false) => "<",
        (Bound::Unbounded, _) => return "1".into(),
    };
    format!("(s.day, s.symbol, s.rowid) {op} (?{}, ?{}, ?{})", n, n + 1, n + 2)
}

/// Whether a sighting states a venue, by code or in words.
fn states_venue(mic: &Option<String>, words: &Option<String>) -> bool {
    mic.is_some() || words.is_some()
}

/// The venue a sighting stating none reads as on: the run's before it.
fn carried(before: Option<&Run>) -> (Option<String>, Option<String>) {
    before.map(|r| (r.venue_mic.clone(), r.venue_name.clone())).unwrap_or_default()
}

/// The SQL a live sighting that states a venue, or states none, is filtered by
/// (each the condition of a partial index, book migration 31).
const STATES_VENUE: &str = "(s.venue_mic IS NOT NULL OR s.venue_name IS NOT NULL)";
/// The SQL a live sighting that states its venue in words is filtered by.
const STATES_WORDS: &str = "s.venue_name IS NOT NULL";
const STATES_NO_VENUE: &str = "(s.venue_mic IS NULL AND s.venue_name IS NULL)";

fn bound_key(b: &Bound<Key>) -> Option<&Key> {
    match b {
        Bound::Included(k) | Bound::Excluded(k) => Some(k),
        Bound::Unbounded => None,
    }
}

impl Book {
    /// The instruments read as one with `head`, itself first.
    fn group_of(&self, head: InstrumentId) -> Result<Vec<String>> {
        let mut st = self.conn().prepare_cached(&format!("SELECT * FROM {}", group_sql("?1")))?;
        let ids = st.query_map([head.to_string()], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    /// Whether anything is due for an instrument of `head`'s group.
    fn names_due_for(&self, head: InstrumentId) -> Result<bool> {
        let sql = format!("SELECT EXISTS (SELECT 1 FROM names_due WHERE instrument_id IN {})", group_sql("?1"));
        Ok(self.conn().prepare_cached(&sql)?.query_row([head.to_string()], |r| r.get(0))?)
    }

    /// The live sightings' names of `head`'s group, folded: the reference the
    /// kept runs are held to.
    fn names_folded(&self, head: InstrumentId) -> Result<Vec<Run>> {
        let mut stmt = self.conn().prepare_cached(&format!(
            "SELECT s.symbol, s.venue_mic, s.venue_name, s.name, s.day, s.rowid, r.source FROM instrument_sightings s
             JOIN source_records r ON r.id = s.record_id AND r.state = 'live'
             WHERE s.instrument_id IN {} ORDER BY s.day, s.symbol, s.rowid",
            group_sql("?1")
        ))?;
        type Row = (String, Option<String>, Option<String>, Option<String>, String, i64, String);
        let rows = stmt.query_map([head.to_string()], |r| -> rusqlite::Result<Row> { Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)) })?;
        let mut out: Vec<Run> = Vec::new();
        for row in rows {
            let (symbol, venue_mic, venue_name, name, day, rowid, source) = row?;
            let key = (day, symbol.clone(), rowid);
            // one stating no venue reads as on the run before it, code and words
            let (venue_mic, venue_name) = if states_venue(&venue_mic, &venue_name) { (venue_mic, venue_name) } else { carried(out.last()) };
            match out.last_mut() {
                Some(last) if last.symbol == symbol && last.venue_mic == venue_mic => {
                    // the latest sighting's words stand for the run
                    last.last = key;
                    last.venue_name = venue_name.or(last.venue_name.take());
                    last.name = name.or(last.name.take());
                    last.source = source;
                }
                _ => out.push(Run { first: key.clone(), last: key, symbol, venue_mic, venue_name, name, source }),
            }
        }
        Ok(out)
    }

    /// The runs kept for `head`, in order.
    fn runs_kept(&self, head: InstrumentId) -> Result<Vec<Run>> {
        let mut st = self.conn().prepare_cached(
            "SELECT first_day, first_symbol, first_sighting, last_day, last_symbol, last_sighting, symbol, venue_mic, venue_name, name, source
             FROM instrument_names WHERE instrument_id = ?1 ORDER BY first_day, first_symbol, first_sighting",
        )?;
        let rows = st.query_map([head.to_string()], |r| {
            Ok(Run {
                first: (r.get(0)?, r.get(1)?, r.get(2)?),
                last: (r.get(3)?, r.get(4)?, r.get(5)?),
                symbol: r.get(6)?,
                venue_mic: r.get(7)?,
                venue_name: r.get(8)?,
                name: r.get(9)?,
                source: r.get(10)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// What the instrument was called, oldest first: each unbroken run of live
    /// records' sightings under one symbol and venue is one name, from its first
    /// day to its last. A ticker that changes and changes back is three names.
    /// The sightings of every instrument read as one with it count. Read from the
    /// kept runs: what is due is settled first, as a write's commit settles it (a
    /// read inside the write that changed them re-forms the runs around the
    /// change once, never folding the whole history on every read); folded only
    /// where its group's due rows cannot be settled.
    pub fn names(&self, id: InstrumentId) -> Result<Vec<Name>> {
        let head = self.canonical(id)?;
        if self.names_due_for(head)? {
            self.atomically(|| self.settle_names())?;
        }
        let runs = if self.names_due_for(head)? { self.names_folded(head)? } else { self.runs_kept(head)? };
        #[cfg(feature = "oracles")]
        if !bagholder_sqlite::work::measuring() {
            assert_eq!(runs, self.names_folded(head)?, "the names kept for {head} differ from its sightings folded");
        }
        let mut out = runs.iter().map(Run::name).collect::<Result<Vec<_>>>()?;
        if out.is_empty() && self.listing_named(id)?.is_none() {
            // neither a record nor the person's pick names it: what a source's own
            // description of it does, its own first
            type Row = (String, Option<String>, Option<String>, Option<String>, String, String);
            let sql = format!(
                "SELECT symbol, venue_mic, venue_name, name, day, source FROM instruments_described WHERE instrument_id IN {} ORDER BY instrument_id = ?1 DESC, day DESC, instrument_id LIMIT 1",
                group_sql("?1")
            );
            let found: Option<Row> = self.conn().query_row(&sql, [head.to_string()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))).optional()?;
            if let Some((symbol, venue_mic, venue_name, name, day_text, source)) = found {
                let seen = text::date("instruments_described", "day", &day_text)?;
                let source = text::parsed("instruments_described", "source", &source, SourceName::parse)?;
                out.push(Name { symbol, venue_mic, venue_name, name, first_seen: seen, last_seen: seen, source });
            }
        }
        Ok(out)
    }

    /// The names `names` reads, folded from the sightings whatever is kept: what a
    /// test holds the kept runs to.
    #[cfg(feature = "oracles")]
    #[doc(hidden)]
    pub fn names_as_folded(&self, id: InstrumentId) -> Result<Vec<Name>> {
        let head = self.canonical(id)?;
        self.names_folded(head)?.iter().map(Run::name).collect()
    }

    /// The names as kept, whatever is due: what a test holds to the fold once
    /// nothing is.
    #[cfg(feature = "oracles")]
    #[doc(hidden)]
    pub fn names_as_kept(&self, id: InstrumentId) -> Result<Vec<Name>> {
        let head = self.canonical(id)?;
        self.runs_kept(head)?.iter().map(Run::name).collect()
    }

    /// What the instrument is called now: the symbol of `names`' last. Where
    /// anything of its group is due (inside a write, before its commit settles
    /// it) the newest live sighting of each member is read through its index
    /// instead of folding every name the group had: a large import looks each of
    /// its rows' instruments up inside its own transaction.
    pub fn current_symbol(&self, id: InstrumentId) -> Result<Option<String>> {
        let head = self.canonical(id)?;
        if self.names_due_for(head)? {
            let mut st = self.conn().prepare_cached(
                "SELECT s.day, s.symbol, s.rowid FROM instrument_sightings s JOIN source_records r ON r.id = s.record_id AND r.state = 'live'
                 WHERE s.instrument_id = ?1 ORDER BY s.day DESC, s.symbol DESC, s.rowid DESC LIMIT 1",
            )?;
            let mut newest: Option<Key> = None;
            for m in self.group_of(head)? {
                let row: Option<Key> = st.query_row([m], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?;
                if row > newest {
                    newest = row;
                }
            }
            if let Some((_, symbol, _)) = newest {
                return Ok(Some(symbol));
            }
        }
        Ok(self.names(id)?.pop().map(|n| n.symbol))
    }

    /// Every due row settled: the runs around each changed sighting re-formed,
    /// one instrument at a time under its own savepoint, so one whose rows
    /// cannot be read fails alone, its error logged and its due rows kept. The
    /// instruments that failed.
    pub(crate) fn settle_names(&self) -> Result<Vec<InstrumentId>> {
        type Due = (i64, String, Option<String>, Option<String>, Option<i64>);
        let due: Vec<Due> = {
            let mut st = self.conn().prepare_cached("SELECT rowid, instrument_id, day, symbol, sighting FROM names_due")?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        if due.is_empty() {
            return Ok(vec![]);
        }
        // by the head each is read as now
        #[derive(Default)]
        struct Work {
            rows: Vec<i64>,
            whole: bool,
            keys: BTreeSet<Key>,
            /// Ids kept under that are not the head: runs kept under them go.
            others: BTreeSet<String>,
        }
        let mut by_head: BTreeMap<InstrumentId, Work> = BTreeMap::new();
        let mut unreadable: Vec<i64> = Vec::new();
        let mut failed = BTreeSet::new();
        for (rowid, inst, day, symbol, sighting) in due {
            let Ok(id) = InstrumentId::parse(&inst) else {
                // an id that names nothing an instrument can be: nothing to keep for it
                unreadable.push(rowid);
                continue;
            };
            // a join that cannot be read fails this instrument alone: its due rows
            // are kept and its reads fold, as a head that fails below
            let head = match self.canonical(id) {
                Ok(h) => h,
                Err(e) => {
                    if failed.insert(id) {
                        bagholder_core::log::line(&format!("bagholder: the names of instrument {id} could not be kept (they are read from its sightings until they can): {e}"));
                    }
                    continue;
                }
            };
            let w = by_head.entry(head).or_default();
            w.rows.push(rowid);
            if head != id {
                w.others.insert(inst.clone());
            }
            match (day, symbol, sighting) {
                (Some(d), Some(s), Some(n)) => {
                    w.keys.insert((d, s, n));
                }
                _ => {
                    // a succession joined or parted: the id may have been a head
                    // with runs of its own
                    w.whole = true;
                    w.others.insert(inst);
                }
            }
        }
        let forget = |rows: &[i64]| -> Result<()> {
            let mut st = self.conn().prepare_cached("DELETE FROM names_due WHERE rowid = ?1")?;
            for r in rows {
                st.execute([r])?;
            }
            Ok(())
        };
        forget(&unreadable)?;
        for (head, w) in by_head {
            self.conn().execute_batch("SAVEPOINT names_settle")?;
            let done = (|| -> Result<()> {
                for other in &w.others {
                    if *other != head.to_string() {
                        self.conn().prepare_cached("DELETE FROM instrument_names WHERE instrument_id = ?1")?.execute([other])?;
                    }
                }
                let exists: bool = self.conn().prepare_cached("SELECT EXISTS (SELECT 1 FROM instruments WHERE id = ?1)")?.query_row([head.to_string()], |r| r.get(0))?;
                if !exists {
                    self.conn().prepare_cached("DELETE FROM instrument_names WHERE instrument_id = ?1")?.execute([head.to_string()])?;
                } else if w.whole {
                    let runs = self.names_folded(head)?;
                    self.conn().prepare_cached("DELETE FROM instrument_names WHERE instrument_id = ?1")?.execute([head.to_string()])?;
                    self.keep_runs(head, &runs)?;
                } else {
                    self.reform(head, &w.keys)?;
                }
                forget(&w.rows)
            })();
            match done {
                Ok(()) => self.conn().execute_batch("RELEASE names_settle")?,
                Err(e) => {
                    self.conn().execute_batch("ROLLBACK TO names_settle; RELEASE names_settle")?;
                    bagholder_core::log::line(&format!("bagholder: the names of instrument {head} could not be kept (they are read from its sightings until they can): {e}"));
                    failed.insert(head);
                }
            }
        }
        Ok(failed.into_iter().collect())
    }

    fn keep_runs(&self, head: InstrumentId, runs: &[Run]) -> Result<()> {
        let mut st = self.conn().prepare_cached(
            "INSERT INTO instrument_names (instrument_id, first_day, first_symbol, first_sighting, last_day, last_symbol, last_sighting, symbol, venue_mic, venue_name, name, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )?;
        for r in runs {
            st.execute(params![head.to_string(), r.first.0, r.first.1, r.first.2, r.last.0, r.last.1, r.last.2, r.symbol, r.venue_mic, r.venue_name, r.name, r.source])?;
        }
        Ok(())
    }

    /// The first (or last) live sighting of the group between two bounds.
    fn live_end(&self, members: &[String], from: &Bound<Key>, to: &Bound<Key>, last: bool, only: Option<&str>) -> Result<Option<(Key, String, Option<String>, Option<String>)>> {
        let order = if last { "DESC" } else { "ASC" };
        let only_sql = match only {
            Some(column) => format!(" AND s.{column} IS NOT NULL"),
            None => String::new(),
        };
        let sql = format!(
            "SELECT s.day, s.symbol, s.rowid, r.source, s.venue_name, s.name FROM instrument_sightings s
             JOIN source_records r ON r.id = s.record_id AND r.state = 'live'
             WHERE s.instrument_id = ?1 AND {} AND {}{only_sql}
             ORDER BY s.day {order}, s.symbol {order}, s.rowid {order} LIMIT 1",
            bound_sql(from, true, 2),
            bound_sql(to, false, 5),
        );
        let mut st = self.conn().prepare_cached(&sql)?;
        let mut best: Option<(Key, String, Option<String>, Option<String>)> = None;
        for m in members {
            let mut values: Vec<rusqlite::types::Value> = vec![m.clone().into()];
            for b in [from, to] {
                if let Some(k) = bound_key(b) {
                    values.extend([k.0.clone().into(), k.1.clone().into(), k.2.into()]);
                } else {
                    values.extend([rusqlite::types::Value::Null, rusqlite::types::Value::Null, rusqlite::types::Value::Null]);
                }
            }
            let row = st
                .query_row(rusqlite::params_from_iter(values), |r| Ok(((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?), r.get::<_, String>(3)?, r.get(4)?, r.get(5)?)))
                .optional()?;
            if let Some(row) = row {
                let better = match &best {
                    None => true,
                    Some(b) => (row.0 > b.0) == last,
                };
                if better {
                    best = Some(row);
                }
            }
        }
        Ok(best)
    }

    /// The first (or last) live sighting of the group between two bounds that
    /// `filter` keeps: its place, its code and its words.
    fn seek(&self, members: &[String], from: &Bound<Key>, to: &Bound<Key>, last: bool, filter: &str) -> Result<Option<(Key, Option<String>, Option<String>)>> {
        let order = if last { "DESC" } else { "ASC" };
        let sql = format!(
            "SELECT s.day, s.symbol, s.rowid, s.venue_mic, s.venue_name FROM instrument_sightings s
             JOIN source_records r ON r.id = s.record_id AND r.state = 'live'
             WHERE s.instrument_id = ?1 AND {} AND {} AND {filter}
             ORDER BY s.day {order}, s.symbol {order}, s.rowid {order} LIMIT 1",
            bound_sql(from, true, 2),
            bound_sql(to, false, 5),
        );
        let mut st = self.conn().prepare_cached(&sql)?;
        let mut best: Option<(Key, Option<String>, Option<String>)> = None;
        for m in members {
            let mut values: Vec<rusqlite::types::Value> = vec![m.clone().into()];
            for b in [from, to] {
                if let Some(k) = bound_key(b) {
                    values.extend([k.0.clone().into(), k.1.clone().into(), k.2.into()]);
                } else {
                    values.extend([rusqlite::types::Value::Null, rusqlite::types::Value::Null, rusqlite::types::Value::Null]);
                }
            }
            // as many values as the statement numbers: an open upper end numbers none past the lower's
            if matches!(to, Bound::Unbounded) {
                values.truncate(if matches!(from, Bound::Unbounded) { 1 } else { 4 });
            }
            let row = st.query_row(rusqlite::params_from_iter(values), |r| Ok(((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?), r.get(3)?, r.get(4)?))).optional()?;
            if let Some(row) = row {
                if best.as_ref().is_none_or(|b| (row.0 > b.0) == last) {
                    best = Some(row);
                }
            }
        }
        Ok(best)
    }

    /// The runs around each of `keys` re-formed: the kept runs that hold or
    /// border one, cut at each into pieces (each kept where a live sighting is
    /// left in it), the changed sightings that are live now put between them,
    /// pieces of one symbol and venue joined, and each run's ends, latest words
    /// and source read by seeks.
    fn reform(&self, head: InstrumentId, keys: &BTreeSet<Key>) -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let members = self.group_of(head)?;
        let runs = self.runs_kept(head)?;
        // every changed sighting placed after the last run: each one live extends
        // that run where it is of the same symbol and venue, and starts the next
        // otherwise, as the fold would go on from there; one not live changes nothing
        if runs.last().is_none_or(|r| keys.iter().next().is_some_and(|k| *k > r.last)) {
            return self.append_runs(head, &members, runs.last().cloned(), keys);
        }
        // a sighting stating no venue reads as on what comes before it, so a change
        // reaches every such sighting after it up to the next one that states a venue
        // (its stop, where the carried code ends, and the pieces are cut): one seek a change
        let mut stops: BTreeSet<Key> = BTreeSet::new();
        let mut reach: Vec<Option<Key>> = Vec::new();
        for k in keys {
            let stop = self.seek(&members, &Bound::Excluded(k.clone()), &Bound::Unbounded, false, STATES_VENUE)?.map(|x| x.0);
            if let Some(s) = stop.clone().filter(|s| !keys.contains(s)) {
                stops.insert(s);
            }
            reach.push(stop);
        }
        // the runs each key reaches: the one holding it and one either side, on to the
        // one holding its stop (every run to the last where it has none), and on past
        // it through each run that reads its words from the run before
        let mut reached: Vec<(usize, usize)> = Vec::new();
        for (k, stop) in keys.iter().zip(&reach) {
            let p = runs.partition_point(|r| r.last < *k);
            let (a, mut b) = if p < runs.len() && runs[p].first <= *k { (p.saturating_sub(1), (p + 1).min(runs.len().saturating_sub(1))) } else { (p.saturating_sub(1), p.min(runs.len().saturating_sub(1))) };
            if !runs.is_empty() {
                let to = match stop {
                    Some(s) => runs.partition_point(|r| r.last < *s).min(runs.len() - 1),
                    None => runs.len() - 1,
                };
                let mut to = b.max(to);
                while to + 1 < runs.len() && self.reads_words_before(&members, &runs[to + 1])? {
                    to += 1;
                }
                b = to;
            }
            reached.push((a, b));
        }
        if runs.is_empty() {
            let pieces = self.pieces(&members, None, &[], keys, &stops)?;
            return self.replace_runs(head, &members, None, &[], pieces);
        }
        reached.sort();
        let mut spans: Vec<(usize, usize)> = Vec::new();
        for (a, b) in reached {
            match spans.last_mut() {
                Some(last) if a <= last.1 + 1 => last.1 = last.1.max(b),
                _ => spans.push((a, b)),
            }
        }
        for (i, (a, b)) in spans.iter().enumerate() {
            // the keys this span covers: from the gap before its first run (none
            // before the first span) to the gap after its last (all after the last)
            let lo: Bound<&Key> = if i == 0 && *a == 0 { Bound::Unbounded } else { Bound::Included(&runs[*a].first) };
            let hi: Bound<&Key> = if i + 1 == spans.len() && *b == runs.len() - 1 { Bound::Unbounded } else { Bound::Included(&runs[*b].last) };
            let span_keys: BTreeSet<Key> = keys.range::<Key, _>((lo.map(|k| k.clone()), hi.map(|k| k.clone()))).cloned().collect();
            if span_keys.is_empty() {
                continue;
            }
            let kept = &runs[*a..=*b];
            let before = a.checked_sub(1).map(|i| &runs[i]);
            let span_stops: BTreeSet<Key> = stops.range::<Key, _>((lo.map(|k| k.clone()), hi.map(|k| k.clone()))).cloned().collect();
            let pieces = self.pieces(&members, before, kept, &span_keys, &span_stops)?;
            self.replace_runs(head, &members, before, kept, pieces)?;
        }
        Ok(())
    }

    /// The changed sightings `keys`, every one after `last`, folded on from it.
    fn append_runs(&self, head: InstrumentId, members: &[String], last: Option<Run>, keys: &BTreeSet<Key>) -> Result<()> {
        let mut st = self.conn().prepare_cached(
            "SELECT s.instrument_id, s.day, s.symbol, s.venue_mic, s.venue_name, s.name, r.source FROM instrument_sightings s
             JOIN source_records r ON r.id = s.record_id AND r.state = 'live' WHERE s.rowid = ?1",
        )?;
        type Row = (String, String, String, Option<String>, Option<String>, Option<String>, String);
        let first_kept = last.as_ref().map(|r| r.first.clone());
        let mut runs: Vec<Run> = last.into_iter().collect();
        let had = runs.len();
        for k in keys {
            let row: Option<Row> = st.query_row([k.2], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))).optional()?;
            let Some((inst, day, symbol, venue_mic, venue_name, name, source)) = row else { continue };
            if !members.contains(&inst) || day != k.0 || symbol != k.1 {
                continue;
            }
            let (venue_mic, venue_name) = if states_venue(&venue_mic, &venue_name) { (venue_mic, venue_name) } else { carried(runs.last()) };
            match runs.last_mut() {
                Some(r) if r.symbol == symbol && r.venue_mic == venue_mic => {
                    r.last = k.clone();
                    r.venue_name = venue_name.or(r.venue_name.take());
                    r.name = name.or(r.name.take());
                    r.source = source;
                }
                _ => runs.push(Run { first: k.clone(), last: k.clone(), symbol, venue_mic, venue_name, name, source }),
            }
        }
        if had == 1 {
            // the last run kept, as it now reads
            let r = &runs[0];
            let first = first_kept.unwrap_or_default();
            self.conn()
                .prepare_cached(
                    "UPDATE instrument_names SET last_day = ?5, last_symbol = ?6, last_sighting = ?7, venue_name = ?8, name = ?9, source = ?10
                     WHERE instrument_id = ?1 AND first_day = ?2 AND first_symbol = ?3 AND first_sighting = ?4
                     AND (last_day, last_symbol, last_sighting, venue_name, name, source) IS NOT (?5, ?6, ?7, ?8, ?9, ?10)",
                )?
                .execute(params![head.to_string(), first.0, first.1, first.2, r.last.0, r.last.1, r.last.2, r.venue_name, r.name, r.source])?;
        }
        self.keep_runs(head, &runs[had..])
    }

    /// The pieces, in order, of `kept` cut at `keys` and at `stops`, with the
    /// changed sightings live now between them, each labelled as the fold reads it:
    /// a changed sighting that states no venue reads as on the piece before it, and
    /// so does every piece after a change up to a stop (the next sighting that
    /// states a venue), the run before the span (`before`) first.
    fn pieces(&self, members: &[String], before: Option<&Run>, kept: &[Run], keys: &BTreeSet<Key>, stops: &BTreeSet<Key>) -> Result<Vec<Piece>> {
        // (place, rank, piece or none for a change's mark): at a key, its sighting,
        // then the mark that a change was passed, then what follows it in its run
        let mut out: Vec<(Key, u8, Option<Piece>)> = Vec::new();
        for r in kept {
            let mut cuts: Vec<(&Key, bool)> = keys.range(r.first.clone()..=r.last.clone()).map(|k| (k, true)).collect();
            cuts.extend(stops.range(r.first.clone()..=r.last.clone()).filter(|s| **s != r.first).map(|s| (s, false)));
            cuts.sort();
            let mut from = Bound::Included(r.first.clone());
            let ends = cuts.iter().map(|(c, _)| Bound::Excluded((*c).clone())).chain(std::iter::once(Bound::Included(r.last.clone())));
            for (i, to) in ends.enumerate() {
                let empty = match (&from, &to) {
                    (Bound::Included(a), Bound::Excluded(b)) | (Bound::Excluded(a), Bound::Excluded(b)) | (Bound::Excluded(a), Bound::Included(b)) => a >= b,
                    (Bound::Included(a), Bound::Included(b)) => a > b,
                    _ => false,
                };
                if !empty && self.live_end(members, &from, &to, false, None)?.is_some() {
                    let at = bound_key(&from).cloned().unwrap_or_default();
                    let rank = if matches!(from, Bound::Excluded(_)) { 2 } else { 0 };
                    out.push((at, rank, Some(Piece::Kept { symbol: r.symbol.clone(), venue_mic: r.venue_mic.clone(), from: from.clone(), to: to.clone() })));
                }
                // after a changed key the next piece starts past it; at a stop, on it
                if let Some((c, is_key)) = cuts.get(i) {
                    from = if *is_key { Bound::Excluded((*c).clone()) } else { Bound::Included((*c).clone()) };
                }
            }
        }
        // each changed sighting as it reads now, where it is live and still at its key
        let mut st = self.conn().prepare_cached(
            "SELECT s.instrument_id, s.day, s.symbol, s.venue_mic, s.venue_name FROM instrument_sightings s
             JOIN source_records r ON r.id = s.record_id AND r.state = 'live' WHERE s.rowid = ?1",
        )?;
        for k in keys {
            out.push((k.clone(), 1, None));
            let row: Option<(String, String, String, Option<String>, Option<String>)> = st.query_row([k.2], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).optional()?;
            if let Some((inst, day, symbol, venue_mic, venue_name)) = row {
                if members.contains(&inst) && day == k.0 && symbol == k.1 {
                    let states = states_venue(&venue_mic, &venue_name);
                    out.push((k.clone(), 0, Some(Piece::Seen { symbol, venue_mic, at: k.clone(), states })));
                }
            }
        }
        out.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
        // the venue each piece reads as on, in order
        let mut prev: Option<String> = before.and_then(|r| r.venue_mic.clone());
        let mut carrying = false;
        let mut pieces = Vec::new();
        for (_, _, p) in out {
            let Some(mut p) = p else {
                carrying = true;
                continue;
            };
            match &mut p {
                Piece::Seen { venue_mic, states: false, .. } => *venue_mic = prev.clone(),
                Piece::Seen { .. } => {}
                Piece::Kept { venue_mic, from, .. } => {
                    if matches!(from, Bound::Included(s) if stops.contains(s)) {
                        carrying = false;
                    }
                    if carrying {
                        *venue_mic = prev.clone();
                    }
                }
            }
            prev = p.label().1.map(str::to_string);
            pieces.push(p);
        }
        Ok(pieces)
    }

    /// Whether a kept run reads its venue in words from the run before it: its
    /// sightings state no words and its first states no venue (`replace_runs`). Two
    /// seeks, on the sightings indexed by what they state.
    fn reads_words_before(&self, members: &[String], run: &Run) -> Result<bool> {
        let (first, last) = (Bound::Included(run.first.clone()), Bound::Included(run.last.clone()));
        Ok(self.seek(members, &first, &first, false, STATES_NO_VENUE)?.is_some() && self.seek(members, &first, &last, false, STATES_WORDS)?.is_none())
    }

    /// `kept` replaced by the runs `pieces` make.
    fn replace_runs(&self, head: InstrumentId, members: &[String], before: Option<&Run>, kept: &[Run], pieces: Vec<Piece>) -> Result<()> {
        let mut groups: Vec<Vec<Piece>> = Vec::new();
        for p in pieces {
            match groups.last_mut() {
                Some(g) if g.last().is_some_and(|l| l.label() == p.label()) => g.push(p),
                _ => groups.push(vec![p]),
            }
        }
        let mut runs: Vec<Run> = Vec::new();
        for g in groups {
            let end = |p: &Piece, last: bool| -> Result<Key> {
                match p {
                    Piece::Seen { at, .. } => Ok(at.clone()),
                    Piece::Kept { from, to, .. } => Ok(self.live_end(members, from, to, last, None)?.map(|x| x.0).unwrap_or_default()),
                }
            };
            let first = end(&g[0], false)?;
            let last = end(&g[g.len() - 1], true)?;
            let (from, to) = (Bound::Included(first.clone()), Bound::Included(last.clone()));
            let source = self.live_end(members, &from, &to, true, None)?.map(|x| x.1).unwrap_or_default();
            let (symbol, venue_mic) = g[0].label();
            // a run whose sightings state no words and whose first states no venue reads
            // as on the run before it, in its words too
            let prior = runs.last().or(before);
            let venue_name = match self.live_end(members, &from, &to, true, Some("venue_name"))?.and_then(|x| x.2) {
                Some(w) => Some(w),
                None if self.seek(members, &from, &Bound::Included(first.clone()), false, STATES_NO_VENUE)?.is_some() => prior.and_then(|r| r.venue_name.clone()),
                None => None,
            };
            let name = self.live_end(members, &from, &to, true, Some("name"))?.and_then(|x| x.3);
            runs.push(Run { first, last, symbol: symbol.to_string(), venue_mic: venue_mic.map(str::to_string), venue_name, name, source });
        }
        let mut del = self.conn().prepare_cached("DELETE FROM instrument_names WHERE instrument_id = ?1 AND first_day = ?2 AND first_symbol = ?3 AND first_sighting = ?4")?;
        for r in kept {
            del.execute(params![head.to_string(), r.first.0, r.first.1, r.first.2])?;
        }
        self.keep_runs(head, &runs)
    }
}
