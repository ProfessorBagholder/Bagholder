//! The order ticket (`SPEC.md` §4, Order ticket): the accounts and the quote it shows,
//! the order it asks for, read strictly from what the page sends, and placing it:
//! a bracket written before its entry is sent, and a sale that first clears a
//! bracket's exit off the shares and puts it back when the sale does not go out.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bagholder_book::orders::{BracketPlace, OrderRequest};
use bagholder_book::Book;
use bagholder_core::bracket::{BracketEvent, Phase, StopLeg, Trail};
use bagholder_core::order::{Asker, OrderKind, OrderRole, OrderState, Side, TimeInForce};
use bagholder_core::{Currency, Dec};
use bagholder_ws::session::CallError;
use bagholder_ws::wire;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use ts_rs::TS;

use super::gate::{self, Held};
use super::preview::off_tick;
use super::{brackets, err_text, gql_as, log, orders_live, ticket_session};
use crate::app::{uuid4, App};

/// The order types the ticket offers. Wealthsimple takes market, limit, stop (a
/// stop-market) and stop-limit orders on Canadian and US stocks, ETFs and options,
/// by kind of product and not by security ("Understanding stop-market orders",
/// help.wealthsimple.com/hc/en-ca/articles/39734190700187, and "Understanding
/// stop-limit orders", …/4413542667675): the ticket asks nothing per security.
pub const ORDER_EXEC_TYPES: [&str; 4] = ["MARKET", "LIMIT", "STOP", "STOP_LIMIT"];

/// One account the ticket may place against, as it offers accounts:
/// tradable, self-directed, open. Named by the broker's own id, which an order
/// names.
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase")]
#[diff(key = id)]
pub struct OrderAccount {
    pub id: String,
    pub name: String,
    pub margin: bool,
    /// The margin account whose margin an order here moves: its own for a margin
    /// account, the margin account it is linked to for one that is collateral.
    pub margin_account_id: String,
}

/// The accounts the ticket offers, from the book (`docs/plans/stage-3c-switch.md`,
/// §3: what orders read).
pub fn order_accounts(app: &Arc<App>) -> Result<Vec<OrderAccount>, String> {
    let f = app.figures.get().ok_or("the figures are not open")?;
    let names = f.names()?;
    // a cash account backing a margin account (Wealthsimple's margin boost), by the broker's ids
    let backing = f.book()?.margin_backing().map_err(|e| e.to_string())?;
    let broker_id = |a: &bagholder_core::AccountId| names.account.get(a).cloned();
    let boosted: std::collections::BTreeMap<String, String> = backing.iter().filter_map(|(a, m)| Some((broker_id(a)?, broker_id(m)?))).collect();
    let accounts = f.read(|e| crate::wire::build::accounts(e.inputs(), &names)).ok_or("the figures are not built yet")?;
    Ok(accounts
        .iter()
        .filter(|a| a.tradable && a.status != "closed")
        .filter_map(|a| {
            let id = a.broker_account.clone()?;
            let margin_account_id = if a.margin {
                id.clone()
            } else {
                // collateral: the margin account it backs, where that is an account the ticket offers
                boosted.get(&id).filter(|m| accounts.iter().any(|x| x.margin && x.broker_account.as_deref() == Some(m.as_str()))).cloned().unwrap_or_default()
            };
            Some(OrderAccount { id, name: a.name.clone(), margin: a.margin, margin_account_id })
        })
        .collect::<Vec<_>>())
    .map(|mut v| {
        // as the person reads them: by name
        v.sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
        v
    })
}

/// What a margin account can borrow, as the broker last stated it, by the
/// broker's id for the account. From the book.
fn ticket_figures(app: &Arc<App>, margin_account: &str) -> Result<Option<f64>, String> {
    let f = app.figures.get().ok_or("the figures are not open")?;
    let names = f.names()?;
    f.read(|e| {
        let i = e.inputs();
        let available = names
            .account
            .iter()
            .find(|(_, b)| !margin_account.is_empty() && b.as_str() == margin_account)
            .and_then(|(a, _)| i.market.brokers.get(a))
            .and_then(|b| b.buying_power.as_ref())
            .and_then(|bp| bp.as_ref().ok())
            // in CAD at the live rate, as the ticket's cost is (`SPEC.md` order-ticket-66)
            .and_then(|m| bagholder_engine::fx::live_to_cad(&i.facts.rates, &i.clock, *m).ok())
            .map(|m| m.amount.to_f64());
        available
    })
    .ok_or_else(|| "the figures are not built yet".to_string())
}

/// The listing an order is on (brief 34, section 1): Wealthsimple's id for it,
/// which every order request names, and what the ticket shows of it. The symbol
/// and the exchange are words to show, never the key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrderListing {
    pub security_id: SecurityId,
    pub symbol: String,
    pub exchange: String,
    pub currency: String,
}

/// Wealthsimple's id for a listing: never empty, read so at the boundary, so a
/// request that names no listing cannot be read at all (brief 36, 5).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, TS)]
#[ts(type = "string")]
pub struct SecurityId(String);

