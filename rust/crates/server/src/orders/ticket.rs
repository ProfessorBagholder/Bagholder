//! The order ticket: the accounts and the quote it shows, the request it becomes, and
//! sending it.

use super::*;

// ---------------------------------------------------------------------------
// order ticket
// ---------------------------------------------------------------------------

pub const ORDER_EXEC_TYPES: [&str; 4] = ["MARKET", "LIMIT", "STOP", "STOP_LIMIT"];
pub const ORDER_TIFS: [&str; 2] = ["DAY", "UNTIL_CANCEL"];

pub(super) fn ticket_session(app: &Arc<App>) -> Option<bagholder_ws::session::Session> {
    #[cfg(test)]
    {
        let _ = app;
        return seam::SESSION.lock().unwrap_or_else(|e| e.into_inner()).clone().flatten();
    }
    #[allow(unreachable_code)]
    let sess = load_session(app)?;
    if sess.access_token.is_empty() {
        return None;
    }
    ensure_fresh_token(app, Some(sess.clone()));
    match load_session(app) {
        Some(v) if !v.access_token.is_empty() || !v.refresh_token.is_empty() => Some(v),
        _ => Some(sess),
    }
}

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
    for q in data.securities.iter().filter_map(parse_quote) {
        out.insert(q.security_id.clone(), q);
    }
    Ok(out)
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
        live: orders_live(),
    })
}

pub fn order_tick(price: Option<f64>) -> Option<f64> {
    price.map(|p| round_half_even(p, if p >= 1.0 { 2 } else { 4 }))
}

/// A number as the page sends it: a number, or the text of one; nothing for what is not.
fn page_num<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(num(Some(&v), None))
}

/// Text as the page sends it: `null` is no text, and a number is its digits -- a ticket is
/// refused for what is wrong with it, in words, never for how a field was spelled.
fn page_text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(s(Some(&v)))
}

/// The stop an entry asks for, as the ticket sends it.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct TicketStop {
    pub kind: Option<String>,
    #[serde(deserialize_with = "page_num")]
    pub price: Option<f64>,
    #[serde(deserialize_with = "page_num")]
    pub trail: Option<f64>,
    pub trail_unit: Option<String>,
}

/// The target an entry asks for.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct TicketTarget {
    #[serde(deserialize_with = "page_num")]
    pub price: Option<f64>,
}

/// A leg that is there: an object with something in it. `null`, `{}` and anything that
/// is not an object say the entry has no such leg.
fn leg<'de, D: serde::Deserializer<'de>, T: serde::de::DeserializeOwned>(d: D) -> Result<Option<T>, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(if v.as_object().map_or(false, |m| !m.is_empty()) { serde_json::from_value(v).ok() } else { None })
}

/// `POST /api/order`: the order ticket, as the page sends it. Nothing here is trusted:
/// `ticket_order` checks every field and says what is wrong with the first that is.
#[derive(Debug, Clone, Default, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct Ticket {
    #[serde(deserialize_with = "page_text")]
    pub symbol: String,
    #[serde(deserialize_with = "page_text")]
    pub security_id: String,
    #[serde(deserialize_with = "page_text")]
    pub account_id: String,
    #[serde(deserialize_with = "page_text")]
    pub side: String,
    #[serde(rename = "type", deserialize_with = "page_text")]
    pub kind: String,
    pub tif: Option<String>,
    #[serde(deserialize_with = "page_num")]
    pub quantity: Option<f64>,
    #[serde(deserialize_with = "page_num")]
    pub limit_price: Option<f64>,
    #[serde(deserialize_with = "page_num")]
    pub stop_price: Option<f64>,
    pub currency: Option<String>,
    #[serde(deserialize_with = "leg")]
    pub stop_loss: Option<TicketStop>,
    #[serde(deserialize_with = "leg")]
    pub take_profit: Option<TicketTarget>,
}

#[cfg(test)]
impl Ticket {
    /// A ticket from a JSON body; one that is not an object is an empty ticket, which
    /// `ticket_order` refuses at its first field.
    pub fn from_json(body: &Value) -> Ticket {
        serde_json::from_value(body.clone()).unwrap_or_default()
    }
}

