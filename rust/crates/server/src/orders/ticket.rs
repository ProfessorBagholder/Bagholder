//! The order ticket: the accounts and the quote it shows, the request it becomes, and
//! sending it.

use super::*;

// ---------------------------------------------------------------------------
// order ticket
// ---------------------------------------------------------------------------

pub const ORDER_EXEC_TYPES: [&str; 4] = ["MARKET", "LIMIT", "STOP", "STOP_LIMIT"];
pub const ORDER_TIFS: [&str; 2] = ["DAY", "UNTIL_CANCEL"];
pub(super) const ORDER_TRADABLE_TYPES: [&str; 1] = ["SELF_DIRECTED"];
pub(super) const ORDER_UNTRADABLE_MARKERS: [&str; 3] = ["CRYPTO", "PREDICTIONS", "MANAGED"];

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
/// tradable, self-directed, open.
#[derive(Clone, Debug, Default, Serialize, Deserialize, TS, bagholder_diff_derive::Diff)]
#[serde(rename_all = "camelCase")]
#[diff(key = id)]
pub struct OrderAccount {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub margin: bool,
    pub currency: String,
    pub margin_account_id: String,
}

pub fn order_accounts(app: &Arc<App>, accounts: Option<&[Value]>) -> Vec<OrderAccount> {
    let owned: Vec<Value>;
    let list: &[Value] = match accounts {
        Some(a) => a,
        None => {
            owned = must(bagholder_store::tables::accounts(&db(app))).iter().map(|a| serde_json::to_value(a).unwrap_or(Value::Null)).collect();
            &owned
        }
    };
    let mut out = Vec::new();
    for a in list {
        let typ = s(or_v(a.get("unifiedAccountType"), a.get("unified_account_type"))).to_uppercase();
        let status = f(a, "status").to_lowercase();
        if !tr(a, "id") || status == "closed" || !ORDER_TRADABLE_TYPES.iter().any(|p| typ.starts_with(p)) {
            continue;
        }
        if ORDER_UNTRADABLE_MARKERS.iter().any(|m| typ.contains(m)) {
            continue;
        }
        let nick_src = if tr(a, "nickname") { f(a, "nickname") } else { typ.clone() };
        let nick = bagholder_model::value::norm_account_name(&nick_src);
        let margin = typ.contains("MARGIN");
        out.push(OrderAccount {
            id: f(a, "id"),
            name: nick,
            kind: typ.clone(),
            margin,
            currency: f(a, "currency"),
            margin_account_id: if margin { f(a, "id") } else { f(a, "marginAccountId") },
        });
    }
    out
}

