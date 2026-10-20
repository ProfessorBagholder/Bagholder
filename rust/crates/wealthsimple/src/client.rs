//! Wealthsimple over the network: its GraphQL endpoint, signed with the session
//! (`crate::session`), every request through the one limiter (`bagholder-net`).
//! Every request here is a read. A reply that says the session is no longer
//! valid (401 or 403) is refreshed once and asked once more; a second refusal
//! is a lapse, and nothing is asked again with it.

use std::sync::atomic::{AtomicUsize, Ordering};

use bagholder_broker::{Answer, Failure};
use bagholder_core::json::{self, Value};
use bagholder_net::{Ask, Net};
use bagholder_sources::reply::Node;

use crate::adapter::Source;
use crate::session::{self, SessionFile, Tokens};

/// The least time between two requests to Wealthsimple.
pub const PACE: std::time::Duration = std::time::Duration::from_millis(150);

fn doc(name: &str) -> &'static str {
    match name {
        "FetchAllAccounts" => include_str!("../graphql/FetchAllAccounts.graphql"),
        "FetchActivityFeedItems" => include_str!("../graphql/FetchActivityFeedItems.graphql"),
        "FetchSoOrdersMultilegOrder" => include_str!("../graphql/FetchSoOrdersMultilegOrder.graphql"),
        "FetchCorporateActionChildActivities" => include_str!("../graphql/FetchCorporateActionChildActivities.graphql"),
        "FetchHoldingsExportPositionsAsOfDate" => include_str!("../graphql/FetchHoldingsExportPositionsAsOfDate.graphql"),
        "FetchAccountHistoricalFinancials" => include_str!("../graphql/FetchAccountHistoricalFinancials.graphql"),
        "FetchFundingIntent" => include_str!("../graphql/FetchFundingIntent.graphql"),
        "FetchInternalTransfer" => include_str!("../graphql/FetchInternalTransfer.graphql"),
        "FetchAccountsWithBalance" => include_str!("../graphql/FetchAccountsWithBalance.graphql"),
        "Securities" => include_str!("../graphql/Securities.graphql"),
        "FetchInstitutionalTransfer" => include_str!("../graphql/FetchInstitutionalTransfer.graphql"),
        "FetchCreditCardAccount" => include_str!("../graphql/FetchCreditCardAccount.graphql"),
        "FetchIdentityNetWorthAccounts" => include_str!("../graphql/FetchIdentityNetWorthAccounts.graphql"),
        "FetchIdentityNetWorthFinancials" => include_str!("../graphql/FetchIdentityNetWorthFinancials.graphql"),
        "FetchAccountCurrentMarginBuyingPowerV2" => include_str!("../graphql/FetchAccountCurrentMarginBuyingPowerV2.graphql"),
        "FetchMonthlyStatementWithTransactions" => include_str!("../graphql/FetchMonthlyStatementWithTransactions.graphql"),
        "FetchTradingBalanceBuyingPower" => include_str!("../graphql/FetchTradingBalanceBuyingPower.graphql"),
        // the probe's only: the enum's values where Wealthsimple answers introspection
        other => panic!("no document named {other}"),
    }
}

/// How Wealthsimple's web app sends one of its documents as it does from a
/// page: the document's own hash in its build (the `__meta__.hash` its
/// compiled query carries), the page it is sent from, and the build's version.
/// The statement document is refused unless sent this way (2026-09-27: the
/// same variables with the client's own headers answered
/// `UNPROCESSABLE_ENTITY`; the page's request answered).
struct AsPage {
    hash: &'static str,
    page: &'static str,
}

/// The web app's build these documents and hashes were read from, sent as
/// `x-web-version` (`assets.wealthsimple.com/app-9351431b1bbd31ba.js`, read 2026-09-27).
const WEB_VERSION: &str = "0.3.671473";

fn as_page(op: &str) -> Option<AsPage> {
    match op {
        // chunk 293 of the build above: the Documents page's Download CSV
        "FetchMonthlyStatementWithTransactions" => Some(AsPage { hash: "d5950e5b8d4b7b49a8fe02d68d3d5f7d33d3b7bd555a95db3089fff3dd4d917a", page: "page-docs" }),
        _ => None,
    }
}

