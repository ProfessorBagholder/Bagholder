//! Live quotes into the market cache (`docs/plans/stage-3a-sources.md`,
//! "Quotes, daily closes and benchmarks"): each listing asked of its market's
//! source, every quote kept with the time its source states for it.
//!
//! - **Canadian listings (TSX, TSX Venture, CSE, Alpha):** TMX's quote, for the
//!   book's TMX form, else the one its venue gives. A form TMX answers for on the
//!   venue it asks is written back to the book as a routing reference. TMX states
//!   no trade time: its quote's time is when it served the quote, which is what
//!   is kept. It says the price is current then; it is never presented as the
//!   time of a trade.
//! - **Cboe Canada listings:** Cboe Canada's own quote.
//! - **US listings:** Yahoo's chart, its forms in order, the winner first, the
//!   day's change in points from the previous close the chart states.
//! - **Indices, futures, rates and currency pairs** (the app's directory): Yahoo's
//!   chart under the code the directory gives each (its `yahoo` route).
//! - **Coins:** Coinbase in the holding's own currency: the Exchange's ticker,
//!   which states its time, for a USD pair; otherwise the spot price, stamped with
//!   its reply's date less the age its origin allows. The day's change is against
//!   the close of the last completed UTC day on the pair's Coinbase Exchange
//!   market, the USD market's converted at that day's Bank of Canada rate where
//!   the pair has none (`SPEC.md` §2, Coinbase): the closes the cache keeps, else
//!   asked of the Exchange once a day and kept.
//!
//! Option contracts are quoted from their chains with their closes (`options`).

use bagholder_core::instrument::{InstrumentKind, RefScheme, Reference};
use bagholder_core::{Currency, InstrumentId, Money, SourceName};

use crate::adapters::{cboe_ca, coinbase, tmx, yahoo};
use crate::cache::StoredQuote;
use crate::contract::{DataKind, Listing, Market};
use crate::market::yahoo_forms;
use crate::outcome::{Noted, Outcome};
use crate::read::{Ctx, Result};
use crate::venue;

fn keep(ctx: &Ctx, id: InstrumentId, source: SourceName, price: Money, change: Option<bagholder_core::Dec>, change_pct: Option<bagholder_core::Dec>, at: bagholder_core::jiff::Timestamp, allowance: std::time::Duration) -> Result<()> {
    ctx.cache.store_quote(&StoredQuote { instrument: id, source, price, change, change_pct, quoted_at: at, allowance, received_at: ctx.now })?;
    Ok(())
}

fn not_carried(ctx: &Ctx, source: &SourceName, host: &str, id: InstrumentId, why: String) -> Result<()> {
    let noted: Noted<()> = Noted { outcome: Outcome::NotCarried(why), shape_change: None };
    ctx.record(source, host, DataKind::Quote, Some(id), &noted)
}

/// An answer in another currency than the listing's is for another listing: a
/// meaning failure, never kept.
pub(crate) fn same_currency<A>(outcome: &mut Outcome<A>, currency: impl Fn(&A) -> Currency, l: &Listing) {
    same_currency_as(outcome, currency, &l.symbol, l.currency)
}

