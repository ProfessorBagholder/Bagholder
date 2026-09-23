//! Asking Wealthsimple for the book: the accounts, the activity feed, the
//! balances and the margin figures.
//!
//! Every one of these walks the broker's own pagination to the end. A page of
//! rows the store already has can still carry a revised one, and the rows
//! behind it are new, so a page is never taken as a stopping point.

use serde::Serialize;
use serde_json::Value;

use bagholder_model::securities::Security;
use bagholder_store::broker::{Balance, Margin, NavPoint};

use crate::session::{CallError, Client, Session};
use crate::wire::{
    AccountNode, AccountsAnswer, AccountsWithBalance, ActivityAnswer, ActivityItem, MarginAnswer, Money, NavAnswer,
    PageInfo, SecurityAnswer, SecuritiesAnswer,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AllAccountsVars<'a> {
    identity_id: &'a str,
    page_size: i64,
    start_date: &'a str,
    cursor: Option<String>,
}

pub fn fetch_all_accounts(client: &Client, sess: &Session, identity_id: &str) -> Result<Vec<AccountNode>, CallError> {
    let mut accounts: Vec<AccountNode> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let variables = AllAccountsVars { identity_id, page_size: 25, start_date: "2015-01-01", cursor: cursor.clone() };
        let data: AccountsAnswer = client.graphql(sess, "FetchAllAccountFinancials", &variables, None)?;
        let conn = data.identity.and_then(|i| i.accounts).unwrap_or_default();
        let next = conn.next_cursor();
        accounts.extend(conn.nodes());
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    Ok(accounts)
}

/// The window one pull asks for. The
/// start date is what turns a daily pull into new rows only.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Condition {
    pub end_date: String,
    pub account_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
}

pub fn activity_fetch_condition(account_id: &str, start_date: Option<&str>, now_unix: i64) -> Condition {
    let end_secs = now_unix + 86400;
    let days = end_secs.div_euclid(86400);
    let rem = end_secs.rem_euclid(86400);
    let (y, m, d) = bagholder_model::dates::from_days(days);
    let end = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.999Z",
        y, m, d, rem / 3600, (rem % 3600) / 60, rem % 60
    );
    let mut cond = Condition { end_date: end, account_ids: vec![account_id.to_string()], start_date: None };
    if let Some(raw) = start_date {
        let raw = raw.trim();
        if !raw.is_empty() {
            let full = if raw.contains('T') {
                raw.to_string()
            } else {
                format!("{}T00:00:00.000Z", raw.chars().take(10).collect::<String>())
            };
            cond.start_date = Some(full);
        }
    }
    cond
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ActivityVars {
    first: i64,
    order_by: &'static str,
    condition: Condition,
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor: Option<String>,
}

