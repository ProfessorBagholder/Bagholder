//! The holdings as the broker states them (brief 79 §1,
//! `docs/plans/every-amount-as-wealthsimple-states-it.md`): each account's
//! units and book value as its positions read states them, valued at
//! Bagholder's quote, the quote the only figure of Bagholder's own. Average
//! cost is the book value over the units; market value the units times the
//! quote; unrealized the market value less the book value.
//!
//! `holdings` reads what the broker states, the instruments' terms and the
//! market alone: no part of the history reaches it, so nothing the journal
//! works out can become an amount. A test scans this file to hold it.

use std::collections::BTreeMap;

use bagholder_core::instrument::InstrumentKind;
use bagholder_core::jiff::civil::Date;
use bagholder_core::{AccountId, Currency, Dec, InstrumentId, Money, Rounding};

use crate::fx::live_to_cad;
use crate::gap::{Fig, Gap, Gaps};
use crate::input::{Clock, InstrumentInfo, Market, Rates};
use crate::ledger::{multiplier, Direction};
use crate::positions::{close_mark_in, mark_in, Mark};
use crate::trades::PRICE_PLACES;

/// What the holdings are valued with: the instruments' terms, the market's
/// prices and the rates.
pub struct Sources<'a> {
    pub instruments: &'a BTreeMap<InstrumentId, InstrumentInfo>,
    pub market: &'a Market,
    pub rates: &'a Rates,
    pub clock: &'a Clock,
}

impl<'a> Sources<'a> {
    pub fn of(inputs: &'a crate::input::Inputs) -> Sources<'a> {
        Sources { instruments: &inputs.ledger.instruments, market: &inputs.market, rates: &inputs.facts.rates, clock: &inputs.clock }
    }
}

/// One account's holdings as one positions read states them: the units of each
/// instrument (a short's negative) and what they cost, in the currency stated.
pub struct Stated<'a> {
    pub account: AccountId,
    pub held: &'a BTreeMap<InstrumentId, Dec>,
    pub book: &'a BTreeMap<InstrumentId, Money>,
}

/// One holding: every amount of it the broker's, the quote aside.
#[derive(Clone, Debug, PartialEq)]
pub struct Holding {
    pub account: AccountId,
    pub instrument: InstrumentId,
    pub kind: InstrumentKind,
    /// The listing's own currency: its quote's, and its market value's.
    pub currency: Currency,
    pub direction: Direction,
    /// The units held, as stated (a short's as many as it owes).
    pub qty: Dec,
    pub multiplier: Fig<Dec>,
    /// The book value as stated, in the currency stated.
    pub book: Fig<Money>,
    /// The book value's currency, the instrument's where none is stated.
    pub book_currency: Currency,
    /// Book ÷ (qty × multiplier), in the book value's currency.
    pub avg: Fig<Dec>,
    pub mark: Fig<Mark>,
    /// Qty × price × multiplier, in the listing's currency.
    pub market: Fig<Money>,
    /// Market − book for a long, book − market for a short, in the book value's
    /// currency.
    pub unrealized: Fig<Money>,
    /// Qty × the day's change × multiplier, reversed for a short.
    pub day_change: Fig<Option<Money>>,
    pub book_cad: Fig<Money>,
    pub market_cad: Fig<Money>,
    pub unrealized_cad: Fig<Money>,
    pub day_change_cad: Fig<Option<Money>>,
    /// What the holding waits on.
    pub gaps: Gaps,
}

/// The holdings `stated` names, in the order it gives them, each instrument in
/// its own order: marked now, or, `as_of` a past day, at that day's close and
/// that day's rate. A line stating no units is no holding.
pub fn holdings<'a>(of: &Sources, stated: impl IntoIterator<Item = Stated<'a>>, as_of: Option<Date>) -> Vec<Holding> {
    let mut out = Vec::new();
    for s in stated {
        for (instrument, units) in s.held {
            if units.is_zero() {
                continue;
            }
            out.push(holding(of, s.account, *instrument, *units, s.book.get(instrument).copied(), as_of));
        }
    }
    out
}

/// The one holding of `instrument` that `stated` names, as `holdings` gives it.
pub fn holding_of(of: &Sources, stated: &Stated, instrument: InstrumentId, as_of: Option<Date>) -> Option<Holding> {
    let units = stated.held.get(&instrument).filter(|u| !u.is_zero())?;
    Some(holding(of, stated.account, instrument, *units, stated.book.get(&instrument).copied(), as_of))
}