impl SecurityId {
    pub fn parse(s: &str) -> Result<SecurityId, String> {
        let s = s.trim();
        if s.is_empty() {
            Err("No listing named.".into())
        } else {
            Ok(SecurityId(s.to_string()))
        }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for SecurityId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<SecurityId, D::Error> {
        SecurityId::parse(&String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

impl std::fmt::Display for SecurityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// An exact listing a ticket opens from, before Wealthsimple's id for it is known
/// (brief 35, 1): the book's own instrument, or a symbol on a named venue as an
/// exchange's own directory gives it, unique there. Neither is a symbol alone.
#[derive(Clone, Debug, PartialEq, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ListingRef {
    Instrument { id: String },
    Venue { symbol: String, venue: String },
}

/// `POST /api/order/listing`'s body: the exact listing a ticket opens from.
#[derive(Clone, Debug, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ListingOf {
    pub of: ListingRef,
}

/// `POST /api/order/listing`: the listing to open a ticket on, or why there is none,
/// in the ticket's words.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(untagged)]
pub enum OrderListingAnswer {
    Ok {
        #[ts(type = "true")]
        ok: bool,
        listing: OrderListing,
    },
    Refused {
        #[ts(type = "false")]
        ok: bool,
        error: String,
    },
}

/// A listing as the ticket shows it, for what is said of it.
pub fn shown(l: &OrderListing) -> String {
    match (l.symbol.trim(), l.exchange.trim()) {
        ("", _) => "the listing".to_string(),
        (s, "") => s.to_string(),
        (s, e) => format!("{s} on {e}"),
    }
}

fn listing_of(s: &bagholder_model::securities::Security) -> Result<OrderListing, String> {
    Ok(OrderListing { security_id: SecurityId::parse(&s.id)?, symbol: s.symbol.clone(), exchange: s.primary_exchange.clone(), currency: s.currency.clone() })
}

/// The listing Wealthsimple's id names, as the book or this run's search holds it,
/// else as Wealthsimple's own summary states it (kept by its id for the run):
/// the symbol, the venue and the currency an order is recorded with are
/// Wealthsimple's, never what a page sent (brief 36, 4). An id Wealthsimple does
/// not know is refused.
/// `shown` is the listing as the ticket shows it (`TD on NYSE`), for what is said.
/// A listing that states no symbol or no currency is refused: an order is recorded
/// with both, and neither is ever taken from the page (brief 37, 2).
pub fn listing_by_id(app: &Arc<App>, security_id: &SecurityId, shown: &str) -> Result<bagholder_model::securities::Security, String> {
    let sec = match known_listing(app, security_id.as_str())? {
        Some(sec) => sec,
        None => {
            let sess = ticket_session(app)?;
            let q = fetch_quotes(app, &sess, &[security_id.to_string()])
                .map_err(|e| format!("Wealthsimple could not be asked for {shown}: {}", err_text(&e)))?
                .remove(security_id.as_str())
                .ok_or_else(|| format!("Wealthsimple does not know {shown}."))?;
            remember_summary(app, &q).ok_or_else(|| format!("Wealthsimple states no symbol or no currency for {shown}."))?
        }
    };
    if sec.symbol.trim().is_empty() || sec.currency.trim().is_empty() {
        return Err(format!("Wealthsimple states no symbol or no currency for {shown}."));
    }
    Ok(sec)
}

/// A listing as Wealthsimple's summary states it, kept by its id for the run; none
/// where the summary states no symbol or no currency, which an order needs.
fn remember_summary(app: &Arc<App>, q: &TicketQuoteDetail) -> Option<bagholder_model::securities::Security> {
    if q.symbol.trim().is_empty() || q.currency.trim().is_empty() {
        return None;
    }
    let sec = bagholder_model::securities::Security {
        id: q.security_id.clone(),
        symbol: q.symbol.clone(),
        name: q.name.clone(),
        primary_exchange: q.exchange.clone(),
        primary_mic: q.mic.clone(),
        currency: q.currency.to_uppercase(),
        ..Default::default()
    };
    app.orders.found.lock().unwrap_or_else(|e| e.into_inner()).insert(sec.id.clone(), sec.clone());
    Some(sec)
}

/// The listing Wealthsimple's id names, as the book records it, else as Wealthsimple's
/// search found it while the app runs; `None` where neither holds it. Read by the id
/// alone: nothing here looks a listing up by its symbol.
pub fn known_listing(app: &Arc<App>, security_id: &str) -> Result<Option<bagholder_model::securities::Security>, String> {
    let sid = security_id.trim();
    if sid.is_empty() {
        return Ok(None);
    }
    let f = app.figures.get().ok_or("the figures are not open")?;
    let names = f.names()?;
    let held = f
        .read(|e| {
            let i = e.inputs();
            let (id, _) = names.security.iter().find(|(_, s)| s.as_str() == sid)?;
            let info = i.ledger.instruments.get(id)?;
            let n = info.current_name()?;
            Some(bagholder_model::securities::Security {
                id: sid.to_string(),
                symbol: n.symbol.clone(),
                name: n.name.clone().unwrap_or_default(),
                primary_exchange: n.venue_name.clone().or_else(|| n.venue_mic.clone()).unwrap_or_default(),
                primary_mic: n.venue_mic.clone().unwrap_or_default(),
                currency: info.instrument.currency.as_str().to_string(),
                ..Default::default()
            })
        })
        .ok_or("the figures are not built yet")?;
    Ok(held.or_else(|| app.orders.found.lock().unwrap_or_else(|e| e.into_inner()).get(sid).cloned()))
}

/// The listing a ticket opens on, from the exact listing it was opened from: the
/// book's instrument by Wealthsimple's id recorded against it, else, for one the
/// broker has stated no id for, and for a listing a directory gave, Wealthsimple's
/// search matched on the symbol and the venue's code. An id the search finds for a
/// book instrument is recorded against it in the book (§5; brief 35, 3).
pub fn resolve_listing(app: &Arc<App>, r: &ListingRef, now: jiff::Timestamp) -> OrderListingAnswer {
    match listing_for(app, r, now) {
        Ok(listing) => OrderListingAnswer::Ok { ok: true, listing },
        Err(error) => OrderListingAnswer::Refused { ok: false, error },
    }
}

fn listing_for(app: &Arc<App>, r: &ListingRef, now: jiff::Timestamp) -> Result<OrderListing, String> {
    use bagholder_core::instrument::{InstrumentKind, RefScheme, Reference};
    use bagholder_sources::venue::{known_mic, mic_of};
    let f = app.figures.get().ok_or("The book is not open.")?;
    // the instrument it is, where the book's; the symbol, the venue's code and its words
    let (instrument, symbol, mic, words) = match r {
        ListingRef::Instrument { id } => {
            let id: bagholder_core::InstrumentId = id.trim().parse().map_err(|_| "No such listing in the book.".to_string())?;
            let names = f.names()?;
            let found = f
                .read(|e| {
                    let info = e.inputs().ledger.instruments.get(&id)?;
                    let n = info.current_name()?;
                    let mic = known_mic(n.venue_mic.as_deref(), n.venue_name.as_deref()).map(str::to_string);
                    let exchange = n.venue_name.clone().or_else(|| n.venue_mic.clone()).unwrap_or_default();
                    Some((n.symbol.clone(), mic, exchange, info.instrument.currency, info.instrument.kind))
                })
                .ok_or("The figures are not built yet.")?
                .ok_or("No such listing in the book.")?;
            let (symbol, mic, exchange, currency, kind) = found;
            if let Some(sid) = names.security.get(&id) {
                return Ok(OrderListing { security_id: SecurityId::parse(sid)?, symbol, exchange, currency: currency.as_str().to_string() });
            }
            let mic = mic.ok_or_else(|| format!("The book names no venue for {symbol}: Wealthsimple cannot be asked for it."))?;
            (Some((id, currency, kind)), symbol, mic, exchange)
        }
        // a venue in words, as a code, or as a listing's key names it (`US` for any US exchange)
        ListingRef::Venue { symbol, venue } => {
            let mic = mic_of(venue).or_else(|| bagholder_core::listing::key_venue(venue)).ok_or_else(|| format!("{venue} is not a venue the app knows: no listing of {symbol} can be found on it."))?;
            (None, symbol.trim().to_string(), mic.to_string(), venue.trim().to_string())
        }
    };
    if symbol.is_empty() {
        return Err("No symbol named.".into());
    }
    let venue = bagholder_core::listing::key_venue(&mic).map(str::to_string).ok_or_else(|| format!("{words} is not a venue the app knows: no listing of {symbol} can be found on it."))?;
    let key = (bare_symbol(&symbol), venue.clone());
    let cached = app.orders.found_at.lock().unwrap_or_else(|e| e.into_inner()).get(&key).cloned();
    let sec = match cached.and_then(|id| app.orders.found.lock().unwrap_or_else(|e| e.into_inner()).get(&id).cloned()) {
        Some(sec) => sec,
        None => {
            let sess = ticket_session(app)?;
            let data: wire::SecuritySearchAnswer = gql_as(app, &sess, "FetchSecuritySearchResult", json!({"query": key.0})).map_err(|e| format!("Wealthsimple's search failed: {}", err_text(&e)))?;
            let sec = parse_listing_search(&data, &symbol, &mic, &words)?;
            app.orders.found.lock().unwrap_or_else(|e| e.into_inner()).insert(sec.id.clone(), sec.clone());
            app.orders.found_at.lock().unwrap_or_else(|e| e.into_inner()).insert(key, sec.id.clone());
            sec
        }
    };
    if let Some((id, currency, kind)) = instrument {
        // learned and remembered against the instrument, so the next order reads it
        // from the book: only a share or fund in the instrument's own currency, and
        // only an id no other instrument already holds (that one names the listing
        // already; the ticket opens on it, and nothing is recorded)
        let r = Reference::new(RefScheme::BrokerSecurity(bagholder_core::Broker::named("wealthsimple")), &sec.id);
        let book = f.book()?;
        let held_by_another = book.instrument_by_ref(&r).map_err(|e| e.to_string())?.is_some_and(|other| other != id);
        if kind == InstrumentKind::Security && sec.currency == currency.as_str() && !held_by_another {
            book.add_instrument_ref(id, &r).map_err(|e| e.to_string())?;
            f.record_changed(now)?;
        }
    }
    listing_of(&sec)
}

/// A ticket's own quote, as `GET /api/order/quote` and the ticket's live
/// document show it.
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase", default)]
pub struct TicketQuoteDetail {
    pub security_id: String,
    pub symbol: String,
    pub name: String,
    pub exchange: String,
    /// The venue's code as Wealthsimple states it (`XTSE`); empty where it states none.
    pub mic: String,
    pub currency: String,
    pub security_type: String,
    pub buyable: bool,
    pub sellable: bool,
    pub trade_eligible: bool,
    pub status: String,
    pub last: Option<f64>,
    pub bid: Option<f64>,
    pub ask: Option<f64>,
    pub bid_size: Option<f64>,
    pub ask_size: Option<f64>,
    pub mid: Option<f64>,
    pub change: Option<f64>,
    pub change_pct: Option<f64>,
    pub market_status: String,
    pub quoted_as_of: String,
    pub multiplier: Option<f64>,
}

/// A security's quote as the ticket shows it; nothing for a security with no id.
pub fn parse_quote(node: &wire::SummarySecurity) -> Option<TicketQuoteDetail> {
    if node.id.is_empty() {
        return None;
    }
    let q = node.quote_v2.clone().unwrap_or_default();
    let stock = node.stock.clone().unwrap_or_default();
    let last = q.price.or(q.last);
    let base = q.previous_baseline.or(q.reference_close);
    let (bid, ask) = (q.bid, q.ask);
    let change = match (last, base) {
        (Some(l), Some(b)) => Some(l - b),
        _ => None,
    };
    let mid = q.mid.or(match (bid, ask) {
        (Some(b), Some(a)) => Some((b + a) / 2.0),
        _ => None,
    });
    let change_pct = match (change, base) {
        (Some(c), Some(b)) if b != 0.0 => Some(c / b),
        _ => None,
    };
    Some(TicketQuoteDetail {
        security_id: node.id.clone(),
        symbol: stock.symbol,
        name: stock.name,
        exchange: stock.primary_exchange,
        mic: stock.primary_mic,
        currency: if q.currency.is_empty() { &node.currency } else { &q.currency }.to_uppercase(),
        security_type: node.security_type.clone(),
        buyable: node.buyable,
        sellable: node.sellable,
        trade_eligible: node.ws_trade_eligible,
        status: node.status.clone(),
        last,
        bid,
        ask,
        bid_size: q.bid_size,
        ask_size: q.ask_size,
        mid,
        change,
        change_pct,
        market_status: q.market_status,
        quoted_as_of: q.quoted_as_of,
        multiplier: node.option_details.as_ref().and_then(|o| o.multiplier),
    })
}

/// The margin rate, from `FetchSecurityMarketData`.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketData {
    pub margin_rate: Option<f64>,
}

pub fn parse_market_data(data: &bagholder_ws::wire::SecurityMarketData) -> MarketData {
    let sec = data.security.clone().unwrap_or_default();
    let mut rate = sec.margin_rates.and_then(|r| r.client_margin_rate);
    if let Some(r) = rate {
        if r > 1.0 {
            rate = Some(r / 100.0);
        }
    }
    MarketData { margin_rate: rate }
}

/// The buying power and cash on one account, from
/// `FetchTradingBalanceBuyingPower`.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuyingPowerFigures {
    pub buying_power: Option<f64>,
    pub cash: Option<f64>,
    /// Read from the answer but not carried into the ticket's own quote,
    /// exactly as the untyped parser left it -- kept here since the two
    /// currencies (buying power's, cash's) are not always the same.
    pub currency: String,
}

