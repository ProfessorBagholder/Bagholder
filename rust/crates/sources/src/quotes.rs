//! Live quotes into the market cache (`docs/plans/stage-3a-sources.md`,
//! "Quotes, daily closes and benchmarks"): each listing asked of its market's
//! source, every quote kept with the time its source states for it.
//!
//! - **Canadian listings (TSX, TSX Venture, CSE, Alpha, and a CAD listing whose
//!   venue is none known):** TMX's quote, for the form that answered last, else
//!   the book's TMX form, else the one its venue gives (bare where it gives
//!   none); where that form is not carried, the other Canadian venues' forms in
//!   turn, each answer checked against the venue its form names (`SPEC.md` §2,
//!   TMX Money). The form that answers is remembered, and written back to the
//!   book as a routing reference where it is the book's own venue's; a listing
//!   no form answers for is asked again under its first form only, until the next
//!   day. A null price is TMX's word that it has none now (a halted listing): the
//!   price read last stands. TMX states no trade time: its quote's time is when it
//!   served the quote, which is what is kept. It says the price is current then;
//!   it is never presented as the time of a trade.
//! - **Cboe Canada listings:** Cboe Canada's own quote.
//! - **US listings:** Yahoo's chart, its forms in order, the winner first, the
//!   day's change in points from the previous close the chart states.
//! - **Indices, futures, rates and currency pairs** (the app's directory): Yahoo's
//!   chart under the code the directory gives each (its `yahoo` route).
//! - **Coins:** Coinbase in the holding's own currency: the Exchange's ticker,
//!   which states its time, for a USD pair; otherwise the spot price, stamped with
//!   its reply's date less the age its origin allows. The day's change is against
//!   the close of the last completed UTC day on the pair's Coinbase Exchange
//!   market, the USD market's where the pair has none, converted at the Bank of
//!   Canada's rate of that day or the last one published within the week before
//!   it, so a weekend's close takes Friday's rate (`SPEC.md` §2, Coinbase): the
//!   closes the cache keeps, else asked of the Exchange once a day and kept.
//!
//! Option contracts are quoted from their chains with their closes (`options`).

use std::collections::BTreeMap;

use bagholder_core::instrument::{InstrumentKind, RefScheme, Reference};
use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::{tz::TimeZone, SignedDuration, Timestamp};
use bagholder_core::{Currency, Dec, InstrumentId, Money, SourceName};

use crate::adapters::{cboe_ca, coinbase, tmx, yahoo};
use crate::cache::StoredQuote;
use crate::contract::{DataKind, Listing, Market};
use crate::market::yahoo_forms;
use crate::outcome::{Noted, Outcome};
use crate::read::{Ctx, Result};
use crate::venue;

fn keep(ctx: &Ctx, id: InstrumentId, source: SourceName, price: Money, change: Option<Dec>, change_pct: Option<Dec>, at: Timestamp, allowance: std::time::Duration) -> Result<()> {
    ctx.cache.store_quote(&StoredQuote { instrument: id, source, price, change, change_pct, quoted_at: at, allowance, received_at: ctx.now })?;
    Ok(())
}

fn not_carried(ctx: &Ctx, source: &SourceName, host: &str, id: InstrumentId, why: String) -> Result<()> {
    let noted: Noted<()> = Noted { outcome: Outcome::NotCarried(why), shape_change: None };
    ctx.record(source, host, DataKind::Quote, Some(id), &noted)
}

/// A day's change in percent of the close it is measured from, as every source
/// states one: to four places.
pub fn percent_of(change: Dec, base: Dec) -> Option<Dec> {
    if base.is_zero() {
        return None;
    }
    change.checked_mul(Dec::new(100, 0).ok()?).ok()?.div_rounded(base, 4, bagholder_core::Rounding::HalfEven).ok()
}

/// An answer in another currency than the listing's is for another listing: a
/// meaning failure, never kept.
pub(crate) fn same_currency<A>(outcome: &mut Outcome<A>, currency: impl Fn(&A) -> Currency, l: &Listing) {
    same_currency_as(outcome, currency, &l.symbol, l.currency)
}

pub(crate) fn same_currency_as<A>(outcome: &mut Outcome<A>, currency: impl Fn(&A) -> Currency, symbol: &str, want: Currency) {
    if let Outcome::Answered(a) = &*outcome {
        let c = currency(a);
        if c != want {
            *outcome = Outcome::Meaning(format!("the answer for {symbol} is in {c}, the listing's currency is {want}"));
        }
    }
}

