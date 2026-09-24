//! The facts a figure is computed from (`docs/architecture.md` §6,
//! `docs/plans/stage-2-engine.md`, "Facts and adjustments in the book"): kept in
//! the book because none of them can be fetched again as it was when it was used.
//! Nothing here is ever filled with a default: a row is a fact a source stated.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{params, OptionalExtension};

use bagholder_core::adjustment::{Adjustment, AdjustmentLeg};
use bagholder_core::instrument::Reference;
use bagholder_core::record::Problem;
use bagholder_core::{Currency, Dec, InstrumentId, Leg, Money, RecordId, SourceName, TransactionId};

use crate::mapping::AdjustmentDraft;
use crate::text::{self, at as at_text, day};
use crate::{Book, BookError, Result};

/// A rate the Bank sent for a day it had already sent a different one for: the
/// first stands, and this is shown to the person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RateConflict {
    pub currency: Currency,
    pub day: jiff::civil::Date,
    pub stands: Dec,
    pub later: Dec,
}

/// A distribution as its payer stated it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclaredRow {
    pub ex_date: jiff::civil::Date,
    pub record_date: Option<jiff::civil::Date>,
    pub pay_date: Option<jiff::civil::Date>,
    /// The cash paid per unit.
    pub amount: Money,
    /// The part reinvested per unit, in the same currency, where the payer
    /// states one.
    pub reinvested: Option<Dec>,
}

/// One series of the Bank's rates for a currency, from one source, and the
/// days it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RateSeries {
    pub currency: Currency,
    pub source: SourceName,
    pub first_day: jiff::civil::Date,
    pub last_day: jiff::civil::Date,
    /// The source states the series is no longer published (an archive, or a
    /// daily series the Bank marks historical): no day after `last_day` is in it.
    pub ended: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclaredReadRow {
    pub read_at: jiff::Timestamp,
    pub source: SourceName,
    pub items: Vec<DeclaredRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatedFrequency {
    pub per_year: u32,
    pub source: SourceName,
    pub stated_at: Option<jiff::civil::Date>,
}

/// An adjustment's place in the book: the record it was derived from and its leg.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AdjustmentKey {
    pub record: RecordId,
    pub leg: Leg,
}

fn parse_day(table: &'static str, column: &'static str, s: &str) -> Result<jiff::civil::Date> {
    s.parse().map_err(|_| text::corrupt(table, column, s, "not a day"))
}

fn parse_dec(table: &'static str, column: &'static str, s: &str) -> Result<Dec> {
    Dec::parse(s).map_err(|_| text::corrupt(table, column, s, "not a decimal"))
}

fn parse_currency(table: &'static str, column: &'static str, s: &str) -> Result<Currency> {
    Currency::parse(s).map_err(|_| text::corrupt(table, column, s, "not a currency"))
}

impl Book {
    // ------------------------------------------------------------------
    // the Bank of Canada's rates
    // ------------------------------------------------------------------

    /// Store what one completed read of a series returned: its observations (CAD
    /// per unit of `currency`) and the span of days it covered. A day already
    /// stored with the same rate writes nothing; with a different rate, the new
    /// one is kept beside it and returned as a conflict, and the first stands.
    pub fn store_rates(&self, currency: Currency, observations: &[(jiff::civil::Date, Dec)], covered: (jiff::civil::Date, jiff::civil::Date), source: &SourceName, at: jiff::Timestamp) -> Result<Vec<RateConflict>> {
        if covered.0 > covered.1 {
            return Err(BookError::Refused(format!("a read covering {} to {} covers nothing", covered.0, covered.1)));
        }
        self.atomically(|| {
            let mut conflicts = Vec::new();
            for (d, rate) in observations {
                if *d < covered.0 || *d > covered.1 {
                    return Err(BookError::Refused(format!("a rate for {d} outside the read's span {} to {}", covered.0, covered.1)));
                }
                if let Some(stands) = self.first_rate(currency, *d)? {
                    if stands == *rate {
                        continue;
                    }
                    conflicts.push(RateConflict { currency, day: *d, stands, later: *rate });
                }
                self.conn().execute(
                    "INSERT OR IGNORE INTO fx_rates(currency, day, rate, source, received_at) VALUES (?, ?, ?, ?, ?)",
                    params![currency.as_str(), day(*d), rate.to_text(), source.as_str(), at_text(at)],
                )?;
            }
            self.conn().execute(
                "INSERT INTO fx_reads(currency, first_day, last_day, source, received_at) VALUES (?, ?, ?, ?, ?)",
                params![currency.as_str(), day(covered.0), day(covered.1), source.as_str(), at_text(at)],
            )?;
            Ok(conflicts)
        })
    }