pub fn parse_buying_power(data: &bagholder_ws::wire::TradingBalanceBuyingPower) -> BuyingPowerFigures {
    let view = data.account.clone().and_then(|a| a.financials).and_then(|f| f.current).and_then(|c| c.trading_balance_view_v2).unwrap_or_default();
    let bp = view.buying_power.unwrap_or_default();
    let cash = view.cash.unwrap_or_default();
    let currency = if !bp.currency.is_empty() { bp.currency } else { cash.currency };
    BuyingPowerFigures { buying_power: bp.quantity, cash: cash.quantity, currency }
}

pub fn fetch_quotes(app: &Arc<App>, sess: &bagholder_ws::session::Session, security_ids: &[String]) -> Result<HashMap<String, TicketQuoteDetail>, CallError> {
    let ids: Vec<String> = security_ids.iter().map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
    let mut out = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let data: wire::SecuritiesSummary = gql_as(app, sess, "FetchSecuritiesSummary", json!({"ids": ids}))?;
    {
        // shares per unit, as the quote states them: a contract's multiplier, 1 for anything else
        let mut units = app.orders.units.lock().unwrap_or_else(|e| e.into_inner());
        for node in data.securities.iter().filter(|n| !n.id.is_empty()) {
            let per = match &node.option_details {
                None => Some(Dec::ONE),
                Some(o) => o.multiplier.filter(|m| m.is_finite() && *m > 0.0).and_then(|m| Dec::parse(&format!("{m}")).ok()),
            };
            if let Some(per) = per {
                units.insert(node.id.clone(), per);
            }
        }
    }
    for q in data.securities.iter().filter_map(parse_quote) {
        out.insert(q.security_id.clone(), q);
    }
    Ok(out)
}

