//! Asking Wealthsimple for the book: the accounts, the activity feed, the
//! balances and the margin figures.
//!
//! Every one of these walks the broker's own pagination to the end. A page of
//! rows the store already has can still carry a revised one, and the rows
//! behind it are new, so a page is never taken as a stopping point.

use serde_json::{json, Value};

use crate::session::{CallError, Client};
use bagholder_model::value::{field_s, get, num};

/// `bagholder.fetch_all_accounts`.
pub fn fetch_all_accounts(client: &Client, sess: &Value, identity_id: &str) -> Result<Vec<Value>, CallError> {
    let mut accounts: Vec<Value> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let variables = json!({
            "identityId": identity_id,
            "pageSize": 25,
            "startDate": "2015-01-01",
            "cursor": cursor,
        });
        let data = client.graphql(sess, "FetchAllAccountFinancials", &variables, None)?;
        let conn = data.get("identity").and_then(|i| i.get("accounts")).cloned().unwrap_or(Value::Null);
        if let Some(edges) = conn.get("edges").and_then(|v| v.as_array()) {
            for edge in edges {
                if let Some(node) = edge.get("node").filter(|n| !n.is_null()) {
                    accounts.push(node.clone());
                }
            }
        }
        let page = conn.get("pageInfo").cloned().unwrap_or(Value::Null);
        if page.get("hasNextPage").and_then(|v| v.as_bool()) != Some(true) {
            break;
        }
        let next = field_s(&page, "endCursor");
        if next.is_empty() {
            break;
        }
        cursor = Some(next);
    }
    Ok(accounts)
}

/// `bagholder.activity_fetch_condition`: the window one pull asks for. The
/// start date is what turns a daily pull into new rows only.
pub fn activity_fetch_condition(account_id: &str, start_date: Option<&str>, now_unix: i64) -> Value {
    let end_secs = now_unix + 86400;
    let days = end_secs.div_euclid(86400);
    let rem = end_secs.rem_euclid(86400);
    let (y, m, d) = bagholder_model::dates::from_days(days);
    let end = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.999Z",
        y, m, d, rem / 3600, (rem % 3600) / 60, rem % 60
    );
    let mut cond = serde_json::Map::new();
    cond.insert("endDate".into(), json!(end));
    cond.insert("accountIds".into(), json!([account_id]));
    if let Some(raw) = start_date {
        let raw = raw.trim();
        if !raw.is_empty() {
            let full = if raw.contains('T') {
                raw.to_string()
            } else {
                format!("{}T00:00:00.000Z", raw.chars().take(10).collect::<String>())
            };
            cond.insert("startDate".into(), json!(full));
        }
    }
    Value::Object(cond)
}

/// `bagholder.fetch_activities_for_account`.
pub fn fetch_activities_for_account(
    client: &Client,
    sess: &Value,
    account_id: &str,
    start_date: Option<&str>,
    now_unix: i64,
) -> Result<Vec<Value>, CallError> {
    let mut items: Vec<Value> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut variables = serde_json::Map::new();
        variables.insert("first".into(), json!(100));
        variables.insert("orderBy".into(), json!("OCCURRED_AT_DESC"));
        variables.insert("condition".into(), activity_fetch_condition(account_id, start_date, now_unix));
        if let Some(c) = &cursor {
            variables.insert("cursor".into(), json!(c));
        }
        let data = client.graphql(sess, "FetchActivityFeedItems", &Value::Object(variables), None)?;
        let feed = data.get("activityFeedItems").cloned().unwrap_or(Value::Null);
        if let Some(edges) = feed.get("edges").and_then(|v| v.as_array()) {
            for edge in edges {
                if let Some(node) = edge.get("node").filter(|n| !n.is_null()) {
                    items.push(node.clone());
                }
            }
        }
        let page = feed.get("pageInfo").cloned().unwrap_or(Value::Null);
        // Walk every page the broker returns for the window: a page of known
        // rows can still carry a revised one, and the rows behind it are new.
        if page.get("hasNextPage").and_then(|v| v.as_bool()) != Some(true) {
            break;
        }
        let next = field_s(&page, "endCursor");
        if next.is_empty() {
            break;
        }
        cursor = Some(next);
    }
    Ok(items)
}