pub fn fetch_activities_for_account(
    client: &Client,
    sess: &Session,
    account_id: &str,
    start_date: Option<&str>,
    now_unix: i64,
) -> Result<Vec<ActivityItem>, CallError> {
    let mut items: Vec<ActivityItem> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let variables = ActivityVars {
            first: 100,
            order_by: "OCCURRED_AT_DESC",
            condition: activity_fetch_condition(account_id, start_date, now_unix),
            cursor: cursor.clone(),
        };
        let data: ActivityAnswer = client.graphql(sess, "FetchActivityFeedItems", &variables, None)?;
        let feed = data.activity_feed_items.unwrap_or_default();
        let next = feed.next_cursor();
        // Walk every page the broker returns for the window: a page of known
        // rows can still carry a revised one, and the rows behind it are new.
        items.extend(feed.nodes());
        match next {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    Ok(items)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BalancesVars<'a> {
    ids: Vec<&'a str>,
    #[serde(rename = "type")]
    kind: &'static str,
}

/// Twenty accounts to a request.
pub fn fetch_balances(client: &Client, sess: &Session, account_ids: &[String]) -> Result<Vec<Balance>, CallError> {
    let ids: Vec<&String> = account_ids.iter().filter(|i| !i.is_empty()).collect();
    let mut balances = Vec::new();
    for chunk in ids.chunks(20) {
        let list: Vec<&str> = chunk.iter().map(|s| s.as_str()).collect();
        let data: AccountsWithBalance = client.graphql(sess, "FetchAccountsWithBalance", &BalancesVars { ids: list, kind: "TRADING" }, None)?;
        for acc in data.accounts {
            for ca in acc.custodian_accounts {
                let bals = ca.financials.map(|f| f.balance).unwrap_or_default();
                for b in bals {
                    balances.push(Balance {
                        account_id: acc.id.clone(),
                        custodian_account_id: ca.id.clone(),
                        security_id: b.security_id,
                        quantity: b.quantity,
                    });
                }
            }
        }
    }
    Ok(balances)
}

/// The buying power as Wealthsimple answers it -- a
/// Money when it is available, the reason when it is not, and nothing at all
/// when the account has no margin figures.
pub fn parse_margin(data: &MarginAnswer) -> Option<Margin> {
    let bp = data
        .account
        .as_ref()?
        .financials
        .as_ref()?
        .current
        .as_ref()?
        .margin_v3
        .as_ref()?
        .trading
        .as_ref()?
        .buying_power
        .as_ref()?;

    if bp.typename == "BuyingPowerMetricAvailable" {
        let total = bp.total.as_ref();
        let amount = match total.and_then(|t| t.amount) {
            Some(f) => f,
            None => return None,
        };
        let currency = total.map(|t| t.currency.clone()).filter(|c| !c.is_empty()).unwrap_or_else(|| "CAD".into());
        return Some(Margin { account_id: String::new(), buying_power: Some(amount), currency, unavailable: String::new(), fetched_at: String::new() });
    }
    let reason = bp.reason.as_ref();
    let mut why = reason.map(|r| r.typename.clone()).unwrap_or_default();
    if why.is_empty() {
        why = bp.typename.clone();
    }
    if why.is_empty() {
        why = "unavailable".into();
    }
    let n = reason.map(|r| r.securities.len()).unwrap_or(0);
    if n > 0 {
        why = format!("{} ({} securities)", why, n);
    }
    Some(Margin { account_id: String::new(), buying_power: None, currency: "CAD".into(), unavailable: why, fetched_at: String::new() })
}

/// The open margin accounts, which are the
/// only ones whose buying power is margin available.
///
/// Wealthsimple answers the buying-power query for every self-directed account
/// with the cash it could buy with, and with an error for cash, card and
/// crypto accounts. Neither is margin.
pub fn margin_account_ids(accounts: &[AccountNode]) -> Vec<String> {
    let mut out = Vec::new();
    for a in accounts {
        let typ = a.unified_account_type.to_uppercase();
        let status = a.status.to_lowercase();
        if !a.id.is_empty() && typ.contains("MARGIN") && status != "closed" {
            out.push(a.id.clone());
        }
    }
    out
}

/// The first `Money` present among the named fields, with its currency.
pub fn money_amount(candidates: &[Option<&Money>]) -> (Option<f64>, Option<String>) {
    for money in candidates.iter().filter_map(|m| *m) {
        if let Some(f) = money.amount {
            let ccy = if money.currency.is_empty() { "CAD".to_string() } else { money.currency.clone() };
            return (Some(f), Some(ccy));
        }
    }
    (None, None)
}

/// One page of daily net liquidation.
///
/// The `PageInfo` returned is always both its fields (unlike the untyped code,
/// which passed the wire's own `pageInfo` object through unshaped): a caller
/// that only asks `has_next_page`/`end_cursor` sees the same pagination
/// either way, which is the only thing every caller but the golden test
/// glue reads it for.
pub fn nav_points_from_payload(data: &NavAnswer) -> (Vec<NavPoint>, PageInfo) {
    let fin = match data.identity.as_ref().and_then(|i| i.financials.as_ref()) {
        Some(f) => Some(f),
        None => data.account.as_ref().and_then(|a| a.financials.as_ref()),
    };
    let hist = fin.and_then(|f| f.historical_daily.as_ref());
    let mut points = Vec::new();
    for node in hist.map(|h| h.edges.iter().filter_map(|e| e.node.as_ref())).into_iter().flatten() {
        let (amt, cur) = money_amount(&[node.net_liquidation_value.as_ref(), node.net_liquidation_value_v2.as_ref()]);
        let d: String = node.date.chars().take(10).collect();
        let amt = match amt { Some(a) if !d.is_empty() => a, _ => continue };
        let (nd, _) = money_amount(&[node.net_deposits.as_ref(), node.net_deposits_v2.as_ref()]);
        points.push(NavPoint {
            account_id: String::new(),
            date: d,
            equity: Some(amt),
            currency: cur.unwrap_or_else(|| "CAD".into()),
            net_deposits: nd,
        });
    }
    (points, hist.map(|h| h.page_info.clone()).unwrap_or_default())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct IdentityNavVars<'a> {
    identity_id: &'a str,
    currency: &'static str,
    limit: i64,
    include_net_deposits: bool,
    start_date: String,
    end_date: String,
    cursor: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountNavVars<'a> {
    id: &'a str,
    currency: &'static str,
    resolution: &'static str,
    first: i64,
    start_date: String,
    end_date: String,
    cursor: Option<String>,
}

/// A year at a time from `since` (or 2020),
/// eight pages a year at most, one point a day.
fn paginate_nav_history<F>(client: &Client, sess: &Session, operation: &str, since: Option<&str>, today: &str, mut make_vars: F) -> Result<Vec<NavPoint>, CallError>
where
    F: FnMut(String, String, Option<String>) -> Value,
{
    let since: String = since.unwrap_or("").chars().take(10).collect();
    if !since.is_empty() && since.as_str() > today {
        return Ok(vec![]);
    }
    let year0: i64 = if since.is_empty() { 2020 } else { since[..4].parse().unwrap_or(2020) };
    let year1: i64 = today[..4].parse().unwrap_or(year0);
    let mut points: Vec<NavPoint> = Vec::new();
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
            let vars = make_vars(start.clone(), end.clone(), cursor.clone());
            let data: NavAnswer = client.graphql(sess, operation, &vars, None)?;
            let (chunk, page) = nav_points_from_payload(&data);
            points.extend(chunk);
            match page.has_next_page && !page.end_cursor.is_empty() {
                true => cursor = Some(page.end_cursor),
                false => break,
            }
        }
    }
    let mut by_date: std::collections::BTreeMap<String, NavPoint> = std::collections::BTreeMap::new();
    for rec in points {
        by_date.insert(rec.date.clone(), rec);
    }
    Ok(by_date.into_values().collect())
}