/// Shares per unit of a Wealthsimple security: the book's contract terms where it
/// holds the instrument, else what Wealthsimple's quote stated; `None` when neither
/// has said.
pub fn units_of(app: &Arc<App>, book: &Book, security: &str) -> Result<Option<Dec>, String> {
    use bagholder_core::instrument::{RefScheme, Reference};
    let r = Reference::new(RefScheme::BrokerSecurity(bagholder_core::Broker::named("wealthsimple")), security);
    if let Some(i) = book.instrument_by_ref(&r).map_err(|e| e.to_string())? {
        let held = book.instrument(i).map_err(|e| e.to_string())?;
        if held.kind != bagholder_core::instrument::InstrumentKind::OptionContract {
            return Ok(Some(Dec::ONE));
        }
        // a contract's size as the book's terms state it, else as a quote stated it
        if let Some(m) = book.option_terms(i).map_err(|e| e.to_string())?.and_then(|t| t.multiplier) {
            return Ok(Some(m));
        }
    }
    Ok(app.orders.units.lock().unwrap_or_else(|e| e.into_inner()).get(security).copied())
}

/// Ask Wealthsimple's quote for the size of every security an order shown names that
/// neither the book nor an earlier quote has stated.
pub(crate) fn learn_units(app: &Arc<App>, book: &Book, sess: &bagholder_ws::session::Session) {
    let mut ids: Vec<String> = Vec::new();
    let mut named: Vec<String> = app.orders.elsewhere.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|e| e.security.clone()).collect();
    match book.orders_before(None, super::doc::PAGE) {
        Ok(orders) => named.extend(orders.into_iter().map(|o| o.request.broker_security)),
        Err(e) => log(&format!("bagholder orders: the orders could not be read: {e}")),
    }
    for s in named {
        match units_of(app, book, &s) {
            Ok(None) if !ids.contains(&s) => ids.push(s),
            Ok(_) => {}
            Err(e) => log(&format!("bagholder orders: what the book holds of {s} could not be read: {e}")),
        }
    }
    if ids.is_empty() {
        return;
    }
    if let Err(e) = fetch_quotes(app, sess, &ids) {
        log(&format!("bagholder orders: the size of {} could not be read: {}", ids.join(", "), err_text(&e)));
    }
}

/// The name of an account the ticket offers, by the broker's id for it; empty when it
/// is not one.
pub fn account_name(app: &Arc<App>, broker_account: &str) -> String {
    match order_accounts(app) {
        Ok(a) => a.into_iter().find(|a| a.id == broker_account).map(|a| a.name).unwrap_or_default(),
        Err(e) => {
            log(&format!("bagholder orders: the accounts could not be read: {e}"));
            String::new()
        }
    }
}

/// Wealthsimple's word for an order type.
pub fn ws_execution(kind: OrderKind) -> &'static str {
    match kind {
        OrderKind::Market => "MARKET",
        OrderKind::Limit => "LIMIT",
        OrderKind::Stop => "STOP",
        OrderKind::StopLimit => "STOP_LIMIT",
    }
}

pub const LOOKUP_TYPES: [&str; 2] = ["EQUITY", "EXCHANGE_TRADED_FUND"];
pub(super) const CANADIAN_SUFFIXES: [&str; 4] = [".TO", ".V", ".CN", ".NE"];

pub(crate) fn bare_symbol(sym: &str) -> String {
    let sym = sym.to_uppercase();
    for suf in CANADIAN_SUFFIXES {
        if let Some(b) = sym.strip_suffix(suf) {
            return b.to_string();
        }
    }
    sym
}

/// The one equity or ETF a search found for this symbol on the venue `mic` (named
/// `words` where it is said): the symbols compared bare (a Canadian suffix on either
/// side ignored), the venues through the app's one venue table, so the directory's
/// words and Wealthsimple's never meet as raw text (brief 35, 2): by their codes in
/// Canada, by the market in the US, where Wealthsimple states the NYSE group as one
/// (`venue::listing_venue`; brief 36, 1). None, or more than one, is said, never
/// chosen between.
pub fn parse_listing_search(data: &wire::SecuritySearchAnswer, symbol: &str, mic: &str, words: &str) -> Result<bagholder_model::securities::Security, String> {
    use bagholder_sources::venue::{listing_venue, mic_of};
    let want_sym = bare_symbol(symbol);
    // `mic` is a venue's code, or the key's `US` for any US exchange
    let want = bagholder_core::listing::key_venue(mic);
    let results = data.security_search.as_ref().map(|s| s.results.as_slice()).unwrap_or_default();
    let found: Vec<bagholder_model::securities::Security> = results
        .iter()
        .filter(|r| {
            let stock = r.stock.clone().unwrap_or_default();
            let venue = mic_of(&stock.primary_mic).or_else(|| mic_of(&stock.primary_exchange)).and_then(listing_venue);
            !r.id.is_empty() && bare_symbol(&stock.symbol) == want_sym && want.is_some() && venue.as_deref() == want && LOOKUP_TYPES.contains(&r.security_type.to_uppercase().as_str())
        })
        .map(|r| {
            let stock = r.stock.clone().unwrap_or_default();
            bagholder_model::securities::Security {
                id: r.id.clone(),
                symbol: stock.symbol.to_uppercase(),
                name: stock.name,
                primary_exchange: stock.primary_exchange,
                primary_mic: stock.primary_mic,
                currency: r.currency.to_uppercase(),
                underlying_id: String::new(),
            }
        })
        .collect();
    let mut ids: Vec<&str> = found.iter().map(|s| s.id.as_str()).collect();
    ids.dedup();
    let on = if words.trim().is_empty() { mic } else { words.trim() };
    match ids.len() {
        1 => Ok(found.into_iter().next().expect("one found")),
        0 => Err(format!("Wealthsimple lists no {want_sym} on {on}.")),
        _ => Err(format!("Wealthsimple lists more than one {want_sym} on {on}: the listing cannot be told apart.")),
    }
}

