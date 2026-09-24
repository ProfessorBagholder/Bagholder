//! The Bank of Canada's rates, from Valet (`bankofcanada.ca/valet`), verified
//! against the live service on 2026-09-23 and 2026-09-24
//! (`docs/plans/stage-3a-sources.md`, "Bank of Canada"; research 3).
//!
//! Two series families, each its own source:
//! - the daily average (`FX<CUR>CAD`), from 2017-01-03 (the zloty's from
//!   2026-05-01), listed by the group `FX_RATES_DAILY`; a series the Bank has
//!   stopped is described as a "historical series";
//! - the noon rate (`LEGACY_NOON_RATES`), an archive from 2007-05-01 that ended on
//!   2017-04-28 and never changes, so which series is each currency's noon spot
//!   rate is the fixed table [`NOON`], written from the Bank's own metadata. A
//!   label does not name one series (`USD_NOON` is also a 90-day forward rate),
//!   so every read checks the reply's description against the table.
//!
//! A reply is read strictly: the series answered is the one asked, every day is
//! a real day inside the span asked, no day repeats, and every rate is a
//! positive decimal, exactly as written.

use bagholder_core::jiff::civil::{date, Date};
use bagholder_core::json::Value;
use bagholder_core::{Currency, Dec, SourceName};
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::outcome::{Noted, Outcome};
use crate::reply::{Keyed, Mismatch, Node, RecordedShape};

pub const DAILY: &str = "bank-of-canada";
pub const NOON_SOURCE: &str = "bank-of-canada-noon";
pub const HOST: &str = "www.bankofcanada.ca";
const VALET: &str = "https://www.bankofcanada.ca/valet";

/// Valet answers 404 for a series it does not publish.
const NOT_CARRIED: &[u16] = &[404];

pub fn daily_source() -> SourceName {
    SourceName::named(DAILY)
}

pub fn noon_source() -> SourceName {
    SourceName::named(NOON_SOURCE)
}

/// What Valet's replies carry, from the recorded ones (`tests/replies`).
pub fn group_shape() -> RecordedShape {
    ask::recorded_shape(include_str!("../../shapes/bank-of-canada-group.paths"))
}

/// The group's series are keyed by their codes.
pub const GROUP_KEYED: &[Keyed] = &[Keyed { parent: "groupDetails.groupSeries", fields: &[] }];
/// An observations reply is keyed by its series' code, in its detail and in each row.
pub const OBSERVATIONS_KEYED: &[Keyed] = &[Keyed { parent: "seriesDetail", fields: &[] }, Keyed { parent: "observations[]", fields: &["d"] }];

pub fn observations_shape() -> RecordedShape {
    ask::recorded_shape(include_str!("../../shapes/bank-of-canada-observations.paths"))
}

/// One daily series the Bank lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DailySeries {
    pub currency: Currency,
    pub code: String,
}

/// The daily series' code for a currency.
pub fn daily_code(c: Currency) -> String {
    format!("FX{}CAD", c.as_str())
}

/// One currency's noon spot series in the archive: its Valet code, the
/// description the Bank gives it, and the days it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoonSeries {
    pub currency: &'static str,
    pub code: &'static str,
    pub description: &'static str,
    pub first: Date,
    pub last: Date,
}

const fn noon(currency: &'static str, code: &'static str, description: &'static str, first: Date, last: Date) -> NoonSeries {
    NoonSeries { currency, code, description, first, last }
}

const START: Date = date(2007, 5, 1);
const END: Date = date(2017, 4, 28);

