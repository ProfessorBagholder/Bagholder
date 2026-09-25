//! When a payer is read, and what is stored (`docs/plans/stage-3a-sources.md`,
//! "Periodic reads").
//!
//! A payer's record changes only when it declares, and it declares on its own
//! schedule. So a payer is read when it first appears, and then when its next
//! distribution is due by the schedule it states: from a week before the day one
//! period after its latest ex-date (declarations come about a week ahead), once a
//! day, until a read shows a distribution from that window on. A payer whose
//! schedule no source states is read once a week: there is nothing else to go by.

use std::collections::BTreeMap;

use bagholder_book::facts::{DeclaredReadRow, DeclaredRow, StatedFrequency};
use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::{SignedDuration, Timestamp};
use bagholder_core::{InstrumentId, Money};

use crate::contract::{Benchmark, DataKind};
use crate::market;
use crate::needs::PayerNeed;
use crate::outcome::{Outcome, OutcomeKind};
use crate::payers::{adapter_for, checked, companies, market_record_for, Payer, Record};
use crate::read::{Ctx, Result};

/// Whether a payer is due, from what the book holds of it.
pub fn due(read: Option<&DeclaredReadRow>, frequency: Option<&StatedFrequency>, now: Timestamp, zone: &TimeZone) -> bool {
    let Some(read) = read else { return true };
    let today = now.to_zoned(zone.clone()).date();
    let read_on = read.read_at.to_zoned(zone.clone()).date();
    let Some(per_year) = frequency.map(|f| f.per_year).filter(|n| *n > 0) else {
        return now.duration_since(read.read_at) >= SignedDuration::from_hours(24 * 7);
    };
    let Some(latest) = read.items.iter().map(|d| d.ex_date).max() else {
        // a record with nothing in it yet: a new fund before its first
        return read_on < today;
    };
    let period = SignedDuration::from_hours(24 * i64::from((365 / per_year).max(7)));
    let window = latest.checked_add(period).and_then(|d| d.checked_sub(SignedDuration::from_hours(24 * 7))).unwrap_or(Date::MAX);
    today >= window && read_on < today
}

fn stored_rows(record: &Record) -> Vec<DeclaredRow> {
    record.rows.iter().map(|r| DeclaredRow { ex_date: r.ex_date, record_date: r.record_date, pay_date: r.pay_date, amount: Money::new(r.cash, r.currency), reinvested: r.reinvested }).collect()
}

/// Date a record's distributions stated by record date on the exchange's
/// sessions, reading XIC's closes for the span first where they are not held:
/// whether every one is dated.
fn dated(ctx: &Ctx, record: &mut Record) -> Result<bool> {
    let (Some(first), Some(last)) = (record.by_record.iter().map(|b| b.record_date).min(), record.by_record.iter().map(|b| b.record_date).max()) else { return Ok(true) };
    // two sessions before a record date lie within a fortnight of it
    let from = first.checked_sub(SignedDuration::from_hours(24 * 14)).unwrap_or(Date::MIN);
    market::read_benchmark(ctx, Benchmark::Tsx, from, last)?;
    let state = market::benchmark_state(ctx, Benchmark::Tsx)?;
    if !market::covered(Benchmark::Tsx.market(), from, last, &state, ctx.now, ctx.bank) {
        return Ok(false);
    }
    let mut rows = Vec::with_capacity(record.by_record.len());
    for b in &record.by_record {
        let Some(ex) = companies::ex_date(b.record_date, &state.days) else { return Ok(false) };
        rows.push(b.with_ex(ex));
    }
    record.rows.extend(rows);
    record.by_record.clear();
    Ok(true)
}

/// Read every held payer that is due, storing what its publication states.
pub fn read(ctx: &Ctx, payers: &[PayerNeed]) -> Result<()> {
    let declared: BTreeMap<InstrumentId, DeclaredReadRow> = ctx.book.declared()?;
    let frequencies: BTreeMap<InstrumentId, StatedFrequency> = ctx.book.frequencies()?;
    for need in payers {
        let id = need.listing.id;
        if !due(declared.get(&id), frequencies.get(&id), ctx.now, ctx.bank) {
            continue;
        }
        let Some(adapter) = adapter_for(need) else {
            // no reader for its market either: the figures wait on it, named
            continue;
        };
        let outcome = read_with(ctx, need, adapter.as_ref())?;
        // its company's publication does not carry it: no company reader serves
        // it, so it has the market's record. A company reader that failed to
        // answer is that source's failure, and is not passed over.
        if outcome == Some(OutcomeKind::NotCarried) {
            if let Some(market) = market_record_for(need).filter(|m| m.source() != adapter.source()) {
                read_with(ctx, need, market.as_ref())?;
            }
        }
    }
    Ok(())
}

/// One reader's read of one payer, unless its source rests: what it answered is
/// recorded, and what it states is stored.
fn read_with(ctx: &Ctx, need: &PayerNeed, adapter: &dyn Payer) -> Result<Option<OutcomeKind>> {
    let id = need.listing.id;
    // a failed read waits out its source's rest
    let subject = id.to_string();
    if ctx.resting(&subject, DataKind::Distributions, adapter.host())? {
        return Ok(None);
    }
    let mut noted = adapter.read(ctx.net, need, ctx.now);
    if let Outcome::Answered(record) = &mut noted.outcome {
        if !dated(ctx, record)? {
            // the exchange's sessions for its older declarations are not all
            // held (their read failed or rests): the payer's answer is kept
            // as a read of its source, nothing is stored, and it stays due
            ctx.record(&adapter.source(), adapter.host(), DataKind::Distributions, Some(id), &noted)?;
            return Ok(None);
        }
    }
    noted.outcome = match noted.outcome {
        Outcome::Answered(record) => match checked(record, ctx.now) {
            Ok(r) => Outcome::Answered(r),
            Err(why) => Outcome::Meaning(why),
        },
        other => other,
    };
    let kind = noted.outcome.kind();
    ctx.record(&adapter.source(), adapter.host(), DataKind::Distributions, Some(id), &noted)?;
    ctx.attempted(&subject, DataKind::Distributions, &adapter.source(), kind)?;
    if let Outcome::Answered(record) = noted.outcome {
        ctx.book.store_declared(id, &stored_rows(&record), &adapter.source(), ctx.now)?;
        if let Some(n) = record.per_year {
            ctx.book.store_frequency(id, n, &adapter.source(), Some(ctx.today()), ctx.now)?;
        }
    }
    Ok(Some(kind))
}

/// The payers no adapter reads: shown as waiting on their payer, named.
pub fn unread(payers: &[PayerNeed]) -> Vec<&PayerNeed> {
    payers.iter().filter(|p| adapter_for(p).is_none()).collect()
}