fn obj(pairs: Vec<(&str, Value)>) -> Value {
    Value::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn text(s: &str) -> Value {
    Value::String(s.to_string())
}

fn num(n: i64) -> Value {
    Value::Number(n.to_string())
}

fn mismatch(op: &str, m: impl std::fmt::Display) -> Failure {
    Failure::Mismatch(format!("{op}: {m}"))
}

pub struct Client<'n> {
    net: &'n Net,
    file: SessionFile,
    tokens: Option<Tokens>,
    sent: AtomicUsize,
    /// Never renew the sign-in: a client run beside the person's app, which
    /// holds the same sign-in, must never rotate its refresh token (brief 68).
    read_only: bool,
}

/// What a read-only client says when the saved sign-in needs renewing.
pub const RENEW_IN_THE_APP: &str = "Open Bagholder once so it renews its sign-in, then run this again.";

impl<'n> Client<'n> {
    /// A client on `net`, Wealthsimple's host paced on its one limiter: a
    /// request at most every 150 ms, the pace the owner's own capture of their
    /// whole history kept without a refusal (2026-09-24).
    pub fn new(net: &'n Net, file: SessionFile) -> Client<'n> {
        let host = bagholder_net::host_of(&crate::hosts::graphql_url());
        let pace = net.limiter().pace(&host);
        if pace.gap < PACE {
            net.limiter().configure(&host, bagholder_net::Pace { gap: PACE, ..pace });
        }
        Client { net, file, tokens: None, sent: AtomicUsize::new(0), read_only: false }
    }

    /// A client that only reads with the saved sign-in as it is: it never
    /// renews it and never writes `session.json`, so the person's app, which
    /// holds the same sign-in, keeps a token that works. An expired sign-in is
    /// said ([`RENEW_IN_THE_APP`]) and nothing is asked.
    pub fn read_only(net: &'n Net, file: SessionFile) -> Client<'n> {
        Client { read_only: true, ..Client::new(net, file) }
    }

    fn tokens(&mut self) -> Answer<Tokens> {
        if let Some(t) = &self.tokens {
            if self.read_only {
                // used as it is until it expires, and never renewed
                let now = self.net.clock().now();
                return if t.expires_at.is_none_or(|e| e > now) { Ok(t.clone()) } else { Err(Failure::Lapsed(RENEW_IN_THE_APP.into())) };
            }
            // a token about to expire is refreshed before it is used
            let soon = self.net.clock().now().checked_add(jiff::Span::new().minutes(5)).ok();
            if t.expires_at.zip(soon).is_none_or(|(e, s)| e > s) {
                return Ok(t.clone());
            }
            let fresh = session::refresh(self.net, &self.file, t)?;
            self.tokens = Some(fresh.clone());
            return Ok(fresh);
        }
        let (_, t) = self.file.load()?.ok_or_else(|| Failure::Lapsed("no saved sign-in: connect Wealthsimple".into()))?;
        self.tokens = Some(t);
        self.tokens()
    }

    /// One GraphQL read: its `data`, or what failed.
    pub fn graphql(&mut self, op: &str, variables: Value) -> Answer<Value> {
        let body = obj(vec![("operationName", text(op)), ("query", text(doc(op))), ("variables", variables)]).canonical();
        let mut refreshed = false;
        loop {
            let t = self.tokens()?;
            let auth = format!("Bearer {}", t.access);
            let mut headers = vec![
                ("Authorization", auth.as_str()),
                ("Content-Type", "application/json"),
                ("Accept", "application/json"),
                ("x-ws-profile", "trade"),
                ("x-ws-api-version", "12"),
                ("x-ws-locale", "en-CA"),
                ("x-platform-os", "web"),
                ("x-ws-identity-id", t.identity.as_str()),
            ];
            // a document the web app sends from a page: with the headers its page
            // sends (the client tier only where the app knows one, as the page
            // itself does: `...i?{"x-ws-client-tier":i}:{}` in its build)
            let page = as_page(op);
            if let Some(p) = &page {
                let device = t.device.as_deref().ok_or_else(|| Failure::Refused(format!("{op}: the saved sign-in has no device id to send it with; connect Wealthsimple again")))?;
                headers.extend([("x-ws-operation-name", op), ("x-ws-operation-hash", p.hash), ("x-ws-device-id", device), ("x-ws-page", p.page), ("x-web-version", WEB_VERSION)]);
            }
            self.sent.fetch_add(1, Ordering::Relaxed);
            bagholder_net::client::named_call("wealthsimple", &format!("graphql {op}"));
            let reply = self.net.send(&Ask::post(&crate::hosts::graphql_url(), &headers, body.as_bytes())).map_err(|e| Failure::Unreachable(e.to_string()))?;
            if reply.status == 401 || reply.status == 403 {
                if self.read_only {
                    return Err(Failure::Lapsed(RENEW_IN_THE_APP.into()));
                }
                if refreshed {
                    return Err(Failure::Lapsed(format!("{op}: Wealthsimple answered {} after a refresh", reply.status)));
                }
                // a query is read again once, after a refresh; never more
                let fresh = session::refresh(self.net, &self.file, &t)?;
                self.tokens = Some(fresh);
                refreshed = true;
                continue;
            }
            if reply.status != 200 {
                return Err(Failure::Refused(format!("{op}: Wealthsimple answered {}", reply.status)));
            }
            let v = json::parse(&String::from_utf8_lossy(&reply.body)).map_err(|e| mismatch(op, e))?;
            let n = Node::root(&v);
            if let Ok(errors) = n.field("errors") {
                if !matches!(errors.value(), Value::Null) {
                    let first = errors.as_list().ok().and_then(|l| l.first().and_then(|e| e.opt_text("message").ok().flatten().map(str::to_string))).unwrap_or_else(|| errors.value().canonical());
                    return Err(Failure::Refused(format!("{op}: {first}")));
                }
            }
            return n.obj("data").map(|d| d.value().clone()).map_err(|m| mismatch(op, m));
        }
    }

    /// Every page of a connection, each page's nodes from `at(data)`.
    fn pages(&mut self, op: &str, vars: impl Fn(Option<&str>) -> Value, at: impl for<'a> Fn(&Node<'a>) -> Result<Node<'a>, bagholder_sources::reply::Mismatch>) -> Answer<Vec<Value>> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let data = self.graphql(op, vars(cursor.as_deref()))?;
            let root = Node::root(&data);
            let conn = at(&root).map_err(|m| mismatch(op, m))?;
            for e in conn.list("edges").map_err(|m| mismatch(op, m))? {
                out.push(e.obj("node").map_err(|m| mismatch(op, m))?.value().clone());
            }
            let page = conn.obj("pageInfo").map_err(|m| mismatch(op, m))?;
            if page.bool("hasNextPage").map_err(|m| mismatch(op, m))? {
                cursor = Some(page.text("endCursor").map_err(|m| mismatch(op, m))?.to_string());
            } else {
                return Ok(out);
            }
        }
    }

