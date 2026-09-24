//! The Bank of Canada's rate for a day (`docs/architecture.md` §7).
//!
//! A transaction uses the Bank's rate for its day when that is a business day,
//! otherwise the previous business day's (the Canadian convention, the one the
//! Canada Revenue Agency applies). What is not a business day is read from the
//! Bank alone: a weekend, a day in its own holiday schedule, or a weekday a
//! completed read of its series covered and found no rate for. A weekday no read
//! has covered is a business day whose rate is not stored: before 16:30 Eastern
//! on that day it is pending, the only time a rate does not exist yet, and after
//! it a failure of the Bank source. Nothing is ever covered by a number of the
//! app's own.
//!
//! The Bank's rate is held in eras, each a series: its daily average from
//! 2017-01-03, its noon rate before that, and Statistics Canada's archive of the
//! noon rate before 2007 for the currencies it holds. A day no series holds is
//! one no source holds a published rate for: before a currency's oldest series,
//! between two (the Bank published no zloty from its noon series' end in 2017 to
//! its daily series' start in 2026), or after the last one ended (the Bank ended
//! its daily ruble series in 2026).
//!
//! A live mark (a holding's value now) is not a transaction on a day: it uses the
//! latest rate the Bank has published, and that rate is a failure once a later
//! business day's 16:30 has passed without it being stored.

use bagholder_core::jiff::civil::{Date, Weekday};
use bagholder_core::{Currency, Dec, Money};

use crate::gap::{Fig, Gap, Gaps};
use crate::input::{Clock, Rates};

fn is_weekend(d: Date) -> bool {
    matches!(d.weekday(), Weekday::Saturday | Weekday::Sunday)
}

/// Whether the Bank does not publish on `d`, as far as the Bank itself says: a
/// weekend, its holiday, or a day a completed read covered and found no rate
/// for, read after that day's rate was due.
fn not_published(rates: &Rates, clock: &Clock, currency: Currency, d: Date) -> bool {
    is_weekend(d)
        || rates.holidays.contains(&d)
        || rates.covered.get(&currency).is_some_and(|reads| {
            reads.iter().any(|r| r.first <= d && d <= r.last && published_by(clock, d).is_some_and(|due| r.at >= due))
        })
}

/// The moment the Bank publishes a business day's rate: 16:30 in its zone.
fn published_by(clock: &Clock, d: Date) -> Option<bagholder_core::jiff::Timestamp> {
    d.at(16, 30, 0, 0).to_zoned(clock.bank.clone()).ok().map(|z| z.timestamp())
}

fn waiting(clock: &Clock, currency: Currency, d: Date) -> Gaps {
    Gaps::of(match published_by(clock, d) {
        Some(at) if clock.now < at => Gap::RatePending { currency, day: d },
        _ => Gap::RateMissing { currency, day: d },
    })
}

fn check_published(rates: &Rates, currency: Currency, day: Date) -> Result<(), Gaps> {
    match rates.series.get(&currency).filter(|s| !s.is_empty()) {
        Some(s) if !s.iter().any(|series| series.holds(day)) => Err(Gaps::of(Gap::RateNotHeld { currency, day })),
        Some(_) => Ok(()),
        // until the lists of series have been read, a currency with no series is
        // a rate not read, not a currency the Bank does not publish
        None if rates.series.is_empty() => {
            if rates.by_currency.get(&currency).is_some_and(|s| !s.is_empty()) {
                Ok(())
            } else {
                Err(Gaps::of(Gap::RateMissing { currency, day }))
            }
        }
        None => Err(Gaps::of(Gap::RateUnpublished(currency))),
    }
}

/// CAD per unit of `currency` for a transaction on `day`.
pub fn rate(rates: &Rates, clock: &Clock, currency: Currency, day: Date) -> Fig<Dec> {
    if currency == Currency::CAD {
        return Ok(Dec::ONE);
    }
    check_published(rates, currency, day)?;
    let series = rates.by_currency.get(&currency);
    let mut d = day;
    loop {
        if let Some(r) = series.and_then(|s| s.get(&d)) {
            return Ok(*r);
        }
        if !not_published(rates, clock, currency, d) {
            // the business day that governs `day`, without its rate
            return Err(waiting(clock, currency, d));
        }
        d = match d.yesterday() {
            Ok(y) => y,
            Err(_) => return Err(Gaps::of(Gap::RateMissing { currency, day })),
        };
        // walked back out of every series: no source holds the governing day
        check_published(rates, currency, d)?;
    }
}

/// An amount in `to` on `day`: itself when it is in `to`; in CAD, the exact
/// product with the day's rate; in another currency, through CAD, the quotient
/// taken to eighteen places.
pub fn convert(rates: &Rates, clock: &Clock, amount: Money, to: Currency, day: Date) -> Fig<Dec> {
    if amount.currency == to {
        return Ok(amount.amount);
    }
    let cad = to_cad(rates, clock, amount, day)?.amount;
    if to == Currency::CAD {
        return Ok(cad);
    }
    let r = rate(rates, clock, to, day)?;
    Ok(cad.div_rounded(r, 18, bagholder_core::Rounding::HalfEven)?)
}

/// An amount in CAD for a transaction on `day`: the exact product.
pub fn to_cad(rates: &Rates, clock: &Clock, amount: Money, day: Date) -> Fig<Money> {
    if amount.currency == Currency::CAD {
        return Ok(amount);
    }
    let r = rate(rates, clock, amount.currency, day)?;
    Ok(Money::new(amount.amount.checked_mul(r)?, Currency::CAD))
}

/// The rate a live mark uses, and the day it is the Bank's rate for: the latest
/// published, unless a business day's 16:30 has passed since without its rate.
pub fn live_rate(rates: &Rates, clock: &Clock, currency: Currency) -> Fig<(Dec, Option<Date>)> {
    if currency == Currency::CAD {
        return Ok((Dec::ONE, None));
    }
    check_published(rates, currency, clock.today)?;
    let latest = rates.by_currency.get(&currency).and_then(|s| s.iter().next_back()).map(|(d, r)| (*d, *r));
    // the latest business day whose rate should be out by now
    let mut d = clock.today;
    loop {
        if latest.is_some_and(|(l, _)| d <= l) {
            break;
        }
        let due = published_by(clock, d).is_some_and(|at| clock.now >= at);
        if due && !not_published(rates, clock, currency, d) {
            return Err(Gaps::of(Gap::RateMissing { currency, day: d }));
        }
        d = match d.yesterday() {
            Ok(y) => y,
            Err(_) => break,
        };
    }
    match latest {
        Some((day, r)) => Ok((r, Some(day))),
        None => Err(Gaps::of(Gap::RateMissing { currency, day: clock.today })),
    }
}

/// An amount in CAD now, at the live rate.
pub fn live_to_cad(rates: &Rates, clock: &Clock, amount: Money) -> Fig<Money> {
    if amount.currency == Currency::CAD {
        return Ok(amount);
    }
    let (r, _) = live_rate(rates, clock, amount.currency)?;
    Ok(Money::new(amount.amount.checked_mul(r)?, Currency::CAD))
}
