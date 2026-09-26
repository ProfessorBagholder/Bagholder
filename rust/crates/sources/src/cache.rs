//! The market cache (`docs/plans/stage-3a-sources.md`, "The market cache"): the
//! second store, `market.db` in the data folder. What the sources answered that
//! can be asked again: quotes, daily closes, the benchmark trackers' closes and
//! events, the option chain last read per underlying, which source won for each
//! instrument, and every request's outcome. Every read and write is
//! typed; a stored value that does not read back is an error naming its table
//! and column, never a default.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;
use std::time::Duration;

use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, Dec, InstrumentId, Money, SourceName};
use bagholder_sqlite::migrate::{self, Migrated, MigrateError, Migration, Schema};
use rusqlite::{params, Connection, OptionalExtension};

use crate::contract::{Benchmark, DataKind};
use crate::outcome::OutcomeKind;

pub static MIGRATIONS: [Migration; 3] = [
    Migration { number: 1, name: "the market cache", sql: include_str!("../migrations/001-the-market-cache.sql") },
    Migration { number: 2, name: "reads", sql: include_str!("../migrations/002-reads.sql") },
    Migration { number: 3, name: "benchmark trackers and option chains", sql: include_str!("../migrations/003-benchmark-trackers-and-option-chains.sql") },
];

pub static SCHEMA: Schema = Schema {
    name: "cache",
    // "BHMC" in the file's header: the market cache, never the book
    application_id: 0x4248_4D43,
    migrations: &MIGRATIONS,
};

/// How many outcomes each source keeps beyond the newest of each kind.
pub const OUTCOMES_KEPT: i64 = 1000;

#[derive(Debug)]
pub enum CacheError {
    Open(MigrateError),
    Sqlite(rusqlite::Error),
    /// A stored value that does not read as what its column holds.
    Corrupt { table: &'static str, column: &'static str, value: String, why: String },
}

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CacheError::Open(e) => write!(f, "the market cache: {e}"),
            CacheError::Sqlite(e) => write!(f, "the market cache: {e}"),
            CacheError::Corrupt { table, column, value, why } => write!(f, "the market cache's {table}.{column} holds {value:?}: {why}"),
        }
    }
}

impl std::error::Error for CacheError {}

impl From<rusqlite::Error> for CacheError {
    fn from(e: rusqlite::Error) -> Self {
        CacheError::Sqlite(e)
    }
}

pub type Result<T> = std::result::Result<T, CacheError>;

fn corrupt(table: &'static str, column: &'static str, value: &str, why: impl ToString) -> CacheError {
    CacheError::Corrupt { table, column, value: value.to_string(), why: why.to_string() }
}

fn dec(table: &'static str, column: &'static str, v: &str) -> Result<Dec> {
    let d = Dec::parse(v).map_err(|e| corrupt(table, column, v, e))?;
    if d.to_text() != v {
        return Err(corrupt(table, column, v, "not in the form the cache writes"));
    }
    Ok(d)
}

fn opt_dec(table: &'static str, column: &'static str, v: Option<String>) -> Result<Option<Dec>> {
    v.map(|s| dec(table, column, &s)).transpose()
}

fn day(table: &'static str, column: &'static str, v: &str) -> Result<Date> {
    crate::reply::day_from(v).map_err(|e| corrupt(table, column, v, e))
}

fn instant(table: &'static str, column: &'static str, v: &str) -> Result<Timestamp> {
    v.parse().map_err(|e| corrupt(table, column, v, e))
}

fn currency(table: &'static str, column: &'static str, v: &str) -> Result<Currency> {
    Currency::parse(v).map_err(|e| corrupt(table, column, v, e))
}

fn instrument(table: &'static str, column: &'static str, v: &str) -> Result<InstrumentId> {
    InstrumentId::parse(v).map_err(|e| corrupt(table, column, v, e))
}

fn source(table: &'static str, column: &'static str, v: &str) -> Result<SourceName> {
    SourceName::parse(v).map_err(|e| corrupt(table, column, v, e))
}

