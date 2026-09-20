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

pub(super) fn ticket_session() -> Option<Value> {
    #[cfg(test)]
    {
        return seam::SESSION.lock().unwrap_or_else(|e| e.into_inner()).clone().flatten();
    }
    #[allow(unreachable_code)]
    let sess = load_session()?;
    if f(&sess, "access_token").is_empty() {
        return None;
    }
    ensure_fresh_token(Some(sess.clone()));
    match load_session() {
        Some(v) if v.as_object().map_or(false, |m| !m.is_empty()) => Some(v),
        _ => Some(sess),
    }
}

pub fn order_accounts(accounts: Option<&[Value]>) -> Vec<Value> {
    let owned;
    let list: &[Value] = match accounts {
        Some(a) => a,
        None => {
            owned = snapshot().get("accounts").and_then(|a| a.as_array()).cloned().unwrap_or_default();
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
        out.push(json!({
            "id": f(a, "id"), "name": nick, "type": typ, "margin": margin, "currency": f(a, "currency"),
            "marginAccountId": if margin { f(a, "id") } else { f(a, "marginAccountId") },
        }));
    }
    out
}

pub fn resolve_security(symbol: &str, security_id: &str) -> Option<Value> {
    let rows = must(bagholder_store::admin::list_securities(&db()));
    let sid = security_id.trim();
    if !sid.is_empty() {
        if let Some(r) = rows.iter().find(|r| f(r, "id") == sid) {
            return Some(r.clone());
        }
        return Some(json!({"id": sid, "symbol": symbol.trim().to_uppercase(), "name": "", "primaryExchange": "", "primaryMic": "", "currency": "", "underlyingId": null}));
    }
    let sym = symbol.trim().to_uppercase();
    if sym.is_empty() {
        return None;
    }
    let mut same: Vec<Value> = rows.into_iter().filter(|r| f(r, "symbol").to_uppercase() == sym).collect();
    same.sort_by_key(|r| (if f(r, "id").starts_with("sec-s-") { 0 } else { 1 }, f(r, "id")));
    same.into_iter().next()
}

pub fn parse_quote(node: &Value) -> Option<Value> {
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
    Some(json!({
        "securityId": f(node, "id"),
        "symbol": f(stock, "symbol"),
        "name": f(stock, "name"),
        "exchange": f(stock, "primaryExchange"),
        "currency": s(or_v(q.get("currency"), node.get("currency"))).to_uppercase(),
        "securityType": f(node, "securityType"),
        "buyable": tr(node, "buyable"),
        "sellable": tr(node, "sellable"),
        "tradeEligible": tr(node, "wsTradeEligible"),
        "status": f(node, "status"),
        "last": jo(last),
        "bid": jo(bid),
        "ask": jo(ask),
        "bidSize": jo(on(q, "bidSize")),
        "askSize": jo(on(q, "askSize")),
        "mid": jo(mid),
        "change": jo(change),
        "changePct": jo(change_pct),
        "marketStatus": f(q, "marketStatus"),
        "quotedAsOf": f(q, "quotedAsOf"),
        "multiplier": jo(multiplier),
    }))
}

pub fn parse_market_data(data: &Value) -> Value {
    let empty = json!({});
    let sec = data.get("security").filter(|v| v.is_object()).unwrap_or(&empty);
    let subtypes: Vec<String> = sec
        .get("allowedOrderSubtypes")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter(|x| truthy(Some(x))).map(|x| s(Some(x)).to_uppercase()).collect())
        .unwrap_or_default();
    let rates = sec.get("marginRates").filter(|v| v.is_object()).unwrap_or(&empty);
    let mut rate = on(rates, "clientMarginRate");
    if let Some(r) = rate {
        if r > 1.0 {
            rate = Some(r / 100.0);
        }
    }
    let types: Vec<&str> = ORDER_EXEC_TYPES.iter().copied().filter(|t| subtypes.iter().any(|x| x == t)).collect();
    json!({"orderTypes": types, "marginRate": jo(rate)})
}

pub fn parse_buying_power(data: &Value) -> Value {
    let empty = json!({});
    let mut view = data;
    for k in ["account", "financials", "current", "tradingBalanceViewV2"] {
        view = view.get(k).filter(|v| v.is_object()).unwrap_or(&empty);
    }
    let bp = view.get("buyingPower").filter(|v| v.is_object()).unwrap_or(&empty);
    let cash = view.get("cash").filter(|v| v.is_object()).unwrap_or(&empty);
    json!({"buyingPower": jo(on(bp, "quantity")), "cash": jo(on(cash, "quantity")), "currency": s(or_v(bp.get("currency"), cash.get("currency")))})
}