/// The archive's noon spot series, from the Bank's own metadata (research 3,
/// 2026-09-24). Where two series state one currency, each holds its own days:
/// the Argentine peso (local) until the plain series begins, the kyat (fixed)
/// until the floating one. The 90-day forward (`IEXE0105`) and the forward-point
/// spreads (`IEXE0124`, `IEXE0125`) are not spot rates and are not here.
pub const NOON: &[NoonSeries] = &[
    noon("ARS", "IEXE2702", "Argentine peso (local)", START, date(2010, 1, 1)),
    noon("ARS", "IEXE2703", "Argentine peso", date(2010, 1, 4), END),
    noon("AUD", "IEXE1601", "Australian dollar", START, END),
    noon("BSD", "IEXE6001", "Bahamian dollar", START, END),
    noon("BRL", "IEXE2801", "Brazilian real", START, END),
    noon("XAF", "IEXE4501", "CFA franc", START, END),
    noon("XPF", "IEXE4601", "CFP franc", START, END),
    noon("CLP", "IEXE2901", "Chilean peso", START, END),
    noon("CNY", "IEXE2201", "Chinese renminbi", START, END),
    noon("COP", "IEXE3901", "Colombian peso", START, END),
    noon("HRK", "IEXE6101", "Croatian kuna", START, END),
    noon("CZK", "IEXE2301", "Czech Republic koruna", START, END),
    noon("DKK", "IEXE0301", "Danish krone", START, END),
    noon("XCD", "IEXE4001", "East Caribbean dollar", START, END),
    noon("EUR", "EUROCAE01", "European Euro", START, END),
    noon("FJD", "IEXE4101", "Fiji dollar", START, END),
    noon("GHC", "IEXE4701", "Ghanaian cedi (old)", START, date(2007, 6, 29)),
    noon("GHS", "IEXE4702", "Ghanaian cedi", date(2007, 7, 3), END),
    noon("GTQ", "IEXE6501", "Guatemalan quetzal", START, END),
    noon("HNL", "IEXE4301", "Honduran lempira", START, END),
    noon("HKD", "IEXE1401", "Hong Kong dollar", START, END),
    noon("HUF", "IEXE2501", "Hungarian forint", START, END),
    noon("ISK", "IEXE4401", "Icelandic krona", START, END),
    noon("INR", "IEXE3001", "Indian rupee", START, END),
    noon("IDR", "IEXE2601", "Indonesian rupiah", START, END),
    noon("ILS", "IEXE5301", "Israeli new shekel", START, END),
    noon("JMD", "IEXE6401", "Jamaican dollar", START, END),
    noon("JPY", "IEXE0701", "Japanese yen", START, END),
    noon("MYR", "IEXE3201", "Malaysian ringgit", START, END),
    noon("MXN", "IEXE2001", "Mexican peso", START, END),
    noon("MAD", "IEXE4801", "Moroccan dirham", START, END),
    noon("MMK", "IEXE3801", "Myanmar (Burma) kyat (fixed)", START, date(2012, 3, 30)),
    noon("MMK", "IEXE3802", "Myanmar (Burma) kyat", date(2012, 4, 2), END),
    noon("ANG", "IEXE4901", "Neth. Antilles guilder", START, END),
    noon("NZD", "IEXE1901", "New Zealand dollar", START, END),
    noon("NOK", "IEXE0901", "Norwegian krone", START, END),
    noon("PKR", "IEXE5001", "Pakistan rupee", START, END),
    noon("PAB", "IEXE5101", "Panamanian balboa", START, END),
    noon("PEN", "IEXE5201", "Peruvian new sol", START, END),
    noon("PHP", "IEXE3301", "Philippine peso", START, END),
    noon("PLN", "IEXE2401", "Polish zloty", START, END),
    noon("RON", "IEXE6505", "Romanian new leu", date(2007, 9, 4), END),
    noon("RUB", "IEXE2101", "Russian ruble", START, END),
    noon("RSD", "IEXE6504", "Serbian dinar", date(2007, 9, 4), END),
    noon("SGD", "IEXE3701", "Singapore dollar", START, END),
    noon("SKK", "IEXE6201", "Slovak koruna", START, date(2008, 12, 31)),
    noon("ZAR", "IEXE3401", "South African rand", START, END),
    noon("KRW", "IEXE3101", "South Korean won", START, END),
    noon("LKR", "IEXE5501", "Sri Lanka rupee", START, END),
    noon("SEK", "IEXE1001", "Swedish krona", START, END),
    noon("CHF", "IEXE1101", "Swiss franc", START, END),
    noon("TWD", "IEXE3501", "Taiwanese new dollar", START, END),
    noon("THB", "IEXE3601", "Thai baht", START, END),
    noon("TTD", "IEXE5601", "Trinidad and Tobago dollar", START, END),
    noon("TND", "IEXE5701", "Tunisian dinar", START, END),
    noon("TRY", "IEXE5802", "Turkish lira", START, END),
    noon("AED", "IEXE6506", "U.A.E. dirham", date(2007, 9, 4), END),
    noon("VEB", "IEXE5901", "Venezuelan bolivar", START, date(2007, 12, 31)),
    noon("VEF", "IEXE5902", "Venezuelan bolivar fuerte", date(2008, 1, 2), END),
    noon("GBP", "IEXE1201", "U.K. pound sterling", START, END),
    noon("VND", "IEXE6503", "Vietnamese dong", START, END),
    noon("USD", "IEXE0101", "U.S. dollar (noon)", START, END),
];

