//! Option contracts' closes into the book, and their quotes into the market
//! cache, from Cboe's delayed chains (`docs/plans/stage-3a-sources.md`, "Option
//! closes").
//!
//! A contract's close is kept in the book because no source gives a past
//! session's again. It is read from what the chain states, never from when the
//! chain was published:
//! - **The session.** A chain carries the session of its underlying's last trade
//!   (its day, Eastern). Its prices are that session's closing ones once Cboe
//!   made it after the session settled (16:30 Eastern, the closing prints fifteen
//!   minutes delayed); a chain made earlier is in session and closes nothing.
//! - **The close.** The closing bid/ask midpoint where both sides are quoted, as
//!   the live price is (`SPEC.md` §2), else the last trade where its stated time
//!   falls on the session, else none: the chain states no close for the contract
//!   that day, and the day takes the broker's figure or waits.
//! - **When it is due.** A held contract's latest settled session day, until the
//!   book holds its close or a read made after it settled covered it. A chain
//!   still carrying an older session writes that session's closes where the book
//!   lacks them and leaves the day due; a chain that has moved past the day
//!   settles it as not read, since no source states it again.
//!
//! One chain is asked per underlying, for its contracts due a close and those
//! held today, which are quoted from it: the midpoint at the chain's time less
//! Cboe's delay, else the last trade at its own time.
//!
//! A contract is found in the chain by the OCC symbol the book states for it;
//! else by its terms, which must match exactly one contract. Where the book
//! records a corporate event on the underlying while the contract was held, the
//! contract may have been adjusted, and without its OCC symbol it is not looked
//! up by its terms.

use std::collections::BTreeMap;

use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::{SignedDuration, Timestamp};
use bagholder_core::{Dec, Money};

use crate::adapters::cboe_options::{self, Chain, ChainContract, HOST, LATE_BY};
use crate::cache::StoredQuote;
use crate::contract::{DataKind, Market};
use crate::market::{due_span, keep_read, latest_settled, settled_at, CloseState};
use crate::needs::ContractNeed;
use crate::outcome::{Noted, Outcome, OutcomeKind};
use crate::read::{Ctx, Result};

/// The last day a contract can have a close: the last day it is held, and never
/// after its expiry.
fn last_day(c: &ContractNeed) -> Date {
    c.to.min(c.expiry)
}

/// The session day due for a contract's close, if any.
fn due_day(ctx: &Ctx, c: &ContractNeed, recorded: Option<&BTreeMap<Date, Money>>) -> Result<Option<Date>> {
    let Some(latest) = latest_settled(Market::UsOptions, last_day(c), ctx.now, ctx.bank) else { return Ok(None) };
    if latest < c.from {
        return Ok(None);
    }
    let state = CloseState { days: recorded.map(|m| m.keys().copied().collect()).unwrap_or_default(), reads: ctx.cache.reads(&c.id.to_string(), DataKind::OptionClose)? };
    let rest = ctx.net.limiter().pace(HOST).rest;
    Ok(due_span(Market::UsOptions, latest, last_day(c), &state, ctx.now, ctx.bank, rest).map(|(d, _)| d))
}

