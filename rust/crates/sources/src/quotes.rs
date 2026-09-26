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
//! - **US listings:** Yahoo's chart, its forms in order, the winner first.
//! - **Coins:** Coinbase in the holding's own currency: the Exchange's ticker,
//!   which states its time, for a USD pair; otherwise the spot price, stamped with
//!   its reply's date less the age its origin allows.
//!
//! Option contracts are quoted from their chains with their closes (`options`).

use bagholder_core::instrument::{RefScheme, Reference};
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
    if let Outcome::Answered(a) = &*outcome {
        let c = currency(a);
        if c != l.currency {
            *outcome = Outcome::Meaning(format!("the answer for {} is in {c}, the listing's currency is {}", l.symbol, l.currency));
        }
    }
}

/// Read each listing's quote once.
pub fn read_quotes(ctx: &Ctx, listings: &[Listing]) -> Result<()> {
    for l in listings {
        let id = l.id;
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
                if let Outcome::Answered(q) = noted.outcome {
                    // Cboe Canada quotes its listings in their own currency
                    keep(ctx, id, cboe_ca::source(), Money::new(q.price, l.currency), q.change, q.change_pct, q.at, std::time::Duration::ZERO)?;
                }
            }
            Some(Market::UnitedStates) => {
                let mut forms = yahoo_forms(l);
                if forms.is_empty() {
                    not_carried(ctx, &yahoo::source(), yahoo::HOST, id, format!("{} names no venue Yahoo carries", l.symbol))?;
                    continue;
                }
                if let Some((_, won)) = ctx.cache.winner(id, DataKind::Quote)? {
                    forms.sort_by_key(|f| *f != won);
                }
                for form in forms {
                    let mut noted = yahoo::ask_quote(ctx.net, &form, ctx.now);
                    same_currency(&mut noted.outcome, |c| c.currency, l);
                    ctx.record_detail(&yahoo::source(), yahoo::HOST, DataKind::Quote, Some(id), &noted, &form)?;
                    match noted.outcome {
                        Outcome::Answered(c) => {
                            keep(ctx, id, yahoo::source(), Money::new(c.quote.price, c.currency), None, c.quote.change_pct, c.quote.at, std::time::Duration::ZERO)?;
                            ctx.cache.won(id, DataKind::Quote, &yahoo::source(), &form, ctx.now)?;
                            break;
                        }
                        Outcome::NotCarried(_) => continue,
                        _ => break,
                    }
                }
            }
            Some(Market::Crypto) => {
                let base = l.symbol.trim().to_ascii_uppercase();
                if l.currency == Currency::USD {
                    let pair = format!("{base}-USD");
                    let noted = coinbase::ask_ticker(ctx.net, &pair);
                    ctx.record_detail(&coinbase::exchange_source(), coinbase::EXCHANGE_HOST, DataKind::Quote, Some(id), &noted, &pair)?;
                    if let Outcome::Answered(s) = noted.outcome {
                        keep(ctx, id, coinbase::exchange_source(), Money::new(s.price, Currency::USD), None, None, s.at, s.allowance)?;
                    }
                } else {
                    let pair = format!("{base}-{}", l.currency.as_str());
                    let noted = coinbase::ask_spot(ctx.net, &base, l.currency);
                    ctx.record_detail(&coinbase::spot_source(), coinbase::SPOT_HOST, DataKind::Quote, Some(id), &noted, &pair)?;
                    if let Outcome::Answered(s) = noted.outcome {
                        keep(ctx, id, coinbase::spot_source(), Money::new(s.price, l.currency), None, None, s.at, s.allowance)?;
                    }
                }
            }
            // option contracts: from their chains, with the option closes
            Some(Market::UsOptions) | None => {}
        }
    }
    Ok(())
}
