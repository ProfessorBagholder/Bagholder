//! BMO Global Asset Management (research 2, 2026-09-24). Its pages answer only a
//! browser, but the data service they read (`df.bmogam.com/api/graphql/
//! etf-funds-production`) answers a plain request: for a fund's units (entity
//! `<TICKER>-a`), the series' ticker, currency and schedule
//! (`distributionFrequency`), and every distribution with its ex, record and pay
//! dates, its cash part, its reinvested part (null where none) and its total. A
//! total that is not the two parts is a meaning failure. An entity the service
//! does not know answers with no profile: not carried.

use bagholder_core::json::Value;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Currency, SourceName};
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::needs::PayerNeed;
use crate::outcome::{Noted, Outcome};
use crate::payers::{Distribution, Payer, Record};
use crate::reply::{Mismatch, Node};
use crate::venue;

pub const SOURCE: &str = "bmo";
pub const HOST: &str = "df.bmogam.com";
const URL: &str = "https://df.bmogam.com/api/graphql/etf-funds-production";
const HEADERS: [(&str, &str); 2] = [("Content-Type", "application/json"), ("User-Agent", "Mozilla/5.0")];
const QUERY: &str = r#"query Fund($locale: String, $entityId: String) { webProfiles(locale: $locale, entityId: $entityId, take: 1) { entityId webProfileSalesOptions { salesOption { currency { iso4217alpha } series { ticker distributionFrequency { name } } } } } monthlyDistributionBreakdowns(locale: $locale, entityId: $entityId, sortDirection: \"D\") { entityId exDate recordDate payDate cashPortion reinvestmentPortion distributionAmount } }"#;

pub fn per_year(word: &str) -> Option<u32> {
    Some(match word {
        "Monthly" => 12,
        "Quarterly" => 4,
        "Semi-Annually" | "Semi-Annual" => 2,
        "Annually" | "Annual" => 1,
        _ => return None,
    })
}

pub fn parse(v: &Value, ticker: &str) -> Outcome<Record> {
    let read = || -> Result<Result<Option<Record>, String>, Mismatch> {
        let data = Node::root(v).obj("data")?;
        let profiles = data.list("webProfiles")?;
        let Some(profile) = profiles.first() else { return Ok(Ok(None)) };
        let entity = format!("{ticker}-a");
        if profile.text("entityId")? != entity {
            return Ok(Err(format!("the service answered {} for {entity}", profile.text("entityId")?)));
        }
        let options = profile.list("webProfileSalesOptions")?;
        let Some(option) = options.first() else { return Err(profile.mismatch("no sales option")) };
        let sales = option.obj("salesOption")?;
        let currency = match Currency::parse(sales.obj("currency")?.text("iso4217alpha")?) {
            Ok(c) => c,
            Err(e) => return Ok(Err(format!("{ticker}'s currency: {e}"))),
        };
        let series = sales.obj("series")?;
        if !series.text("ticker")?.eq_ignore_ascii_case(ticker) {
            return Ok(Err(format!("the series is {}, not {ticker}", series.text("ticker")?)));
        }
        let word = series.obj("distributionFrequency")?.text("name")?;
        let per_year = Some(self::per_year(word).ok_or_else(|| series.mismatch(format!("{word:?} is not a schedule this reader knows")))?);
        let mut rows = Vec::new();
        for r in data.list("monthlyDistributionBreakdowns")? {
            if r.text("entityId")? != entity {
                return Ok(Err(format!("a distribution of {} in {entity}'s list", r.text("entityId")?)));
            }
            let (cash, units, total) = (r.dec("cashPortion")?, r.opt_dec("reinvestmentPortion")?, r.dec("distributionAmount")?);
            let sum = cash.checked_add(units.unwrap_or(bagholder_core::Dec::ZERO)).map_err(|e| r.mismatch(e.to_string()))?;
            let ex = r.day("exDate")?;
            if sum != total {
                return Ok(Err(format!("{ticker}'s distribution going ex {ex} totals {total}, its parts {sum}")));
            }
            rows.push(Distribution { ex_date: ex, record_date: r.opt_day("recordDate")?, pay_date: r.opt_day("payDate")?, cash, reinvested: units.filter(|u| !u.is_zero()), currency });
        }
        Ok(Ok(Some(Record { rows, per_year })))
    };
    match read() {
        Ok(Ok(Some(r))) => Outcome::Answered(r),
        Ok(Ok(None)) => Outcome::NotCarried(format!("BMO's service does not know {ticker}")),
        Ok(Err(why)) => Outcome::Meaning(why),
        Err(m) => Outcome::Mismatch(m),
    }
}

pub struct Bmo;

impl Payer for Bmo {
    fn source(&self) -> SourceName {
        SourceName::named(SOURCE)
    }

    fn host(&self) -> &'static str {
        HOST
    }

    fn brands(&self) -> &'static [&'static str] {
        &["bmo"]
    }

    fn markets(&self) -> &'static [crate::contract::Market] {
        crate::payers::CANADA
    }

    fn read(&self, net: &Net, need: &PayerNeed, _now: Timestamp) -> Noted<Record> {
        let ticker = venue::root(&need.listing.symbol);
        let body = format!("{{\"operationName\":\"Fund\",\"query\":\"{QUERY}\",\"variables\":{{\"locale\":\"en-US\",\"entityId\":\"{ticker}-a\"}}}}").into_bytes();
        let reply = match ask::send(net, &Ask::post(URL, &HEADERS, &body), &[]) {
            Outcome::Answered(r) => r,
            other => return Noted { outcome: other.failed().expect("not answered"), shape_change: None },
        };
        let outcome = match ask::json(&reply.body) {
            Ok(v) => parse(&v, &ticker),
            Err(m) => Outcome::Mismatch(m),
        };
        Noted { outcome, shape_change: None }
    }
}