/// The order a ticket asks for, and what Wealthsimple is sent for it; or what is wrong
/// with the ticket, in the words the ticket shows.
pub fn ticket_order(app: &Arc<App>, t: &Ticket) -> Result<(Order, Value), String> {
    let side = Side::parse(&t.side.to_uppercase());
    if !side.is_set() {
        return Err("Side must be Buy or Sell.".into());
    }
    let kind = OrderType::parse(&t.kind.to_uppercase());
    if !kind.is_set() {
        return Err("Order type must be Market, Limit, Stop or Stop limit.".into());
    }
    let tif = t.tif.clone().filter(|x| !x.is_empty()).unwrap_or_else(|| "DAY".into()).to_uppercase();
    if !ORDER_TIFS.contains(&tif.as_str()) {
        return Err("Time in force must be Day or Good till cancelled.".into());
    }
    let positive = |p: Option<f64>| p.map_or(false, |x| x > 0.0);
    let qty = t.quantity.unwrap_or(0.0);
    if !positive(Some(qty)) {
        return Err("Quantity must be more than zero.".into());
    }
    let limit_price = order_tick(t.limit_price);
    let stop_price = order_tick(t.stop_price);
    let (has_limit, has_stop) = (matches!(kind, OrderType::Limit | OrderType::StopLimit), matches!(kind, OrderType::Stop | OrderType::StopLimit));
    if has_limit && !positive(limit_price) {
        return Err("A limit price is required.".into());
    }
    if has_stop && !positive(stop_price) {
        return Err("A stop price is required.".into());
    }
    let acct = match order_accounts(app)?.into_iter().find(|a| a.id == t.account_id) {
        Some(a) => a,
        None => return Err("Choose an account.".into()),
    };
    let sec = match resolve_security(app, &t.symbol, &t.security_id)? {
        Some(s) => s,
        None => return Err(format!("No listing stored for {}.", t.symbol)),
    };
    // exits are an entry's: a sale has none
    let (mut stop_loss, mut take_profit) = (None, None);
    if side == Side::Buy {
        if let Some(sl) = &t.stop_loss {
            let sl_kind = SlKind::parse(&sl.kind.clone().filter(|k| !k.is_empty()).unwrap_or_else(|| "stop".into()).to_lowercase());
            if !sl_kind.is_set() {
                return Err("Stop loss type must be Stop or Trailing stop.".into());
            }
            if sl_kind == SlKind::Stop && !positive(sl.price) {
                return Err("A stop loss price is required.".into());
            }
            if sl_kind == SlKind::Trail && !positive(sl.trail) {
                return Err("A trail is required.".into());
            }
            let trail_unit = if sl.trail_unit.as_deref().map_or(false, |u| u.to_lowercase() == "amt") { TrailUnit::Amt } else { TrailUnit::Pct };
            stop_loss = Some(StopLoss { kind: sl_kind, price: order_tick(sl.price), trail: sl.trail, trail_unit });
        }
        if let Some(tp) = &t.take_profit {
            if !positive(tp.price) {
                return Err("A take profit price is required.".into());
            }
            take_profit = Some(TakeProfit { price: order_tick(tp.price) });
        }
    }
    let oid = format!("order-{}", uuid4());
    let mut req = json!({
        "canonicalAccountId": acct.id.clone(),
        "externalId": oid,
        "executionType": kind.as_str(),
        "orderType": format!("{}_QUANTITY", side),
        "quantity": qty,
        "securityId": sec.id.clone(),
        "timeInForce": tif,
    });
    if has_limit {
        set(&mut req, "limitPrice", jo(limit_price));
    }
    if has_stop {
        set(&mut req, "stopPrice", jo(stop_price));
    }
    let currency = t.currency.clone().filter(|c| !c.is_empty()).unwrap_or_else(|| sec.currency.clone()).to_uppercase();
    let order = Order {
        id: oid,
        created_at: now_iso(),
        account_id: acct.id.clone(),
        account: acct.name.clone(),
        security_id: sec.id.clone(),
        symbol: sec.symbol.clone(),
        currency,
        side,
        kind,
        quantity: Some(qty),
        limit_price: if has_limit { limit_price } else { None },
        stop_price: if has_stop { stop_price } else { None },
        tif,
        stop_loss,
        take_profit,
        request: req.clone(),
        ..Order::default()
    };
    Ok((order, req))
}