/// Ask Yahoo for a listing's quote under each of its forms in turn, the one that
/// answered last first; the first answer is kept, with the day's change in points
/// from the previous close the chart states.
fn ask_yahoo(ctx: &Ctx, l: &Listing, mut forms: Vec<String>) -> Result<()> {
    let id = l.id;
    if let Some((_, won)) = ctx.cache.winner(id, DataKind::Quote)? {
        forms.sort_by_key(|f| *f != won);
    }
    for form in forms {
        let mut noted = yahoo::ask_quote(ctx.net, &form, ctx.now);
        same_currency(&mut noted.outcome, |c| c.currency, l);
        ctx.record_detail(&yahoo::source(), yahoo::HOST, DataKind::Quote, Some(id), &noted, &form)?;
        match noted.outcome {
            Outcome::Answered(c) => {
                let change = c.previous_close.and_then(|p| c.quote.price.checked_sub(p).ok());
                keep(ctx, id, yahoo::source(), Money::new(c.quote.price, c.currency), change, c.quote.change_pct, c.quote.at, std::time::Duration::ZERO)?;
                ctx.cache.won(id, DataKind::Quote, &yahoo::source(), &form, ctx.now)?;
                break;
            }
            Outcome::NotCarried(_) => continue,
            _ => break,
        }
    }
    Ok(())
}

/// Read each listing's quote once.
pub fn read_quotes(ctx: &Ctx, listings: &[Listing]) -> Result<()> {
    for l in listings {
        let id = l.id;
        // an index, a future, a rate or a pair: Yahoo, under the code the app's
        // directory gives it (its `yahoo` route)
        if matches!(l.kind, InstrumentKind::Index | InstrumentKind::Future | InstrumentKind::Rate | InstrumentKind::CurrencyPair) {
            let forms: Vec<String> = l.routes.get(&RefScheme::Yahoo).cloned().unwrap_or_default();
            if forms.is_empty() {
                not_carried(ctx, &yahoo::source(), yahoo::HOST, id, format!("{} has no code Yahoo quotes it under", l.symbol))?;
                continue;
            }
            ask_yahoo(ctx, l, forms)?;
            continue;
        }
        match l.market() {
            Some(Market::Canada) => read_tmx(ctx, l)?,
            Some(Market::CboeCanada) => {
                let symbol = venue::root(&l.symbol);
                let noted = cboe_ca::ask(ctx.net, &symbol);
                ctx.record_detail(&cboe_ca::source(), cboe_ca::HOST, DataKind::Quote, Some(id), &noted, &symbol)?;
                // Cboe Canada quotes its listings in their own currency; a listing with no
                // trade this session (a weekend, or before the first trade) stands at the
                // previous session's close, which Cboe states: that is its price now
                match noted.outcome {
                    Outcome::Answered(cboe_ca::CboeAnswer::Traded(q)) => keep(ctx, id, cboe_ca::source(), Money::new(q.price, l.currency), q.change, q.change_pct, q.at, std::time::Duration::ZERO)?,
                    Outcome::Answered(cboe_ca::CboeAnswer::NoTradeYet { prev_close }) => keep(ctx, id, cboe_ca::source(), Money::new(prev_close, l.currency), None, None, ctx.now, std::time::Duration::ZERO)?,
                    _ => {}
                }
            }
            Some(Market::UnitedStates) => {
                let forms = yahoo_forms(l);
                if forms.is_empty() {
                    not_carried(ctx, &yahoo::source(), yahoo::HOST, id, format!("{} names no venue Yahoo carries", l.symbol))?;
                    continue;
                }
                ask_yahoo(ctx, l, forms)?;
            }
            Some(Market::Crypto) => {
                let base = l.symbol.trim().to_ascii_uppercase();
                if l.currency == Currency::USD {
                    let pair = format!("{base}-USD");
                    let noted = coinbase::ask_ticker(ctx.net, &pair);
                    ctx.record_detail(&coinbase::exchange_source(), coinbase::EXCHANGE_HOST, DataKind::Quote, Some(id), &noted, &pair)?;
                    if let Outcome::Answered(s) = noted.outcome {
                        let price = Money::new(s.price, Currency::USD);
                        let (change, pct) = coin_change(ctx, l, price)?;
                        keep(ctx, id, coinbase::exchange_source(), price, change, pct, s.at, s.allowance)?;
                    }
                } else {
                    let pair = format!("{base}-{}", l.currency.as_str());
                    let noted = coinbase::ask_spot(ctx.net, &base, l.currency);
                    ctx.record_detail(&coinbase::spot_source(), coinbase::SPOT_HOST, DataKind::Quote, Some(id), &noted, &pair)?;
                    if let Outcome::Answered(s) = noted.outcome {
                        let price = Money::new(s.price, l.currency);
                        let (change, pct) = coin_change(ctx, l, price)?;
                        keep(ctx, id, coinbase::spot_source(), price, change, pct, s.at, s.allowance)?;
                    }
                }
            }
            // option contracts: from their chains, with the option closes
            Some(Market::UsOptions) | None => {}
        }
    }
    Ok(())
}