/// `GET /api/order/quote`, and the ticket's own live document (`quote:…`).
#[derive(Clone, Debug, Serialize, TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase")]
pub struct TicketQuoteOk {
    #[ts(type = "true")]
    pub ok: bool,
    pub quote: TicketQuoteDetail,
    pub order_types: Vec<String>,
    pub margin_rate: Option<f64>,
    pub accounts: Vec<OrderAccount>,
    pub account: Option<OrderAccount>,
    pub buying_power: Option<f64>,
    pub cash: Option<f64>,
    pub margin_available: Option<f64>,
    pub live: bool,
    /// What could not be read for the ticket, each in its own words: the ticket says
    /// it, and figures that need it are not shown.
    pub unread: Vec<String>,
}

/// The full quote, or why there is none, in the ticket's own words.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(untagged)]
pub enum TicketQuote {
    Ok(TicketQuoteOk),
    Refused(crate::http::OkOr),
}

impl bagholder_diff::Diff for TicketQuote {
    fn diff(&self, new: &Self, path: &mut Vec<Value>, ops: &mut Vec<Value>) {
        match (self, new) {
            (TicketQuote::Ok(a), TicketQuote::Ok(b)) => a.diff(b, path, ops),
            // a refusal never becomes another shape while the same page holds it
            _ => bagholder_diff::as_json(self, new, path, ops),
        }
    }
    fn keys(path: &mut Vec<String>, out: &mut Vec<(String, &'static str)>) {
        TicketQuoteOk::keys(path, out)
    }
}

impl TicketQuote {
    fn err(e: impl Into<String>) -> TicketQuote {
        TicketQuote::Refused(crate::http::OkOr::err(e))
    }
}

pub fn ticket_quote(app: &Arc<App>, security_id: &SecurityId, account_id: &str) -> TicketQuote {
    let sid = security_id.to_string();
    // the book's or the search's record of it, where either holds it; Wealthsimple's
    // own summary, asked by the id, states the rest
    let known = match known_listing(app, &sid) {
        Ok(s) => s,
        Err(e) => return TicketQuote::err(format!("The book could not be read: {e}")),
    };
    let sec = known.clone().unwrap_or_else(|| bagholder_model::securities::Security { id: sid.clone(), ..Default::default() });
    let sess = match ticket_session(app) {
        Ok(s) => s,
        Err(e) => return TicketQuote::err(e),
    };
    let mut quotes = match fetch_quotes(app, &sess, &[sid.clone()]) {
        Ok(q) => q,
        Err(CallError::NotAuthorized) => return TicketQuote::err("Wealthsimple refused the session. Connect Wealthsimple again."),
        Err(e) => return TicketQuote::err(format!("Quote failed: {}", err_text(&e))),
    };
    let mut quote = match quotes.remove(&sid) {
        Some(q) => q,
        None => {
            let label = if !sec.symbol.is_empty() { sec.symbol.clone() } else { sid.clone() };
            return TicketQuote::err(format!("Wealthsimple has no quote for {}.", label));
        }
    };
    if quote.symbol.is_empty() {
        quote.symbol = sec.symbol.clone();
    }
    if quote.name.is_empty() {
        quote.name = sec.name.clone();
    }
    if quote.exchange.is_empty() {
        quote.exchange = sec.primary_exchange.clone();
    }
    if quote.currency.is_empty() {
        quote.currency = sec.currency.to_uppercase();
    }
    let mut unread = Vec::new();
    // the listing as Wealthsimple states it, for the preview and the order; one that
    // states no symbol or currency cannot be ordered, and the ticket says so
    if known.is_none() && remember_summary(app, &quote).is_none() {
        unread.push("Wealthsimple states no symbol or no currency for this listing, so it cannot be ordered.".to_string());
    }
    let mut md = MarketData::default();
    match gql_as(app, &sess, "FetchSecurityMarketData", json!({"id": sid})) {
        Ok(d) => md = parse_market_data(&d),
        Err(e) => unread.push(format!("The margin rate could not be read: {}", err_text(&e))),
    }
    let accounts = match order_accounts(app) {
        Ok(a) => a,
        Err(e) => return TicketQuote::err(format!("The accounts could not be read: {e}")),
    };
    let acct = accounts.iter().find(|a| a.id == account_id).cloned();
    let mut balance = BuyingPowerFigures::default();
    if let Some(a) = &acct {
        let cur = if quote.currency.is_empty() { "CAD".to_string() } else { quote.currency.clone() };
        match gql_as(app, &sess, "FetchTradingBalanceBuyingPower", json!({"accountCanonicalId": a.id, "currency": cur, "securityId": sid})) {
            Ok(d) => balance = parse_buying_power(&d),
            Err(e) => unread.push(format!("The buying power could not be read: {}", err_text(&e))),
        }
    }
    let margin_available = match ticket_figures(app, acct.as_ref().map_or("", |a| a.margin_account_id.as_str())) {
        Ok(v) => v,
        Err(e) => return TicketQuote::err(format!("The book could not be read: {e}")),
    };
    let order_types = ORDER_EXEC_TYPES.iter().map(|s| s.to_string()).collect();
    TicketQuote::Ok(TicketQuoteOk {
        ok: true,
        quote,
        order_types,
        margin_rate: md.margin_rate,
        accounts,
        account: acct,
        buying_power: balance.buying_power,
        cash: balance.cash,
        margin_available,
        live: orders_live(app),
        unread,
    })
}

// ---------------------------------------------------------------------------
// the ticket as the page sends it
// ---------------------------------------------------------------------------

/// A number the page sends: a JSON number or the decimal text of one, read exactly;
/// empty or absent is no number. What is neither is kept as the text it was, for the
/// ticket to refuse in words.
#[derive(Clone, Debug, PartialEq, TS)]
#[ts(type = "number | string | null")]
pub struct PageDec(#[ts(skip)] pub Result<Option<Dec>, String>);

impl Default for PageDec {
    fn default() -> PageDec {
        PageDec(Ok(None))
    }
}

impl<'de> Deserialize<'de> for PageDec {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<PageDec, D::Error> {
        let v = Value::deserialize(d)?;
        Ok(PageDec(match &v {
            Value::Null => Ok(None),
            Value::String(s) if s.trim().is_empty() => Ok(None),
            Value::String(s) => Dec::parse(s.trim()).map(Some).map_err(|_| s.clone()),
            Value::Number(n) => Dec::parse(&n.to_string()).map(Some).map_err(|_| n.to_string()),
            other => Err(other.to_string()),
        }))
    }
}

impl PageDec {
    /// The number, or why it is not one, naming the field as the ticket does.
    fn read(&self, name: &str) -> Result<Option<Dec>, String> {
        self.0.clone().map_err(|t| format!("{name} {t:?} is not a number."))
    }
}

/// The stop an entry asks for, as the ticket sends it.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct TicketStop {
    /// `stop` or `trail`.
    pub kind: Option<String>,
    pub price: PageDec,
    pub trail: PageDec,
    /// `pct` or `amt`.
    pub trail_unit: Option<String>,
}

/// The target an entry asks for.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct TicketTarget {
    pub price: PageDec,
}