/// Identity-wide net liquidation.
pub fn fetch_nav_history(client: &Client, sess: &Session, identity_id: &str, since: Option<&str>, today: &str) -> Result<Vec<NavPoint>, CallError> {
    paginate_nav_history(client, sess, "IdentityHistoricalFinancialsQuery", since, today, |start, end, cursor| {
        serde_json::to_value(IdentityNavVars { identity_id, currency: "CAD", limit: 400, include_net_deposits: true, start_date: start, end_date: end, cursor }).unwrap()
    })
}

/// One account's daily net liquidation.
pub fn fetch_account_nav_history(client: &Client, sess: &Session, account_id: &str, since: Option<&str>, today: &str) -> Result<Vec<NavPoint>, CallError> {
    let aid = account_id.trim();
    if aid.is_empty() {
        return Ok(vec![]);
    }
    paginate_nav_history(client, sess, "FetchAccountHistoricalFinancials", since, today, |start, end, cursor| {
        serde_json::to_value(AccountNavVars { id: aid, currency: "CAD", resolution: "DAILY", first: 400, start_date: start, end_date: end, cursor }).unwrap()
    })
}

/// Equity and net deposits summed by date across
/// account series.
pub fn merge_nav_points(series: &[Vec<NavPoint>]) -> Vec<NavPoint> {
    let mut by_date: std::collections::BTreeMap<String, NavPoint> = std::collections::BTreeMap::new();
    for list in series {
        for rec in list {
            let d: String = rec.date.chars().take(10).collect();
            if d.is_empty() {
                continue;
            }
            let eq = match rec.equity { None => continue, Some(f) => f };
            let cur = by_date.entry(d.clone()).or_insert_with(|| NavPoint {
                account_id: String::new(),
                date: d.clone(),
                equity: Some(0.0),
                currency: if rec.currency.is_empty() { "CAD".to_string() } else { rec.currency.clone() },
                net_deposits: None,
            });
            cur.equity = Some(cur.equity.unwrap_or(0.0) + eq);
            if !rec.currency.is_empty() {
                cur.currency = rec.currency.clone();
            }
            if let Some(nd) = rec.net_deposits {
                cur.net_deposits = Some(cur.net_deposits.unwrap_or(0.0) + nd);
            }
        }
    }
    by_date.into_values().collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MarginVars<'a> {
    account_id: &'a str,
    currency: &'static str,
}