/// Read a Canadian listing's quote from TMX: the form that answered last first,
/// else the book's, else its venue's own; where that is not carried, the other
/// Canadian forms in turn, unless none answered earlier today.
fn read_tmx(ctx: &Ctx, l: &Listing) -> Result<()> {
    let id = l.id;
    let source = tmx::source();
    let routed = l.route(&RefScheme::TmxForm).map(str::to_string);
    // the venue's own form, where the book names a venue TMX does
    let own = l.venue_mic.as_deref().and_then(|mic| venue::tmx_form(&l.symbol, mic));
    let won = ctx.cache.winner(id, DataKind::Quote)?.filter(|(by, _)| *by == source).map(|w| w.1);
    let mut forms: Vec<String> = Vec::new();
    for f in won.into_iter().chain(routed.clone()).chain(venue::tmx_forms(&l.symbol, l.venue_mic.as_deref(), l.currency)) {
        if !forms.contains(&f) {
            forms.push(f);
        }
    }
    if forms.is_empty() {
        return not_carried(ctx, &source, tmx::HOST, id, format!("{} names no venue TMX quotes", l.symbol));
    }
    // no form answered earlier today: only the first is asked until tomorrow
    let subject = id.to_string();
    let today = ctx.today();
    let missed = ctx.cache.reads(&subject, DataKind::Quote)?.iter().any(|r| r.source == source && r.outcome == crate::outcome::OutcomeKind::NotCarried && r.first == today);
    if missed {
        forms.truncate(1);
    }
    for form in forms {
        let mut noted = tmx::ask_quote(ctx.net, &form);
        same_currency(&mut noted.outcome, |q| q.currency, l);
        ctx.record_detail(&source, tmx::HOST, DataKind::Quote, Some(id), &noted, &form)?;
        match noted.outcome {
            Outcome::Answered(q) => {
                // a null price: TMX has none now, and the price read last stands
                if let Some(price) = q.price {
                    keep(ctx, id, source.clone(), Money::new(price, q.currency), q.change, q.change_pct, q.datetime, std::time::Duration::ZERO)?;
                }
                ctx.cache.won(id, DataKind::Quote, &source, &form, ctx.now)?;
                // the reply is for the venue the form asks: where that is the venue
                // the book names, the form is this listing's
                if routed.is_none() && own.as_deref() == Some(form.as_str()) {
                    ctx.book.add_instrument_ref(id, &Reference { scheme: RefScheme::TmxForm, value: form })?;
                }
                return Ok(());
            }
            Outcome::NotCarried(_) => continue,
            // a failure stops the chain: nothing is learned from no answer
            _ => return Ok(()),
        }
    }
    if !missed {
        ctx.cache.store_read(&subject, DataKind::Quote, &crate::cache::ReadRow { source, first: today, last: today, outcome: crate::outcome::OutcomeKind::NotCarried, at: ctx.now })?;
    }
    Ok(())
}