/// `ticket_order`, from JSON and to it: how the tests ask.
#[cfg(test)]
pub fn order_request(app: &Arc<App>, body: &Value) -> Result<(Value, Value), String> {
    ticket_order(app, &Ticket::from_json(body)).map(|(o, req)| (serde_json::to_value(&o).unwrap_or(Value::Null), req))
}

/// Write the order, then send it; what became of it is written over what was written.
/// With orders off it is written as `dry` and nothing is sent.
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
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub order: Option<Order>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ws_order_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub bracket_id: Option<String>,
    /// Wealthsimple's code for a refusal, for the server's own decisions; never sent.
    #[serde(skip)]
    #[ts(skip)]
    pub refusal_code: Option<String>,
}

impl PlaceTicketAnswer {
    fn err(e: impl Into<String>) -> PlaceTicketAnswer {
        PlaceTicketAnswer { ok: false, error: Some(e.into()), ..PlaceTicketAnswer::default() }
    }
    fn err_with_id(e: impl Into<String>, id: impl Into<String>) -> PlaceTicketAnswer {
        PlaceTicketAnswer { ok: false, error: Some(e.into()), id: Some(id.into()), ..PlaceTicketAnswer::default() }
    }
}

pub fn submit_order(app: &Arc<App>, row: &mut Order, req: &Value) -> PlaceTicketAnswer {
    let id = row.id.clone();
    let now_is = |status: OrderStatus, error: &str| {
        patch_order(app, &id, OrderPatch { status: Some(status), error: Some(error.into()), ..OrderPatch::default() });
    };
    if !orders_live() {
        row.status = OrderStatus::Dry;
        must(so::typed::insert_order(&db(app), row, &now_iso()));
        log(&format!("bagholder order (dry run, not sent): {}", bagholder_store::tables::json_text_sorted(req)));
        return PlaceTicketAnswer { ok: true, id: Some(id), status: Some("dry".into()), order: Some(row.clone()), ..PlaceTicketAnswer::default() };
    }
    let sess = match ticket_session(app) {
        Some(s) => s,
        None => {
            // every submit is a row: this one, with why nothing was sent
            row.status = OrderStatus::Failed;
            row.error = "Not connected.".into();
            must(so::typed::insert_order(&db(app), row, &now_iso()));
            log(&format!("bagholder order: {} not sent: not connected", id));
            return PlaceTicketAnswer::err_with_id("Not connected.", id);
        }
    };
    // known to be in flight before its row can be read as `sending`
    struct Sending<'a>(&'a App, String);
    impl Drop for Sending<'_> {
        fn drop(&mut self) {
            self.0.orders.sending.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.1);
        }
    }
    app.orders.sending.lock().unwrap_or_else(|e| e.into_inner()).insert(id.clone());
    let _sending = Sending(app, id.clone());
    row.status = OrderStatus::Sending;
    must(so::typed::insert_order(&db(app), row, &now_iso()));
    let data: wire::CreateOrderAnswer = match gql_as(app, &sess, "SoOrdersOrderCreate", json!({"input": req})) {
        Ok(d) => d,
        Err(CallError::NotAuthorized) => {
            now_is(OrderStatus::Failed, "Wealthsimple refused the session.");
            return PlaceTicketAnswer::err_with_id("Wealthsimple refused the session. Connect Wealthsimple again.", id);
        }
        Err(e) => {
            let msg = err_text(&e);
            now_is(OrderStatus::Failed, &msg);
            log(&format!("bagholder order: {} failed: {}", id, msg));
            return PlaceTicketAnswer::err_with_id(format!("Order failed: {}", msg), id);
        }
    };
    // an answer that names no order still leaves the order sent: the read-back finds it by our id
    let result = data.so_orders_create_order.unwrap_or_default();
    let code = result.errors.code().map(str::to_string);
    if let Some(reason) = result.errors.0 {
        now_is(OrderStatus::Rejected, &reason);
        log(&format!("bagholder order: {} rejected: {}{}", id, reason, code.as_deref().map(|c| format!(" ({c})")).unwrap_or_default()));
        return PlaceTicketAnswer { refusal_code: code, ..PlaceTicketAnswer::err_with_id(refused_words("Wealthsimple rejected the order", &reason), id) };
    }
    let ws_id = result.order.map(|o| o.order_id).unwrap_or_default();
    patch_order(app, &id, OrderPatch { status: Some(OrderStatus::Sent), ws_order_id: Some(ws_id.clone()), ..OrderPatch::default() });
    log(&format!("bagholder order: {} sent, Wealthsimple order {}", id, ws_id));
    let rid = id.clone();
    let a = app.clone();
    spawn("bagholder-order-refresh", move || {
        let _ = catch_unwind(AssertUnwindSafe(|| refresh_orders(&a, &rid)));
    });
    PlaceTicketAnswer { ok: true, id: Some(id), status: Some("sent".into()), ws_order_id: Some(ws_id), ..PlaceTicketAnswer::default() }
}