/// One buying-power request per margin account; only
/// accounts that answer are rows. A failure is said once on the terminal.
pub fn fetch_margin(client: &Client, sess: &Session, account_ids: &[String], now: &str) -> Vec<Margin> {
    let mut rows = Vec::new();
    let mut failed = 0;
    let mut first_error = String::new();
    for aid in account_ids.iter().filter(|a| !a.is_empty()) {
        let data: MarginAnswer = match client.graphql(sess, "FetchAccountCurrentMarginBuyingPowerV2", &MarginVars { account_id: aid, currency: "CAD" }, None) {
            Ok(d) => d,
            Err(e) => {
                failed += 1;
                if first_error.is_empty() {
                    first_error = e.to_string();
                }
                continue;
            }
        };
        if let Some(mut m) = parse_margin(&data) {
            m.account_id = aid.clone();
            m.fetched_at = now.to_string();
            rows.push(m);
        }
    }
    if failed > 0 {
        eprintln!("bagholder portfolio: buying power request failed for {} of {} accounts ({})", failed, account_ids.iter().filter(|a| !a.is_empty()).count(), first_error);
    }
    rows
}

/// `sec` is `None` for "no answer at all"; an empty object -- Wealthsimple's
/// way of saying it has no record -- is also read as no security, matching
/// today's behaviour exactly.
pub fn security_record(sec: &Value, sid: &str) -> Option<Security> {
    if !sec.is_object() || sec.as_object().map(|m| m.is_empty()).unwrap_or(true) {
        return None;
    }
    let node: crate::wire::SecurityNode = serde_json::from_value(sec.clone()).unwrap_or_default();
    let stock = node.stock.unwrap_or_default();
    let under_id = node.option_details.and_then(|o| o.underlying_security).map(|u| u.id.trim().to_string()).unwrap_or_default();
    let id = if node.id.trim().is_empty() { sid.to_string() } else { node.id.trim().to_string() };
    Some(Security {
        id,
        symbol: stock.symbol.trim().to_string(),
        name: stock.name.trim().to_string(),
        primary_exchange: stock.primary_exchange.trim().to_string(),
        primary_mic: stock.primary_mic.trim().to_string(),
        currency: node.currency.trim().to_string(),
        underlying_id: under_id,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SecurityVars<'a> {
    security_id: &'a str,
}

pub fn fetch_security(client: &Client, sess: &Session, security_id: &str) -> Option<Security> {
    let sid = security_id.trim();
    if sid.is_empty() {
        return None;
    }
    let data: SecurityAnswer = client.graphql(sess, "FetchSecurity", &SecurityVars { security_id: sid }, None).ok()?;
    security_record(&data.security.unwrap_or(Value::Null), sid)
}

pub const SECURITY_BATCH: usize = 50;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SecuritiesVars<'a> {
    ids: &'a [String],
}

/// One request per fifty ids; a failed batch
/// falls back to one request per id.
pub fn fetch_securities(client: &Client, sess: &Session, ids: &[String]) -> Vec<Security> {
    let mut uniq: Vec<String> = Vec::new();
    for raw in ids {
        let sid = raw.trim().to_string();
        if !sid.is_empty() && !uniq.contains(&sid) {
            uniq.push(sid);
        }
    }
    let mut out = Vec::new();
    for chunk in uniq.chunks(SECURITY_BATCH) {
        let rows: Option<Vec<Value>> = client
            .graphql::<SecuritiesAnswer>(sess, "FetchSecurities", &SecuritiesVars { ids: chunk }, None)
            .ok()
            .and_then(|d| d.securities);
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