/// `POST /api/order`: the order ticket, as the page sends it. `ticket_order` checks
/// every field and says what is wrong with the first that is.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Ticket {
    /// The order's id, made by the page when it opened the review: the same order
    /// submitted twice is one order (brief 15 §2; brief 19, change 8).
    #[serde(default)]
    pub id: String,
    /// The listing the order is on, by Wealthsimple's id for it: the ticket opened
    /// on it, and nothing here looks one up by its symbol.
    pub listing: OrderListing,
    #[serde(default)]
    pub account_id: String,
    /// `BUY` or `SELL`.
    #[serde(default)]
    pub side: String,
    /// `MARKET`, `LIMIT`, `STOP` or `STOP_LIMIT`.
    #[serde(rename = "type")]
    #[serde(default)]
    pub kind: String,
    /// `DAY` or `UNTIL_CANCEL`; a day order when absent.
    #[serde(default)]
    pub tif: Option<String>,
    #[serde(default)]
    pub quantity: PageDec,
    #[serde(default)]
    pub limit_price: PageDec,
    #[serde(default)]
    pub stop_price: PageDec,
    #[serde(default)]
    pub stop_loss: Option<TicketStop>,
    #[serde(default)]
    pub take_profit: Option<TicketTarget>,
}

/// What a ticket asks for: the order, and the legs of the bracket it wants.
#[derive(Clone, Debug, PartialEq)]
pub struct TicketOrder {
    pub order: OrderRequest,
    pub stop: Option<StopLeg>,
    pub target: Option<Dec>,
}

/// The order a ticket asks for, and what Wealthsimple is sent for it; or what is wrong
/// with the ticket, in the words the ticket shows.
pub fn ticket_order(app: &Arc<App>, t: &Ticket) -> Result<TicketOrder, String> {
    let side = match t.side.to_uppercase().as_str() {
        "BUY" => Side::Buy,
        "SELL" => Side::Sell,
        _ => return Err("Side must be Buy or Sell.".into()),
    };
    let kind = match t.kind.to_uppercase().as_str() {
        "MARKET" => OrderKind::Market,
        "LIMIT" => OrderKind::Limit,
        "STOP" => OrderKind::Stop,
        "STOP_LIMIT" => OrderKind::StopLimit,
        _ => return Err("Order type must be Market, Limit, Stop or Stop limit.".into()),
    };
    let tif = match t.tif.as_deref().map(str::to_uppercase).as_deref() {
        None | Some("") | Some("DAY") => TimeInForce::Day,
        Some("UNTIL_CANCEL") => TimeInForce::UntilCancel,
        _ => return Err("Time in force must be Day or Good till cancelled.".into()),
    };
    let positive = |p: Option<Dec>| p.filter(|x| x.is_positive());
    let Some(quantity) = positive(t.quantity.read("Quantity")?) else { return Err("Quantity must be more than zero.".into()) };
    let (has_limit, has_stop) = (matches!(kind, OrderKind::Limit | OrderKind::StopLimit), matches!(kind, OrderKind::Stop | OrderKind::StopLimit));
    // a price goes out as confirmed, or not at all: never rounded after the review
    let on_tick = |name: &str, p: Dec| -> Result<Dec, String> { off_tick(name, p).map_or(Ok(p), Err) };
    let limit_price = if has_limit {
        Some(on_tick("The limit price", positive(t.limit_price.read("The limit price")?).ok_or("A limit price is required.")?)?)
    } else {
        None
    };
    let stop_price = if has_stop {
        Some(on_tick("The stop price", positive(t.stop_price.read("The stop price")?).ok_or("A stop price is required.")?)?)
    } else {
        None
    };
    let acct = order_accounts(app)?.into_iter().find(|a| a.id == t.account_id).ok_or("Choose an account.")?;
    // the listing as the book or Wealthsimple states it, by its id: the symbol and the
    // currency the order is recorded with are never the page's (brief 36, 4)
    let sec = listing_by_id(app, &t.listing.security_id, &shown(&t.listing))?;
    // exits are an entry's: a sale has none
    let (mut stop, mut target) = (None, None);
    if side == Side::Buy {
        if let Some(sl) = &t.stop_loss {
            let trail_kind = match sl.kind.as_deref().map(str::to_lowercase).as_deref() {
                None | Some("") | Some("stop") => false,
                Some("trail") => true,
                _ => return Err("Stop loss type must be Stop or Trailing stop.".into()),
            };
            let price = positive(sl.price.read("The stop loss price")?).map(|p| on_tick("The stop loss price", p)).transpose()?;
            let leg = if trail_kind {
                let distance = positive(sl.trail.read("The trail")?).ok_or("A trail is required.")?;
                let trail = match sl.trail_unit.as_deref().map(str::to_lowercase).as_deref() {
                    None | Some("") | Some("pct") => Trail::Pct(distance),
                    Some("amt") => Trail::Amount(distance),
                    _ => return Err("A trail is a percent or an amount.".into()),
                };
                // it starts under the entry's price and follows the high from the fill
                let level = match price.or_else(|| limit_price.or(stop_price).map(|p| StopLeg::trailed(trail, p))) {
                    Some(l) if l.is_positive() => l,
                    _ => return Err("A stop loss price is required.".into()),
                };
                StopLeg { level, trail: Some(trail), high: None }
            } else {
                StopLeg { level: price.ok_or("A stop loss price is required.")?, trail: None, high: None }
            };
            stop = Some(leg);
        }
        if let Some(tp) = &t.take_profit {
            target = Some(on_tick("The take profit price", positive(tp.price.read("The take profit price")?).ok_or("A take profit price is required.")?)?);
        }
    }
    let currency = Currency::parse(&sec.currency.trim().to_uppercase()).map_err(|_| format!("The currency of {} is not known.", sec.symbol))?;
    let id = order_id(&t.id)?;
    // the venue of the listing traded as Wealthsimple states it, code and words, as its
    // own rows state it: its fill states it the same, so its names never split on a
    // spelling (`XTSV` stays `XTSV`; the venue rule reads either as the same venue)
    let venue_mic = Some(sec.primary_mic.trim().to_string()).filter(|m| !m.is_empty());
    let venue_name = Some(sec.primary_exchange.trim().to_string()).filter(|w| !w.is_empty());
    let mut order = OrderRequest {
        venue_mic,
        venue_name,
        id,
        broker: "wealthsimple".into(),
        broker_account: acct.id,
        broker_security: sec.id,
        symbol: sec.symbol,
        currency,
        side,
        kind,
        quantity,
        limit_price,
        stop_price,
        time_in_force: tif,
        bracket: None,
        request: Value::Null,
    };
    order.request = gate::request_record(&order);
    Ok(TicketOrder { order, stop, target })
}

