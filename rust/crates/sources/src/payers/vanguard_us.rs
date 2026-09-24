//! Vanguard in the US (research 2, 2026-09-24): its advisor site's API
//! (`advisors.vanguard.com/investments/products/api/funds/…`) maps a ticker to
//! the fund's id (`<ticker>/validate`, 404 for one it does not have), states the
//! schedule (`<id>/profile`, `distributionFrequency`) and lists ten years of
//! distributions (`<id>/pricing/distributions`): each with its type (`INC`
//! income, `CGST` and `CGLT` short- and long-term capital gains, `ROC` a return
//! of capital, all paid in cash), amount, ex, record and pay dates. The parts of
//! one ex-date are one distribution. It states no currency: a US fund's
//! distributions are in US dollars.

use std::collections::BTreeMap;

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

pub const SOURCE: &str = "vanguard-us";
pub const HOST: &str = "advisors.vanguard.com";
const API: &str = "https://advisors.vanguard.com/investments/products/api/funds/";
const HEADERS: [(&str, &str); 1] = [("User-Agent", "Mozilla/5.0")];

pub fn per_year(word: &str) -> Option<u32> {
    Some(match word {
        "Monthly" => 12,
        "Quarterly" => 4,
        "Semi-annually" | "Semi-Annually" => 2,
        "Annually" => 1,
        _ => return None,
    })
}

pub fn port_id(v: &Value) -> Result<String, Mismatch> {
    Ok(Node::root(v).text("portId")?.to_string())
}

pub fn schedule(v: &Value) -> Result<Option<u32>, Mismatch> {
    let root = Node::root(v);
    match root.opt_text("distributionFrequency")? {
        None => Ok(None),
        Some(w) => per_year(w).map(Some).ok_or_else(|| root.mismatch(format!("{w:?} is not a schedule this reader knows"))),
    }
}

pub fn parse_distributions(v: &Value) -> Result<Vec<Distribution>, Mismatch> {
    let mut by_ex: BTreeMap<Date, Distribution> = BTreeMap::new();
    for r in Node::root(v).as_list()? {
        let code = r.text("typeCode")?;
        if !matches!(code, "INC" | "CGST" | "CGLT" | "ROC") {
            return Err(r.field("typeCode")?.mismatch(format!("{code:?} is not a kind of distribution this reader knows")));
        }
        let ex = r.day("exDividendDate")?;
        let amount = r.dec("amount")?;
        let row = by_ex.entry(ex).or_insert(Distribution { ex_date: ex, record_date: r.opt_day("recordDate")?, pay_date: r.opt_day("payableDate")?, cash: Dec::ZERO, reinvested: None, currency: Currency::USD });
        row.cash = row.cash.checked_add(amount).map_err(|e| r.mismatch(e.to_string()))?;
    }
    Ok(by_ex.into_values().collect())
}

pub struct VanguardUs;

impl Payer for VanguardUs {
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
        crate::payers::US
    }

    fn read(&self, net: &Net, need: &PayerNeed, _now: Timestamp) -> Noted<Record> {
        let get = |path: &str, not_carried: &[u16]| -> Result<Value, Outcome<()>> {
            match ask::send(net, &Ask::get(&format!("{API}{path}"), &HEADERS), not_carried) {
                Outcome::Answered(r) => ask::json(&r.body).map_err(Outcome::Mismatch),
                other => Err(other.failed().expect("not answered")),
            }
        };
        let fail = |o: Outcome<()>| Noted { outcome: o.failed().expect("not answered"), shape_change: None };
        let ticker = venue::root(&need.listing.symbol).to_ascii_lowercase();
        let id = match get(&format!("{ticker}/validate"), &[404]).and_then(|v| port_id(&v).map_err(Outcome::Mismatch)) {
            Ok(id) => id,
            Err(o) => return fail(o),
        };
        let per_year = match get(&format!("{id}/profile"), &[]).and_then(|v| schedule(&v).map_err(Outcome::Mismatch)) {
            Ok(p) => p,
            Err(o) => return fail(o),
        };
        let outcome = match get(&format!("{id}/pricing/distributions?hasDistributionYield=true"), &[]) {
            Ok(v) => match parse_distributions(&v) {
                Ok(rows) => Outcome::Answered(Record { rows, per_year }),
                Err(m) => Outcome::Mismatch(m),
            },
            Err(o) => o.failed().expect("not answered"),
        };
        Noted { outcome, shape_change: None }
    }
}