fn same_currency_as<A>(outcome: &mut Outcome<A>, currency: impl Fn(&A) -> Currency, symbol: &str, want: Currency) {
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
            Some(Market::Canada) => {
                let routed = l.route(&RefScheme::TmxForm).map(str::to_string);
                let Some(form) = routed.clone().or_else(|| l.venue_mic.as_deref().and_then(|mic| venue::tmx_form(&l.symbol, mic))) else {
                    not_carried(ctx, &tmx::source(), tmx::HOST, id, format!("{} names no venue TMX quotes", l.symbol))?;
                    continue;
                };
                let mut noted = tmx::ask_quote(ctx.net, &form);
                same_currency(&mut noted.outcome, |q| q.currency, l);
                ctx.record_detail(&tmx::source(), tmx::HOST, DataKind::Quote, Some(id), &noted, &form)?;
                if let Outcome::Answered(q) = noted.outcome {
                    keep(ctx, id, tmx::source(), Money::new(q.price, q.currency), q.change, q.change_pct, q.datetime, std::time::Duration::ZERO)?;
                    // the reply is for the venue the form asks: the form is this listing's
                    if routed.is_none() {
                        ctx.book.add_instrument_ref(id, &Reference { scheme: RefScheme::TmxForm, value: form })?;
                    }
                }
            }
            Some(Market::CboeCanada) => {
                let symbol = venue::root(&l.symbol);
                let noted = cboe_ca::ask(ctx.net, &symbol);
                ctx.record_detail(&cboe_ca::source(), cboe_ca::HOST, DataKind::Quote, Some(id), &noted, &symbol)?;
                // Cboe Canada quotes its listings in their own currency; a listing with no
                // trade this session has no live quote, and is marked at its stored close
                if let Outcome::Answered(cboe_ca::CboeAnswer::Traded(q)) = noted.outcome {
                    keep(ctx, id, cboe_ca::source(), Money::new(q.price, l.currency), q.change, q.change_pct, q.at, std::time::Duration::ZERO)?;
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

/// A coin's day change at `price`, in points and in percent: against the close of
/// the last completed UTC day the cache keeps for it (the newest of the last four
/// days, as the Exchange closes a day at midnight UTC), converted into the price's
/// currency at that day's Bank of Canada rate when it was kept in another. With no
/// close of yesterday kept and none asked for today, the Exchange is asked for the
/// four days, each pair in turn, and what it answers kept.
fn coin_change(ctx: &Ctx, l: &Listing, price: Money) -> Result<(Option<bagholder_core::Dec>, Option<bagholder_core::Dec>)> {
    use bagholder_core::jiff::{tz::TimeZone, SignedDuration};
    use bagholder_core::{Dec, Rounding};
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
    for (d, close) in kept {
        let prev = if close.currency == price.currency {
            Some(close.amount)
        } else if price.currency == Currency::CAD {
            // a close in another currency, in CAD at that day's rate; a day with no rate is passed over
            rates.get(&close.currency).and_then(|r| r.get(&d)).and_then(|r| close.amount.checked_mul(*r).ok())
        } else {
            None
        };
        let Some(prev) = prev.filter(|p| !p.is_zero()) else { continue };
        let Ok(change) = price.amount.checked_sub(prev) else { continue };
        // in percent, as every source states a day change
    let hundred = Dec::new(100, 0).expect("a hundred");
    let pct = change.checked_mul(hundred).ok().and_then(|x| x.div_rounded(prev, 4, Rounding::HalfEven).ok());
        return Ok((Some(change), pct));
    }
    Ok((None, None))
}

/// A listing's quote for a glance: asked of its market's source, as a held
/// listing's would be, and kept nowhere (`SPEC.md` §4 Markets, Watchlist: the
/// add row's matches, the News card's chip, a listing's own page).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Glance {
    pub price: Money,
    /// The day's change in points, where the source states it or its previous close.
    pub change: Option<bagholder_core::Dec>,
    /// The day's change in percent, as stated.
    pub change_pct: Option<bagholder_core::Dec>,
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

/// The glance's quote, or why there is none: the first source's answer, else its
/// failure.
pub fn glance(net: &bagholder_net::Net, now: bagholder_core::jiff::Timestamp, g: &GlanceOf) -> Outcome<Glance> {
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
            Outcome::Answered(s) => Outcome::Answered(Glance { price: Money::new(s.price, g.currency), change: None, change_pct: None }),
            other => other.failed().unwrap_or(Outcome::Unreachable("no reply".into())),
        };
    }
    match venue::market_of(g.venue_mic.as_deref()) {
        Some(Market::Canada) => {
            let Some(form) = g.venue_mic.as_deref().and_then(|mic| venue::tmx_form(&g.symbol, mic)) else {
                return not_carried(format!("{} names no venue TMX quotes", g.symbol));
            };
            let mut noted = tmx::ask_quote(net, &form);
            same_currency_as(&mut noted.outcome, |q| q.currency, &g.symbol, g.currency);
            match noted.outcome {
                Outcome::Answered(q) => Outcome::Answered(Glance { price: Money::new(q.price, q.currency), change: q.change, change_pct: q.change_pct }),
                other => other.failed().unwrap_or(Outcome::Unreachable("no reply".into())),
            }
        }
        Some(Market::CboeCanada) => match cboe_ca::ask(net, &venue::root(&g.symbol)).outcome {
            Outcome::Answered(cboe_ca::CboeAnswer::Traded(q)) => Outcome::Answered(Glance { price: Money::new(q.price, g.currency), change: q.change, change_pct: q.change_pct }),
            // no trade this session: its last price is the previous close, and no change yet
            Outcome::Answered(cboe_ca::CboeAnswer::NoTradeYet { prev_close }) => Outcome::Answered(Glance { price: Money::new(prev_close, g.currency), change: None, change_pct: None }),
            other => other.failed().unwrap_or(Outcome::Unreachable("no reply".into())),
        },
        Some(Market::UnitedStates) => yahoo_first(g.venue_mic.as_deref().map(|mic| venue::yahoo_forms(&g.symbol, mic)).unwrap_or_default()),
        _ => not_carried(format!("{} names no venue a source quotes", g.symbol)),
    }
}
