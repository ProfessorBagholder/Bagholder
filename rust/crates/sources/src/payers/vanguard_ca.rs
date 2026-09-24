//! Vanguard Canada (research 2, 2026-09-24): its product list page names every
//! fund by its id (`"portIds":"1811,…"` in the page's site configuration); its
//! GraphQL service (`www.vanguard.ca/gpx/graphql`, consumer `ca0`) states each
//! fund's Canadian ticker and schedule (`fundDistributionFrequency`), and, with no
//! start date, the fund's whole distribution history: ex, record and pay dates,
//! and each amount with its type and currency. `INC` (`Dividend Amount`) is paid
//! in cash; a type described as reinvested (`CGCA`, `Capital Gain - Reinvested
//! (Canada)`) is paid in units. A type not in that list is a mismatch naming it.

use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::Timestamp;
use bagholder_core::json::Value;
use bagholder_core::{Currency, Dec, SourceName};
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::needs::PayerNeed;
use crate::outcome::{Noted, Outcome};
use crate::payers::{Distribution, Payer, Record};
use crate::reply::{Mismatch, Node};
use crate::venue;

pub const SOURCE: &str = "vanguard-canada";
pub const HOST: &str = "www.vanguard.ca";
const LIST: &str = "https://www.vanguard.ca/en/product";
const GRAPHQL: &str = "https://www.vanguard.ca/gpx/graphql";
const HEADERS: [(&str, &str); 3] = [("Content-Type", "application/json"), ("x-consumer-id", "ca0"), ("User-Agent", "Mozilla/5.0")];
const LISTINGS: &str = r#"query Listings($portIds: [String!]!) { funds(portIds: $portIds) { profile { portId fundDistributionFrequency listings { identifiers(altIds: [\"Ticker - Canada\"]) { altIdCode altIdValue } } } } }"#;
const DISTRIBUTIONS: &str = "query Distributions($portIds: [String!]!) { funds(portIds: $portIds) { portId profile { fundCurrency } distributionDetails { periodicDistributions(limit: 0) { items { exDividendDate recordDate payableDate scheduleType { scheduleCode scheduleDesc } taxDetails { distributionAmount distributionType { distCode distDesc } currencyCode } } } } } }";

pub fn per_year(word: &str) -> Option<u32> {
    Some(match word {
        "Monthly" => 12,
        "Quarterly" => 4,
        "Semi-Annually" => 2,
        "Annually" => 1,
        _ => return None,
    })
}

/// The funds' ids the product list page names.
pub fn port_ids(html: &str) -> Option<Vec<String>> {
    let i = html.find("\"portIds\":\"")? + "\"portIds\":\"".len();
    let end = html[i..].find('"')? + i;
    let ids: Vec<String> = html[i..end].split(',').map(str::to_string).filter(|s| !s.is_empty()).collect();
    (!ids.is_empty()).then_some(ids)
}

/// A fund's id and stated schedule, by its Canadian ticker.
pub fn fund_of(v: &Value, ticker: &str) -> Result<Option<(String, Option<u32>)>, Mismatch> {
    for f in Node::root(v).obj("data")?.list("funds")? {
        let p = f.obj("profile")?;
        let tickers: Vec<String> = p
            .list("listings")?
            .iter()
            .map(|l| l.list("identifiers"))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .map(|i| i.text("altIdValue").map(str::to_string))
            .collect::<Result<_, _>>()?;
        if tickers.iter().any(|t| t.eq_ignore_ascii_case(ticker)) {
            // Vanguard states a schedule for every fund it lists: a fund without
            // one is a change of what it publishes
            let w = p.text("fundDistributionFrequency")?;
            let per = Some(per_year(w).ok_or_else(|| p.mismatch(format!("{w:?} is not a schedule this reader knows")))?);
            return Ok(Some((p.text("portId")?.to_string(), per)));
        }
    }
    Ok(None)
}

