//! When a payer is read, and what is stored (`docs/plans/stage-3a-sources.md`,
//! "Periodic reads").
//!
//! A payer's record changes only when it declares, and it declares on its own
//! schedule. So a payer is read when it first appears, and then when its next
//! distribution is due by the schedule it states: from a week before the day one
//! period after its latest ex-date (declarations come about a week ahead), once a
//! day, until a read shows a distribution from that window on. A payer whose
//! schedule no source states is read once a week: there is nothing else to go by.
//!
//! A company lists a declaration on its own publication days after the exchange
//! lists it (Harvest and Ninepoint, observed 2026-09-26: TMX listed the
//! September distributions declared 2026-09-23 while the
//! companies' pages did not). So while a Canadian payer's company record lists
//! nothing still to be paid, the exchange's record (TMX) is read beside it for
//! the next distribution (`SPEC.md` §5, Ex-Div): at once, then a day from a week
//! before the next is due until TMX lists it. The rate and schedule stay the
//! company's.

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

/// The first instant at which [`due`] holds for this payer: now, for one never
/// read; a week after its last read, where no source states its schedule; else
/// the start of the day its window opens in `zone`, or of the next day where it
/// was read today.
pub fn next_due(read: Option<&DeclaredReadRow>, frequency: Option<&StatedFrequency>, now: Timestamp, zone: &TimeZone) -> Timestamp {
    let Some(read) = read else { return now };
    let start = |d: Date| d.to_zoned(zone.clone()).map(|z| z.timestamp()).unwrap_or(Timestamp::MAX);
    let read_on = read.read_at.to_zoned(zone.clone()).date();
    let day_after_read = read_on.tomorrow().map(start).unwrap_or(Timestamp::MAX);
    let Some(per_year) = frequency.map(|f| f.per_year).filter(|n| *n > 0) else {
        return read.read_at.checked_add(SignedDuration::from_hours(24 * 7)).unwrap_or(Timestamp::MAX);
    };
    let Some(latest) = read.items.iter().map(|d| d.ex_date).max() else { return day_after_read };
    let period = SignedDuration::from_hours(24 * i64::from((365 / per_year).max(7)));
    let window = latest.checked_add(period).and_then(|d| d.checked_sub(SignedDuration::from_hours(24 * 7))).map(start).unwrap_or(Timestamp::MAX);
    window.max(day_after_read)
}

/// Whether a record lists a distribution paying cash still to be paid on or
/// after `today` (its pay date, or its ex-date where it states none).
fn lists_one_to_pay(items: &[DeclaredRow], today: Date) -> bool {
    items.iter().any(|d| d.amount.amount.is_positive() && d.pay_date.unwrap_or(d.ex_date) >= today)
}

/// The day a payer's next distribution is due by its stated schedule, less the
/// week declarations come ahead: none where its record lists nothing yet.
fn window_opens(items: &[DeclaredRow], per_year: u32) -> Option<Date> {
    let latest = items.iter().map(|d| d.ex_date).max()?;
    let period = SignedDuration::from_hours(24 * i64::from((365 / per_year).max(7)));
    Some(latest.checked_add(period).and_then(|d| d.checked_sub(SignedDuration::from_hours(24 * 7))).unwrap_or(Date::MAX))
}

/// Whether the market's record beside a company's is due (`SPEC.md` §5,
/// Cashflow Positions, Ex-Div): the company's record lists nothing still to be
/// paid, and the market's has not been read, or lists nothing after the
/// company's latest still to be paid and was last read before today, with the
/// payer's next distribution due within a week by its stated schedule (a week
/// after its last read where no source states one).
pub fn market_due(read: &DeclaredReadRow, frequency: Option<&StatedFrequency>, now: Timestamp, zone: &TimeZone) -> bool {
    market_next_due(read, frequency, now, zone).is_some_and(|at| at <= now)
}

/// The first instant at which [`market_due`] holds: none while the company's
/// record lists a distribution still to be paid, or the market's lists one
/// after it (each is read again once that is paid, on its schedule).
pub fn market_next_due(read: &DeclaredReadRow, frequency: Option<&StatedFrequency>, now: Timestamp, zone: &TimeZone) -> Option<Timestamp> {
    let today = now.to_zoned(zone.clone()).date();
    if lists_one_to_pay(&read.items, today) {
        return None;
    }
    let Some(market) = &read.market else { return Some(now) };
    let latest = read.items.iter().map(|d| d.ex_date).max();
    let after: Vec<DeclaredRow> = market.items.iter().filter(|d| latest.is_none_or(|l| d.ex_date > l)).cloned().collect();
    if lists_one_to_pay(&after, today) {
        return None;
    }
    let start = |d: Date| d.to_zoned(zone.clone()).map(|z| z.timestamp()).unwrap_or(Timestamp::MAX);
    let day_after_read = market.read_at.to_zoned(zone.clone()).date().tomorrow().map(start).unwrap_or(Timestamp::MAX);
    match frequency.map(|f| f.per_year).filter(|n| *n > 0) {
        None => Some(market.read_at.checked_add(SignedDuration::from_hours(24 * 7)).unwrap_or(Timestamp::MAX)),
        Some(n) => Some(window_opens(&read.items, n).map(start).unwrap_or(now).max(day_after_read)),
    }
}