    fn identity(&mut self) -> Answer<String> {
        Ok(self.tokens()?.identity)
    }
}

/// The start of a day in Wealthsimple's filing zone, as the feed's condition takes it.
fn start_of(day: jiff::civil::Date) -> Answer<String> {
    let tz = jiff::tz::TimeZone::get(crate::mapping::ZONE).map_err(|e| Failure::Mismatch(e.to_string()))?;
    let at = day.at(0, 0, 0, 0).to_zoned(tz).map_err(|e| Failure::Mismatch(e.to_string()))?.timestamp();
    Ok(at.to_string())
}

impl Source for Client<'_> {
    fn accounts(&mut self) -> Answer<Vec<Value>> {
        let identity = self.identity()?;
        self.pages(
            "FetchAllAccounts",
            |c| obj(vec![("identityId", text(&identity)), ("pageSize", num(25)), ("cursor", c.map(text).unwrap_or(Value::Null))]),
            |d| d.obj("identity")?.obj("accounts"),
        )
    }
    fn activity(&mut self, account: &str, from: Option<jiff::civil::Date>) -> Answer<Vec<Value>> {
        let mut condition = vec![("accountIds", Value::Array(vec![text(account)]))];
        if let Some(d) = from {
            condition.push(("startDate", text(&start_of(d)?)));
        }
        let condition = obj(condition);
        self.pages("FetchActivityFeedItems", |c| obj(vec![("first", num(100)), ("cursor", c.map(text).unwrap_or(Value::Null)), ("condition", condition.clone())]), |d| d.obj("activityFeedItems"))
    }
    fn securities(&mut self, ids: &[String]) -> Answer<Vec<Value>> {
        let data = self.graphql("Securities", obj(vec![("ids", Value::Array(ids.iter().map(|i| text(i)).collect()))]))?;
        let list = Node::root(&data).list("securities").map_err(|m| mismatch("Securities", m))?;
        Ok(list.into_iter().map(|n| n.value().clone()).collect())
    }
    fn order(&mut self, batch: &str) -> Answer<Option<Value>> {
        let data = self.graphql("FetchSoOrdersMultilegOrder", obj(vec![("branchId", text("TR")), ("orderBatchId", text(batch))]))?;
        let o = Node::root(&data).field("soOrdersMultilegOrder").map_err(|m| mismatch("FetchSoOrdersMultilegOrder", m))?;
        Ok(if matches!(o.value(), Value::Null) { None } else { Some(o.value().clone()) })
    }
    fn entitlements(&mut self, activity: &str) -> Answer<Option<Value>> {
        let data = self.graphql("FetchCorporateActionChildActivities", obj(vec![("activityCanonicalId", text(activity))]))?;
        let c = Node::root(&data).obj("corporateActionChildActivities").map_err(|m| mismatch("FetchCorporateActionChildActivities", m))?;
        Ok(Some(c.value().clone()))
    }
    fn conversion(&mut self, id: &str) -> Answer<Option<Value>> {
        // a funding intent's id starts with its kind (`funding_intent-…`); a
        // recorded reply's stand-in keeps that word (`anon-funding-3`)
        if id.strip_prefix(crate::anonymise::STAND_IN).unwrap_or(id).starts_with("funding") {
            let data = self.graphql("FetchFundingIntent", obj(vec![("ids", Value::Array(vec![text(id)]))]))?;
            let edges = Node::root(&data).obj("searchFundingIntents").and_then(|f| f.list("edges")).map_err(|m| mismatch("FetchFundingIntent", m))?;
            return Ok(edges.first().and_then(|e| e.obj("node").ok()).map(|n| n.value().clone()));
        }
        let data = self.graphql("FetchInternalTransfer", obj(vec![("id", text(id))]))?;
        let t = Node::root(&data).field("internalTransfer").map_err(|m| mismatch("FetchInternalTransfer", m))?;
        Ok(if matches!(t.value(), Value::Null) { None } else { Some(t.value().clone()) })
    }
    fn transfer(&mut self, id: &str) -> Answer<Option<Value>> {
        let data = self.graphql("FetchInstitutionalTransfer", obj(vec![("id", text(id))]))?;
        let t = Node::root(&data).field("accountTransfer").map_err(|m| mismatch("FetchInstitutionalTransfer", m))?;
        Ok(if matches!(t.value(), Value::Null) { None } else { Some(t.value().clone()) })
    }
    fn card(&mut self, account: &str) -> Answer<Value> {
        let data = self.graphql("FetchCreditCardAccount", obj(vec![("id", text(account))]))?;
        Ok(Node::root(&data).obj("creditCardAccount").map_err(|m| mismatch("FetchCreditCardAccount", m))?.value().clone())
    }
    fn net_worth_history(&mut self, accounts: &[String], outside: &[String]) -> Answer<Option<Vec<Value>>> {
        let identity = self.identity()?;
        let op = "FetchIdentityNetWorthFinancials";
        let list = |xs: &[String]| Value::Array(xs.iter().map(|x| text(x)).collect());
        // from the first day the page's own "All" asks for, every day in one answer
        let vars = obj(vec![("identityId", text(&identity)), ("accountIds", list(accounts)), ("externalFinancialEntityIds", list(outside)), ("currency", text("CAD")), ("startDate", text("2014-01-01")), ("endDate", Value::Null), ("first", num(10_000)), ("accountScope", text("OWN"))]);
        let data = self.graphql(op, vars)?;
        let edges = Node::root(&data).obj("identity").and_then(|i| i.obj("netWorth")).and_then(|n| n.obj("financials")).and_then(|f| f.obj("historicalDaily")).and_then(|h| h.list("edges")).map_err(|m| mismatch(op, m))?;
        let mut out = Vec::new();
        for e in edges {
            out.push(e.obj("node").map_err(|m| mismatch(op, m))?.value().clone());
        }
        Ok(Some(out))
    }
    fn net_worth(&mut self) -> Answer<Option<Value>> {
        let identity = self.identity()?;
        let op = "FetchIdentityNetWorthAccounts";
        let data = self.graphql(op, obj(vec![("identityId", text(&identity))]))?;
        // the identity whole: its id says which outside accounts are the person's own
        Ok(Some(Node::root(&data).obj("identity").map_err(|m| mismatch(op, m))?.value().clone()))
    }
    fn positions(&mut self, account: &str, day: jiff::civil::Date) -> Answer<Value> {
        let op = "FetchHoldingsExportPositionsAsOfDate";
        let data = self.graphql(op, obj(vec![("accountIds", Value::Array(vec![text(account)])), ("asOf", text(&day.to_string())), ("currency", text("CAD"))]))?;
        let mut nodes = Vec::new();
        for a in Node::root(&data).list("accounts").map_err(|m| mismatch(op, m))? {
            let edges = a.obj("financials").and_then(|f| f.obj("current")).and_then(|c| c.obj("positionsAsOfDate")).and_then(|p| p.list("edges")).map_err(|m| mismatch(op, m))?;
            for e in edges {
                nodes.push(e.obj("node").map_err(|m| mismatch(op, m))?.value().clone());
            }
        }
        Ok(Value::Array(nodes))
    }
    fn balances(&mut self, accounts: &[String]) -> Answer<Vec<Value>> {
        let mut out = Vec::new();
        for chunk in accounts.chunks(20) {
            let data = self.graphql("FetchAccountsWithBalance", obj(vec![("ids", Value::Array(chunk.iter().map(|a| text(a)).collect())), ("type", text("TRADING"))]))?;
            out.extend(Node::root(&data).list("accounts").map_err(|m| mismatch("FetchAccountsWithBalance", m))?.into_iter().map(|n| n.value().clone()));
        }
        Ok(out)
    }
    fn buying_power(&mut self, account: &str) -> Answer<Value> {
        let op = "FetchAccountCurrentMarginBuyingPowerV2";
        let data = self.graphql(op, obj(vec![("accountId", text(account)), ("currency", text("CAD"))]))?;
        Ok(Node::root(&data).obj("account").map_err(|m| mismatch(op, m))?.value().clone())
    }
    fn history(&mut self, account: &str, from: Option<jiff::civil::Date>) -> Answer<Vec<Value>> {
        let start = from.map(|d| text(&d.to_string())).unwrap_or(Value::Null);
        self.pages(
            "FetchAccountHistoricalFinancials",
            |c| obj(vec![("id", text(account)), ("currency", text("CAD")), ("resolution", text("DAILY")), ("startDate", start.clone()), ("first", num(1000)), ("cursor", c.map(text).unwrap_or(Value::Null))]),
            |d| d.obj("account")?.obj("financials")?.obj("historicalDaily"),
        )
    }
    fn statement(&mut self, account: &str, month: jiff::civil::Date, kind: &str) -> Answer<Option<Value>> {
        let op = "FetchMonthlyStatementWithTransactions";
        let data = self.graphql(op, obj(vec![("accountId", text(account)), ("period", text(&month.to_string())), ("statementType", text(kind))]))?;
        let s = Node::root(&data).field("monthlyStatement").map_err(|m| mismatch(op, m))?;
        Ok(if matches!(s.value(), Value::Null) { None } else { Some(s.value().clone()) })
    }
    fn requests(&self) -> usize {
        self.sent.load(Ordering::Relaxed)
    }
}