/// A fund's whole distribution history.
pub fn parse_distributions(v: &Value, port_id: &str) -> Outcome<Vec<Distribution>> {
    let read = || -> Result<Result<Vec<Distribution>, String>, Mismatch> {
        let funds = Node::root(v).obj("data")?.list("funds")?;
        let [fund] = funds.as_slice() else {
            return Ok(Err(format!("the reply holds {} funds, not {port_id}", funds.len())));
        };
        if fund.text("portId")? != port_id {
            return Ok(Err(format!("the reply is fund {}, not {port_id}", fund.text("portId")?)));
        }
        let mut out = Vec::new();
        for item in fund.obj("distributionDetails")?.obj("periodicDistributions")?.list("items")? {
            let ex: Date = item.day("exDividendDate")?;
            let (mut cash, mut units, mut currency): (Dec, Option<Dec>, Option<Currency>) = (Dec::ZERO, None, None);
            for t in item.list("taxDetails")? {
                let amount = t.dec("distributionAmount")?;
                let kind = t.obj("distributionType")?;
                let (code, desc) = (kind.text("distCode")?, kind.text("distDesc")?);
                let c = match Currency::parse(t.text("currencyCode")?) {
                    Ok(c) => c,
                    Err(e) => return Ok(Err(format!("{port_id} {ex}: {e}"))),
                };
                if currency.is_some_and(|x| x != c) {
                    return Ok(Err(format!("{port_id} {ex}: its amounts are in two currencies")));
                }
                currency = Some(c);
                match code {
                    "INC" => cash = cash.checked_add(amount).map_err(|e| t.mismatch(e.to_string()))?,
                    _ if desc.contains("Reinvested") => units = Some(units.unwrap_or(Dec::ZERO).checked_add(amount).map_err(|e| t.mismatch(e.to_string()))?),
                    _ => return Err(kind.mismatch(format!("{code} ({desc}) is not a kind of distribution this reader knows"))),
                }
            }
            let Some(currency) = currency else {
                return Ok(Err(format!("{port_id} {ex} states no amount")));
            };
            out.push(Distribution { ex_date: ex, record_date: item.opt_day("recordDate")?, pay_date: item.opt_day("payableDate")?, cash, reinvested: units, currency });
        }
        Ok(Ok(out))
    };
    match read() {
        Ok(Ok(d)) => Outcome::Answered(d),
        Ok(Err(why)) => Outcome::Meaning(why),
        Err(m) => Outcome::Mismatch(m),
    }
}

fn gql(operation: &str, query: &str, port_ids: &[String]) -> Vec<u8> {
    let ids: Vec<String> = port_ids.iter().map(|i| format!("\"{i}\"")).collect();
    format!("{{\"operationName\":\"{operation}\",\"query\":\"{query}\",\"variables\":{{\"portIds\":[{}]}}}}", ids.join(",")).into_bytes()
}

pub struct VanguardCanada;

impl Payer for VanguardCanada {
    fn source(&self) -> SourceName {
        SourceName::named(SOURCE)
    }

    fn host(&self) -> &'static str {
        HOST
    }

    fn brands(&self) -> &'static [&'static str] {
        &["vanguard"]
    }

    fn markets(&self) -> &'static [crate::contract::Market] {
        crate::payers::CANADA
    }

    fn read(&self, net: &Net, need: &PayerNeed, _now: Timestamp) -> Noted<Record> {
        let fail = |o: Outcome<()>| Noted { outcome: o.failed().expect("not answered"), shape_change: None };
        let send = |ask: Ask| -> Result<bagholder_net::Reply, Outcome<()>> {
            match ask::send(net, &ask, &[]) {
                Outcome::Answered(r) => Ok(r),
                other => Err(other.failed().expect("not answered")),
            }
        };
        let page = match send(Ask::get(LIST, &HEADERS[2..])) {
            Ok(r) => r,
            Err(o) => return fail(o),
        };
        let Some(ids) = ask::text(&page.body).ok().and_then(port_ids) else {
            return Noted { outcome: Outcome::Mismatch(Mismatch { path: "site-config.portIds".into(), why: "the product list names no funds".into() }), shape_change: None };
        };
        let body = gql("Listings", LISTINGS, &ids);
        let listings = match send(Ask::post(GRAPHQL, &HEADERS, &body)) {
            Ok(r) => r,
            Err(o) => return fail(o),
        };
        let v = match ask::json(&listings.body) {
            Ok(v) => v,
            Err(m) => return Noted { outcome: Outcome::Mismatch(m), shape_change: None },
        };
        let ticker = venue::root(&need.listing.symbol);
        let (port_id, per_year) = match fund_of(&v, &ticker) {
            Ok(Some(f)) => f,
            Ok(None) => return Noted { outcome: Outcome::NotCarried(format!("Vanguard Canada lists no fund {ticker}")), shape_change: None },
            Err(m) => return Noted { outcome: Outcome::Mismatch(m), shape_change: None },
        };
        let body = gql("Distributions", DISTRIBUTIONS, std::slice::from_ref(&port_id));
        let history = match send(Ask::post(GRAPHQL, &HEADERS, &body)) {
            Ok(r) => r,
            Err(o) => return fail(o),
        };
        let outcome = match ask::json(&history.body) {
            Ok(v) => parse_distributions(&v, &port_id).map(|rows| Record { rows, per_year, by_record: vec![] }),
            Err(m) => Outcome::Mismatch(m),
        };
        Noted { outcome, shape_change: None }
    }
}