/// The close the chain states for a contract on its session, if any.
pub fn close_of(k: &ChainContract, session: Date) -> Option<Dec> {
    k.midpoint().or_else(|| k.last.filter(|(_, at)| at.date() == session).map(|(p, _)| p))
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

fn note(ctx: &Ctx, c: &ContractNeed, outcome: Outcome<()>) -> Result<()> {
    ctx.record(&cboe_options::source(), HOST, DataKind::OptionClose, Some(c.id), &Noted { outcome, shape_change: None })
}

/// The quote a chain gives a contract: the midpoint at the chain's time less
/// Cboe's delay, else the last trade at its own time.
fn quote_of(ctx: &Ctx, k: &ChainContract, made_at: Timestamp) -> Option<(Dec, Timestamp)> {
    if let Some(mid) = k.midpoint() {
        return Some((mid, made_at - SignedDuration::try_from(LATE_BY).ok()?));
    }
    let (price, at) = k.last?;
    Some((price, at.to_zoned(ctx.bank.clone()).ok()?.timestamp()))
}

/// Read each underlying's chain once for its contracts due a close or held today.
pub fn read(ctx: &Ctx, contracts: &[ContractNeed]) -> Result<()> {
    let recorded = ctx.book.closes()?;
    let today = ctx.today();
    let mut by: BTreeMap<&str, Vec<(&ContractNeed, Option<Date>)>> = BTreeMap::new();
    for c in contracts {
        let due = due_day(ctx, c, recorded.get(&c.id))?;
        if due.is_some() || last_day(c) >= today {
            by.entry(c.underlying.as_str()).or_default().push((c, due));
        }
    }
    let source = cboe_options::source();
    for (symbol, group) in by {
        let noted = cboe_options::ask(ctx.net, symbol);
        ctx.record_detail(&source, HOST, DataKind::OptionClose, None, &noted, symbol)?;
        let chain = match noted.outcome {
            Outcome::Answered(chain) => chain,
            other => {
                let kind = other.kind();
                for (c, due) in &group {
                    if let Some(d) = due {
                        keep_read(ctx, &c.id.to_string(), DataKind::OptionClose, &source, (*d, *d), kind, None)?;
                    }
                }
                continue;
            }
        };
        let settled = settled_at(Market::UsOptions, chain.session, ctx.bank).is_some_and(|s| chain.made_at >= s);
        for (c, due) in group {
            let subject = c.id.to_string();
            // a chain past the day due: that session's close is stated nowhere now
            if let Some(d) = due.filter(|d| settled && chain.session > *d) {
                let named = c.occ.clone().unwrap_or_else(|| format!("{symbol} {} {} {}", c.expiry, c.strike, c.right.as_str()));
                note(ctx, c, Outcome::NotCarried(format!("{named} {d}: the chain has moved on to {}", chain.session)))?;
                keep_read(ctx, &subject, DataKind::OptionClose, &source, (d, chain.session.yesterday().unwrap_or(d)), OutcomeKind::NotCarried, None)?;
                if last_day(c) < chain.session {
                    continue;
                }
            }
            let k = match find(&chain, c, symbol) {
                Ok(k) => k,
                Err(outcome) => {
                    let kind = outcome.kind();
                    note(ctx, c, outcome)?;
                    // the chain settles the due day when it carries that session
                    if let Some(d) = due.filter(|d| settled && chain.session == *d) {
                        keep_read(ctx, &subject, DataKind::OptionClose, &source, (d, d), kind, None)?;
                    }
                    continue;
                }
            };
            if last_day(c) >= today {
                if let Some((price, at)) = quote_of(ctx, k, chain.made_at) {
                    ctx.cache.store_quote(&StoredQuote { instrument: c.id, source: source.clone(), price: Money::new(price, c.currency), change: None, change_pct: None, quoted_at: at, allowance: std::time::Duration::ZERO, received_at: ctx.now })?;
                }
            }
            if !settled {
                continue;
            }
            let s = chain.session;
            if c.from <= s && s <= last_day(c) {
                let held = recorded.get(&c.id).and_then(|m| m.get(&s));
                match (close_of(k, s), held) {
                    (Some(close), None) => ctx.book.store_close(c.id, s, Money::new(close, c.currency), &source, ctx.now)?,
                    (Some(close), Some(before)) if before.amount != close || before.currency != c.currency => {
                        note(ctx, c, Outcome::Meaning(format!("{} {s}: {} stands, {close} came later", k.occ, before.amount)))?;
                    }
                    _ => {}
                }
                keep_read(ctx, &subject, DataKind::OptionClose, &source, (s, s), OutcomeKind::Answered, Some(s))?;
            }
        }
    }
    Ok(())
}