/// A quote as the cache keeps it: the latest per instrument and source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredQuote {
    pub instrument: InstrumentId,
    pub source: SourceName,
    pub price: Money,
    pub change: Option<Dec>,
    pub change_pct: Option<Dec>,
    /// When the source says the price was current: a trade's time where the
    /// source states one (Yahoo, Cboe Canada, a contract's last trade), else the
    /// time the source served it (TMX) or made it (a chain, less its delay).
    pub quoted_at: Timestamp,
    /// How much older than `quoted_at` the price may be, where the source does
    /// not state its time exactly.
    pub allowance: Duration,
    pub received_at: Timestamp,
}

/// A day's value a source sent that differs from the one already standing: the
/// first stands, and this is recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disagreement {
    pub day: Date,
    pub stands: Dec,
    pub later: Dec,
    pub source: SourceName,
}

/// A benchmark tracker's event, as its source states it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackerEvent {
    /// Per unit, by its ex-date.
    Dividend(Dec),
    /// `numerator` new units for every `denominator` held, the day it took effect.
    Split { numerator: Dec, denominator: Dec },
}

/// A later statement of a tracker's event that differs from the one standing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventDisagreement {
    pub day: Date,
    pub kind: &'static str,
    pub stands: String,
    pub later: String,
}

/// A tracker's standing closes and events.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackerSeries {
    pub closes: BTreeMap<Date, Dec>,
    pub dividends: BTreeMap<Date, Dec>,
    pub splits: BTreeMap<Date, (Dec, Dec)>,
}

/// The option chain last read for an underlying, as it stated itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainRead {
    pub underlying: String,
    /// The day of the underlying's last trade: the session the chain carries.
    pub session: Date,
    /// When Cboe made it.
    pub made_at: Timestamp,
    /// Its `Last-Modified`, sent back with the next read.
    pub last_modified: Option<String>,
    pub received_at: Timestamp,
}

/// One read of a subject: the days it settled, how it ended, and when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadRow {
    pub source: SourceName,
    pub first: Date,
    pub last: Date,
    pub outcome: OutcomeKind,
    pub at: Timestamp,
}

/// One request's outcome, as the cache keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutcomeRow {
    pub source: SourceName,
    pub host: String,
    pub kind: DataKind,
    pub instrument: Option<InstrumentId>,
    pub outcome: OutcomeKind,
    pub detail: String,
    pub shape_change: Option<String>,
    pub at: Timestamp,
}

pub struct MarketCache {
    conn: Connection,
}

impl MarketCache {
    /// Open the cache at `path` (made when it does not exist), bringing it to this
    /// build's schema; `at` is now and names a snapshot taken before a migration.
    pub fn open(path: &Path, app_version: &str, at: Timestamp) -> Result<(MarketCache, Migrated)> {
        let (conn, done) = migrate::open(&SCHEMA, path, app_version, at).map_err(CacheError::Open)?;
        Ok((MarketCache { conn }, done))
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Have `heard` run at each commit on this connection. It runs inside
    /// SQLite's commit, on the writer's thread: it must only signal a waiter and
    /// return, never touch the database.
    pub fn on_commit(&self, heard: std::sync::Arc<dyn Fn() + Send + Sync>) {
        self.conn.commit_hook(Some(move || {
            heard();
            false // never veto the commit
        }));
    }

    // -- quotes ----------------------------------------------------------------

    /// Keep a quote as the latest from its source for its instrument.
    pub fn store_quote(&self, q: &StoredQuote) -> Result<()> {
        self.conn.execute(
            "INSERT INTO quotes (instrument_id, source, price, currency, change, change_pct, quoted_at, allowance_secs, received_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT (instrument_id, source) DO UPDATE SET price = ?3, currency = ?4, change = ?5, change_pct = ?6, quoted_at = ?7, allowance_secs = ?8, received_at = ?9",
            params![
                q.instrument.to_string(),
                q.source.as_str(),
                q.price.amount.to_text(),
                q.price.currency.as_str(),
                q.change.map(|d| d.to_text()),
                q.change_pct.map(|d| d.to_text()),
                q.quoted_at.to_string(),
                i64::try_from(q.allowance.as_secs()).unwrap_or(i64::MAX),
                q.received_at.to_string()
            ],
        )?;
        Ok(())
    }

    pub fn quotes(&self) -> Result<Vec<StoredQuote>> {
        const T: &str = "quotes";
        let mut stmt = self.conn.prepare("SELECT instrument_id, source, price, currency, change, change_pct, quoted_at, allowance_secs, received_at FROM quotes ORDER BY instrument_id, source")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, Option<String>>(5)?, r.get::<_, String>(6)?, r.get::<_, i64>(7)?, r.get::<_, String>(8)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (i, s, p, c, ch, pct, q, allow, rec) = row?;
            out.push(StoredQuote {
                instrument: instrument(T, "instrument_id", &i)?,
                source: source(T, "source", &s)?,
                price: Money::new(dec(T, "price", &p)?, currency(T, "currency", &c)?),
                change: opt_dec(T, "change", ch)?,
                change_pct: opt_dec(T, "change_pct", pct)?,
                quoted_at: instant(T, "quoted_at", &q)?,
                allowance: Duration::from_secs(u64::try_from(allow).map_err(|_| corrupt(T, "allowance_secs", &allow.to_string(), "negative"))?),
                received_at: instant(T, "received_at", &rec)?,
            });
        }
        Ok(out)
    }