/// `POST /api/order`: what a placed (or refused) order is answered as.
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlaceTicketAnswer {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub id: Option<String>,
    /// Where the order stands after the answer: `dry`, `unconfirmed`, `pending`, …
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub bracket_id: Option<String>,
}

impl PlaceTicketAnswer {
    fn err(e: impl Into<String>) -> PlaceTicketAnswer {
        PlaceTicketAnswer { ok: false, error: Some(e.into()), ..PlaceTicketAnswer::default() }
    }
}

/// How long a sale from the ticket waits for Wealthsimple to confirm a bracket's exit
/// cancelled before the bracket guards again and nothing is sold.
pub const CANCEL_CONFIRM_SECONDS: u32 = 30;

/// The id an order is sent and kept under, from the one the page made at the review:
/// a UUID's text, nothing else.
fn order_id(page: &str) -> Result<String, String> {
    let p = page.trim().to_ascii_lowercase();
    let uuid = p.len() == 36 && p.char_indices().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() });
    if uuid { Ok(format!("order-{p}")) } else { Err("The order has no id: review it again.".into()) }
}

/// What a ticket order is, for telling a repeat of it from another order under its id.
fn same_order(a: &OrderRequest, b: &OrderRequest) -> bool {
    let key = |o: &OrderRequest| (o.broker_account.clone(), o.broker_security.clone(), o.side, o.kind, o.quantity, o.limit_price, o.stop_price, o.time_in_force);
    key(a) == key(b)
}

/// The answer to a ticket order the book already holds under its id: a repeat is
/// answered as the first was; another order under the same id is refused.
fn repeated(book: &Book, asked: &TicketOrder) -> Result<Option<PlaceTicketAnswer>, String> {
    let Some(first) = book.order(&asked.order.id).map_err(|e| e.to_string())? else { return Ok(None) };
    let bracket = first.request.bracket.as_ref().map(|(b, _)| b.clone());
    let legs = match &bracket {
        Some(b) => book.bracket_log(b).map_err(|e| e.to_string())?.into_iter().find_map(|l| match l.event {
            BracketEvent::Created { stop, target, .. } => Some((stop, target)),
            _ => None,
        }),
        None => None,
    };
    // a bracket is written only while orders can be sent: one sent without carries no legs to compare
    let same_legs = legs.is_none_or(|l| l == (asked.stop, asked.target));
    if !same_order(&first.request, &asked.order) || !same_legs {
        return Ok(Some(PlaceTicketAnswer::err("An order was already sent under this review with other details. Review it again.")));
    }
    Ok(Some(told(&first.request, &first.fold, bracket)))
}

/// Place what a ticket asks for, once: a repeat of it is answered as the first was,
/// and one that comes while the first is being placed is answered as in flight.
pub fn place_ticket(app: &Arc<App>, t: &Ticket) -> PlaceTicketAnswer {
    let asked = match ticket_order(app, t) {
        Ok(x) => x,
        Err(e) => return PlaceTicketAnswer::err(e),
    };
    let Some(_placing) = gate::Placing::start(app, &asked.order.id) else {
        return PlaceTicketAnswer { ok: true, error: None, id: Some(asked.order.id), status: Some(OrderState::Sending.as_str().into()), bracket_id: None };
    };
    let Some(f) = app.figures.get() else { return PlaceTicketAnswer::err("The book is not open.") };
    let book = match f.book() {
        Ok(b) => b,
        Err(e) => return PlaceTicketAnswer::err(format!("The book could not be opened: {e}")),
    };
    match repeated(&book, &asked) {
        Ok(Some(answer)) => return answer,
        Ok(None) => {}
        Err(e) => return PlaceTicketAnswer::err(format!("The book could not be read: {e}")),
    }
    let answer = if asked.order.side == Side::Sell && orders_live(app) && ticket_session(app).is_ok() {
        sell(app, &book, &asked.order)
    } else {
        entry(app, &book, asked)
    };
    answer.unwrap_or_else(|e| PlaceTicketAnswer::err(format!("The order could not be recorded: {e}")))
}

/// What the gate's answer for the ticket's order is told as.
fn told(o: &OrderRequest, fold: &bagholder_core::order::OrderFold, bracket: Option<String>) -> PlaceTicketAnswer {
    match fold.state {
        OrderState::Rejected => PlaceTicketAnswer {
            ok: false,
            error: Some(match &fold.why {
                Some(w) if !w.is_empty() => format!("Wealthsimple rejected the order: {w}"),
                _ => "Wealthsimple rejected the order.".into(),
            }),
            id: Some(o.id.clone()),
            status: Some(fold.state.as_str().into()),
            bracket_id: bracket,
        },
        // it never reached Wealthsimple: nothing was placed (a lapsed session, too many requests)
        OrderState::Failed => PlaceTicketAnswer { ok: false, error: Some(format!("Not sent: {}", fold.why.clone().unwrap_or_default())), id: Some(o.id.clone()), status: Some(fold.state.as_str().into()), bracket_id: bracket },
        s => PlaceTicketAnswer { ok: true, error: None, id: Some(o.id.clone()), status: Some(s.as_str().into()), bracket_id: bracket },
    }
}

/// An order with no sale to clear: a buy (its bracket written first when it asks for
/// legs, so a fill is never without one), or any order with orders off or no session.
pub(crate) fn entry(app: &Arc<App>, book: &Book, asked: TicketOrder) -> Result<PlaceTicketAnswer, String> {
    let now = Timestamp::now();
    let mut o = asked.order;
    // a bracket waits on a fill: with orders off, or nothing to send with, none can come
    let wants_bracket = (asked.stop.is_some() || asked.target.is_some()) && orders_live(app) && ticket_session(app).is_ok();
    let bracket = if wants_bracket {
        let id = format!("bracket-{}", uuid4());
        let place = BracketPlace { venue_mic: o.venue_mic.clone(), venue_name: o.venue_name.clone(), id: id.clone(), broker: o.broker.clone(), broker_account: o.broker_account.clone(), broker_security: o.broker_security.clone(), symbol: o.symbol.clone(), currency: o.currency };
        book.write_bracket(&place, &BracketEvent::Created { quantity: o.quantity, stop: asked.stop, target: asked.target }, &Asker::Person, now).map_err(|e| e.to_string())?;
        o.bracket = Some((id.clone(), OrderRole::Entry));
        Some(id)
    } else {
        None
    };
    let fold = match gate::place(app, book, &o, &Asker::Person, now)? {
        Ok(fold) => fold,
        Err(Held::Dry) => {
            log(&format!("bagholder order (orders are off, not sent): {}", o.request));
            return Ok(PlaceTicketAnswer { ok: true, error: None, id: Some(o.id), status: Some(OrderState::Dry.as_str().into()), bracket_id: None });
        }
        Err(held) => return Ok(PlaceTicketAnswer::err(format!("Not sent: {held:?}."))),
    };
    log(&format!("bagholder order: {} {} ({})", o.id, super::order_words(o.side, o.quantity, o.kind, o.limit_price, o.stop_price), fold.state.as_str()));
    if let Some(b) = &bracket {
        if fold.state.is_final() {
            // an entry that never reached Wealthsimple ends its bracket at once
            brackets::check_bracket(app, book, b, &HashMap::new(), now)?;
        }
    }
    super::ask_read(app);
    Ok(told(&o, &fold, bracket))
}