/// `bagholder.fetch_balances`: twenty accounts to a request.
pub fn fetch_balances(client: &Client, sess: &Value, account_ids: &[String]) -> Result<Vec<Value>, CallError> {
    let ids: Vec<&String> = account_ids.iter().filter(|i| !i.is_empty()).collect();
    let mut balances = Vec::new();
    for chunk in ids.chunks(20) {
        let list: Vec<&str> = chunk.iter().map(|s| s.as_str()).collect();
        let data = client.graphql(sess, "FetchAccountsWithBalance", &json!({"ids": list, "type": "TRADING"}), None)?;
        let accounts = data.get("accounts").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        for acc in accounts {
            let aid = acc.get("id").cloned().unwrap_or(Value::Null);
            let custodians = acc.get("custodianAccounts").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            for ca in custodians {
                let fin = ca.get("financials").cloned().unwrap_or(Value::Null);
                let bals = match fin.get("balance") {
                    Some(Value::Array(a)) => a.clone(),
                    Some(o) if o.is_object() => vec![o.clone()],
                    _ => vec![],
                };
                for b in bals {
                    balances.push(json!({
                        "accountId": aid,
                        "custodianAccountId": ca.get("id").cloned().unwrap_or(Value::Null),
                        "securityId": b.get("securityId").cloned().unwrap_or(Value::Null),
                        "quantity": b.get("quantity").cloned().unwrap_or(Value::Null),
                    }));
                }
            }
        }
    }
    Ok(balances)
}

/// `bagholder.parse_margin`: the buying power as Wealthsimple answers it -- a
/// Money when it is available, the reason when it is not, and nothing at all
/// when the account has no margin figures.
pub fn parse_margin(data: &Value) -> Option<Value> {
    let trading = data
        .get("account")?
        .get("financials")?
        .get("current")?
        .get("marginV3")?
        .get("trading")
        .cloned()
        .unwrap_or(Value::Null);
    let bp = trading.get("buyingPower").filter(|b| b.is_object())?;

    if field_s(bp, "__typename") == "BuyingPowerMetricAvailable" {
        let total = bp.get("total").cloned().unwrap_or(Value::Null);
        let amount = match get(&total, "amount") {
            Some(v) => {
                let f = num(Some(v), f64::NAN);
                if f.is_nan() { return None } else { f }
            }
            None => return None,
        };
        let currency = { let c = field_s(&total, "currency"); if c.is_empty() { "CAD".to_string() } else { c } };
        return Some(json!({"buyingPower": amount, "currency": currency, "unavailable": ""}));
    }
    let reason = bp.get("reason").cloned().unwrap_or(Value::Null);
    let mut why = field_s(&reason, "__typename");
    if why.is_empty() {
        why = field_s(bp, "__typename");
    }
    if why.is_empty() {
        why = "unavailable".into();
    }
    let n = reason.get("securities").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    if n > 0 {
        why = format!("{} ({} securities)", why, n);
    }
    Some(json!({"buyingPower": Value::Null, "currency": "CAD", "unavailable": why}))
}

/// `bagholder.margin_account_ids`: the open margin accounts, which are the
/// only ones whose buying power is margin available.
///
/// Wealthsimple answers the buying-power query for every self-directed account
/// with the cash it could buy with, and with an error for cash, card and
/// crypto accounts. Neither is margin.
pub fn margin_account_ids(accounts: &[Value]) -> Vec<String> {
    let mut out = Vec::new();
    for a in accounts {
        let typ = {
            let t = field_s(a, "unifiedAccountType");
            if t.is_empty() { field_s(a, "unified_account_type") } else { t }
        }
        .to_uppercase();
        let status = field_s(a, "status").to_lowercase();
        let id = field_s(a, "id");
        if !id.is_empty() && typ.contains("MARGIN") && status != "closed" {
            out.push(id);
        }
    }
    out
}

/// `bagholder._money_amount`: the first Money.amount present among the named
/// keys, with its currency.
pub fn money_amount(node: &Value, keys: &[&str]) -> (Option<f64>, Option<String>) {
    if !node.is_object() {
        return (None, None);
    }
    for key in keys {
        let money = match node.get(*key) { Some(m) if m.is_object() => m, _ => continue };
        let amount = match get(money, "amount") { Some(a) => a, None => continue };
        let f = num(Some(amount), f64::NAN);
        if f.is_nan() {
            continue;
        }
        let ccy = { let c = field_s(money, "currency"); if c.is_empty() { "CAD".to_string() } else { c } };
        return (Some(f), Some(ccy));
    }
    (None, None)
}