/// How long a sale from the ticket waits for Wealthsimple to confirm a bracket's
/// exits cancelled before it is held back.
const CANCEL_CONFIRM_SECONDS: u32 = 30;

/// Place what a ticket asks for. A sale first takes its shares out from under any
/// bracket guarding them -- the bracket ended, or kept on what is left -- so that the
/// bracket's own exits and this sale never sell the same shares twice.
pub fn place_ticket(app: &Arc<App>, t: &Ticket) -> PlaceTicketAnswer {
    let (mut row, req) = match ticket_order(app, t) {
        Ok(x) => x,
        Err(e) => return PlaceTicketAnswer::err(e),
    };
    if orders_live() && ticket_session(app).is_none() {
        // nothing can reach Wealthsimple: the order is written with that failure,
        // and no bracket is ended for a sale that cannot go out
        return submit_order(app, &mut row, &req);
    }
    if row.side == Side::Sell {
        let mut left = row.quantity.unwrap_or(0.0);
        // a bracket kept on fewer shares for this sale, and how many it guarded
        let mut released: Option<(String, f64)> = None;
        for b in live_brackets(app) {
            if b.account_id != row.account_id || b.security_id != row.security_id || b.status == BracketStatus::Waiting {
                continue;
            }
            let held = b.quantity.unwrap_or(0.0);
            if b.status == BracketStatus::Closing {
                // already ended: its exits are being cancelled, and the sale waits for that too
            } else if left >= held {
                end_bracket(app, &b, "sold from the ticket", "");
                left -= held;
            } else if left > 0.0 {
                release_shares(app, &b, left);
                released = Some((b.id.clone(), held));
                left = 0.0;
            } else {
                continue;
            }
            // the sell goes out only once Wealthsimple confirms the bracket's exits
            // gone: a stop and this sell never rest on the same shares
            if !await_cancels(app, &b, CANCEL_CONFIRM_SECONDS) {
                // a bracket kept on the shares left guards them all again, so trying
                // the sale again takes its shares out once, not twice
                if let Some((id, held)) = released.take() {
                    patch_bracket(app, &id, BracketPatch { quantity: Some(Some(held)), ..BracketPatch::default() });
                }
                log(&format!("bagholder order: sell of {} held back: the bracket's stop is not confirmed cancelled", row.symbol));
                return PlaceTicketAnswer::err(format!(
                    "Nothing was sold: Wealthsimple has not confirmed the bracket's stop on {} cancelled yet. Try again in a moment.",
                    row.symbol
                ));
            }
        }
    }
    let mut r = submit_order(app, &mut row, &req);
    if r.ok && (row.stop_loss.is_some() || row.take_profit.is_some()) {
        let b = create_bracket(app, &row);
        r.bracket_id = Some(b.id);
    }
    r
}

/// `place_ticket`, from JSON: how the tests ask.
#[cfg(test)]
pub fn place_order(app: &Arc<App>, body: &Value) -> PlaceTicketAnswer {
    place_ticket(app, &Ticket::from_json(body))
}

