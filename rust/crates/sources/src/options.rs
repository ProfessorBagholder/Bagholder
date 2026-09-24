//! Held option contracts' prices into the market cache, from Cboe's delayed
//! chains, read only when needed (`SPEC.md` §2, Cboe; owner, 2026-09-24;
//! `docs/plans/stage-3a-brief-06.md`).
//!
//! A chain is read for the contracts a screen shows, never because a page
//! opened. It is due when:
//! - no chain of the underlying has been read; or
//! - the chain held was made in session, and the market is open or its session
//!   has settled (16:30 Eastern: options trade to 16:15, the prints fifteen
//!   minutes delayed), so Cboe may have a later one or the final prices; or
//! - the chain held carries a settled session's final prices, and a later
//!   session is open: until then the final prices stand.
//!
//! A contract shown for the first time is due whatever the chain's age. A failed
//! read waits out the source's rest. Every
//! other read sends back the `Last-Modified` of the chain held, so Cboe answers
//! with a chain only when it has a newer one, and otherwise with a bare 304.
//!
//! What a chain states is read as it states it (research 4): the session it
//! carries is the day of the underlying's last trade, and it holds that
//! session's final prices once Cboe made it after the session settled. A
//! contract's price is the bid/ask midpoint where both are quoted, at the
//! chain's time less Cboe's delay, else its last trade at that trade's time.
//!
//! A contract is found in the chain by the OCC symbol the book states for it;
//! else by its terms, which must match exactly one contract. Where the book
//! records a corporate event on the underlying while the contract was held, the
//! contract may have been adjusted, and without its OCC symbol it is not looked
//! up by its terms.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::jiff::civil::Weekday;
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::{SignedDuration, Timestamp};
use bagholder_core::{Dec, InstrumentId, Money};

use crate::adapters::cboe_options::{self, Chain, ChainContract, ChainReply, HOST, LATE_BY};
use crate::cache::{ChainRead, ReadRow, StoredQuote};
use crate::contract::{DataKind, Market};
use crate::market::{resting, settled_at};
use crate::needs::ContractNeed;
use crate::outcome::{Noted, Outcome};
use crate::read::{Ctx, Result};

/// Whether US options trade (their delayed prints still arriving) at `now`: a
/// weekday from 09:30 to 16:30 Eastern.
fn in_session(now: Timestamp, bank: &TimeZone) -> bool {
    let z = now.to_zoned(bank.clone());
    let minutes = i32::from(z.hour()) * 60 + i32::from(z.minute());
    !matches!(z.weekday(), Weekday::Saturday | Weekday::Sunday) && (9 * 60 + 30..16 * 60 + 30).contains(&minutes)
}

/// Whether a chain holds its session's final prices: Cboe made it after the
/// session settled.
pub fn is_final(c: &ChainRead, bank: &TimeZone) -> bool {
    settled_at(Market::UsOptions, c.session, bank).is_some_and(|s| c.made_at >= s)
}

/// Whether an underlying's chain is due for a contract on screen, given the
/// chain last read.
pub fn chain_due(held: Option<&ChainRead>, now: Timestamp, bank: &TimeZone) -> bool {
    let Some(c) = held else { return true };
    let open = in_session(now, bank);
    if is_final(c, bank) {
        // the final prices stand until a later session trades
        return open && now.to_zoned(bank.clone()).date() > c.session;
    }
    open || settled_at(Market::UsOptions, c.session, bank).is_some_and(|s| now >= s)
}

/// The contract in the chain, or why it cannot be told.
fn find<'a>(chain: &'a Chain, c: &ContractNeed, symbol: &str) -> std::result::Result<&'a ChainContract, Outcome<()>> {
    if let Some(occ) = &c.occ {
        return chain.contracts.iter().find(|k| &k.occ == occ).ok_or_else(|| Outcome::NotCarried(format!("{occ} is not in {symbol}'s chain of {}", chain.session)));
    }
    let terms = format!("{symbol} {} {} {}", c.expiry, c.strike, c.right.as_str());
    if let Some(on) = c.event_on {
        return Err(Outcome::Meaning(format!("{terms}: the underlying's corporate event of {on} may have adjusted it, and the book states no OCC symbol")));
    }
    let found: Vec<&ChainContract> = chain.contracts.iter().filter(|k| k.expiry == c.expiry && k.right == c.right && k.strike == c.strike).collect();
    match found.as_slice() {
        [k] => Ok(k),
        [] => Err(Outcome::NotCarried(format!("{terms} is not in the chain of {}", chain.session))),
        many => Err(Outcome::Meaning(format!("{terms} matches {} in the chain, and the book states no OCC symbol", many.iter().map(|k| k.occ.as_str()).collect::<Vec<_>>().join(" and ")))),
    }
}