    // -- daily closes ----------------------------------------------------------

    /// Keep a listing's or a coin's closes, as traded, in `price_currency`. A day
    /// with no close yet takes this one; a day that has one keeps it, and a
    /// different value is kept beside it and returned.
    pub fn store_closes(&self, id: InstrumentId, closes: &[(Date, Dec)], price_currency: Currency, from: &SourceName, at: Timestamp) -> Result<Vec<Disagreement>> {
        let mut disagreements = Vec::new();
        bagholder_sqlite::atomically(&self.conn, || {
            for (d, close) in closes {
                let standing: Option<(String, String)> = self
                    .conn
                    .query_row("SELECT close, currency FROM daily_closes WHERE instrument_id = ?1 AND day = ?2 AND first = 1", params![id.to_string(), d.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))
                    .optional()?;
                let first = match &standing {
                    None => true,
                    Some((c, cur)) if *c == close.to_text() && cur == price_currency.as_str() => continue,
                    Some(_) => false,
                };
                self.conn.execute(
                    "INSERT OR IGNORE INTO daily_closes (instrument_id, day, close, currency, source, first, received_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![id.to_string(), d.to_string(), close.to_text(), price_currency.as_str(), from.as_str(), first as i64, at.to_string()],
                )?;
                if let Some((c, _)) = standing {
                    disagreements.push((*d, c, *close));
                }
            }
            Ok(())
        })?;
        disagreements.into_iter().map(|(d, c, later)| Ok(Disagreement { day: d, stands: dec("daily_closes", "close", &c)?, later, source: from.clone() })).collect()
    }

    /// The standing close of every instrument on every day, in its currency.
    pub fn closes(&self) -> Result<BTreeMap<InstrumentId, BTreeMap<Date, Money>>> {
        const T: &str = "daily_closes";
        let mut stmt = self.conn.prepare("SELECT instrument_id, day, close, currency FROM daily_closes WHERE first = 1")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?;
        let mut out: BTreeMap<InstrumentId, BTreeMap<Date, Money>> = BTreeMap::new();
        for row in rows {
            let (i, d, c, cur) = row?;
            out.entry(instrument(T, "instrument_id", &i)?).or_default().insert(day(T, "day", &d)?, Money::new(dec(T, "close", &c)?, currency(T, "currency", &cur)?));
        }
        Ok(out)
    }

    /// The last day an instrument has a close for.
    pub fn last_close_day(&self, id: InstrumentId) -> Result<Option<Date>> {
        let d: Option<String> = self.conn.query_row("SELECT MAX(day) FROM daily_closes WHERE instrument_id = ?1", params![id.to_string()], |r| r.get(0))?;
        d.map(|d| day("daily_closes", "day", &d)).transpose()
    }

    /// The days an instrument has a close for.
    pub fn close_days(&self, id: InstrumentId) -> Result<BTreeSet<Date>> {
        let mut stmt = self.conn.prepare("SELECT day FROM daily_closes WHERE instrument_id = ?1 AND first = 1")?;
        let rows = stmt.query_map(params![id.to_string()], |r| r.get::<_, String>(0))?;
        let mut out = BTreeSet::new();
        for d in rows {
            out.insert(day("daily_closes", "day", &d?)?);
        }
        Ok(out)
    }