/// The market's record read beside a payer's company record: the exchange's
/// (TMX), which lists a declaration the day the fund makes it. A US listing's
/// market record (Yahoo's dividend events) lists what has gone ex, never a
/// declaration ahead of it, so nothing is read beside a US company's record.
pub fn beside_record_for(need: &PayerNeed, company: &dyn Payer) -> Option<Box<dyn Payer>> {
    market_record_for(need).filter(|m| m.source() != company.source() && m.source() == crate::adapters::tmx::source())
}

fn stored_rows(record: &Record) -> Vec<DeclaredRow> {
    record.rows.iter().map(|r| DeclaredRow { form: record.form, ex_date: r.ex_date, record_date: r.record_date, pay_date: r.pay_date, amount: Money::new(r.cash, r.currency), reinvested: r.reinvested }).collect()
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
        let Some(adapter) = adapter_for(need) else {
            // no reader for its market either: the figures wait on it, named
            continue;
        };
        if due(declared.get(&id), frequencies.get(&id), ctx.now, ctx.bank) {
            let outcome = read_with(ctx, need, adapter.as_ref(), Role::Payer)?;
            // its company's publication does not carry it: no company reader serves
            // it, so it has the market's record. A company reader that failed to
            // answer is that source's failure, and is not passed over.
            if outcome == Some(OutcomeKind::NotCarried) {
                if let Some(market) = market_record_for(need).filter(|m| m.source() != adapter.source()) {
                    read_with(ctx, need, market.as_ref(), Role::Payer)?;
                }
            }
        }
        read_beside(ctx, need, adapter.as_ref(), false)?;
    }
    Ok(())
}

/// Read the market's record beside a payer's company record when it is due
/// (or `at_once`, whenever the company's lists nothing still to be paid).
fn read_beside(ctx: &Ctx, need: &PayerNeed, company: &dyn Payer, at_once: bool) -> Result<()> {
    let id = need.listing.id;
    let Some(market) = beside_record_for(need, company) else { return Ok(()) };
    // what the book holds now, after any read of the company's just made
    let Some(read) = ctx.book.declared()?.remove(&id) else { return Ok(()) };
    if read.source != company.source() {
        return Ok(());
    }
    let today = ctx.now.to_zoned(ctx.bank.clone()).date();
    let due = if at_once { !lists_one_to_pay(&read.items, today) } else { market_due(&read, ctx.book.frequencies()?.get(&id), ctx.now, ctx.bank) };
    if due {
        read_with(ctx, need, market.as_ref(), Role::Market)?;
    }
    Ok(())
}

/// Read one payer now, whether or not its next distribution is due: a release
/// announcing one has just arrived, and the record is read again so what is told
/// of it is not a day behind (`SPEC.md` §2, Notifications, Releases).
pub fn read_at_once(ctx: &Ctx, need: &PayerNeed) -> Result<()> {
    let Some(adapter) = adapter_for(need) else { return Ok(()) };
    if read_with(ctx, need, adapter.as_ref(), Role::Payer)? == Some(OutcomeKind::NotCarried) {
        if let Some(market) = market_record_for(need).filter(|m| m.source() != adapter.source()) {
            read_with(ctx, need, market.as_ref(), Role::Payer)?;
        }
    }
    read_beside(ctx, need, adapter.as_ref(), true)
}

/// Which record a read is stored as.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    /// The payer's record, which the rate and schedule come from.
    Payer,
    /// The market's record beside a company's, read for the next distribution.
    Market,
}

/// One reader's read of one payer, unless its source rests: what it answered is
/// recorded, and what it states is stored.
fn read_with(ctx: &Ctx, need: &PayerNeed, adapter: &dyn Payer, role: Role) -> Result<Option<OutcomeKind>> {
    let id = need.listing.id;
    // a failed read waits out its source's rest; another source's failure for
    // the payer (its company's, the market's beside it) does not rest this one
    let subject = id.to_string();
    if ctx.source_resting(&subject, DataKind::Distributions, &adapter.source(), adapter.host())? {
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
    if role == Role::Market {
        // read beside a company's for a distribution paying cash still to be
        // paid: a row of no cash is not one (TMX lists a year-end distribution
        // paid in units as a row of 0 beside the cash row of the same ex-date)
        if let Outcome::Answered(record) = &mut noted.outcome {
            record.rows.retain(|r| !r.cash.is_zero());
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
        match role {
            Role::Payer => {
                ctx.book.store_declared(id, &stored_rows(&record), &adapter.source(), ctx.now)?;
                if let Some(n) = record.per_year {
                    ctx.book.store_frequency(id, n, &adapter.source(), Some(ctx.today()), ctx.now)?;
                }
            }
            // the rate and the schedule stay the company's: only the rows are kept
            Role::Market => ctx.book.store_market_declared(id, &stored_rows(&record), &adapter.source(), ctx.now)?,
        }
    }
    Ok(Some(kind))
}

/// The payers no adapter reads: shown as waiting on their payer, named.
pub fn unread(payers: &[PayerNeed]) -> Vec<&PayerNeed> {
    payers.iter().filter(|p| adapter_for(p).is_none()).collect()
}