/// The price a chain gives a contract, and when it was current: the midpoint
/// at the chain's time less Cboe's delay, else the last trade at its own time.
fn quote_of(bank: &TimeZone, k: &ChainContract, made_at: Timestamp) -> Option<(Dec, Timestamp)> {
    if let Some(mid) = k.midpoint() {
        return Some((mid, made_at - SignedDuration::try_from(LATE_BY).ok()?));
    }
    let (price, at) = k.last?;
    Some((price, at.to_zoned(bank.clone()).ok()?.timestamp()))
}

/// Read the chain of each shown contract's underlying where it is due, and keep
/// each contract's price with its time.
pub fn read(ctx: &Ctx, shown: &[ContractNeed]) -> Result<()> {
    let mut by: BTreeMap<&str, Vec<&ContractNeed>> = BTreeMap::new();
    for c in shown.iter().filter(|c| c.expiry >= ctx.today()) {
        by.entry(c.underlying.as_str()).or_default().push(c);
    }
    let source = cboe_options::source();
    let priced: BTreeSet<InstrumentId> = ctx.cache.quotes()?.into_iter().filter(|q| q.source == source).map(|q| q.instrument).collect();
    for (symbol, group) in by {
        let held = ctx.cache.option_chain(symbol)?;
        // a failed read waits out the source's rest, whatever is shown
        let subject = format!("chain:{symbol}");
        if resting(&ctx.cache.reads(&subject, DataKind::Quote)?, ctx.now, ctx.net.limiter().pace(HOST).rest) {
            continue;
        }
        // a contract shown for the first time needs the chain whatever its age
        let new = group.iter().any(|c| !priced.contains(&c.id));
        if !new && !chain_due(held.as_ref(), ctx.now, ctx.bank) {
            continue;
        }
        let since = held.as_ref().filter(|_| !new).and_then(|h| h.last_modified.as_deref());
        let noted = cboe_options::ask(ctx.net, symbol, since);
        ctx.record_detail(&source, HOST, DataKind::Quote, None, &noted, symbol)?;
        let today = ctx.today();
        ctx.cache.store_read(&subject, DataKind::Quote, &ReadRow { source: source.clone(), first: today, last: today, outcome: noted.outcome.kind(), at: ctx.now })?;
        let (chain, last_modified) = match noted.outcome {
            Outcome::Answered(ChainReply::Chain { chain, last_modified }) => (chain, last_modified),
            Outcome::Answered(ChainReply::NotModified) => {
                if let Some(h) = held {
                    ctx.cache.store_option_chain(&ChainRead { received_at: ctx.now, ..h })?;
                }
                continue;
            }
            _ => continue,
        };
        ctx.cache.store_option_chain(&ChainRead { underlying: symbol.to_string(), session: chain.session, made_at: chain.made_at, last_modified, received_at: ctx.now })?;
        for c in group {
            let k = match find(&chain, c, symbol) {
                Ok(k) => k,
                Err(outcome) => {
                    ctx.record(&source, HOST, DataKind::Quote, Some(c.id), &Noted { outcome, shape_change: None })?;
                    continue;
                }
            };
            if let Some((price, at)) = quote_of(ctx.bank, k, chain.made_at) {
                ctx.cache.store_quote(&StoredQuote { instrument: c.id, source: source.clone(), price: Money::new(price, c.currency), change: None, change_pct: None, quoted_at: at, allowance: std::time::Duration::ZERO, received_at: ctx.now })?;
            }
        }
    }
    Ok(())
}