/// `bagholder._nav_points_from_payload`: one page of daily net liquidation.
pub fn nav_points_from_payload(data: &Value) -> (Vec<Value>, Value) {
    let ident = data.get("identity").cloned().unwrap_or(Value::Null);
    let acc = data.get("account").cloned().unwrap_or(Value::Null);
    let fin = match ident.get("financials") {
        Some(f) if !f.is_null() => f.clone(),
        _ => acc.get("financials").cloned().unwrap_or(Value::Null),
    };
    let hist = fin.get("historicalDaily").cloned().unwrap_or(Value::Null);
    let mut points = Vec::new();
    for edge in hist.get("edges").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
        let node = edge.get("node").cloned().unwrap_or(Value::Null);
        let (amt, cur) = money_amount(&node, &["netLiquidationValue", "netLiquidationValueV2"]);
        let d: String = field_s(&node, "date").chars().take(10).collect();
        let amt = match amt { Some(a) if !d.is_empty() => a, _ => continue };
        let mut rec = serde_json::Map::new();
        rec.insert("date".into(), json!(d));
        rec.insert("equity".into(), json!(amt));
        rec.insert("currency".into(), json!(cur.unwrap_or_else(|| "CAD".into())));
        if let (Some(nd), _) = money_amount(&node, &["netDeposits", "netDepositsV2"]) {
            rec.insert("netDeposits".into(), json!(nd));
        }
        points.push(Value::Object(rec));
    }
    (points, hist.get("pageInfo").cloned().unwrap_or(json!({})))
}

/// `bagholder._paginate_nav_history`: a year at a time from `since` (or 2020),
/// eight pages a year at most, one point a day.
pub fn paginate_nav_history(client: &Client, sess: &Value, operation: &str, extra: &Value, since: Option<&str>, today: &str) -> Result<Vec<Value>, CallError> {
    let since: String = since.unwrap_or("").chars().take(10).collect();
    if !since.is_empty() && since.as_str() > today {
        return Ok(vec![]);
    }
    let year0: i64 = if since.is_empty() { 2020 } else { since[..4].parse().unwrap_or(2020) };
    let year1: i64 = today[..4].parse().unwrap_or(year0);
    let mut points: Vec<Value> = Vec::new();
    for year in year0..=year1 {
        let mut start = format!("{}-01-01", year);
        if !since.is_empty() && start < since {
            start = since.clone();
        }
        let end = if year == year1 { today.to_string() } else { format!("{}-12-31", year) };
        if start > end {
            continue;
        }
        let mut cursor: Option<String> = None;
        for _ in 0..8 {
            let mut vars = extra.as_object().cloned().unwrap_or_default();
            vars.insert("startDate".into(), json!(start));
            vars.insert("endDate".into(), json!(end));
            vars.insert("cursor".into(), match &cursor { Some(c) => json!(c), None => Value::Null });
            let data = client.graphql(sess, operation, &Value::Object(vars), None)?;
            let (chunk, page) = nav_points_from_payload(&data);
            points.extend(chunk);
            if page.get("hasNextPage").map(|v| v.as_bool().unwrap_or(false)) != Some(true) {
                break;
            }
            let next = field_s(&page, "endCursor");
            if next.is_empty() {
                break;
            }
            cursor = Some(next);
        }
    }
    let mut by_date: std::collections::BTreeMap<String, Value> = std::collections::BTreeMap::new();
    for rec in points {
        by_date.insert(field_s(&rec, "date"), rec);
    }
    Ok(by_date.into_values().collect())
}

/// `bagholder.fetch_nav_history`: identity-wide net liquidation.
pub fn fetch_nav_history(client: &Client, sess: &Value, identity_id: &str, since: Option<&str>, today: &str) -> Result<Vec<Value>, CallError> {
    paginate_nav_history(client, sess, "IdentityHistoricalFinancialsQuery",
        &json!({"identityId": identity_id, "currency": "CAD", "limit": 400, "includeNetDeposits": true}), since, today)
}

/// `bagholder.fetch_account_nav_history`: one account's daily net liquidation.
pub fn fetch_account_nav_history(client: &Client, sess: &Value, account_id: &str, since: Option<&str>, today: &str) -> Result<Vec<Value>, CallError> {
    let aid = account_id.trim();
    if aid.is_empty() {
        return Ok(vec![]);
    }
    paginate_nav_history(client, sess, "FetchAccountHistoricalFinancials",
        &json!({"id": aid, "currency": "CAD", "resolution": "DAILY", "first": 400}), since, today)
}

/// `bagholder.merge_nav_points`: equity and net deposits summed by date across
/// account series.
pub fn merge_nav_points(series: &[Vec<Value>]) -> Vec<Value> {
    let mut by_date: std::collections::BTreeMap<String, serde_json::Map<String, Value>> = std::collections::BTreeMap::new();
    for list in series {
        for rec in list {
            if !rec.is_object() {
                continue;
            }
            let d: String = field_s(rec, "date").chars().take(10).collect();
            if d.is_empty() {
                continue;
            }
            let eq = match rec.get("equity") {
                None | Some(Value::Null) => continue,
                Some(v) => { let f = num(Some(v), f64::NAN); if f.is_nan() { continue } else { f } }
            };
            let cur = by_date.entry(d.clone()).or_insert_with(|| {
                let c = { let c = field_s(rec, "currency"); if c.is_empty() { "CAD".to_string() } else { c } };
                let mut m = serde_json::Map::new();
                m.insert("date".into(), json!(d));
                m.insert("equity".into(), json!(0.0));
                m.insert("currency".into(), json!(c));
                m
            });
            let e = cur["equity"].as_f64().unwrap_or(0.0) + eq;
            cur.insert("equity".into(), json!(e));
            let c = field_s(rec, "currency");
            if !c.is_empty() {
                cur.insert("currency".into(), json!(c));
            }
            if let Some(nd) = rec.get("netDeposits").filter(|v| !v.is_null()) {
                let f = num(Some(nd), f64::NAN);
                if !f.is_nan() {
                    let prev = cur.get("netDeposits").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    cur.insert("netDeposits".into(), json!(prev + f));
                }
            }
        }
    }
    by_date.into_values().map(Value::Object).collect()
}