/// A coin's day change at `price`, in points and in percent: against the close of
/// the last completed UTC day the cache keeps for it (the newest of the last four
/// days, as the Exchange closes a day at midnight UTC), converted into the price's
/// currency at the Bank of Canada's rate of that day, or the last one published
/// within the week before it, when it was kept in another. With no
/// close of yesterday kept and none asked for today, the Exchange is asked for the
/// four days, each pair in turn, and what it answers kept.
fn coin_change(ctx: &Ctx, l: &Listing, price: Money) -> Result<(Option<Dec>, Option<Dec>)> {
    let today = ctx.now.to_zoned(TimeZone::UTC).date();
    let day = |n: i64| today.checked_sub(SignedDuration::from_hours(24 * n));
    let (Ok(yesterday), Ok(from)) = (day(1), day(4)) else { return Ok((None, None)) };
    let mut kept = ctx.cache.closes_between(l.id, from, yesterday)?;
    if kept.first().map(|c| c.0) != Some(yesterday) {
        let subject = l.id.to_string();
        let midnight = today.to_zoned(TimeZone::UTC).map(|z| z.timestamp()).ok();
        let asked_today = ctx.cache.reads(&subject, DataKind::DailyClose)?.iter().any(|r| r.first <= yesterday && yesterday <= r.last && midnight.is_some_and(|m| r.at >= m));
        if !asked_today {
            let source = coinbase::exchange_source();
            let mut result = crate::outcome::OutcomeKind::NotCarried;
            let mut held_last = None;
            for pair in crate::market::coin_pairs(l) {
                let noted = coinbase::ask_candles(ctx.net, &pair, from, yesterday, ctx.now);
                ctx.record_detail(&source, coinbase::EXCHANGE_HOST, DataKind::DailyClose, Some(l.id), &noted, &pair)?;
                result = noted.outcome.kind();
                if let Outcome::Answered(closes) = noted.outcome {
                    let currency = pair.rsplit('-').next().and_then(|c| Currency::parse(c).ok()).unwrap_or(l.currency);
                    held_last = closes.last().map(|c| c.0);
                    crate::market::store(ctx, l.id, &closes, currency, &source, coinbase::EXCHANGE_HOST)?;
                }
                if result != crate::outcome::OutcomeKind::NotCarried {
                    break;
                }
            }
            crate::market::keep_read(ctx, &subject, DataKind::DailyClose, &source, (from, yesterday), result, held_last)?;
            kept = ctx.cache.closes_between(l.id, from, yesterday)?;
        }
    }
    let rates = if kept.iter().any(|(_, c)| c.currency != price.currency) { ctx.book.rates()? } else { Default::default() };
    Ok(change_against(price, &kept, &rates))
}

/// The day change of `price` against the newest of `closes` (newest first) that
/// can be read in its currency: a close in USD, for a CAD price, at the Bank's
/// rate of its day or the last one published within the week before it; a close
/// no rate converts is passed over for the one before.
fn change_against(price: Money, closes: &[(Date, Money)], rates: &BTreeMap<Currency, BTreeMap<Date, Dec>>) -> (Option<Dec>, Option<Dec>) {
    for (d, close) in closes {
        let prev = if close.currency == price.currency {
            Some(close.amount)
        } else if price.currency == Currency::CAD {
            let week_before = d.checked_sub(SignedDuration::from_hours(24 * 6)).ok();
            rates.get(&close.currency).and_then(|r| r.range(..=*d).next_back()).filter(|(on, _)| week_before.is_some_and(|w| **on >= w)).and_then(|(_, r)| close.amount.checked_mul(*r).ok())
        } else {
            None
        };
        let Some(prev) = prev.filter(|p| !p.is_zero()) else { continue };
        let Ok(change) = price.amount.checked_sub(prev) else { continue };
        return (Some(change), percent_of(change, prev));
    }
    (None, None)
}

/// A listing's quote for a glance: asked of its market's source, as a held
/// listing's would be, and kept nowhere (`SPEC.md` §4 Markets, Watchlist: the
/// add row's matches, the News card's chip, a listing's own page).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Glance {
    pub price: Money,
    /// The day's change in points, where the source states it or its previous close.
    pub change: Option<Dec>,
    /// The day's change in percent, as stated, else from its previous close.
    pub change_pct: Option<Dec>,
}

/// What a glance is asked for: a listing no record may name yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlanceOf {
    pub kind: InstrumentKind,
    pub currency: Currency,
    pub symbol: String,
    pub venue_mic: Option<String>,
    /// The code Yahoo quotes an index, a future, a rate or a pair under.
    pub yahoo: Option<String>,
}

/// A coin's last completed UTC day's close for a glance, newest first: asked of
/// the Exchange, its own pair's market first, else its USD market's.
fn glance_closes(net: &bagholder_net::Net, now: Timestamp, symbol: &str, currency: Currency) -> Vec<(Date, Money)> {
    let today = now.to_zoned(TimeZone::UTC).date();
    let day = |n: i64| today.checked_sub(SignedDuration::from_hours(24 * n));
    let (Ok(yesterday), Ok(from)) = (day(1), day(4)) else { return vec![] };
    let base = symbol.trim().to_ascii_uppercase();
    let mut pairs = vec![(format!("{base}-{}", currency.as_str()), currency)];
    if currency != Currency::USD {
        pairs.push((format!("{base}-USD"), Currency::USD));
    }
    for (pair, quoted) in pairs {
        match coinbase::ask_candles(net, &pair, from, yesterday, now).outcome {
            Outcome::Answered(closes) => return closes.into_iter().rev().map(|(d, c)| (d, Money::new(c, quoted))).collect(),
            Outcome::NotCarried(_) => continue,
            _ => break,
        }
    }
    vec![]
}

