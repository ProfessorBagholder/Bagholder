//! Fidelity Canada (research 2, 2026-09-24): its fund data file
//! (`www.fidelity.ca/content/fidelity-data/pnp-cached-en.json`, every fund)
//! states each series' codes and schedule (`distribution_frequency`); its
//! history service (`fidcaapi.fidelity.ca/FidcaAPI/api/fund/distributions/
//! history/EN/<code>`) lists the cash distributions a series paid. It answers
//! an empty body for a series that has paid none (FBTC, which has paid none in
//! cash). Where it lists some, each carries one date and its template names no
//! ex-date, so no ex-date can be read from it: that is a mismatch naming what
//! is missing, never an ex-date taken to be that one date.

use bagholder_core::json::Value;
use bagholder_core::jiff::Timestamp;
use bagholder_core::SourceName;
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::needs::PayerNeed;
use crate::outcome::{Noted, Outcome};
use crate::payers::{Payer, Record};
use crate::reply::{self, Mismatch, Node};
use crate::venue;

pub const SOURCE: &str = "fidelity-canada";
pub const HOST: &str = "www.fidelity.ca";
const FUNDS: &str = "https://www.fidelity.ca/content/fidelity-data/pnp-cached-en.json";
const HISTORY: &str = "https://fidcaapi.fidelity.ca/FidcaAPI/api/fund/distributions/history/EN/";
const HEADERS: [(&str, &str); 1] = [("User-Agent", "Mozilla/5.0")];

pub fn per_year(word: &str) -> Option<u32> {
    Some(match word {
        "Monthly" => 12,
        "Quarterly" => 4,
        "Semi-Annually" => 2,
        "Annually" => 1,
        _ => return None,
    })
}

/// The series whose code is the ticker: its stated schedule.
pub fn schedule_of(v: &Value, ticker: &str) -> Result<Option<Option<u32>>, Mismatch> {
    for fund in Node::root(v).list("funds")? {
        for series in fund.list("series")? {
            let codes: Vec<String> = series.list("codes")?.iter().map(|c| c.text("code").map(str::to_string)).collect::<Result<_, _>>()?;
            if codes.iter().any(|c| c.eq_ignore_ascii_case(ticker)) {
                let word = series.text("distribution_frequency")?;
                return match per_year(word) {
                    Some(n) => Ok(Some(Some(n))),
                    None if word == "-" => Ok(Some(None)),
                    None => Err(series.mismatch(format!("{word:?} is not a schedule this reader knows"))),
                };
            }
        }
    }
    Ok(None)
}

/// The history service's answer: none paid, or a list this reader cannot place.
pub fn parse_history(body: &[u8], ticker: &str) -> Outcome<()> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Outcome::Answered(());
    }
    let listed = ask::text(body).and_then(reply::parse).and_then(|v| {
        let d = Node::root(&v).obj("distributions")?;
        Ok(!d.keys()?.is_empty())
    });
    match listed {
        Ok(false) => Outcome::Answered(()),
        Ok(true) => Outcome::Mismatch(Mismatch { path: "distributions".into(), why: format!("{ticker}'s distributions carry one date each and no ex-date") }),
        Err(m) => Outcome::Mismatch(m),
    }
}

pub struct FidelityCanada;

impl Payer for FidelityCanada {
    fn source(&self) -> SourceName {
        SourceName::named(SOURCE)
    }

    fn host(&self) -> &'static str {
        HOST
    }

    fn brands(&self) -> &'static [&'static str] {
        &["fidelity"]
    }

    fn markets(&self) -> &'static [crate::contract::Market] {
        crate::payers::CANADA
    }

    fn read(&self, net: &Net, need: &PayerNeed, _now: Timestamp) -> Noted<Record> {
        let get = |url: &str| -> Result<bagholder_net::Reply, Outcome<()>> {
            match ask::send(net, &Ask::get(url, &HEADERS), &[]) {
                Outcome::Answered(r) => Ok(r),
                other => Err(other.failed().expect("not answered")),
            }
        };
        let fail = |o: Outcome<()>| Noted { outcome: o.failed().expect("not answered"), shape_change: None };
        let funds = match get(FUNDS) {
            Ok(r) => r,
            Err(o) => return fail(o),
        };
        let ticker = venue::root(&need.listing.symbol);
        let per_year = match ask::json(&funds.body).and_then(|v| schedule_of(&v, &ticker)) {
            Ok(Some(p)) => p,
            Ok(None) => return Noted { outcome: Outcome::NotCarried(format!("Fidelity lists no series {ticker}")), shape_change: None },
            Err(m) => return Noted { outcome: Outcome::Mismatch(m), shape_change: None },
        };
        let history = match get(&format!("{HISTORY}{ticker}")) {
            Ok(r) => r,
            Err(o) => return fail(o),
        };
        let outcome = parse_history(&history.body, &ticker).map(|()| Record { rows: vec![], per_year });
        Noted { outcome, shape_change: None }
    }
}