pub fn resolve_security(app: &Arc<App>, symbol: &str, security_id: &str) -> Option<Value> {
    let rows = must(bagholder_store::admin::list_securities(&db(app)));
    let sid = security_id.trim();
    if !sid.is_empty() {
        if let Some(r) = rows.iter().find(|r| r.id == sid) {
            return Some(serde_json::to_value(r).unwrap());
        }
        return Some(json!({"id": sid, "symbol": symbol.trim().to_uppercase(), "name": "", "primaryExchange": "", "primaryMic": "", "currency": "", "underlyingId": null}));
    }
    let sym = symbol.trim().to_uppercase();
    if sym.is_empty() {
        return None;
    }
    let mut same: Vec<bagholder_model::securities::Security> = rows.into_iter().filter(|r| r.symbol.to_uppercase() == sym).collect();
    same.sort_by_key(|r| (if r.id.starts_with("sec-s-") { 0 } else { 1 }, r.id.clone()));
    same.into_iter().next().map(|r| serde_json::to_value(&r).unwrap())
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

pub fn parse_quote(node: &Value) -> Option<TicketQuoteDetail> {
    if !node.is_object() || !tr(node, "id") {
        return None;
    }
    let empty = json!({});
    let q = node.get("quoteV2").filter(|v| v.is_object()).unwrap_or(&empty);
    let stock = node.get("stock").filter(|v| v.is_object()).unwrap_or(&empty);
    let opt = node.get("optionDetails").filter(|v| v.is_object()).unwrap_or(&empty);
    let last = on(q, "price").or_else(|| on(q, "last"));
    let base = on(q, "previousBaseline").or_else(|| on(q, "referenceClose"));
    let (bid, ask) = (on(q, "bid"), on(q, "ask"));
    let change = match (last, base) {
        (Some(l), Some(b)) => Some(l - b),
        _ => None,
    };
    let mid = if q.get("mid").map_or(false, |v| !v.is_null()) {
        on(q, "mid")
    } else {
        match (bid, ask) {
            (Some(b), Some(a)) => Some((b + a) / 2.0),
            _ => None,
        }
    };
    let change_pct = match (change, base) {
        (Some(c), Some(b)) if b != 0.0 => Some(c / b),
        _ => None,
    };
    let multiplier = if opt.as_object().map_or(false, |m| !m.is_empty()) { on(opt, "multiplier") } else { None };
    Some(TicketQuoteDetail {
        security_id: f(node, "id"),
        symbol: f(stock, "symbol"),
        name: f(stock, "name"),
        exchange: f(stock, "primaryExchange"),
        currency: s(or_v(q.get("currency"), node.get("currency"))).to_uppercase(),
        security_type: f(node, "securityType"),
        buyable: tr(node, "buyable"),
        sellable: tr(node, "sellable"),
        trade_eligible: tr(node, "wsTradeEligible"),
        status: f(node, "status"),
        last,
        bid,
        ask,
        bid_size: on(q, "bidSize"),
        ask_size: on(q, "askSize"),
        mid,
        change,
        change_pct,
        market_status: f(q, "marketStatus"),
        quoted_as_of: f(q, "quotedAsOf"),
        multiplier,
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
    let data = gql(app, sess, "FetchSecuritiesSummary", json!({"ids": ids}))?;
    if let Some(a) = data.get("securities").and_then(|v| v.as_array()) {
        for node in a {
            if let Some(q) = parse_quote(node) {
                out.insert(q.security_id.clone(), q);
            }
        }
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

pub fn parse_listing_search(data: &Value, symbol: &str, exchange: &str) -> Option<Value> {
    let (want_sym, want_ex) = (bare_symbol(symbol), exchange.trim().to_uppercase());
    let empty = json!({});
    let results = data.get("securitySearch").and_then(|v| v.get("results")).and_then(|v| v.as_array())?;
    for r in results {
        if !r.is_object() || !tr(r, "id") {
            continue;
        }
        let stock = r.get("stock").filter(|v| v.is_object()).unwrap_or(&empty);
        if bare_symbol(&f(stock, "symbol")) != want_sym || f(stock, "primaryExchange").to_uppercase() != want_ex {
            continue;
        }
        if !LOOKUP_TYPES.contains(&f(r, "securityType").to_uppercase().as_str()) {
            continue;
        }
        return Some(json!({"id": f(r, "id"), "symbol": f(stock, "symbol").to_uppercase(), "name": f(stock, "name"), "primaryExchange": f(stock, "primaryExchange"),
            "primaryMic": f(stock, "primaryMic"), "currency": f(r, "currency").to_uppercase(), "underlyingId": null}));
    }
    None
}

pub fn lookup_listing(app: &Arc<App>, sess: &bagholder_ws::session::Session, symbol: &str, exchange: &str) -> Option<Value> {
    let data = match gql(app, sess, "FetchSecuritySearchResult", json!({"query": symbol.trim()})) {
        Ok(d) => d,
        Err(e) => {
            log(&format!("bagholder ticket: listing search for {} failed: {}", symbol, e));
            return None;
        }
    };
    let sec = parse_listing_search(&data, symbol, exchange);
    if let Some(sec) = &sec {
        let typed: bagholder_model::securities::Security = serde_json::from_value(sec.clone()).unwrap_or_default();
        must(bagholder_store::admin::upsert_securities(&db(app), std::slice::from_ref(&typed), &now_iso()));
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
    pub fx_usd_cad: Option<f64>,
    pub live: bool,
}

/// The full quote, or why there is none, in the ticket's own words.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(untagged)]
pub enum TicketQuote {
    Ok(TicketQuoteOk),
    Refused(crate::http::OkOr),
}

impl bagholder_model::patch::Diff for TicketQuote {
    fn diff(&self, new: &Self, path: &mut Vec<Value>, ops: &mut Vec<Value>) {
        match (self, new) {
            (TicketQuote::Ok(a), TicketQuote::Ok(b)) => a.diff(b, path, ops),
            // a refusal never becomes another shape while the same page holds it
            _ => bagholder_model::patch::as_json(self, new, path, ops),
        }
    }
}

impl TicketQuote {
    fn err(e: impl Into<String>) -> TicketQuote {
        TicketQuote::Refused(crate::http::OkOr::err(e))
    }
}

pub fn ticket_quote(app: &Arc<App>, symbol: &str, security_id: &str, account_id: &str, exchange: &str) -> TicketQuote {
    let name_of = || if symbol.is_empty() { security_id.to_string() } else { symbol.to_string() };
    let mut sec = resolve_security(app, symbol, security_id);
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
    let sid = f(&sec, "id");
    let mut quotes = match fetch_quotes(app, &sess, &[sid.clone()]) {
        Ok(q) => q,
        Err(CallError::NotAuthorized) => return TicketQuote::err("Wealthsimple refused the session. Connect Wealthsimple again."),
        Err(e) => return TicketQuote::err(format!("Quote failed: {}", err_text(&e))),
    };
    let mut quote = match quotes.remove(&sid) {
        Some(q) => q,
        None => {
            let label = if tr(&sec, "symbol") { f(&sec, "symbol") } else { sid.clone() };
            return TicketQuote::err(format!("Wealthsimple has no quote for {}.", label));
        }
    };
    if quote.symbol.is_empty() {
        quote.symbol = f(&sec, "symbol");
    }
    if quote.name.is_empty() {
        quote.name = f(&sec, "name");
    }
    if quote.exchange.is_empty() {
        quote.exchange = f(&sec, "primaryExchange");
    }
    if quote.currency.is_empty() {
        quote.currency = f(&sec, "currency").to_uppercase();
    }
    let mut md = MarketData::default();
    match gql(app, &sess, "FetchSecurityMarketData", json!({"id": sid})) {
        Ok(d) => md = parse_market_data(&serde_json::from_value(d).unwrap_or_default()),
        Err(e) => log(&format!("bagholder ticket: market data for {} failed: {}", sid, e)),
    }
    let accounts = order_accounts(app, None);
    let acct = accounts.iter().find(|a| a.id == account_id).cloned();
    let mut balance = BuyingPowerFigures::default();
    if let Some(a) = &acct {
        let cur = if quote.currency.is_empty() { "CAD".to_string() } else { quote.currency.clone() };
        match gql(app, &sess, "FetchTradingBalanceBuyingPower", json!({"accountCanonicalId": a.id, "currency": cur, "securityId": sid})) {
            Ok(d) => balance = parse_buying_power(&serde_json::from_value(d).unwrap_or_default()),
            Err(e) => log(&format!("bagholder ticket: buying power for {} failed: {}", a.id, e)),
        }
    }
    let mut margin_available = None;
    if let Some(a) = &acct {
        if !a.margin_account_id.is_empty() {
            for m in must(bagholder_store::tables::margin(&db(app))) {
                if m.account_id == a.margin_account_id {
                    if let Some(bp) = m.buying_power {
                        margin_available = Some(bp);
                    }
                }
            }
        }
    }
    let fx_map = must(bagholder_store::tables::fx_rates(&db(app), "USDCAD"));
    let fx_usd_cad = if fx_map.is_empty() {
        None
    } else {
        let fx: bagholder_model::fx::Fx = fx_map.into_iter().collect();
        Some(bagholder_model::fx::rate_on(&fx, &bagholder_model::clock::today_local()))
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
        fx_usd_cad,
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
    let acct = match order_accounts(app, None).into_iter().find(|a| a.id == t.account_id) {
        Some(a) => a,
        None => return Err("Choose an account.".into()),
    };
    let sec = match resolve_security(app, &t.symbol, &t.security_id) {
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
        "securityId": f(&sec, "id"),
        "timeInForce": tif,
    });
    if has_limit {
        set(&mut req, "limitPrice", jo(limit_price));
    }
    if has_stop {
        set(&mut req, "stopPrice", jo(stop_price));
    }
    let currency = t.currency.clone().filter(|c| !c.is_empty()).unwrap_or_else(|| f(&sec, "currency")).to_uppercase();
    let order = Order {
        id: oid,
        created_at: now_iso(),
        account_id: acct.id.clone(),
        account: acct.name.clone(),
        security_id: f(&sec, "id"),
        symbol: f(&sec, "symbol"),
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
        None => return PlaceTicketAnswer::err("Not connected."),
    };
    row.status = OrderStatus::Sending;
    must(so::typed::insert_order(&db(app), row, &now_iso()));
    let data = match gql(app, &sess, "SoOrdersOrderCreate", json!({"input": req})) {
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
    let empty = json!({});
    let result = data.get("soOrdersCreateOrder").filter(|v| truthy(Some(v))).unwrap_or(&empty);
    if let Some(msg) = result.get("errors").filter(|v| truthy(Some(v))).and_then(first_error) {
        now_is(OrderStatus::Rejected, &msg);
        log(&format!("bagholder order: {} rejected: {}", id, msg));
        return PlaceTicketAnswer::err_with_id(format!("Wealthsimple rejected the order: {}", msg), id);
    }
    let ws_id = f(result.get("order").filter(|v| truthy(Some(v))).unwrap_or(&empty), "orderId");
    patch_order(app, &id, OrderPatch { status: Some(OrderStatus::Sent), ws_order_id: Some(ws_id.clone()), ..OrderPatch::default() });
    log(&format!("bagholder order: {} sent, Wealthsimple order {}", id, ws_id));
    let rid = id.clone();
    let a = app.clone();
    spawn("bagholder-order-refresh", move || {
        let _ = catch_unwind(AssertUnwindSafe(|| refresh_orders(&a, &rid)));
    });
    PlaceTicketAnswer { ok: true, id: Some(id), status: Some("sent".into()), ws_order_id: Some(ws_id), ..PlaceTicketAnswer::default() }
}

/// Place what a ticket asks for. A sale first takes its shares out from under any
/// bracket guarding them -- the bracket ended, or kept on what is left -- so that the
/// bracket's own exits and this sale never sell the same shares twice.
pub fn place_ticket(app: &Arc<App>, t: &Ticket) -> PlaceTicketAnswer {
    let (mut row, req) = match ticket_order(app, t) {
        Ok(x) => x,
        Err(e) => return PlaceTicketAnswer::err(e),
    };
    if row.side == Side::Sell {
        let mut left = row.quantity.unwrap_or(0.0);
        for b in live_brackets(app) {
            if b.account_id != row.account_id || b.security_id != row.security_id || matches!(b.status, BracketStatus::Waiting | BracketStatus::Closing) {
                continue;
            }
            let held = b.quantity.unwrap_or(0.0);
            if left >= held {
                end_bracket(app, &b, "sold from the ticket", "");
                await_cancels(app, &b, 8);
                left -= held;
            } else if left > 0.0 {
                release_shares(app, &b, left);
                await_cancels(app, &b, 8);
                left = 0.0;
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