pub fn fetch_quotes(sess: &Value, security_ids: &[String]) -> Result<HashMap<String, Value>, CallError> {
    let ids: Vec<String> = security_ids.iter().map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
    let mut out = HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let data = gql(sess, "FetchSecuritiesSummary", json!({"ids": ids}))?;
    if let Some(a) = data.get("securities").and_then(|v| v.as_array()) {
        for node in a {
            if let Some(q) = parse_quote(node) {
                out.insert(f(&q, "securityId"), q);
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

pub fn lookup_listing(sess: &Value, symbol: &str, exchange: &str) -> Option<Value> {
    let data = match gql(sess, "FetchSecuritySearchResult", json!({"query": symbol.trim()})) {
        Ok(d) => d,
        Err(e) => {
            log(&format!("bagholder ticket: listing search for {} failed: {}", symbol, e));
            return None;
        }
    };
    let sec = parse_listing_search(&data, symbol, exchange);
    if let Some(sec) = &sec {
        must(bagholder_store::admin::upsert_securities(&db(), std::slice::from_ref(sec), &now_iso()));
    }
    sec
}

pub fn ticket_quote(symbol: &str, security_id: &str, account_id: &str, exchange: &str) -> Value {
    let name_of = || if symbol.is_empty() { security_id.to_string() } else { symbol.to_string() };
    let mut sec = resolve_security(symbol, security_id);
    if sec.is_none() && exchange.is_empty() {
        return json!({"ok": false, "error": format!("No listing stored for {}.", name_of())});
    }
    let sess = match ticket_session() {
        Some(s) => s,
        None => return json!({"ok": false, "error": "Not connected."}),
    };
    if sec.is_none() {
        sec = lookup_listing(&sess, &symbol.trim().to_uppercase(), exchange);
    }
    let sec = match sec {
        Some(s) => s,
        None => return json!({"ok": false, "error": format!("No listing stored for {}.", name_of())}),
    };
    let sid = f(&sec, "id");
    let mut quotes = match fetch_quotes(&sess, &[sid.clone()]) {
        Ok(q) => q,
        Err(CallError::NotAuthorized) => return json!({"ok": false, "error": "Wealthsimple refused the session. Connect Wealthsimple again."}),
        Err(e) => return json!({"ok": false, "error": format!("Quote failed: {}", err_text(&e))}),
    };
    let mut quote = match quotes.remove(&sid) {
        Some(q) => q,
        None => {
            let label = if tr(&sec, "symbol") { f(&sec, "symbol") } else { sid.clone() };
            return json!({"ok": false, "error": format!("Wealthsimple has no quote for {}.", label)});
        }
    };
    for (qk, sk) in [("symbol", "symbol"), ("name", "name"), ("exchange", "primaryExchange")] {
        if f(&quote, qk).is_empty() {
            set(&mut quote, qk, json!(f(&sec, sk)));
        }
    }
    if f(&quote, "currency").is_empty() {
        set(&mut quote, "currency", json!(f(&sec, "currency").to_uppercase()));
    }
    let mut md = json!({"orderTypes": ORDER_EXEC_TYPES, "marginRate": null});
    match gql(&sess, "FetchSecurityMarketData", json!({"id": sid})) {
        Ok(d) => md = parse_market_data(&d),
        Err(e) => log(&format!("bagholder ticket: market data for {} failed: {}", sid, e)),
    }
    let accounts = order_accounts(None);
    let acct = accounts.iter().find(|a| f(a, "id") == account_id).cloned();
    let mut balance = json!({"buyingPower": null, "cash": null, "currency": ""});
    if let Some(a) = &acct {
        let cur = if f(&quote, "currency").is_empty() { "CAD".to_string() } else { f(&quote, "currency") };
        match gql(&sess, "FetchTradingBalanceBuyingPower", json!({"accountCanonicalId": f(a, "id"), "currency": cur, "securityId": sid})) {
            Ok(d) => balance = parse_buying_power(&d),
            Err(e) => log(&format!("bagholder ticket: buying power for {} failed: {}", f(a, "id"), e)),
        }
    }
    let mut margin_available = Value::Null;
    if let Some(a) = &acct {
        if tr(a, "marginAccountId") {
            let snap = snapshot();
            for m in snap.get("margin").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
                if f(&m, "accountId") == f(a, "marginAccountId") && m.get("buyingPower").map_or(false, |v| !v.is_null()) {
                    margin_available = jo(on(&m, "buyingPower"));
                }
            }
        }
    }
    let fx_map = must(bagholder_store::tables::fx_rates(&db(), "USDCAD"));
    let fx_usd_cad = if fx_map.is_empty() {
        Value::Null
    } else {
        let fx: bagholder_model::fx::Fx = fx_map.iter().filter_map(|(k, v)| v.as_f64().map(|x| (k.clone(), x))).collect();
        json!(bagholder_model::fx::rate_on(&fx, &bagholder_model::clock::today_local()))
    };
    let order_types = match md.get("orderTypes") {
        Some(Value::Array(a)) if !a.is_empty() => Value::Array(a.clone()),
        _ => json!(ORDER_EXEC_TYPES),
    };
    json!({
        "ok": true,
        "quote": quote,
        "orderTypes": order_types,
        "marginRate": gv(&md, "marginRate"),
        "accounts": accounts,
        "account": acct,
        "buyingPower": gv(&balance, "buyingPower"),
        "cash": gv(&balance, "cash"),
        "marginAvailable": margin_available,
        "fxUsdCad": fx_usd_cad,
        "live": orders_live(),
    })
}

pub fn order_tick(price: Option<f64>) -> Option<f64> {
    price.map(|p| round_half_even(p, if p >= 1.0 { 2 } else { 4 }))
}

/// (row, request) or the error.
pub fn order_request(body: &Value) -> Result<(Value, Value), String> {
    let empty = json!({});
    let b = if body.is_object() { body } else { &empty };
    let side = f(b, "side").to_uppercase();
    if side != "BUY" && side != "SELL" {
        return Err("Side must be Buy or Sell.".into());
    }
    let exec_type = f(b, "type").to_uppercase();
    if !ORDER_EXEC_TYPES.contains(&exec_type.as_str()) {
        return Err("Order type must be Market, Limit, Stop or Stop limit.".into());
    }
    let tif = s(or_v(b.get("tif"), Some(&json!("DAY")))).to_uppercase();
    if !ORDER_TIFS.contains(&tif.as_str()) {
        return Err("Time in force must be Day or Good till cancelled.".into());
    }
    let qty = num(b.get("quantity"), Some(0.0)).unwrap_or(0.0);
    if qty == 0.0 || qty <= 0.0 {
        return Err("Quantity must be more than zero.".into());
    }
    let limit_price = order_tick(on(b, "limitPrice"));
    let stop_price = order_tick(on(b, "stopPrice"));
    let positive = |p: Option<f64>| p.map_or(false, |x| x != 0.0 && x > 0.0);
    if (exec_type == "LIMIT" || exec_type == "STOP_LIMIT") && !positive(limit_price) {
        return Err("A limit price is required.".into());
    }
    if (exec_type == "STOP" || exec_type == "STOP_LIMIT") && !positive(stop_price) {
        return Err("A stop price is required.".into());
    }
    let acct = match order_accounts(None).into_iter().find(|a| f(a, "id") == f(b, "accountId")) {
        Some(a) => a,
        None => return Err("Choose an account.".into()),
    };
    let sec = match resolve_security(&f(b, "symbol"), &f(b, "securityId")) {
        Some(s) => s,
        None => return Err(format!("No listing stored for {}.", f(b, "symbol"))),
    };
    let mut sl = b.get("stopLoss").filter(|v| v.is_object()).cloned();
    let mut tp = b.get("takeProfit").filter(|v| v.is_object()).cloned();
    if side == "SELL" {
        sl = None;
        tp = None;
    }
    let sl_row = match sl.filter(|v| truthy(Some(v))) {
        None => Value::Null,
        Some(sl) => {
            let kind = s(or_v(sl.get("kind"), Some(&json!("stop")))).to_lowercase();
            if kind != "stop" && kind != "trail" {
                return Err("Stop loss type must be Stop or Trailing stop.".into());
            }
            if kind == "stop" && !(num(sl.get("price"), Some(0.0)).unwrap_or(0.0) > 0.0) {
                return Err("A stop loss price is required.".into());
            }
            if kind == "trail" && !(num(sl.get("trail"), Some(0.0)).unwrap_or(0.0) > 0.0) {
                return Err("A trail is required.".into());
            }
            json!({"kind": kind, "price": jo(order_tick(on(&sl, "price"))), "trail": jo(on(&sl, "trail")),
                   "trailUnit": if f(&sl, "trailUnit").to_lowercase() == "amt" { "amt" } else { "pct" }})
        }
    };
    let tp_row = match tp.filter(|v| truthy(Some(v))) {
        None => Value::Null,
        Some(tp) => {
            if !(num(tp.get("price"), Some(0.0)).unwrap_or(0.0) > 0.0) {
                return Err("A take profit price is required.".into());
            }
            json!({"price": jo(order_tick(on(&tp, "price")))})
        }
    };
    let oid = format!("order-{}", uuid4());
    let mut req = json!({
        "canonicalAccountId": f(&acct, "id"),
        "externalId": oid,
        "executionType": exec_type,
        "orderType": format!("{}_QUANTITY", side),
        "quantity": qty,
        "securityId": f(&sec, "id"),
        "timeInForce": tif,
    });
    if exec_type == "LIMIT" || exec_type == "STOP_LIMIT" {
        set(&mut req, "limitPrice", jo(limit_price));
    }
    if exec_type == "STOP" || exec_type == "STOP_LIMIT" {
        set(&mut req, "stopPrice", jo(stop_price));
    }
    let row = json!({
        "id": oid,
        "createdAt": now_iso(),
        "accountId": f(&acct, "id"),
        "account": f(&acct, "name"),
        "securityId": f(&sec, "id"),
        "symbol": f(&sec, "symbol"),
        "currency": s(or_v(b.get("currency"), sec.get("currency"))).to_uppercase(),
        "side": side,
        "type": exec_type,
        "quantity": qty,
        "limitPrice": gv(&req, "limitPrice"),
        "stopPrice": gv(&req, "stopPrice"),
        "tif": tif,
        "stopLoss": sl_row,
        "takeProfit": tp_row,
        "status": "",
        "wsOrderId": "",
        "error": "",
        "request": req.clone(),
    });
    Ok((row, req))
}

pub fn submit_order(row: &mut Value, req: &Value) -> Value {
    let id = f(row, "id");
    if !orders_live() {
        set(row, "status", json!("dry"));
        insert_order(row);
        log(&format!("bagholder order (dry run, not sent): {}", bagholder_store::tables::json_text_sorted(req)));
        return json!({"ok": true, "id": id, "status": "dry", "order": row.clone()});
    }
    let sess = match ticket_session() {
        Some(s) => s,
        None => return json!({"ok": false, "error": "Not connected."}),
    };
    set(row, "status", json!("sending"));
    insert_order(row);
    let data = match gql(&sess, "SoOrdersOrderCreate", json!({"input": req})) {
        Ok(d) => d,
        Err(CallError::NotAuthorized) => {
            update_order(&id, json!({"status": "failed", "error": "Wealthsimple refused the session."}));
            return json!({"ok": false, "error": "Wealthsimple refused the session. Connect Wealthsimple again.", "id": id});
        }
        Err(e) => {
            let msg = err_text(&e);
            update_order(&id, json!({"status": "failed", "error": msg}));
            log(&format!("bagholder order: {} failed: {}", id, msg));
            return json!({"ok": false, "error": format!("Order failed: {}", msg), "id": id});
        }
    };
    let empty = json!({});
    let result = data.get("soOrdersCreateOrder").filter(|v| truthy(Some(v))).unwrap_or(&empty);
    if let Some(msg) = result.get("errors").filter(|v| truthy(Some(v))).and_then(first_error) {
        update_order(&id, json!({"status": "rejected", "error": msg}));
        log(&format!("bagholder order: {} rejected: {}", id, msg));
        return json!({"ok": false, "error": format!("Wealthsimple rejected the order: {}", msg), "id": id});
    }
    let order = result.get("order").filter(|v| truthy(Some(v))).unwrap_or(&empty);
    let ws_id = f(order, "orderId");
    update_order(&id, json!({"status": "sent", "wsOrderId": ws_id}));
    log(&format!("bagholder order: {} sent, Wealthsimple order {}", id, ws_id));
    let rid = id.clone();
    spawn("bagholder-order-refresh", move || {
        let _ = catch_unwind(|| refresh_orders(&rid));
    });
    json!({"ok": true, "id": id, "status": "sent", "wsOrderId": ws_id})
}

pub fn place_order(body: &Value) -> Value {
    let (mut row, req) = match order_request(body) {
        Ok(x) => x,
        Err(e) => return json!({"ok": false, "error": e}),
    };
    if f(&row, "side") == "SELL" {
        let mut left = or0(&row, "quantity");
        for b in live_brackets() {
            if b.account_id != f(&row, "accountId") || b.security_id != f(&row, "securityId") || matches!(b.status, BracketStatus::Waiting | BracketStatus::Closing) {
                continue;
            }
            let held = b.quantity.unwrap_or(0.0);
            if left >= held {
                end_bracket(&b, "sold from the ticket", "");
                await_cancels(&b, 8);
                left -= held;
            } else if left > 0.0 {
                release_shares(&b, left);
                await_cancels(&b, 8);
                left = 0.0;
            }
        }
    }
    let mut r = submit_order(&mut row, &req);
    if tr(&r, "ok") && (tr(&row, "stopLoss") || tr(&row, "takeProfit")) {
        // the row as the store now has it: what was sent, under the id it was given
        let entry: Order = serde_json::from_value(row.clone()).unwrap_or_default();
        let b = create_bracket(&entry);
        set(&mut r, "bracketId", json!(b.id));
    }
    r
}
