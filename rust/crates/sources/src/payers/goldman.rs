//! Goldman Sachs Asset Management (research 2, 2026-09-24): its funds service
//! (`am.gs.com/services/funds`, GraphQL) lists every fund's share classes with
//! their ticker, schedule (`distributionFrequency`) and currency; a fund's detail
//! (by its `pvNumber` and the class's id) lists its distributions: `amount`, the
//! ex-date (`expirationDate`, the site's "Ex-Date" column), `recordDate`,
//! `payableDate`, dates MM/DD/YYYY, amounts as text, `--` for none. A row with
//! no amount is a date the fund states nothing paid on. The fund once called
//! GLOV is GSWO under the same fund.

use bagholder_core::jiff::civil::Date;
use bagholder_core::json::Value;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, Dec, SourceName};
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::needs::PayerNeed;
use crate::outcome::{Noted, Outcome};
use crate::payers::{Distribution, Payer, Record};
use crate::reply::{Mismatch, Node};
use crate::venue;

pub const SOURCE: &str = "goldman-sachs";
pub const HOST: &str = "am.gs.com";
const URL: &str = "https://am.gs.com/services/funds";
const HEADERS: [(&str, &str); 2] = [("Content-Type", "application/json"), ("User-Agent", "Mozilla/5.0")];
const FUNDS: &str = "query Funds($fundRequest: FundRequest) { fundData(fundRequest: $fundRequest) { funds { pvNumber fundType shareClasses { shareClassId ticker distributionFrequency baseCurrency } } } }";
const FUND: &str = "query Fund($fundDetailRequest: FundDetailRequest) { fundsDetail(fundDetailRequest: $fundDetailRequest) { ticker baseCurrency shareClasses { shareClassId ticker distributionFrequency } distributions } }";

pub fn per_year(word: &str) -> Option<u32> {
    Some(match word {
        "Monthly" => 12,
        "Quarterly" => 4,
        "Semi-Annually" => 2,
        "Annually" => 1,
        _ => return None,
    })
}

/// A share class by its ticker: the fund's number, the class's id, its schedule
/// and currency.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Class {
    pub pv_number: String,
    pub share_class: String,
    pub per_year: Option<u32>,
    pub currency: Currency,
}

pub fn class_of(v: &Value, ticker: &str) -> Result<Option<Class>, Mismatch> {
    for fund in Node::root(v).obj("data")?.obj("fundData")?.list("funds")? {
        for c in fund.list("shareClasses")? {
            if c.opt_text("ticker")?.is_some_and(|t| t.eq_ignore_ascii_case(ticker)) {
                let per_year = match c.opt_text("distributionFrequency")? {
                    None => None,
                    Some(w) => Some(per_year(w).ok_or_else(|| c.mismatch(format!("{w:?} is not a schedule this reader knows")))?),
                };
                let cur = c.text("baseCurrency")?;
                let currency = Currency::parse(cur).map_err(|e| c.mismatch(e.to_string()))?;
                return Ok(Some(Class { pv_number: fund.text("pvNumber")?.to_string(), share_class: c.text("shareClassId")?.to_string(), per_year, currency }));
            }
        }
    }
    Ok(None)
}

fn mdy(n: &Node, key: &str) -> Result<Date, Mismatch> {
    let s = n.text(key)?;
    let mut it = s.split('/');
    let parsed = (|| {
        let (m, d, y) = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
        Date::new(y, m, d).ok()
    })();
    parsed.ok_or_else(|| n.field(key).map_or_else(|m| m, |f| f.mismatch(format!("{s:?} is not a day"))))
}

pub fn parse_detail(v: &Value, ticker: &str, currency: Currency) -> Result<Vec<Distribution>, Mismatch> {
    let detail = Node::root(v).obj("data")?.obj("fundsDetail")?;
    let answered = detail.text("ticker")?;
    if !answered.eq_ignore_ascii_case(ticker) {
        return Err(detail.mismatch(format!("the detail is {answered}'s, not {ticker}'s")));
    }
    let mut rows = Vec::new();
    for r in detail.list("distributions")? {
        let amount = r.text("amount")?;
        if amount == "--" {
            continue;
        }
        let cash = Dec::parse(amount).map_err(|e| r.mismatch(format!("{amount:?}: {e}")))?;
        rows.push(Distribution { ex_date: mdy(&r, "expirationDate")?, record_date: Some(mdy(&r, "recordDate")?), pay_date: Some(mdy(&r, "payableDate")?), cash, reinvested: None, currency });
    }
    Ok(rows)
}

fn body(operation: &str, query: &str, variables: &str) -> Vec<u8> {
    format!("{{\"operationName\":\"{operation}\",\"query\":\"{query}\",\"variables\":{variables}}}").into_bytes()
}

pub struct GoldmanSachs;

impl Payer for GoldmanSachs {
    fn source(&self) -> SourceName {
        SourceName::named(SOURCE)
    }

    fn host(&self) -> &'static str {
        HOST
    }

    fn brands(&self) -> &'static [&'static str] {
        &["goldman sachs"]
    }

    fn markets(&self) -> &'static [crate::contract::Market] {
        crate::payers::US
    }

    fn read(&self, net: &Net, need: &PayerNeed, _now: Timestamp) -> Noted<Record> {
        let post = |b: Vec<u8>| -> Result<Value, Outcome<()>> {
            match ask::send(net, &Ask::post(URL, &HEADERS, &b), &[]) {
                Outcome::Answered(r) => ask::json(&r.body).map_err(Outcome::Mismatch),
                other => Err(other.failed().expect("not answered")),
            }
        };
        let fail = |o: Outcome<()>| Noted { outcome: o.failed().expect("not answered"), shape_change: None };
        let request = r#"{"fundRequest":{"country":"us","language":"en","audience":"advisors","offset":0,"limit":2000,"sortBy":"FN","sortOrder":"ASC"}}"#;
        let ticker = venue::root(&need.listing.symbol);
        let class = match post(body("Funds", FUNDS, request)).and_then(|v| class_of(&v, &ticker).map_err(Outcome::Mismatch)) {
            Ok(Some(c)) => c,
            Ok(None) => return Noted { outcome: Outcome::NotCarried(format!("Goldman Sachs lists no class {ticker}")), shape_change: None },
            Err(o) => return fail(o),
        };
        let detail = format!(r#"{{"fundDetailRequest":{{"country":"us","language":"en","audience":"advisors","pvNumber":"{}","shareClassId":"{}"}}}}"#, class.pv_number, class.share_class);
        let outcome = match post(body("Fund", FUND, &detail)) {
            Ok(v) => match parse_detail(&v, &ticker, class.currency) {
                Ok(rows) => Outcome::Answered(Record { rows, per_year: class.per_year }),
                Err(m) => Outcome::Mismatch(m),
            },
            Err(o) => o.failed().expect("not answered"),
        };
        Noted { outcome, shape_change: None }
    }
}
