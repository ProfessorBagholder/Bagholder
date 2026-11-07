//! Quotes from Wealthsimple, while the person is signed in, for every share and
//! fund it carries (the owner, 2026-11-06; `SPEC.md` `market-data-37`): the
//! person's own broker's price, as every other amount of theirs is Wealthsimple's,
//! twenty listings to a request (`FetchSecuritiesSummary`, the order ticket's own
//! quote) where the public feeds were asked once a listing. A listing it carries
//! is quoted by it alone while a session exists: a failed read is asked again at
//! its turn, never answered from Yahoo or TMX in between. The public feeds quote
//! what it does not carry (the tiles' indices, futures, rates and pairs, coins,
//! option contracts), and every listing while no one is signed in.

use std::sync::Arc;

use bagholder_core::instrument::{InstrumentKind, RefScheme};
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Broker, Currency, Dec, Money, SourceName};
use bagholder_sources::cache::StoredQuote;
use bagholder_sources::contract::{DataKind, Listing, Market};
use bagholder_sources::outcome::{Noted, Outcome};
use bagholder_sources::read::Ctx;
use bagholder_ws::session::Session;
use bagholder_ws::wire;
use serde_json::json;

use crate::app::App;

/// Securities to a request, as the balances are asked: Wealthsimple's own web
/// client (its build `assets.wealthsimple.com/app-378b1c4acb7ba965.js`, read
/// 2026-11-07) sends no request for several securities' quotes to take a size from
/// (it asks `FetchSecurityQuoteV2` or `FetchSecuritySummary` of one security, and
/// its holdings and watchlist by the person), so a batch it refuses whole is asked
/// one security at a time from the next pass (`read`): a large one can only slow a
/// pass, never stop its prices.
pub const CHUNK: usize = 20;

/// The source a Wealthsimple quote is kept under.
pub fn source() -> SourceName {
    SourceName::named("wealthsimple")
}

/// The Wealthsimple security `l` is quoted under: a share or fund on a venue in
/// Canada or the US whose Wealthsimple id the book holds; `None` for anything
/// else, which the public feeds quote.
pub fn carried(l: &Listing) -> Option<&str> {
    let on_its_venues = matches!(l.market(), Some(Market::Canada | Market::CboeCanada | Market::UnitedStates | Market::Otc));
    (l.kind == InstrumentKind::Security && on_its_venues).then(|| l.route(&RefScheme::BrokerSecurity(Broker::named("wealthsimple")))).flatten()
}

/// The session quotes are asked with: the orders' own while Wealthsimple holds the
/// person signed in; none signed out, or once Wealthsimple has ended the sign-in.
pub fn session(app: &Arc<App>) -> Option<Session> {
    if !app.state.lock().unwrap().connected {
        return None;
    }
    crate::orders::ticket_session(app).ok()
}

/// What quotes each listing: Wealthsimple the ones it carries while signed in, the
/// public feeds the rest.
pub fn split(listings: &[Listing], signed_in: bool) -> (Vec<Listing>, Vec<Listing>) {
    listings.iter().cloned().partition(|l| signed_in && carried(l).is_some())
}

/// Quote `listings` (each carried) from Wealthsimple, `CHUNK` to a request, each
/// answer kept as it lands and its outcome recorded: what is kept. A request
/// Wealthsimple answers with an error is refused whole for one security it rejects
/// (one delisted or merged, still in the book): its securities are asked one to a
/// request from the next pass, so the one it rejects fails alone and the rest are
/// priced; one that answers alone goes back to the batches.
pub fn read(app: &Arc<App>, ctx: &Ctx, sess: &Session, listings: &[Listing]) -> Result<Vec<Listing>, String> {
    // the host its reads are recorded under: the stand-in's where one is named
    let host = bagholder_net::host_of(&bagholder_ws::session::graphql_url());
    let mut kept = Vec::new();
    let alone = app.ws_alone.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let (single, batched): (Vec<Listing>, Vec<Listing>) = listings.iter().cloned().partition(|l| carried(l).is_some_and(|id| alone.contains(id)));
    for chunk in batched.chunks(CHUNK).chain(single.chunks(1)) {
        let ids: Vec<&str> = chunk.iter().filter_map(carried).collect();
        let answer: Result<wire::SecuritiesSummary, _> = crate::orders::gql_as(app, sess, "FetchSecuritiesSummary", json!({ "ids": ids }));
        {
            let mut alone = app.ws_alone.lock().unwrap_or_else(|e| e.into_inner());
            match &answer {
                // refused whole by Wealthsimple: each asked alone from the next pass
                Err(bagholder_ws::session::CallError::Graphql(_)) => alone.extend(ids.iter().map(|id| id.to_string())),
                // answered: back to the batches
                Ok(_) if ids.len() == 1 => {
                    alone.remove(ids[0]);
                }
                _ => {}
            }
        }
        for l in chunk {
            let id = carried(l).unwrap_or_default();
            let outcome = match &answer {
                // asked again at its turn, never from another source in between
                Err(e) => Outcome::Unreachable(e.to_string()),
                Ok(data) => match data.securities.iter().find(|s| s.id == id) {
                    None => Outcome::NotCarried(format!("Wealthsimple states no security {id}")),
                    Some(s) => match quote_of(s, l, ctx.now) {
                        Ok(q) => {
                            ctx.cache.store_quote(&q).map_err(|e| e.to_string())?;
                            kept.push(l.clone());
                            Outcome::Answered(())
                        }
                        Err(why) => Outcome::Meaning(why),
                    },
                },
            };
            ctx.record_detail(&source(), &host, DataKind::Quote, Some(l.id), &Noted { outcome, shape_change: None }, id).map_err(|e| e.to_string())?;
        }
    }
    Ok(kept)
}

/// A number Wealthsimple states, as an amount: its shortest form is the decimal it
/// was sent as.
fn decimal(v: f64) -> Option<Dec> {
    v.is_finite().then(|| Dec::parse(&format!("{v}")).ok()).flatten()
}

/// `s`'s quote for `l`: the price (its last trade where it states no price), the
/// day's change against the previous session's close it states, and the time it
/// states the quote is of (else when it was read). A quote in another currency
/// than the listing's is for another listing, and none is kept.
pub fn quote_of(s: &wire::SummarySecurity, l: &Listing, now: Timestamp) -> Result<StoredQuote, String> {
    let q = s.quote_v2.clone().unwrap_or_default();
    let price = q.price.or(q.last).and_then(decimal).ok_or_else(|| format!("Wealthsimple states no price for {}", l.symbol))?;
    let stated = if q.currency.is_empty() { &s.currency } else { &q.currency };
    let currency = Currency::parse(&stated.to_ascii_uppercase()).map_err(|_| format!("Wealthsimple states the quote for {} in {stated:?}", l.symbol))?;
    if currency != l.currency {
        return Err(format!("the quote for {} is in {currency}, the listing's currency is {}", l.symbol, l.currency));
    }
    let base = q.previous_baseline.or(q.reference_close).and_then(decimal);
    let change = base.and_then(|b| price.checked_sub(b).ok());
    let change_pct = change.zip(base).and_then(|(c, b)| bagholder_sources::quotes::percent_of(c, b));
    let quoted_at = q.quoted_as_of.parse::<Timestamp>().unwrap_or(now);
    Ok(StoredQuote { instrument: l.id, source: source(), price: Money::new(price, currency), change, change_pct, quoted_at, allowance: std::time::Duration::ZERO, received_at: now })
}