    fn first_rate(&self, currency: Currency, d: jiff::civil::Date) -> Result<Option<Dec>> {
        let found: Option<String> = self
            .conn()
            .query_row(
                "SELECT rate FROM fx_rates WHERE currency = ? AND day = ? ORDER BY rowid LIMIT 1",
                params![currency.as_str(), day(d)],
                |r| r.get(0),
            )
            .optional()?;
        found.map(|s| parse_dec("fx_rates", "rate", &s)).transpose()
    }

    /// Every stored rate: the first stored for each day (the order rows were
    /// written in, whatever time a read was stamped with).
    pub fn rates(&self) -> Result<BTreeMap<Currency, BTreeMap<jiff::civil::Date, Dec>>> {
        let mut stmt = self.conn().prepare_cached("SELECT currency, day, rate FROM fx_rates ORDER BY currency, day, rowid DESC")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        let mut out: BTreeMap<Currency, BTreeMap<jiff::civil::Date, Dec>> = BTreeMap::new();
        for row in rows {
            let (c, d, r) = row?;
            // ordered newest first within a day, so the first received is written last
            out.entry(parse_currency("fx_rates", "currency", &c)?).or_default().insert(parse_day("fx_rates", "day", &d)?, parse_dec("fx_rates", "rate", &r)?);
        }
        Ok(out)
    }

    /// Every rate that came later and differed from the one that stands.
    pub fn rate_conflicts(&self) -> Result<Vec<RateConflict>> {
        let mut out = Vec::new();
        let mut stmt = self.conn().prepare_cached("SELECT currency, day, rate FROM fx_rates ORDER BY currency, day, rowid")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        let mut first: BTreeMap<(String, String), Dec> = BTreeMap::new();
        for row in rows {
            let (c, d, r) = row?;
            let rate = parse_dec("fx_rates", "rate", &r)?;
            match first.get(&(c.clone(), d.clone())) {
                None => {
                    first.insert((c, d), rate);
                }
                Some(stands) => out.push(RateConflict { currency: parse_currency("fx_rates", "currency", &c)?, day: parse_day("fx_rates", "day", &d)?, stands: *stands, later: rate }),
            }
        }
        Ok(out)
    }

    /// The spans completed reads covered, per currency.
    pub fn rate_reads(&self) -> Result<BTreeMap<Currency, Vec<(jiff::civil::Date, jiff::civil::Date, jiff::Timestamp)>>> {
        let mut stmt = self.conn().prepare_cached("SELECT currency, first_day, last_day, received_at FROM fx_reads ORDER BY currency, first_day")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?;
        let mut out: BTreeMap<Currency, Vec<(jiff::civil::Date, jiff::civil::Date, jiff::Timestamp)>> = BTreeMap::new();
        for row in rows {
            let (c, a, b, at) = row?;
            let at = at.parse().map_err(|_| text::corrupt("fx_reads", "received_at", &at, "not an instant"))?;
            out.entry(parse_currency("fx_reads", "currency", &c)?).or_default().push((parse_day("fx_reads", "first_day", &a)?, parse_day("fx_reads", "last_day", &b)?, at));
        }
        Ok(out)
    }

    /// The series a source holds, each with its first and last observation day
    /// as the source states them now: a later statement of the same series
    /// replaces the earlier (a daily series' last day moves on).
    pub fn store_rate_series(&self, series: &[RateSeries], at: jiff::Timestamp) -> Result<()> {
        self.atomically(|| {
            for s in series {
                if s.first_day > s.last_day {
                    return Err(BookError::Refused(format!("a {} series from {} to {} holds no day", s.currency, s.first_day, s.last_day)));
                }
                self.conn().execute(
                    "INSERT OR REPLACE INTO fx_series(currency, source, first_day, last_day, ended, received_at) VALUES (?, ?, ?, ?, ?, ?)",
                    params![s.currency.as_str(), s.source.as_str(), day(s.first_day), day(s.last_day), s.ended as i64, at_text(at)],
                )?;
            }
            Ok(())
        })
    }