    // -- benchmarks ------------------------------------------------------------

    /// Keep a tracker's closes, a closed day written once as an instrument's are.
    pub fn store_benchmark_closes(&self, b: Benchmark, closes: &[(Date, Dec)], from: &SourceName, at: Timestamp) -> Result<Vec<Disagreement>> {
        let mut disagreements = Vec::new();
        bagholder_sqlite::atomically(&self.conn, || {
            for (d, close) in closes {
                let standing: Option<String> = self.conn.query_row("SELECT close FROM benchmark_closes WHERE benchmark = ?1 AND day = ?2 AND first = 1", params![b.key(), d.to_string()], |r| r.get(0)).optional()?;
                let first = match &standing {
                    None => true,
                    Some(c) if *c == close.to_text() => continue,
                    Some(_) => false,
                };
                self.conn.execute(
                    "INSERT OR IGNORE INTO benchmark_closes (benchmark, day, close, source, first, received_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![b.key(), d.to_string(), close.to_text(), from.as_str(), first as i64, at.to_string()],
                )?;
                if let Some(c) = standing {
                    disagreements.push((*d, c, *close));
                }
            }
            Ok(())
        })?;
        disagreements.into_iter().map(|(d, c, later)| Ok(Disagreement { day: d, stands: dec("benchmark_closes", "close", &c)?, later, source: from.clone() })).collect()
    }

    /// Keep a tracker's dividends and splits, each written once: a later
    /// different statement of the same day's event is kept beside it and
    /// returned, and the first stands.
    pub fn store_benchmark_events(&self, b: Benchmark, events: &[(Date, TrackerEvent)], from: &SourceName, at: Timestamp) -> Result<Vec<EventDisagreement>> {
        let mut out = Vec::new();
        bagholder_sqlite::atomically(&self.conn, || {
            for (d, e) in events {
                let (kind, amount, denominator) = match e {
                    TrackerEvent::Dividend(a) => ("dividend", a.to_text(), None),
                    TrackerEvent::Split { numerator, denominator } => ("split", numerator.to_text(), Some(denominator.to_text())),
                };
                let standing: Option<(String, Option<String>)> = self
                    .conn
                    .query_row("SELECT amount, denominator FROM benchmark_events WHERE benchmark = ?1 AND day = ?2 AND kind = ?3 AND first = 1", params![b.key(), d.to_string(), kind], |r| Ok((r.get(0)?, r.get(1)?)))
                    .optional()?;
                let first = match &standing {
                    None => true,
                    Some((a, den)) if *a == amount && *den == denominator => continue,
                    Some(_) => false,
                };
                self.conn.execute(
                    "INSERT OR IGNORE INTO benchmark_events (benchmark, day, kind, amount, denominator, source, first, received_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![b.key(), d.to_string(), kind, amount, denominator, from.as_str(), first as i64, at.to_string()],
                )?;
                if let Some((a, den)) = standing {
                    let text = |a: &str, den: &Option<String>| den.as_ref().map_or_else(|| a.to_string(), |den| format!("{a}:{den}"));
                    out.push(EventDisagreement { day: *d, kind, stands: text(&a, &den), later: text(&amount, &denominator) });
                }
            }
            Ok(())
        })?;
        Ok(out)
    }