/// `bagholder.fetch_margin`: one buying-power request per margin account; only
/// accounts that answer are rows. A failure is said once on the terminal.
pub fn fetch_margin(client: &Client, sess: &Value, account_ids: &[String], now: &str) -> Vec<Value> {
    let mut rows = Vec::new();
    let mut failed = 0;
    let mut first_error = String::new();
    for aid in account_ids.iter().filter(|a| !a.is_empty()) {
        let data = match client.graphql(sess, "FetchAccountCurrentMarginBuyingPowerV2", &json!({"accountId": aid, "currency": "CAD"}), None) {
            Ok(d) => d,
            Err(e) => {
                failed += 1;
                if first_error.is_empty() {
                    first_error = e.to_string();
                }
                continue;
            }
        };
        if let Some(Value::Object(mut m)) = parse_margin(&data) {
            m.insert("accountId".into(), json!(aid));
            m.insert("fetchedAt".into(), json!(now));
            rows.push(Value::Object(m));
        }
    }
    if failed > 0 {
        eprintln!("bagholder portfolio: buying power request failed for {} of {} accounts ({})", failed, account_ids.iter().filter(|a| !a.is_empty()).count(), first_error);
    }
    rows
}

/// `bagholder._security_record`.
pub fn security_record(sec: &Value, sid: &str) -> Option<Value> {
    let m = sec.as_object()?;
    if m.is_empty() {
        return None;
    }
    let obj = |k: &str| sec.get(k).filter(|v| v.is_object()).cloned().unwrap_or(json!({}));
    let stock = obj("stock");
    let option = obj("optionDetails");
    let under = option.get("underlyingSecurity").filter(|v| v.is_object()).cloned().unwrap_or(json!({}));
    let under_id = field_s(&under, "id").trim().to_string();
    let id = { let i = field_s(sec, "id").trim().to_string(); if i.is_empty() { sid.to_string() } else { i } };
    Some(json!({
        "id": id,
        "symbol": field_s(&stock, "symbol").trim(),
        "name": field_s(&stock, "name").trim(),
        "primaryExchange": field_s(&stock, "primaryExchange").trim(),
        "primaryMic": field_s(&stock, "primaryMic").trim(),
        "currency": field_s(sec, "currency").trim(),
        "underlyingId": if under_id.is_empty() { Value::Null } else { json!(under_id) },
    }))
}

/// `bagholder.fetch_security`.
pub fn fetch_security(client: &Client, sess: &Value, security_id: &str) -> Option<Value> {
    let sid = security_id.trim();
    if sid.is_empty() {
        return None;
    }
    let data = client.graphql(sess, "FetchSecurity", &json!({"securityId": sid}), None).ok()?;
    security_record(data.get("security").unwrap_or(&Value::Null), sid)
}

/// `bagholder.SECURITY_BATCH`.
pub const SECURITY_BATCH: usize = 50;

/// `bagholder.fetch_securities`: one request per fifty ids; a failed batch
/// falls back to one request per id.
pub fn fetch_securities(client: &Client, sess: &Value, ids: &[String]) -> Vec<Value> {
    let mut uniq: Vec<String> = Vec::new();
    for raw in ids {
        let sid = raw.trim().to_string();
        if !sid.is_empty() && !uniq.contains(&sid) {
            uniq.push(sid);
        }
    }
    let mut out = Vec::new();
    for chunk in uniq.chunks(SECURITY_BATCH) {
        let rows = client
            .graphql(sess, "FetchSecurities", &json!({"ids": chunk}), None)
            .ok()
            .and_then(|d| d.get("securities").and_then(|s| s.as_array()).cloned());
        match rows {
            Some(rows) => {
                for sec in rows {
                    if let Some(rec) = security_record(&sec, "") {
                        out.push(rec);
                    }
                }
            }
            None => {
                for sid in chunk {
                    if let Some(rec) = fetch_security(client, sess, sid) {
                        out.push(rec);
                    }
                }
            }
        }
    }
    out
}