fn holding(of: &Sources, account: AccountId, instrument: InstrumentId, units: Dec, book: Option<Money>, as_of: Option<Date>) -> Holding {
    let info = of.instruments.get(&instrument);
    let kind = info.map(|i| i.instrument.kind).unwrap_or(InstrumentKind::Security);
    let currency = info.map(|i| i.instrument.currency).unwrap_or(Currency::CAD);
    let direction = if units.is_negative() { Direction::Short } else { Direction::Long };
    let qty = units.abs();
    let mult = multiplier(info, instrument);
    let book_currency = book.map(|b| b.currency).unwrap_or(currency);
    // a short's book value is what it brought in, stated as Wealthsimple states it
    let book: Fig<Money> = book.map(|b| Money::new(b.amount.abs(), b.currency)).ok_or_else(|| Gaps::of(Gap::BookUnstated(instrument)));
    let shares: Fig<Dec> = mult.clone().and_then(|m| Ok(qty.checked_mul(m)?));
    let avg = match (&book, &shares) {
        (Ok(b), Ok(u)) if !u.is_zero() => b.amount.div_rounded(*u, PRICE_PLACES, Rounding::HalfEven).map_err(Gaps::from),
        (Ok(_), Ok(_)) => Ok(Dec::ZERO),
        (Err(g), _) | (_, Err(g)) => Err(g.clone()),
    };
    let mark = match as_of {
        Some(day) => close_mark_in(&of.market.closes, of.rates, of.clock, instrument, currency, day),
        None => mark_in(of.market, of.rates, of.clock, instrument, kind, currency),
    };
    let market: Fig<Money> = match (&mark, &shares) {
        (Ok(m), Ok(u)) => m.price.mul_to_fit(*u).map(|v| Money::new(v, currency)).map_err(Gaps::from),
        (Err(g), _) | (_, Err(g)) => Err(g.clone()),
    };
    let to_cad = |m: Money| match as_of {
        Some(day) => crate::fx::to_cad(of.rates, of.clock, m, day),
        None => live_to_cad(of.rates, of.clock, m),
    };
    // the market value in the book value's currency: through CAD where the two differ
    let in_book: Fig<Money> = market.clone().and_then(|m| {
        if m.currency == book_currency {
            Ok(m)
        } else {
            let day = as_of.unwrap_or(of.clock.today);
            crate::fx::convert(of.rates, of.clock, m, book_currency, day).map(|a| Money::new(a, book_currency))
        }
    });
    let unrealized = match (&in_book, &book) {
        (Ok(mk), Ok(bk)) => match direction {
            Direction::Long => mk.checked_sub(*bk).map_err(Gaps::from),
            Direction::Short => bk.checked_sub(*mk).map_err(Gaps::from),
        },
        (Err(g), Ok(_)) | (Ok(_), Err(g)) => Err(g.clone()),
        (Err(a), Err(b)) => {
            let mut g = a.clone();
            g.merge(b);
            Err(g)
        }
    };
    let day_change: Fig<Option<Money>> = match (&mark, &shares) {
        (Ok(m), Ok(u)) => match m.change.map(|c| c.mul_to_fit(*u)) {
            Some(Ok(v)) => Ok(Some(Money::new(if direction == Direction::Short { v.neg() } else { v }, currency))),
            Some(Err(e)) => Err(Gaps::from(e)),
            None => Ok(None),
        },
        (Err(g), _) | (_, Err(g)) => Err(g.clone()),
    };
    let cad = |f: &Fig<Money>| f.clone().and_then(to_cad);
    let day_change_cad = match &day_change {
        Ok(Some(m)) => to_cad(*m).map(Some),
        Ok(None) => Ok(None),
        Err(g) => Err(g.clone()),
    };
    let mut gaps = Gaps::none();
    for f in [&book, &market] {
        if let Err(g) = f {
            gaps.merge(g);
        }
    }
    Holding {
        account,
        instrument,
        kind,
        currency,
        direction,
        qty,
        multiplier: mult,
        book_cad: cad(&book),
        market_cad: cad(&market),
        unrealized_cad: cad(&unrealized),
        book,
        book_currency,
        avg,
        mark,
        market,
        unrealized,
        day_change,
        day_change_cad,
        gaps,
    }
}
