//! The market's record of a payer's distributions, for every held payer no
//! reader of its own company serves (brief 09): the exchange's record for a
//! Canadian listing (TMX's declared distributions, and the schedule TMX states
//! as `dividendFrequency`), Yahoo's dividend events for a US listing (which state
//! no schedule, so none is stored and the payer's annual income waits, named).
//!
//! The choice is by the listing's market alone: no brand, symbol or company is
//! named in it. A company reader, where one serves the payer, comes first
//! (`payers::all`), since a company states a schedule change first; its failed
//! read is that source's failure, retried after its rest, and never a switch to
//! the market's record. Each record is marked with the market source's name.

use bagholder_core::jiff::civil::date;
use bagholder_core::jiff::Timestamp;
use bagholder_core::SourceName;
use bagholder_net::Net;

use crate::adapters::{tmx, yahoo};
use crate::needs::PayerNeed;
use crate::outcome::{Noted, Outcome};
use crate::payers::{Distribution, Payer, Record};
use crate::venue;

/// A Canadian listing's record, on TMX.
pub struct TmxRecord;

impl Payer for TmxRecord {
    fn source(&self) -> SourceName {
        tmx::source()
    }

    fn host(&self) -> &'static str {
        tmx::HOST
    }

    fn brands(&self) -> &'static [&'static str] {
        &[]
    }

    fn markets(&self) -> &'static [crate::contract::Market] {
        crate::payers::CANADA
    }

    /// Every listing in its markets, whatever its name.
    fn serves(&self, need: &PayerNeed) -> bool {
        need.listing.market().is_some_and(|m| self.markets().contains(&m))
    }

    fn read(&self, net: &Net, need: &PayerNeed, _now: Timestamp) -> Noted<Record> {
        let l = &need.listing;
        let Some(form) = l.venue_mic.as_deref().and_then(|mic| venue::tmx_form(&l.symbol, mic)) else {
            return Noted { outcome: Outcome::NotCarried(format!("{} names no venue TMX carries", l.symbol)), shape_change: None };
        };
        let quote = tmx::ask_quote(net, &form);
        let per_year = match quote.outcome {
            // the quote's reader refuses an answer for another venue than the form's
            Outcome::Answered(q) => q.per_year,
            other => return Noted { outcome: other.failed().expect("not answered"), shape_change: quote.shape_change },
        };
        let rows = tmx::ask_dividends(net, &form);
        let shape_change = quote.shape_change.or(rows.shape_change);
        // TMX states each distribution's amount per unit and not whether it is paid
        // in cash or in units: the form is found from the record
        let outcome = rows.outcome.map(|rows| Record {
            form: bagholder_core::distribution::Form::Unstated,
            rows: rows.into_iter().map(|r| Distribution { ex_date: r.ex_date, record_date: r.record_date, pay_date: r.pay_date, cash: r.amount, reinvested: None, currency: r.currency }).collect(),
            per_year,
            by_record: vec![],
        });
        Noted { outcome, shape_change }
    }
}

/// A US listing's record, on Yahoo.
pub struct YahooRecord;

impl Payer for YahooRecord {
    fn source(&self) -> SourceName {
        yahoo::source()
    }

    fn host(&self) -> &'static str {
        yahoo::HOST
    }

    fn brands(&self) -> &'static [&'static str] {
        &[]
    }

    fn markets(&self) -> &'static [crate::contract::Market] {
        crate::payers::US
    }

    /// Every listing in its market, whatever its name.
    fn serves(&self, need: &PayerNeed) -> bool {
        need.listing.market().is_some_and(|m| self.markets().contains(&m))
    }

    fn read(&self, net: &Net, need: &PayerNeed, now: Timestamp) -> Noted<Record> {
        let l = &need.listing;
        let Some(form) = l.venue_mic.as_deref().and_then(|mic| venue::yahoo_forms(&l.symbol, mic).into_iter().next()) else {
            return Noted { outcome: Outcome::NotCarried(format!("{} names no venue Yahoo carries", l.symbol)), shape_change: None };
        };
        // every dividend event the chart holds, from before any fund's first trade
        let today = now.to_zoned(bagholder_core::jiff::tz::TimeZone::UTC).date();
        let chart = yahoo::ask_span(net, &form, date(2000, 1, 3), today, now);
        // Yahoo states each dividend event's amount and not whether it is paid in
        // cash or in units: the form is found from the record
        let outcome = chart.outcome.map(|c| Record {
            form: bagholder_core::distribution::Form::Unstated,
            rows: c.dividends.into_iter().map(|(ex, amount)| Distribution { ex_date: ex, record_date: None, pay_date: None, cash: amount, reinvested: None, currency: c.currency }).collect(),
            per_year: None,
            by_record: vec![],
        });
        Noted { outcome, shape_change: chart.shape_change }
    }
}