/// The brackets guarding shares the sale would sell, and how many of them each gives.
fn guarding(book: &Book, o: &OrderRequest) -> Result<Vec<(String, Dec)>, String> {
    let mut left = o.quantity;
    let mut out = Vec::new();
    for sb in book.live_brackets().map_err(|e| e.to_string())? {
        if sb.place.broker_account != o.broker_account || sb.place.broker_security != o.broker_security {
            continue;
        }
        if matches!(sb.bracket.phase, Phase::Waiting | Phase::Closing | Phase::Ended | Phase::ClosingForSale) || !left.is_positive() {
            continue;
        }
        let take = if left < sb.bracket.quantity { left } else { sb.bracket.quantity };
        left = left.checked_sub(take).map_err(|e| e.to_string())?;
        out.push((sb.place.id, take));
    }
    Ok(out)
}

fn record(book: &Book, id: &str, now: Timestamp, e: &BracketEvent) -> Result<(), String> {
    if let Err(why) = book.bracket_event(id, &Asker::Person, now, e).map_err(|e| e.to_string())? {
        log(&format!("bagholder bracket {id}: {why}"));
    }
    Ok(())
}

/// A sale from the ticket. The brackets guarding the shares clear the way first (their
/// exit's cancel confirmed, so a stop and this sale never rest on the same shares);
/// the sale goes out only then, and each bracket holds it until it fills or ends,
/// watching its stop level meanwhile. A sale not confirmed in time or refused puts
/// each bracket back to guarding, its stop placed again by the next check; a run that
/// stops part way leaves brackets the next check gives their stop back (`Selling`
/// marks the ones this run is sending): the position is never left with neither.
pub(crate) fn sell(app: &Arc<App>, book: &Book, o: &OrderRequest) -> Result<PlaceTicketAnswer, String> {
    let brackets = guarding(book, o)?;
    let _selling = gate::Selling::start(app, brackets.iter().map(|(id, _)| id.clone()).collect());
    let started = Timestamp::now();
    for (id, take) in &brackets {
        let lock = gate::bracket_lock(app, id);
        let _one = lock.lock().unwrap_or_else(|e| e.into_inner());
        record(book, id, started, &BracketEvent::SaleAsked { quantity: *take })?;
    }
    let drop_all = |why: &str| -> Result<(), String> {
        for (id, _) in &brackets {
            let lock = gate::bracket_lock(app, id);
            let _one = lock.lock().unwrap_or_else(|e| e.into_inner());
            if book.bracket(id).map_err(|e| e.to_string())?.is_some_and(|sb| sb.bracket.phase == Phase::ClosingForSale) {
                record(book, id, Timestamp::now(), &BracketEvent::SaleDropped { why: why.into() })?;
            }
        }
        Ok(())
    };
    // each second the brackets take their steps: the exit's cancel, and its read-back
    let mut waited = 0;
    loop {
        let now = Timestamp::now();
        let mut clear = true;
        for (id, _) in &brackets {
            brackets::check_bracket(app, book, id, &HashMap::new(), now)?;
            let sb = book.bracket(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("no bracket {id}"))?;
            if sb.bracket.phase != Phase::ClosingForSale {
                // its exit filled before the cancel landed: those shares are sold already
                drop_all("its exit filled first")?;
                return Ok(PlaceTicketAnswer::err(format!("Nothing was sold: the bracket's exit on {} filled first.", o.symbol)));
            }
            clear &= sb.bracket.clear_for_sale();
        }
        if clear {
            break;
        }
        if waited >= app.orders.sale_wait.load(std::sync::atomic::Ordering::SeqCst) || app.wait(Duration::from_secs(1)) {
            drop_all("Wealthsimple did not confirm the exit cancelled in time")?;
            log(&format!("bagholder order: sell of {} held back: the bracket's exit is not confirmed cancelled", o.symbol));
            return Ok(PlaceTicketAnswer::err(format!("Nothing was sold: Wealthsimple has not confirmed the bracket's stop on {} cancelled yet. Try again in a moment.", o.symbol)));
        }
        waited += 1;
    }
    let now = Timestamp::now();
    let mut fold = match gate::place(app, book, o, &Asker::Person, now)? {
        Ok(fold) => fold,
        Err(held) => {
            drop_all("the sale was not sent")?;
            return Ok(PlaceTicketAnswer::err(format!("Not sent: {held:?}.")));
        }
    };
    // an answer that was not Wealthsimple's is settled by reading the order back, within the wait
    let mut tries = 0;
    while matches!(fold.state, OrderState::Unconfirmed | OrderState::Sending) && tries < app.orders.sale_wait.load(std::sync::atomic::Ordering::SeqCst) {
        if app.wait(Duration::from_secs(1)) {
            break;
        }
        tries += 1;
        match gate::read_back(app, book, &o.id, Timestamp::now()) {
            Ok(f) => fold = f,
            Err(e) => log(&format!("bagholder order: the sale {} could not be read back: {e}", o.id)),
        }
    }
    // at Wealthsimple, or not known yet: each bracket holds the sale as its exit, so
    // nothing else is placed on those shares until it fills or ends (a sale that ends
    // unfilled gives the stop back; one never found by the read-back fails, likewise)
    let held = matches!(fold.state, OrderState::Pending | OrderState::PartlyFilled | OrderState::Filled | OrderState::Cancelling | OrderState::Sending | OrderState::Unconfirmed);
    if held {
        for (id, take) in &brackets {
            let lock = gate::bracket_lock(app, id);
            let _one = lock.lock().unwrap_or_else(|e| e.into_inner());
            record(book, id, Timestamp::now(), &BracketEvent::SaleSent { order_id: o.id.clone(), quantity: *take })?;
        }
    } else {
        // refused, or never reached Wealthsimple: the stop goes back
        drop_all(&format!("the sale is {}", fold.state.as_str()))?;
    }
    log(&format!("bagholder order: {} {} ({})", o.id, super::order_words(o.side, o.quantity, o.kind, o.limit_price, o.stop_price), fold.state.as_str()));
    super::ask_read(app);
    Ok(told(o, &fold, None))
}