/// The day the Bank's daily series began for the currencies it launched with, on
/// 2017-01-03: the noon archive stands until the day before. A daily series that
/// began later (the zloty's, 2026-05-01) began after the archive ended, so the
/// archive stands to its own last day.
pub const DAILY_LAUNCH: Date = date(2017, 1, 3);
pub const DAILY_AT_LAUNCH: [&str; 26] = [
    "AUD", "BRL", "CNY", "EUR", "HKD", "INR", "IDR", "JPY", "MYR", "MXN", "NZD", "NOK", "PEN", "RUB", "SAR", "SGD", "ZAR", "KRW", "SEK", "CHF", "TWD", "THB", "TRY", "GBP", "USD", "VND",
];

/// A currency's noon series, oldest first, each cut to the days the noon era
/// holds: to the day before the daily series began where that is sooner.
pub fn noon_series(c: Currency) -> Vec<NoonSeries> {
    let cutoff = if DAILY_AT_LAUNCH.contains(&c.as_str()) { DAILY_LAUNCH.yesterday().expect("a real day") } else { END };
    NOON.iter()
        .filter(|s| s.currency == c.as_str() && s.first <= cutoff)
        .map(|s| NoonSeries { last: s.last.min(cutoff), ..*s })
        .collect()
}

/// A span of observations one series answered, read strictly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observations {
    pub description: String,
    pub rates: Vec<(Date, Dec)>,
}

fn meaning<A>(why: String) -> Outcome<A> {
    Outcome::Meaning(why)
}

/// The group `FX_RATES_DAILY`: every currency the Bank publishes daily.
pub fn parse_daily_list(v: &Value) -> Outcome<Vec<DailySeries>> {
    match read_daily_list(v) {
        Ok(Ok(list)) => Outcome::Answered(list),
        Ok(Err(why)) => meaning(why),
        Err(m) => Outcome::Mismatch(m),
    }
}

fn read_daily_list(v: &Value) -> Result<Result<Vec<DailySeries>, String>, Mismatch> {
    let root = Node::root(v);
    let group = root.obj("groupDetails")?;
    let name = group.text("name")?;
    if name != "FX_RATES_DAILY" {
        return Ok(Err(format!("the group answered is {name}, not FX_RATES_DAILY")));
    }
    let series = group.obj("groupSeries")?;
    let mut out = Vec::new();
    for code in series.keys()? {
        let label = series.obj(code)?.text("label")?;
        let currency = code.strip_prefix("FX").and_then(|r| r.strip_suffix("CAD")).and_then(|c| Currency::parse(c).ok());
        match currency {
            Some(c) if label == format!("{}/CAD", c.as_str()) => out.push(DailySeries { currency: c, code: code.to_string() }),
            _ => return Ok(Err(format!("the group lists {code} labelled {label}, not a daily rate in Canadian dollars"))),
        }
    }
    if out.is_empty() {
        return Ok(Err("the group lists no series".into()));
    }
    Ok(Ok(out))
}