    /// Every tracker's standing closes and events.
    pub fn benchmark_series(&self) -> Result<BTreeMap<Benchmark, TrackerSeries>> {
        let mut out: BTreeMap<Benchmark, TrackerSeries> = BTreeMap::new();
        let bench = |t: &'static str, s: &str| Benchmark::parse(s).ok_or_else(|| corrupt(t, "benchmark", s, "not a benchmark"));
        {
            const T: &str = "benchmark_closes";
            let mut stmt = self.conn.prepare("SELECT benchmark, day, close FROM benchmark_closes WHERE first = 1")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
            for row in rows {
                let (b, d, c) = row?;
                out.entry(bench(T, &b)?).or_default().closes.insert(day(T, "day", &d)?, dec(T, "close", &c)?);
            }
        }
        const T: &str = "benchmark_events";
        let mut stmt = self.conn.prepare("SELECT benchmark, day, kind, amount, denominator FROM benchmark_events WHERE first = 1")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, Option<String>>(4)?)))?;
        for row in rows {
            let (b, d, kind, amount, den) = row?;
            let series = out.entry(bench(T, &b)?).or_default();
            let (d, amount) = (day(T, "day", &d)?, dec(T, "amount", &amount)?);
            match (kind.as_str(), den) {
                ("dividend", None) => {
                    series.dividends.insert(d, amount);
                }
                ("split", Some(den)) => {
                    series.splits.insert(d, (amount, dec(T, "denominator", &den)?));
                }
                _ => return Err(corrupt(T, "kind", &kind, "not a dividend or a split with its denominator")),
            }
        }
        Ok(out)
    }

    /// The days a tracker has a close for.
    pub fn benchmark_days(&self, b: Benchmark) -> Result<BTreeSet<Date>> {
        let mut stmt = self.conn.prepare("SELECT day FROM benchmark_closes WHERE benchmark = ?1 AND first = 1")?;
        let rows = stmt.query_map(params![b.key()], |r| r.get::<_, String>(0))?;
        let mut out = BTreeSet::new();
        for d in rows {
            out.insert(day("benchmark_closes", "day", &d?)?);
        }
        Ok(out)
    }

    // -- option chains ---------------------------------------------------------

    pub fn store_option_chain(&self, c: &ChainRead) -> Result<()> {
        self.conn.execute(
            "INSERT INTO option_chains (underlying, session, made_at, last_modified, received_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (underlying) DO UPDATE SET session = excluded.session, made_at = excluded.made_at, last_modified = excluded.last_modified, received_at = excluded.received_at",
            params![c.underlying, c.session.to_string(), c.made_at.to_string(), c.last_modified, c.received_at.to_string()],
        )?;
        Ok(())
    }

    /// The chain last read for an underlying.
    pub fn option_chain(&self, underlying: &str) -> Result<Option<ChainRead>> {
        const T: &str = "option_chains";
        let row: Option<(String, String, Option<String>, String)> =
            self.conn.query_row("SELECT session, made_at, last_modified, received_at FROM option_chains WHERE underlying = ?1", params![underlying], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional()?;
        row.map(|(s, m, lm, r)| Ok(ChainRead { underlying: underlying.to_string(), session: day(T, "session", &s)?, made_at: instant(T, "made_at", &m)?, last_modified: lm, received_at: instant(T, "received_at", &r)? })).transpose()
    }

    // -- chains ----------------------------------------------------------------

    /// Remember the source and form that answered for an instrument and kind.
    pub fn won(&self, id: InstrumentId, kind: DataKind, by: &SourceName, form: &str, at: Timestamp) -> Result<()> {
        self.conn.execute(
            "INSERT INTO chains (instrument_id, kind, source, form, won_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (instrument_id, kind) DO UPDATE SET source = ?3, form = ?4, won_at = ?5",
            params![id.to_string(), kind.as_str(), by.as_str(), form, at.to_string()],
        )?;
        Ok(())
    }

    /// The source and form that last answered for an instrument and kind.
    pub fn winner(&self, id: InstrumentId, kind: DataKind) -> Result<Option<(SourceName, String)>> {
        let got: Option<(String, String)> = self.conn.query_row("SELECT source, form FROM chains WHERE instrument_id = ?1 AND kind = ?2", params![id.to_string(), kind.as_str()], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        got.map(|(s, f)| Ok((source("chains", "source", &s)?, f))).transpose()
    }

    // -- outcomes --------------------------------------------------------------

    /// Record one request's outcome, and keep the source's newest thousand
    /// outcomes and the newest of each kind.
    pub fn record(&self, row: &OutcomeRow) -> Result<()> {
        bagholder_sqlite::atomically(&self.conn, || {
            self.conn.execute(
                "INSERT INTO outcomes (source, host, kind, instrument_id, outcome, detail, shape_change, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![row.source.as_str(), row.host, row.kind.as_str(), row.instrument.map(|i| i.to_string()), row.outcome.as_str(), row.detail, row.shape_change, row.at.to_string()],
            )?;
            self.conn.execute(
                "DELETE FROM outcomes WHERE source = ?1
                   AND id NOT IN (SELECT id FROM outcomes WHERE source = ?1 ORDER BY id DESC LIMIT ?2)
                   AND id NOT IN (SELECT MAX(id) FROM outcomes WHERE source = ?1 GROUP BY outcome)",
                params![row.source.as_str(), OUTCOMES_KEPT],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    /// A source's outcomes, newest first.
    pub fn outcomes(&self, of: &SourceName) -> Result<Vec<OutcomeRow>> {
        self.outcome_rows("SELECT source, host, kind, instrument_id, outcome, detail, shape_change, at FROM outcomes WHERE source = ?1 ORDER BY id DESC", params![of.as_str()])
    }

    /// Each source's newest outcome that says how the source is, one per
    /// source: a "not carried" answer says nothing of it and is passed over
    /// (`health`). The record keeps the newest of each kind, so this is there
    /// for every source that has answered or failed.
    pub fn newest_counted(&self) -> Result<Vec<OutcomeRow>> {
        self.outcome_rows(
            "SELECT source, host, kind, instrument_id, outcome, detail, shape_change, at FROM outcomes
              WHERE id IN (SELECT MAX(id) FROM outcomes WHERE outcome != ?1 GROUP BY source) ORDER BY source",
            params![OutcomeKind::NotCarried.as_str()],
        )
    }

    fn outcome_rows(&self, sql: &str, args: impl rusqlite::Params) -> Result<Vec<OutcomeRow>> {
        const T: &str = "outcomes";
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(args, |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?, r.get::<_, Option<String>>(6)?, r.get::<_, String>(7)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (s, host, k, i, o, detail, shape_change, at) = row?;
            out.push(OutcomeRow {
                source: source(T, "source", &s)?,
                host,
                kind: DataKind::parse(&k).ok_or_else(|| corrupt(T, "kind", &k, "not a kind of data"))?,
                instrument: i.map(|i| instrument(T, "instrument_id", &i)).transpose()?,
                outcome: OutcomeKind::parse(&o).ok_or_else(|| corrupt(T, "outcome", &o, "not an outcome"))?,
                detail,
                shape_change,
                at: instant(T, "at", &at)?,
            });
        }
        Ok(out)
    }

    // -- reads -----------------------------------------------------------------

    /// Keep a read of `subject`'s `kind`: the newest per span and source.
    pub fn store_read(&self, subject: &str, kind: DataKind, r: &ReadRow) -> Result<()> {
        self.conn.execute(
            "INSERT INTO reads (subject, kind, source, first, last, outcome, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (subject, kind, source, first, last) DO UPDATE SET outcome = ?6, at = ?7",
            params![subject, kind.as_str(), r.source.as_str(), r.first.to_string(), r.last.to_string(), r.outcome.as_str(), r.at.to_string()],
        )?;
        Ok(())
    }

    /// The reads of `subject`'s `kind`, newest first.
    pub fn reads(&self, subject: &str, kind: DataKind) -> Result<Vec<ReadRow>> {
        const T: &str = "reads";
        let mut stmt = self.conn.prepare("SELECT source, first, last, outcome, at FROM reads WHERE subject = ?1 AND kind = ?2 ORDER BY at DESC, rowid DESC")?;
        let rows = stmt.query_map(params![subject, kind.as_str()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (s, f, l, o, at) = row?;
            out.push(ReadRow {
                source: source(T, "source", &s)?,
                first: day(T, "first", &f)?,
                last: day(T, "last", &l)?,
                outcome: OutcomeKind::parse(&o).ok_or_else(|| corrupt(T, "outcome", &o, "not an outcome"))?,
                at: instant(T, "at", &at)?,
            });
        }
        Ok(out)
    }

    /// Every source with an outcome recorded.
    pub fn sources(&self) -> Result<BTreeSet<SourceName>> {
        let mut stmt = self.conn.prepare("SELECT DISTINCT source FROM outcomes")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = BTreeSet::new();
        for s in rows {
            out.insert(source("outcomes", "source", &s?)?);
        }
        Ok(out)
    }
}
