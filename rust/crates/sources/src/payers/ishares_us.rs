//! iShares in the US (research 2, 2026-09-24): the product screener
//! (`ishares.com/us/product-screener/product-screener-v3.jsn`, a table of
//! columns and rows) maps a ticker to its fund's page. A wrong id serves another
//! fund's page, so the page is only ever the one the screener names. The page
//! states the schedule (`"name":"Distribution Frequency","value":"Semi-Annual"`
//! in its structured data) and carries the fund's whole history in its
//! `DistributionV3` component's properties: columns of record, ex and pay dates
//! (`20260615`) and of amounts (`totalDistribution`, with its parts), all paid
//! in cash, in US dollars.

use bagholder_core::jiff::civil::Date;
use bagholder_core::json::Value;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, Dec, SourceName};
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::html::unescape;
use crate::needs::PayerNeed;
use crate::outcome::{Noted, Outcome};
use crate::payers::{Distribution, Payer, Record};
use crate::reply::{self, Mismatch, Node};
use crate::venue;

pub const SOURCE: &str = "ishares-us";
pub const HOST: &str = "www.ishares.com";
const SCREENER: &str = "https://www.ishares.com/us/product-screener/product-screener-v3.jsn?dcrPath=/templatedata/config/product-screener-v3/data/en/us-ishares/ishares-product-screener-backend-config&siteEntryPassthrough=true";
const HEADERS: [(&str, &str); 1] = [("User-Agent", "Mozilla/5.0")];

pub fn per_year(word: &str) -> Option<u32> {
    Some(match word {
        "Monthly" => 12,
        "Quarterly" => 4,
        "Semi-Annual" => 2,
        "Annual" => 1,
        _ => return None,
    })
}

/// The fund's page path, by its ticker.
pub fn page_of(v: &Value, ticker: &str) -> Result<Option<String>, Mismatch> {
    let table = Node::root(v).obj("data")?.obj("tableData")?;
    let names: Vec<String> = table.list("columns")?.iter().map(|c| c.text("name").map(str::to_string)).collect::<Result<_, _>>()?;
    let col = |name: &str| names.iter().position(|n| n == name).ok_or_else(|| table.mismatch(format!("no column {name}")));
    let (t, url) = (col("localExchangeTicker")?, col("productPageUrl")?);
    for row in table.list("data")? {
        let cells = row.as_list()?;
        let text = |i: usize| cells.get(i).ok_or_else(|| row.mismatch(format!("no cell {i}"))).and_then(|c| c.as_text());
        if text(t)?.eq_ignore_ascii_case(ticker) {
            return Ok(Some(text(url)?.to_string()));
        }
    }
    Ok(None)
}

/// The schedule the page's structured data states.
pub fn schedule_word(html: &str) -> Option<String> {
    let key = "\"name\":\"Distribution Frequency\",\"value\":\"";
    let i = html.find(key)? + key.len();
    let end = html[i..].find('"')? + i;
    Some(html[i..end].to_string())
}

/// The `DistributionV3` component's properties.
pub fn component(html: &str) -> Result<Value, Mismatch> {
    let missing = || Mismatch { path: "[componentkey=DistributionV3]".into(), why: "the page carries no distribution component".into() };
    let k = html.find("componentkey=\"DistributionV3\"").ok_or_else(missing)?;
    let open = html[..k].rfind('<').ok_or_else(missing)?;
    let close = html[k..].find('>').ok_or_else(missing)? + k;
    let tag = &html[open..close];
    let p = tag.find("componentprops=\"").ok_or_else(missing)? + "componentprops=\"".len();
    let end = tag[p..].find('"').ok_or_else(missing)? + p;
    reply::parse(&unescape(&tag[p..end]))
}

fn yyyymmdd(n: &Node) -> Result<Date, Mismatch> {
    let raw = n.as_int()?;
    Date::new((raw / 10000) as i16, (raw / 100 % 100) as i8, (raw % 100) as i8).map_err(|_| n.mismatch(format!("{raw} is not a day")))
}

pub fn parse_component(v: &Value) -> Result<Vec<Distribution>, Mismatch> {
    let columns = Node::root(v).list("distributionTableData")?;
    let column = |name: &str| -> Result<Vec<Node<'_>>, Mismatch> {
        for c in &columns {
            if c.text("name")? == name {
                return c.list("value");
            }
        }
        Err(Node::root(v).mismatch(format!("no column {name}")))
    };
    let (ex, record, pay, total) = (column("exDate")?, column("recordDate")?, column("payableDate")?, column("totalDistribution")?);
    if [record.len(), pay.len(), total.len()].iter().any(|n| *n != ex.len()) {
        return Err(Node::root(v).mismatch("its columns are of different lengths"));
    }
    let mut rows = Vec::new();
    for i in 0..ex.len() {
        let amount = total[i].as_text()?;
        let cash = Dec::parse(amount).map_err(|e| total[i].mismatch(format!("{amount:?}: {e}")))?;
        rows.push(Distribution { ex_date: yyyymmdd(&ex[i])?, record_date: Some(yyyymmdd(&record[i])?), pay_date: Some(yyyymmdd(&pay[i])?), cash, reinvested: None, currency: Currency::USD });
    }
    Ok(rows)
}

pub struct ISharesUs;

impl Payer for ISharesUs {
    fn source(&self) -> SourceName {
        SourceName::named(SOURCE)
    }

    fn host(&self) -> &'static str {
        HOST
    }

    fn brands(&self) -> &'static [&'static str] {
        &["ishares"]
    }

    fn markets(&self) -> &'static [crate::contract::Market] {
        crate::payers::US
    }

    fn read(&self, net: &Net, need: &PayerNeed, _now: Timestamp) -> Noted<Record> {
        let get = |url: &str| -> Result<String, Outcome<()>> {
            match ask::send(net, &Ask::get(url, &HEADERS), &[]) {
                Outcome::Answered(r) => ask::text(&r.body).map(|t| t.strip_prefix('\u{feff}').unwrap_or(t).to_string()).map_err(Outcome::Mismatch),
                other => Err(other.failed().expect("not answered")),
            }
        };
        let fail = |o: Outcome<()>| Noted { outcome: o.failed().expect("not answered"), shape_change: None };
        let mismatch = |m: Mismatch| Noted { outcome: Outcome::Mismatch(m), shape_change: None };
        let ticker = venue::root(&need.listing.symbol);
        let path = match get(SCREENER).and_then(|t| reply::parse(&t).map_err(Outcome::Mismatch)).and_then(|v| page_of(&v, &ticker).map_err(Outcome::Mismatch)) {
            Ok(Some(p)) => p,
            Ok(None) => return Noted { outcome: Outcome::NotCarried(format!("iShares lists no US fund {ticker}")), shape_change: None },
            Err(o) => return fail(o),
        };
        let page = match get(&format!("https://{HOST}{path}")) {
            Ok(p) => p,
            Err(o) => return fail(o),
        };
        let per_year = match schedule_word(&page) {
            Some(w) => match per_year(&w) {
                Some(n) => Some(n),
                None => return mismatch(Mismatch { path: "Distribution Frequency".into(), why: format!("{w:?} is not a schedule this reader knows") }),
            },
            None => return mismatch(Mismatch { path: "Distribution Frequency".into(), why: "the page states no schedule".into() }),
        };
        let outcome = match component(&page).and_then(|v| parse_component(&v)) {
            Ok(rows) => Outcome::Answered(Record { form: bagholder_core::distribution::Form::Stated, rows, per_year, by_record: vec![] }),
            Err(m) => Outcome::Mismatch(m),
        };
        Noted { outcome, shape_change: None }
    }
}
