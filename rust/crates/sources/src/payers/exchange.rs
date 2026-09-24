//! The exchange-side record, for the two fund companies whose own publication
//! cannot be read at all (the owner's exception of 2026-09-24; research 2):
//!
//! - **Mackenzie** (QCN, QUU): TMX's declared distributions for the Canadian
//!   listing, and the schedule TMX states (`dividendFrequency`, `Quarterly` for
//!   both). The quote is read first: it says whether TMX knows the listing, on
//!   the venue the book names, and states the schedule.
//! - **WisdomTree** (WQTM): Yahoo's dividend events for the US listing. No source
//!   states its schedule, so none is stored, and the fund's annual income waits,
//!   named.
//!
//! Each is marked with the exchange-side source's name, never the company's. It
//! is a fixed list, not a fallback: a fund company whose own page fails to answer
//! is a failure of that source.

use bagholder_core::jiff::civil::date;
use bagholder_core::jiff::Timestamp;
use bagholder_core::SourceName;
use bagholder_net::Net;

use crate::adapters::{tmx, yahoo};
use crate::needs::PayerNeed;
use crate::outcome::{Noted, Outcome};
use crate::payers::{Distribution, Payer, Record};
use crate::venue;

pub struct Mackenzie;

impl Payer for Mackenzie {
    fn source(&self) -> SourceName {
        tmx::source()
    }

    fn host(&self) -> &'static str {
        tmx::HOST
    }

    fn brands(&self) -> &'static [&'static str] {
        &["mackenzie"]
    }

    fn markets(&self) -> &'static [crate::contract::Market] {
        crate::payers::CANADA
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
        let outcome = rows.outcome.map(|rows| Record {
            rows: rows.into_iter().map(|r| Distribution { ex_date: r.ex_date, record_date: r.record_date, pay_date: r.pay_date, cash: r.cash, reinvested: r.in_units, currency: r.currency }).collect(),
            per_year,
        });
        Noted { outcome, shape_change }
    }
}

pub struct WisdomTree;

impl Payer for WisdomTree {
    fn source(&self) -> SourceName {
        yahoo::source()
    }

    fn host(&self) -> &'static str {
        yahoo::HOST
    }

    fn brands(&self) -> &'static [&'static str] {
        &["wisdomtree"]
    }

    fn markets(&self) -> &'static [crate::contract::Market] {
        crate::payers::US
    }

    fn read(&self, net: &Net, need: &PayerNeed, now: Timestamp) -> Noted<Record> {
        let l = &need.listing;
        let Some(form) = l.venue_mic.as_deref().and_then(|mic| venue::yahoo_forms(&l.symbol, mic).into_iter().next()) else {
            return Noted { outcome: Outcome::NotCarried(format!("{} names no venue Yahoo carries", l.symbol)), shape_change: None };
        };
        // every dividend event the chart holds, from before any fund's first trade
        let today = now.to_zoned(bagholder_core::jiff::tz::TimeZone::UTC).date();
        let chart = yahoo::ask_span(net, &form, date(2000, 1, 3), today, now);
        let outcome = chart.outcome.map(|c| Record {
            rows: c.dividends.into_iter().map(|(ex, amount)| Distribution { ex_date: ex, record_date: None, pay_date: None, cash: amount, reinvested: None, currency: c.currency }).collect(),
            per_year: None,
        });
        Noted { outcome, shape_change: chart.shape_change }
    }
}
