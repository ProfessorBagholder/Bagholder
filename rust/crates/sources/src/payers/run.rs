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

use crate::contract::DataKind;
use crate::needs::PayerNeed;
use crate::outcome::Outcome;
use crate::payers::{adapter_for, checked, Record};
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
            // no adapter reads this payer: the figures wait on it, named
            continue;
        };
        let mut noted = adapter.read(ctx.net, need, ctx.now);
        if let Outcome::Answered(record) = std::mem::replace(&mut noted.outcome, Outcome::NotCarried(String::new())) {
            noted.outcome = match checked(record, ctx.now) {
                Ok(r) => Outcome::Answered(r),
                Err(why) => Outcome::Meaning(why),
            };
        }
        ctx.record(&adapter.source(), adapter.host(), DataKind::Distributions, Some(id), &noted)?;
        if let Outcome::Answered(record) = noted.outcome {
            ctx.book.store_declared(id, &stored_rows(&record), &adapter.source(), ctx.now)?;
            if let Some(n) = record.per_year {
                ctx.book.store_frequency(id, n, &adapter.source(), Some(ctx.today()), ctx.now)?;
            }
        }
    }
    Ok(())
}

/// The payers no adapter reads: shown as waiting on their payer, named.
pub fn unread(payers: &[PayerNeed]) -> Vec<&PayerNeed> {
    payers.iter().filter(|p| adapter_for(p).is_none()).collect()
}
