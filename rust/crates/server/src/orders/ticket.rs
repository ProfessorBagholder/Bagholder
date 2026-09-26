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
use super::preview::tick;
use super::{brackets, err_text, gql_as, log, orders_live, ticket_session};
use crate::app::{uuid4, App};

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
            .map(|d| d.to_f64());
        available
    })
    .ok_or_else(|| "the figures are not built yet".to_string())
}

/// The listing an order names, from the book: by the broker's id for it where one
/// is given, else by the symbol the book knows it by, a share before a contract of
/// it. A broker id the book does not hold is taken as given (a listing found
/// outside the book). `None` where a symbol names nothing the book holds.
pub fn resolve_security(app: &Arc<App>, symbol: &str, security_id: &str) -> Result<Option<bagholder_model::securities::Security>, String> {
    use bagholder_core::instrument::InstrumentKind;
    let f = app.figures.get().ok_or("the figures are not open")?;
    let names = f.names()?;
    let sid = security_id.trim();
    let sym = symbol.trim().to_uppercase();
    f.read(|e| {
        let i = e.inputs();
        let of = |id: &bagholder_core::InstrumentId| -> Option<bagholder_model::securities::Security> {
            let info = i.ledger.instruments.get(id)?;
            let n = info.current_name()?;
            Some(bagholder_model::securities::Security {
                id: names.security.get(id)?.clone(),
                symbol: n.symbol.clone(),
                name: n.name.clone().unwrap_or_default(),
                primary_exchange: n.venue_name.clone().or_else(|| n.venue_mic.clone()).unwrap_or_default(),
                primary_mic: n.venue_mic.clone().unwrap_or_default(),
                currency: info.instrument.currency.as_str().to_string(),
                ..Default::default()
            })
        };
        if !sid.is_empty() {
            let held = names.security.iter().find(|(_, s)| s.as_str() == sid).and_then(|(id, _)| of(id));
            return Some(held.unwrap_or_else(|| bagholder_model::securities::Security { id: sid.to_string(), symbol: sym.clone(), ..Default::default() }));
        }
        if sym.is_empty() {
            return None;
        }
        let mut same: Vec<(bool, bagholder_model::securities::Security)> = i
            .ledger
            .instruments
            .iter()
            .filter(|(_, info)| info.current_name().is_some_and(|n| n.symbol.to_uppercase() == sym))
            .filter_map(|(id, info)| of(id).map(|s| (info.instrument.kind != InstrumentKind::Security, s)))
            .collect();
        same.sort_by(|a, b| (a.0, &a.1.id).cmp(&(b.0, &b.1.id)));
        // the book's, else one Wealthsimple's search found while the app runs
        same.into_iter().next().map(|(_, s)| s).or_else(|| app.orders.found.lock().unwrap_or_else(|e| e.into_inner()).get(&sym).cloned())
    })
    .ok_or_else(|| "the figures are not built yet".to_string())
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

/// The order types the ticket offers, and the margin rate, from
/// `FetchSecurityMarketData`.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketData {
    pub order_types: Vec<String>,
    pub margin_rate: Option<f64>,
}