    /// Every series of the Bank's rates held, oldest first within a currency.
    pub fn rate_series(&self) -> Result<Vec<RateSeries>> {
        let mut stmt = self.conn().prepare_cached("SELECT currency, source, first_day, last_day, ended FROM fx_series ORDER BY currency, first_day")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (c, source, first, last, ended) = row?;
            out.push(RateSeries {
                currency: parse_currency("fx_series", "currency", &c)?,
                source: text::parsed("fx_series", "source", &source, SourceName::parse)?,
                first_day: parse_day("fx_series", "first_day", &first)?,
                last_day: parse_day("fx_series", "last_day", &last)?,
                ended: match ended {
                    0 => false,
                    1 => true,
                    n => return Err(text::corrupt("fx_series", "ended", &n.to_string(), "not 0 or 1")),
                },
            });
        }
        Ok(out)
    }

    /// The Bank's own holiday schedule, as read.
    pub fn store_bank_holidays(&self, holidays: &[(jiff::civil::Date, String)], source: &SourceName, at: jiff::Timestamp) -> Result<()> {
        self.atomically(|| {
            for (d, name) in holidays {
                self.conn().execute(
                    "INSERT OR IGNORE INTO bank_holidays(day, name, source, received_at) VALUES (?, ?, ?, ?)",
                    params![day(*d), name, source.as_str(), at_text(at)],
                )?;
            }
            Ok(())
        })
    }

    pub fn bank_holidays(&self) -> Result<BTreeSet<jiff::civil::Date>> {
        let mut stmt = self.conn().prepare_cached("SELECT day FROM bank_holidays")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|d| parse_day("bank_holidays", "day", &d?)).collect()
    }

    // ------------------------------------------------------------------
    // declared distributions and stated frequencies
    // ------------------------------------------------------------------

    /// One read of a fund's declared record, whole.
    pub fn store_declared(&self, instrument: InstrumentId, items: &[DeclaredRow], source: &SourceName, at: jiff::Timestamp) -> Result<()> {
        self.atomically(|| {
            drop(self.instrument(instrument)?);
            // a read identical to the newest, from the same source, records only its time
            let newest: Option<(i64, String)> = self
                .conn()
                .query_row("SELECT id, source FROM declared_reads WHERE instrument_id = ? ORDER BY read_at DESC, id DESC LIMIT 1", [instrument.to_string()], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?;
            if let Some((id, stored_source)) = newest {
                let key = |d: &DeclaredRow| (d.ex_date, d.record_date, d.pay_date, d.amount.amount.to_text(), d.amount.currency.as_str().to_string(), d.reinvested.map(Dec::to_text));
                let mut stored: Vec<_> = self.declared_rows(id)?.iter().map(key).collect();
                let mut now: Vec<_> = items.iter().map(key).collect();
                stored.sort();
                now.sort();
                if stored_source == source.as_str() && stored == now {
                    self.conn().execute("UPDATE declared_reads SET read_at = ? WHERE id = ?", params![at_text(at), id])?;
                    return Ok(());
                }
            }
            self.conn().execute(
                "INSERT INTO declared_reads(instrument_id, source, read_at) VALUES (?, ?, ?)",
                params![instrument.to_string(), source.as_str(), at_text(at)],
            )?;
            let read = self.conn().last_insert_rowid();
            for d in items {
                self.conn().execute(
                    "INSERT INTO declared_distributions(read_id, ex_date, record_date, pay_date, amount, reinvested, currency) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    params![read, day(d.ex_date), d.record_date.map(day), d.pay_date.map(day), d.amount.amount.to_text(), d.reinvested.map(Dec::to_text), d.amount.currency.as_str()],
                )?;
            }
            Ok(())
        })
    }

    /// The distributions one read of a declared record stored, by ex-date.
    fn declared_rows(&self, read: i64) -> Result<Vec<DeclaredRow>> {
        let mut items = Vec::new();
        let mut s = self.conn().prepare_cached("SELECT ex_date, record_date, pay_date, amount, reinvested, currency FROM declared_distributions WHERE read_id = ? ORDER BY ex_date")?;
        let rows = s.query_map([read], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, String>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, String>(5)?))
        })?;
        for row in rows {
            let (ex, rec, pay, amount, reinvested, currency) = row?;
            items.push(DeclaredRow {
                ex_date: parse_day("declared_distributions", "ex_date", &ex)?,
                record_date: rec.map(|d| parse_day("declared_distributions", "record_date", &d)).transpose()?,
                pay_date: pay.map(|d| parse_day("declared_distributions", "pay_date", &d)).transpose()?,
                amount: Money::new(parse_dec("declared_distributions", "amount", &amount)?, parse_currency("declared_distributions", "currency", &currency)?),
                reinvested: reinvested.map(|r| parse_dec("declared_distributions", "reinvested", &r)).transpose()?,
            });
        }
        Ok(items)
    }

    /// The newest read of each fund's declared record.
    pub fn declared(&self) -> Result<BTreeMap<InstrumentId, DeclaredReadRow>> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT r.id, r.instrument_id, r.source, r.read_at FROM declared_reads r
             WHERE r.id = (SELECT id FROM declared_reads x WHERE x.instrument_id = r.instrument_id ORDER BY x.read_at DESC, x.id DESC LIMIT 1)",
        )?;
        let reads = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut out = BTreeMap::new();
        for (id, instrument, source, read_at) in reads {
            let items = self.declared_rows(id)?;
            out.insert(
                text::parsed("declared_reads", "instrument_id", &instrument, InstrumentId::parse)?,
                DeclaredReadRow {
                    read_at: read_at.parse().map_err(|_| text::corrupt("declared_reads", "read_at", &read_at, "not an instant"))?,
                    source: text::parsed("declared_reads", "source", &source, SourceName::parse)?,
                    items,
                },
            );
        }
        Ok(out)
    }

    pub fn store_frequency(&self, instrument: InstrumentId, per_year: u32, source: &SourceName, stated_at: Option<jiff::civil::Date>, at: jiff::Timestamp) -> Result<()> {
        if per_year == 0 {
            return Err(BookError::Refused("a frequency of no payments a year".into()));
        }
        self.atomically(|| {
            drop(self.instrument(instrument)?);
            self.conn().execute(
                "INSERT OR REPLACE INTO stated_frequencies(instrument_id, per_year, source, stated_at, received_at) VALUES (?, ?, ?, ?, ?)",
                params![instrument.to_string(), per_year, source.as_str(), stated_at.map(day), at_text(at)],
            )?;
            Ok(())
        })
    }

    /// The newest stated frequency of each instrument.
    pub fn frequencies(&self) -> Result<BTreeMap<InstrumentId, StatedFrequency>> {
        let mut stmt = self.conn().prepare_cached("SELECT instrument_id, per_year, source, stated_at FROM stated_frequencies ORDER BY instrument_id, received_at")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?)))?;
        let mut out = BTreeMap::new();
        for row in rows {
            let (i, n, source, stated) = row?;
            out.insert(
                text::parsed("stated_frequencies", "instrument_id", &i, InstrumentId::parse)?,
                StatedFrequency {
                    per_year: u32::try_from(n).map_err(|_| text::corrupt("stated_frequencies", "per_year", &n.to_string(), "not a count"))?,
                    source: text::parsed("stated_frequencies", "source", &source, SourceName::parse)?,
                    stated_at: stated.map(|d| parse_day("stated_frequencies", "stated_at", &d)).transpose()?,
                },
            );
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // recorded closes
    // ------------------------------------------------------------------

    /// A daily close no source can give again later; the first stored for a
    /// day stands, and a different later one is refused.
    pub fn store_close(&self, instrument: InstrumentId, d: jiff::civil::Date, close: Money, source: &SourceName, at: jiff::Timestamp) -> Result<()> {
        self.atomically(|| {
            let stored: Option<(String, String)> = self
                .conn()
                .query_row("SELECT close, currency FROM recorded_closes WHERE instrument_id = ? AND day = ?", params![instrument.to_string(), day(d)], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?;
            if let Some((c, cur)) = stored {
                let before = Money::new(parse_dec("recorded_closes", "close", &c)?, parse_currency("recorded_closes", "currency", &cur)?);
                return if before == close { Ok(()) } else { Err(BookError::Refused(format!("{instrument} already closed at {before:?} on {d}; {close:?} does not replace it"))) };
            }
            self.conn().execute(
                "INSERT INTO recorded_closes(instrument_id, day, close, currency, source, received_at) VALUES (?, ?, ?, ?, ?, ?)",
                params![instrument.to_string(), day(d), close.amount.to_text(), close.currency.as_str(), source.as_str(), at_text(at)],
            )?;
            Ok(())
        })
    }

    pub fn closes(&self) -> Result<BTreeMap<InstrumentId, BTreeMap<jiff::civil::Date, Money>>> {
        let mut stmt = self.conn().prepare_cached("SELECT instrument_id, day, close, currency FROM recorded_closes")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?;
        let mut out: BTreeMap<InstrumentId, BTreeMap<jiff::civil::Date, Money>> = BTreeMap::new();
        for row in rows {
            let (i, d, c, cur) = row?;
            out.entry(text::parsed("recorded_closes", "instrument_id", &i, InstrumentId::parse)?)
                .or_default()
                .insert(parse_day("recorded_closes", "day", &d)?, Money::new(parse_dec("recorded_closes", "close", &c)?, parse_currency("recorded_closes", "currency", &cur)?));
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // adjustments
    // ------------------------------------------------------------------

    /// The instrument a list of references names, or a problem.
    fn adjustment_instrument(&self, refs: &[Reference]) -> Result<std::result::Result<InstrumentId, Problem>> {
        let mut found: Option<InstrumentId> = None;
        for r in refs.iter().filter(|r| r.identifies()) {
            if let Some(i) = self.instrument_by_ref(r)? {
                match found {
                    Some(f) if f != i => return Ok(Err(Problem::new("adjustment-instrument-conflict", format!("its references name two instruments, {f} and {i}")))),
                    _ => found = Some(i),
                }
            }
        }
        Ok(found.ok_or_else(|| Problem::new("adjustment-instrument-unknown", format!("no instrument has the references {refs:?}"))))
    }

    /// Write a record's adjustments as its mapping describes them, replacing
    /// what it had; returns the problems of those that could not be written.
    pub(crate) fn write_adjustments(&self, record: RecordId, drafts: &[AdjustmentDraft]) -> Result<Vec<Problem>> {
        self.clear_adjustments(record)?;
        let mut problems = Vec::new();
        'each: for a in drafts {
            let mut legs = Vec::new();
            for l in &a.legs {
                let side = |refs: &Option<Vec<Reference>>| -> Result<std::result::Result<Option<InstrumentId>, Problem>> {
                    match refs {
                        None => Ok(Ok(None)),
                        Some(refs) => Ok(self.adjustment_instrument(refs)?.map(Some)),
                    }
                };
                let (from, to) = match (side(&l.from)?, side(&l.to)?) {
                    (Ok(f), Ok(t)) => (f, t),
                    (Err(p), _) | (_, Err(p)) => {
                        problems.push(p);
                        continue 'each;
                    }
                };
                if from.is_none() && to.is_none() {
                    problems.push(Problem::new("adjustment-names-nothing", format!("adjustment {} has a leg naming no instrument", a.leg)));
                    continue 'each;
                }
                legs.push(AdjustmentLeg { from, to, units_per_unit: l.units_per_unit, cost_share: l.cost_share, cash_per_unit: l.cash_per_unit, cost: l.cost, acquired: l.acquired });
            }
            self.conn().execute(
                "INSERT INTO adjustments(record_id, leg, applies_record, applies_leg) VALUES (?, ?, ?, ?)",
                params![record.to_string(), a.leg.as_str(), a.applies_to.record.to_string(), a.applies_to.leg.as_str()],
            )?;
            for (n, l) in legs.iter().enumerate() {
                self.conn().execute(
                    "INSERT INTO adjustment_legs(record_id, leg, position, from_instrument, to_instrument, units_per_unit, cost_share, cash_per_unit, cash_currency, cost, cost_currency, acquired)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        record.to_string(),
                        a.leg.as_str(),
                        n as i64,
                        l.from.map(|i| i.to_string()),
                        l.to.map(|i| i.to_string()),
                        l.units_per_unit.map(|d| d.to_text()),
                        l.cost_share.map(|d| d.to_text()),
                        l.cash_per_unit.map(|m| m.amount.to_text()),
                        l.cash_per_unit.map(|m| m.currency.to_string()),
                        l.cost.map(|m| m.amount.to_text()),
                        l.cost.map(|m| m.currency.to_string()),
                        l.acquired.map(day),
                    ],
                )?;
            }
        }
        Ok(problems)
    }

    pub(crate) fn clear_adjustments(&self, record: RecordId) -> Result<()> {
        self.conn().execute("DELETE FROM adjustment_legs WHERE record_id = ?", [record.to_string()])?;
        self.conn().execute("DELETE FROM adjustments WHERE record_id = ?", [record.to_string()])?;
        Ok(())
    }

    /// The adjustments explaining a record's transactions, and which each explains.
    pub(crate) fn adjustments_applying_to(&self, record: RecordId) -> Result<Vec<(AdjustmentKey, TransactionId)>> {
        let mut stmt = self.conn().prepare_cached("SELECT record_id, leg, applies_leg FROM adjustments WHERE applies_record = ?")?;
        let rows = stmt.query_map([record.to_string()], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (r, leg, applies_leg) = row?;
            out.push((
                AdjustmentKey { record: text::parsed("adjustments", "record_id", &r, RecordId::parse)?, leg: text::parsed("adjustments", "leg", &leg, Leg::parse)? },
                TransactionId::new(record, text::parsed("adjustments", "applies_leg", &applies_leg, Leg::parse)?),
            ));
        }
        Ok(out)
    }

    /// Whether a live adjustment on `tx` moves units into `instrument`.
    pub(crate) fn adjustment_moves(&self, tx: &TransactionId, instrument: InstrumentId) -> Result<bool> {
        let n: i64 = self.conn().query_row(
            "SELECT COUNT(*) FROM adjustments a JOIN adjustment_legs l ON l.record_id = a.record_id AND l.leg = a.leg
             JOIN source_records s ON s.id = a.record_id
             WHERE s.state = 'live' AND a.applies_record = ? AND a.applies_leg = ? AND l.to_instrument = ?",
            params![tx.record.to_string(), tx.leg.as_str(), instrument.to_string()],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    /// Report on an adjustment's own record that the transaction it explains is
    /// gone: it then explains nothing, and says so.
    pub(crate) fn adjustment_target_gone(&self, key: &AdjustmentKey, applies: &TransactionId, why: &str) -> Result<()> {
        self.add_problems(key.record, &[bagholder_core::record::Problem::new("adjustment-target-gone", format!("the transaction it explains, {applies}, {why}"))])
    }

    pub(crate) fn move_adjustment(&self, key: &AdjustmentKey, to: &TransactionId) -> Result<()> {
        self.conn().execute(
            "UPDATE adjustments SET applies_record = ?, applies_leg = ? WHERE record_id = ? AND leg = ?",
            params![to.record.to_string(), to.leg.as_str(), key.record.to_string(), key.leg.as_str()],
        )?;
        Ok(())
    }

    /// Every adjustment of a live record.
    pub fn adjustments(&self) -> Result<Vec<Adjustment>> {
        let mut stmt = self.conn().prepare_cached(
            "SELECT a.record_id, a.leg, a.applies_record, a.applies_leg, s.source FROM adjustments a JOIN source_records s ON s.id = a.record_id
             WHERE s.state = 'live' ORDER BY s.first_received_at, a.record_id, a.leg",
        )?;
        let heads = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut out = Vec::new();
        for (record, leg, applies_record, applies_leg, source) in heads {
            let mut s = self.conn().prepare_cached(
                "SELECT from_instrument, to_instrument, units_per_unit, cost_share, cash_per_unit, cash_currency, cost, cost_currency, acquired
                 FROM adjustment_legs WHERE record_id = ? AND leg = ? ORDER BY position",
            )?;
            type Row = (Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, Option<String>);
            let rows = s.query_map(params![record, leg], |r| -> rusqlite::Result<Row> {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?))
            })?;
            let mut legs = Vec::new();
            for row in rows {
                let (from, to, units, share, cash, cash_cur, cost, cost_cur, acquired) = row?;
                let money = |v: Option<String>, c: Option<String>, col: &'static str| -> Result<Option<Money>> {
                    match (v, c) {
                        (Some(v), Some(c)) => Ok(Some(Money::new(parse_dec("adjustment_legs", col, &v)?, parse_currency("adjustment_legs", col, &c)?))),
                        _ => Ok(None),
                    }
                };
                legs.push(AdjustmentLeg {
                    from: from.map(|i| text::parsed("adjustment_legs", "from_instrument", &i, InstrumentId::parse)).transpose()?,
                    to: to.map(|i| text::parsed("adjustment_legs", "to_instrument", &i, InstrumentId::parse)).transpose()?,
                    units_per_unit: units.map(|d| parse_dec("adjustment_legs", "units_per_unit", &d)).transpose()?,
                    cost_share: share.map(|d| parse_dec("adjustment_legs", "cost_share", &d)).transpose()?,
                    cash_per_unit: money(cash, cash_cur, "cash_per_unit")?,
                    cost: money(cost, cost_cur, "cost")?,
                    acquired: acquired.map(|d| parse_day("adjustment_legs", "acquired", &d)).transpose()?,
                });
            }
            out.push(Adjustment {
                applies_to: TransactionId::new(text::parsed("adjustments", "applies_record", &applies_record, RecordId::parse)?, text::parsed("adjustments", "applies_leg", &applies_leg, Leg::parse)?),
                legs,
                source: text::parsed("source_records", "source", &source, SourceName::parse)?,
            });
        }
        Ok(out)
    }
}