/// The glance's quote, or why there is none: the first source's answer, else its
/// failure. `rates` are the Bank's, for a coin's close kept in another currency
/// than the one it is glanced in.
pub fn glance(net: &bagholder_net::Net, now: Timestamp, g: &GlanceOf, rates: &BTreeMap<Currency, BTreeMap<Date, Dec>>) -> Outcome<Glance> {
    let not_carried = |why: String| Outcome::NotCarried(why);
    let yahoo_first = |forms: Vec<String>| -> Outcome<Glance> {
        let mut last = not_carried(format!("{} names no venue Yahoo carries", g.symbol));
        for form in forms {
            let mut noted = yahoo::ask_quote(net, &form, now);
            same_currency_as(&mut noted.outcome, |c| c.currency, &g.symbol, g.currency);
            match noted.outcome {
                Outcome::Answered(c) => {
                    let change = c.previous_close.and_then(|p| c.quote.price.checked_sub(p).ok());
                    return Outcome::Answered(Glance { price: Money::new(c.quote.price, c.currency), change, change_pct: c.quote.change_pct });
                }
                Outcome::NotCarried(why) => last = not_carried(why),
                other => return other.failed().unwrap_or(last),
            }
        }
        last
    };
    if matches!(g.kind, InstrumentKind::Index | InstrumentKind::Future | InstrumentKind::Rate | InstrumentKind::CurrencyPair) {
        return yahoo_first(g.yahoo.clone().into_iter().collect());
    }
    if g.kind == InstrumentKind::Crypto {
        let base = g.symbol.trim().to_ascii_uppercase();
        let noted = if g.currency == Currency::USD { coinbase::ask_ticker(net, &format!("{base}-USD")) } else { coinbase::ask_spot(net, &base, g.currency) };
        return match noted.outcome {
            Outcome::Answered(s) => {
                // against the last completed UTC day's close, as a held coin's is
                let price = Money::new(s.price, g.currency);
                let (change, change_pct) = change_against(price, &glance_closes(net, now, &g.symbol, g.currency), rates);
                Outcome::Answered(Glance { price, change, change_pct })
            }
            other => other.failed().unwrap_or(Outcome::Unreachable("no reply".into())),
        };
    }
    match venue::market_of_listing(g.venue_mic.as_deref(), g.currency) {
        Some(Market::Canada) => {
            let mut last = not_carried(format!("{} names no venue TMX quotes", g.symbol));
            for form in venue::tmx_forms(&g.symbol, g.venue_mic.as_deref(), g.currency) {
                let mut noted = tmx::ask_quote(net, &form);
                same_currency_as(&mut noted.outcome, |q| q.currency, &g.symbol, g.currency);
                match noted.outcome {
                    Outcome::Answered(q) => {
                        return match q.price {
                            Some(price) => Outcome::Answered(Glance { price: Money::new(price, q.currency), change: q.change, change_pct: q.change_pct }),
                            // a halted listing: TMX has no price for it now
                            None => not_carried(format!("TMX states no price for {form} now")),
                        };
                    }
                    Outcome::NotCarried(why) => last = not_carried(why),
                    other => return other.failed().unwrap_or(last),
                }
            }
            last
        }
        Some(Market::CboeCanada) => match cboe_ca::ask(net, &venue::root(&g.symbol)).outcome {
            Outcome::Answered(cboe_ca::CboeAnswer::Traded(q)) => Outcome::Answered(Glance { price: Money::new(q.price, g.currency), change: q.change, change_pct: q.change_pct }),
            // no trade this session: its last price is the previous close, and no change yet
            Outcome::Answered(cboe_ca::CboeAnswer::NoTradeYet { prev_close }) => Outcome::Answered(Glance { price: Money::new(prev_close, g.currency), change: None, change_pct: None }),
            other => other.failed().unwrap_or(Outcome::Unreachable("no reply".into())),
        },
        Some(Market::UnitedStates) => yahoo_first(venue::yahoo_forms_of(&g.symbol, g.venue_mic.as_deref(), g.currency)),
        _ => not_carried(format!("{} names no venue a source quotes", g.symbol)),
    }
}