pub fn parse_market_data(data: &bagholder_ws::wire::SecurityMarketData) -> MarketData {
    let sec = data.security.clone().unwrap_or_default();
    let subtypes: Vec<String> = sec.allowed_order_subtypes.iter().filter(|x| !x.is_empty()).map(|x| x.to_uppercase()).collect();
    let mut rate = sec.margin_rates.and_then(|r| r.client_margin_rate);
    if let Some(r) = rate {
        if r > 1.0 {
            rate = Some(r / 100.0);
        }
    }
    let order_types: Vec<String> = ORDER_EXEC_TYPES.iter().filter(|t| subtypes.iter().any(|x| x == *t)).map(|t| t.to_string()).collect();
    MarketData { order_types, margin_rate: rate }
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

/// Whether Wealthsimple takes stop orders for a security, asked once while the app runs.
pub fn stop_allowed(app: &Arc<App>, security_id: &str) -> Result<bool, String> {
    #[cfg(test)]
    if let Some(v) = *app.orders.seam.stop_allowed.lock().unwrap_or_else(|e| e.into_inner()) {
        return Ok(v);
    }
    if let Some(v) = app.orders.stop_allowed.lock().unwrap_or_else(|e| e.into_inner()).get(security_id) {
        return Ok(*v);
    }
    let sess = ticket_session(app).ok_or("Not connected.")?;
    let d = gql_as(app, &sess, "FetchSecurityMarketData", json!({"id": security_id})).map_err(|e| format!("the order types Wealthsimple takes for it could not be read: {}", err_text(&e)))?;
    let ok = parse_market_data(&d).order_types.iter().any(|t| t == "STOP");
    app.orders.stop_allowed.lock().unwrap_or_else(|e| e.into_inner()).insert(security_id.to_string(), ok);
    Ok(ok)
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

pub(super) const LOOKUP_TYPES: [&str; 2] = ["EQUITY", "EXCHANGE_TRADED_FUND"];
pub(super) const CANADIAN_SUFFIXES: [&str; 4] = [".TO", ".V", ".CN", ".NE"];

pub(super) fn bare_symbol(sym: &str) -> String {
    let sym = sym.to_uppercase();
    for suf in CANADIAN_SUFFIXES {
        if let Some(b) = sym.strip_suffix(suf) {
            return b.to_string();
        }
    }
    sym
}

/// The equity or ETF a search found for this symbol on this exchange, a Canadian
/// suffix on either side ignored.
pub fn parse_listing_search(data: &wire::SecuritySearchAnswer, symbol: &str, exchange: &str) -> Option<bagholder_model::securities::Security> {
    let (want_sym, want_ex) = (bare_symbol(symbol), exchange.trim().to_uppercase());
    let results = &data.security_search.as_ref()?.results;
    results.iter().find_map(|r| {
        let stock = r.stock.clone().unwrap_or_default();
        let found = !r.id.is_empty()
            && bare_symbol(&stock.symbol) == want_sym
            && stock.primary_exchange.to_uppercase() == want_ex
            && LOOKUP_TYPES.contains(&r.security_type.to_uppercase().as_str());
        found.then(|| bagholder_model::securities::Security {
            id: r.id.clone(),
            symbol: stock.symbol.to_uppercase(),
            name: stock.name,
            primary_exchange: stock.primary_exchange,
            primary_mic: stock.primary_mic,
            currency: r.currency.to_uppercase(),
            underlying_id: String::new(),
        })
    })
}

pub fn lookup_listing(app: &Arc<App>, sess: &bagholder_ws::session::Session, symbol: &str, exchange: &str) -> Option<bagholder_model::securities::Security> {
    let data: wire::SecuritySearchAnswer = match gql_as(app, sess, "FetchSecuritySearchResult", json!({"query": symbol.trim()})) {
        Ok(d) => d,
        Err(e) => {
            log(&format!("bagholder ticket: listing search for {} failed: {}", symbol, e));
            return None;
        }
    };
    let sec = parse_listing_search(&data, symbol, exchange);
    if let Some(sec) = &sec {
        // known by its symbol while the app runs
        app.orders.found.lock().unwrap_or_else(|e| e.into_inner()).insert(sec.symbol.to_uppercase(), sec.clone());
    }
    sec
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

pub fn ticket_quote(app: &Arc<App>, symbol: &str, security_id: &str, account_id: &str, exchange: &str) -> TicketQuote {
    let name_of = || if symbol.is_empty() { security_id.to_string() } else { symbol.to_string() };
    let mut sec = match resolve_security(app, symbol, security_id) {
        Ok(s) => s,
        Err(e) => return TicketQuote::err(format!("The book could not be read: {e}")),
    };
    if sec.is_none() && exchange.is_empty() {
        return TicketQuote::err(format!("No listing stored for {}.", name_of()));
    }
    let sess = match ticket_session(app) {
        Some(s) => s,
        None => return TicketQuote::err("Not connected."),
    };
    if sec.is_none() {
        sec = lookup_listing(app, &sess, &symbol.trim().to_uppercase(), exchange);
    }
    let sec = match sec {
        Some(s) => s,
        None => return TicketQuote::err(format!("No listing stored for {}.", name_of())),
    };
    let sid = sec.id.clone();
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
    let mut md = MarketData::default();
    match gql_as(app, &sess, "FetchSecurityMarketData", json!({"id": sid})) {
        Ok(d) => md = parse_market_data(&d),
        Err(e) => log(&format!("bagholder ticket: market data for {} failed: {}", sid, e)),
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
            Err(e) => log(&format!("bagholder ticket: buying power for {} failed: {}", a.id, e)),
        }
    }
    let margin_available = match ticket_figures(app, acct.as_ref().map_or("", |a| a.margin_account_id.as_str())) {
        Ok(v) => v,
        Err(e) => return TicketQuote::err(format!("The book could not be read: {e}")),
    };
    let order_types = if !md.order_types.is_empty() { md.order_types } else { ORDER_EXEC_TYPES.iter().map(|s| s.to_string()).collect() };
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
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct Ticket {
    pub symbol: String,
    pub security_id: String,
    pub account_id: String,
    /// `BUY` or `SELL`.
    pub side: String,
    /// `MARKET`, `LIMIT`, `STOP` or `STOP_LIMIT`.
    #[serde(rename = "type")]
    pub kind: String,
    /// `DAY` or `UNTIL_CANCEL`; a day order when absent.
    pub tif: Option<String>,
    pub quantity: PageDec,
    pub limit_price: PageDec,
    pub stop_price: PageDec,
    pub currency: Option<String>,
    pub stop_loss: Option<TicketStop>,
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
    let limit_price = if has_limit {
        Some(positive(t.limit_price.read("The limit price")?).map(tick).ok_or("A limit price is required.")?)
    } else {
        None
    };
    let stop_price = if has_stop {
        Some(positive(t.stop_price.read("The stop price")?).map(tick).ok_or("A stop price is required.")?)
    } else {
        None
    };
    let acct = order_accounts(app)?.into_iter().find(|a| a.id == t.account_id).ok_or("Choose an account.")?;
    let sec = resolve_security(app, &t.symbol, &t.security_id)?.ok_or_else(|| format!("No listing stored for {}.", t.symbol))?;
    // exits are an entry's: a sale has none
    let (mut stop, mut target) = (None, None);
    if side == Side::Buy {
        if let Some(sl) = &t.stop_loss {
            let trail_kind = match sl.kind.as_deref().map(str::to_lowercase).as_deref() {
                None | Some("") | Some("stop") => false,
                Some("trail") => true,
                _ => return Err("Stop loss type must be Stop or Trailing stop.".into()),
            };
            let price = positive(sl.price.read("The stop loss price")?).map(tick);
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
            target = Some(positive(tp.price.read("The take profit price")?).map(tick).ok_or("A take profit price is required.")?);
        }
    }
    let named = if sec.currency.trim().is_empty() { t.currency.clone().unwrap_or_default() } else { sec.currency.clone() };
    let currency = Currency::parse(&named.trim().to_uppercase()).map_err(|_| format!("The currency of {} is not known.", sec.symbol))?;
    let id = format!("order-{}", uuid4());
    let mut request = json!({
        "canonicalAccountId": acct.id,
        "externalId": id,
        "executionType": ws_execution(kind),
        "orderType": if side == Side::Buy { "BUY_QUANTITY" } else { "SELL_QUANTITY" },
        "quantity": quantity.to_f64(),
        "securityId": sec.id,
        "timeInForce": if tif == TimeInForce::Day { "DAY" } else { "UNTIL_CANCEL" },
    });
    if let Some(p) = limit_price {
        request["limitPrice"] = json!(p.to_f64());
    }
    if let Some(p) = stop_price {
        request["stopPrice"] = json!(p.to_f64());
    }
    Ok(TicketOrder {
        order: OrderRequest {
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
            request,
        },
        stop,
        target,
    })
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

/// Place what a ticket asks for.
pub fn place_ticket(app: &Arc<App>, t: &Ticket) -> PlaceTicketAnswer {
    let asked = match ticket_order(app, t) {
        Ok(x) => x,
        Err(e) => return PlaceTicketAnswer::err(e),
    };
    let Some(f) = app.figures.get() else { return PlaceTicketAnswer::err("The book is not open.") };
    let book = match f.book() {
        Ok(b) => b,
        Err(e) => return PlaceTicketAnswer::err(format!("The book could not be opened: {e}")),
    };
    let answer = if asked.order.side == Side::Sell && orders_live(app) && ticket_session(app).is_some() {
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
        OrderState::Failed => PlaceTicketAnswer { ok: false, error: Some(format!("Order failed: {}", fold.why.clone().unwrap_or_default())), id: Some(o.id.clone()), status: Some(fold.state.as_str().into()), bracket_id: bracket },
        s => PlaceTicketAnswer { ok: true, error: None, id: Some(o.id.clone()), status: Some(s.as_str().into()), bracket_id: bracket },
    }
}

/// An order with no sale to clear: a buy (its bracket written first when it asks for
/// legs, so a fill is never without one), or any order with orders off or no session.
pub(crate) fn entry(app: &Arc<App>, book: &Book, asked: TicketOrder) -> Result<PlaceTicketAnswer, String> {
    let now = Timestamp::now();
    let mut o = asked.order;
    // a bracket waits on a fill: with orders off, or nothing to send with, none can come
    let wants_bracket = (asked.stop.is_some() || asked.target.is_some()) && orders_live(app) && ticket_session(app).is_some();
    let bracket = if wants_bracket {
        let id = format!("bracket-{}", uuid4());
        let place = BracketPlace { id: id.clone(), broker: o.broker.clone(), broker_account: o.broker_account.clone(), broker_security: o.broker_security.clone(), symbol: o.symbol.clone(), currency: o.currency };
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
/// the sale goes out only then. A sale not confirmed in time, refused, or unsettled
/// puts each bracket back to guarding, its stop placed again by the next check: the
/// position is never left with neither.
pub(crate) fn sell(app: &Arc<App>, book: &Book, o: &OrderRequest) -> Result<PlaceTicketAnswer, String> {
    let brackets = guarding(book, o)?;
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
    let at_broker = matches!(fold.state, OrderState::Pending | OrderState::PartlyFilled | OrderState::Filled | OrderState::Cancelling);
    if at_broker {
        for (id, take) in &brackets {
            let lock = gate::bracket_lock(app, id);
            let _one = lock.lock().unwrap_or_else(|e| e.into_inner());
            record(book, id, Timestamp::now(), &BracketEvent::Sold { quantity: *take })?;
        }
    } else {
        // refused, never reached Wealthsimple, or still unsettled: the stop goes back. Were
        // the sale at Wealthsimple after all, it holds the shares and the stop is refused for
        // them, which ends the bracket
        drop_all(&format!("the sale is {}", fold.state.as_str()))?;
    }
    log(&format!("bagholder order: {} {} ({})", o.id, super::order_words(o.side, o.quantity, o.kind, o.limit_price, o.stop_price), fold.state.as_str()));
    super::ask_read(app);
    Ok(told(o, &fold, None))
}
