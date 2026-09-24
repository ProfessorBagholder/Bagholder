//! Purpose Investments (research 2, 2026-09-24): the fund's page
//! (`purposeinvest.com/funds/<ticker>`, redirected to the fund's own path)
//! carries its data in the page itself (`__NEXT_DATA__`, `props.pageProps.
//! fundData`): each series by its code, with its currency; per series, its
//! details (`distribution_frequency`) and its distributions (`amount`, `date_ex`,
//! `date_rec`, `date_pay`, `dvd_type`), oldest first. The series whose code is
//! the ticker is the listing's.

use bagholder_core::json::Value;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, SourceName};
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::needs::PayerNeed;
use crate::outcome::{Noted, Outcome};
use crate::payers::{Distribution, Payer, Record};
use crate::reply::{self, Mismatch, Node};
use crate::venue;

pub const SOURCE: &str = "purpose";
pub const HOST: &str = "www.purposeinvest.com";

pub fn per_year(word: &str) -> Option<u32> {
    Some(match word {
        "Monthly" => 12,
        "Quarterly" => 4,
        "Semi-Annually" => 2,
        "Annually" => 1,
        _ => return None,
    })
}

/// The JSON the page carries in its `__NEXT_DATA__` script.
pub fn page_data(html: &str) -> Result<Value, Mismatch> {
    let missing = || Mismatch { path: "script#__NEXT_DATA__".into(), why: "the page carries no data".into() };
    let open = html.find("<script id=\"__NEXT_DATA__\"").ok_or_else(missing)?;
    let start = html[open..].find('>').ok_or_else(missing)? + open + 1;
    let end = html[start..].find("</script>").ok_or_else(missing)? + start;
    reply::parse(&html[start..end])
}

/// The listing's series: its schedule and its distributions.
pub fn parse(v: &Value, ticker: &str) -> Outcome<Record> {
    let read = || -> Result<Result<Option<Record>, String>, Mismatch> {
        let fund = Node::root(v).obj("props")?.obj("pageProps")?.obj("fundData")?;
        let series = fund.obj("series")?;
        let mut found = None;
        for key in series.keys()? {
            let s = series.obj(key)?;
            if s.text("code")?.eq_ignore_ascii_case(ticker) {
                found = Some((key.to_string(), s.text("currency")?.to_string()));
            }
        }
        let Some((key, currency)) = found else { return Ok(Ok(None)) };
        let currency = match Currency::parse(&currency) {
            Ok(c) => c,
            Err(e) => return Ok(Err(format!("{ticker}'s currency: {e}"))),
        };
        let mut per_year = None;
        for d in fund.obj("details")?.list(&key)? {
            if d.text("name")? == "distribution_frequency" {
                let w = d.text("val")?;
                per_year = Some(self::per_year(w).ok_or_else(|| d.mismatch(format!("{w:?} is not a schedule this reader knows")))?);
            }
        }
        let mut rows = Vec::new();
        for r in fund.obj("distributions")?.list(&key)? {
            let kind = r.text("dvd_type")?;
            if kind != "Regular" {
                return Err(r.field("dvd_type")?.mismatch(format!("{kind:?} is not a kind of distribution this reader knows")));
            }
            rows.push(Distribution { ex_date: r.day("date_ex")?, record_date: r.opt_day("date_rec")?, pay_date: r.opt_day("date_pay")?, cash: r.dec("amount")?, reinvested: None, currency });
        }
        Ok(Ok(Some(Record { rows, per_year })))
    };
    match read() {
        Ok(Ok(Some(r))) => Outcome::Answered(r),
        Ok(Ok(None)) => Outcome::NotCarried(format!("Purpose's page has no series {ticker}")),
        Ok(Err(why)) => Outcome::Meaning(why),
        Err(m) => Outcome::Mismatch(m),
    }
}

pub struct Purpose;

impl Payer for Purpose {
    fn source(&self) -> SourceName {
        SourceName::named(SOURCE)
    }

    fn host(&self) -> &'static str {
        HOST
    }

    fn brands(&self) -> &'static [&'static str] {
        &["purpose"]
    }

    fn markets(&self) -> &'static [crate::contract::Market] {
        crate::payers::CANADA
    }

    fn read(&self, net: &Net, need: &PayerNeed, _now: Timestamp) -> Noted<Record> {
        let ticker = venue::root(&need.listing.symbol);
        let url = format!("https://{HOST}/funds/{}", ticker.to_ascii_lowercase());
        let page = match ask::send(net, &Ask::get(&url, &[]), &[404]) {
            Outcome::Answered(r) => r,
            other => return Noted { outcome: other.failed().expect("not answered"), shape_change: None },
        };
        let outcome = match ask::text(&page.body).and_then(page_data) {
            Ok(v) => parse(&v, &ticker),
            Err(m) => Outcome::Mismatch(m),
        };
        Noted { outcome, shape_change: None }
    }
}