/// A series' observations, checked: the series answered is `code`; each day
/// is inside `span` where one was asked, and after the one before; each rate is
/// a positive decimal.
pub fn parse_observations(v: &Value, code: &str, span: Option<(Date, Date)>) -> Outcome<Observations> {
    match read_observations(v, code, span) {
        Ok(Ok(o)) => Outcome::Answered(o),
        Ok(Err(why)) => meaning(why),
        Err(m) => Outcome::Mismatch(m),
    }
}

fn read_observations(v: &Value, code: &str, span: Option<(Date, Date)>) -> Result<Result<Observations, String>, Mismatch> {
    let root = Node::root(v);
    let detail = root.obj("seriesDetail")?;
    let named = detail.keys()?;
    if named != [code] {
        return Ok(Err(format!("the reply names the series {}, not {code}", named.join(", "))));
    }
    let description = detail.obj(code)?.text("description")?.to_string();
    let mut rates: Vec<(Date, Dec)> = Vec::new();
    for row in root.list("observations")? {
        let d = row.day("d")?;
        let r = row.obj(code)?.dec_text("v")?;
        if let Some((from, to)) = span {
            if d < from || d > to {
                return Ok(Err(format!("{code} answered {d}, outside the {from} to {to} asked")));
            }
        }
        if let Some((last, _)) = rates.last() {
            if d <= *last {
                return Ok(Err(format!("{code} answered {d} after {last}: a day repeated or out of order")));
            }
        }
        if r <= Dec::ZERO {
            return Ok(Err(format!("{code} answered {r} for {d}: a rate is positive")));
        }
        rates.push((d, r));
    }
    Ok(Ok(Observations { description, rates }))
}

/// Whether a daily series' description is the Bank's for a daily average, and
/// whether it says the series is historical (stopped).
pub fn daily_description(description: &str) -> Result<bool, String> {
    if !description.starts_with("Daily average exchange rate") {
        return Err(format!("the series is described as {description:?}, not a daily average exchange rate"));
    }
    Ok(description.contains("historical series"))
}

fn observations_url(code: &str, span: Option<(Date, Date)>) -> String {
    match span {
        Some((from, to)) => format!("{VALET}/observations/{code}/json?start_date={from}&end_date={to}"),
        None => format!("{VALET}/observations/{code}/json"),
    }
}

fn noted<A>(outcome: Outcome<A>, shape_change: Option<crate::reply::ShapeChange>) -> Noted<A> {
    Noted { outcome, shape_change }
}

/// Ask Valet for the group of daily series.
pub fn ask_daily_list(net: &Net) -> Noted<Vec<DailySeries>> {
    let url = format!("{VALET}/groups/FX_RATES_DAILY/json");
    let reply = match ask::send(net, &Ask::get(&url, &[]), NOT_CARRIED) {
        Outcome::Answered(r) => r,
        other => return noted(other.failed().expect("not answered"), None),
    };
    let v = match ask::json(&reply.body) {
        Ok(v) => v,
        Err(m) => return noted(Outcome::Mismatch(m), None),
    };
    noted(parse_daily_list(&v), ask::noticed(&v, &group_shape(), GROUP_KEYED))
}

/// Ask Valet for one series' observations over `span`, or its whole history.
pub fn ask_observations(net: &Net, code: &str, span: Option<(Date, Date)>) -> Noted<Observations> {
    let url = observations_url(code, span);
    let reply = match ask::send(net, &Ask::get(&url, &[]), NOT_CARRIED) {
        Outcome::Answered(r) => r,
        other => return noted(other.failed().expect("not answered"), None),
    };
    let v = match ask::json(&reply.body) {
        Ok(v) => v,
        Err(m) => return noted(Outcome::Mismatch(m), None),
    };
    noted(parse_observations(&v, code, span), ask::noticed(&v, &observations_shape(), OBSERVATIONS_KEYED))
}
