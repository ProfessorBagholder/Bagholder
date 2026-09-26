//! TMX Money's GraphQL service (`app-money.tmx.com/graphql`): the quote of a
//! Canadian listing, and the exchange's record of distributions: the market's
//! record for every Canadian payer no company reader serves (`payers::exchange`).
//!
//! The service wants its own site's origin and a locale. An unknown symbol
//! answers with an error naming code 404 and no data: not carried. A form of the
//! symbol answered on another venue than the one asked is another listing, and is
//! refused by the caller against the venue the book names.
//!
//! **Distributions.** TMX states each distribution's ex-date and amount per
//! unit, and its record, pay and declaration dates where it has them. It does
//! not say whether a distribution is paid in cash or in units: a year-end
//! distribution paid in units can be listed with or without a pay date, and a
//! cash payment can be listed without any of its dates. So each row's amount is
//! kept as TMX lists it, its form unstated, and the form is found from the
//! record (`payers::exchange`, `SPEC.md` §2, Distribution rate).

use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::Timestamp;
use bagholder_core::json::Value;
use bagholder_core::{Currency, Dec, SourceName};
use bagholder_net::{Ask, Net};

use crate::ask;
use crate::outcome::{Noted, Outcome};
use crate::reply::{Mismatch, Node, RecordedShape};

pub const SOURCE: &str = "tmx";
pub const HOST: &str = "app-money.tmx.com";
const URL: &str = "https://app-money.tmx.com/graphql";
const HEADERS: [(&str, &str); 5] = [("Content-Type", "application/json"), ("locale", "en"), ("Origin", "https://money.tmx.com"), ("Referer", "https://money.tmx.com/"), ("User-Agent", "Mozilla/5.0")];

const QUOTE: &str = "query getQuoteBySymbol($symbol: String, $locale: String) { getQuoteBySymbol(symbol: $symbol, locale: $locale) { symbol name exchangeName exchangeCode price priceChange percentChange prevClose currency datetime dividendFrequency } }";
const DIVIDENDS: &str = "query getDividendsForSymbol($symbol: String!, $page: Int, $batch: Int) { dividends: getDividendsForSymbol(symbol: $symbol, page: $page, batch: $batch) { dividends { exDate recordDate payableDate declarationDate amount currency } } }";
/// Rows per page of distributions; a full page means another is asked.
const BATCH: usize = 100;
/// The most pages asked: a century of monthly distributions.
const PAGES_MAX: usize = 12;

pub fn source() -> SourceName {
    SourceName::named(SOURCE)
}

pub fn quote_shape() -> RecordedShape {
    ask::recorded_shape(include_str!("../../shapes/tmx-quote.paths"))
}

pub fn dividends_shape() -> RecordedShape {
    ask::recorded_shape(include_str!("../../shapes/tmx-dividends.paths"))
}


/// A listing's quote as TMX states it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TmxQuote {
    pub symbol: String,
    /// The venue the answer is for, as TMX names it.
    pub exchange_name: String,
    pub price: Dec,
    pub change: Option<Dec>,
    pub change_pct: Option<Dec>,
    pub currency: Currency,
    /// When TMX served the quote, its only time: in session and out it equals the
    /// request's time within seconds. It says the price is current then, never
    /// when the last trade was.
    pub datetime: Timestamp,
    /// How often the listing pays, where TMX states it.
    pub per_year: Option<u32>,
}

/// A distribution as TMX lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TmxDistribution {
    pub ex_date: Date,
    pub record_date: Option<Date>,
    pub pay_date: Option<Date>,
    /// The amount per unit, as TMX lists it: TMX does not say whether it is paid
    /// in cash or in units.
    pub amount: Dec,
    pub currency: Currency,
}

/// TMX's words for how often a listing pays, and how many times a year each is.
/// A word not here is a mismatch naming it.
pub fn per_year(word: &str) -> Option<u32> {
    Some(match word {
        "Weekly" => 52,
        // "Bi-Monthly" is left out: it says every two months as often as twice a
        // month, and no reply has shown which TMX means
        "Semi-Monthly" => 24,
        "Monthly" => 12,
        "Quarterly" => 4,
        "Semi-Annual" | "Semi-Annually" => 2,
        "Annual" | "Annually" => 1,
        _ => return None,
    })
}

fn escaped(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A GraphQL request body: the operation, its variables (already JSON) and its query.
fn body(operation: &str, variables: &str, query: &str) -> Vec<u8> {
    format!("{{\"operationName\":{},\"variables\":{variables},\"query\":{}}}", escaped(operation), escaped(query)).into_bytes()
}

/// The `errors` of a reply that answers "not found" for the symbol.
fn not_found(root: &Node) -> bool {
    root.list("errors").is_ok_and(|errs| errs.iter().any(|e| e.text("code").is_ok_and(|c| c == "404")))
}

fn post(net: &Net, body: &[u8]) -> Result<Value, Outcome<()>> {
    let reply = match ask::send(net, &Ask::post(URL, &HEADERS, body), &[]) {
        Outcome::Answered(r) => r,
        other => return Err(other.failed().expect("not answered")),
    };
    ask::json(&reply.body).map_err(Outcome::Mismatch)
}

/// Read a quote reply for `form`.
pub fn parse_quote(v: &Value, form: &str) -> Outcome<TmxQuote> {
    let root = Node::root(v);
    if not_found(&root) {
        return Outcome::NotCarried(format!("TMX does not know {form}"));
    }
    match read_quote(&root, form) {
        // TMX answers a form with the listing it knows by that symbol, which can
        // be on another venue than the form asks: that listing is not this one
        Ok(Ok(q)) if !crate::venue::tmx_venue_matches(form.find(':').map_or("", |i| &form[i..]), &q.exchange_name) => {
            Outcome::NotCarried(format!("TMX answers {form} on {}, not the venue asked", q.exchange_name))
        }
        Ok(Ok(q)) => Outcome::Answered(q),
        Ok(Err(why)) => Outcome::Meaning(why),
        Err(m) => Outcome::Mismatch(m),
    }
}

fn read_quote(root: &Node, form: &str) -> Result<Result<TmxQuote, String>, Mismatch> {
    let q = root.obj("data")?.obj("getQuoteBySymbol")?;
    let symbol = q.text("symbol")?;
    let bare = form.split(':').next().unwrap_or(form);
    if !symbol.eq_ignore_ascii_case(bare) {
        return Ok(Err(format!("TMX answered {symbol} for {form}")));
    }
    let price = q.dec("price")?;
    if price <= Dec::ZERO {
        return Ok(Err(format!("{form}'s price is {price}")));
    }
    let currency = match Currency::parse(q.text("currency")?) {
        Ok(c) => c,
        Err(e) => return Ok(Err(format!("{form}'s currency: {e}"))),
    };
    let datetime_text = q.text("datetime")?;
    let Ok(datetime) = datetime_text.parse::<Timestamp>() else {
        return Ok(Err(format!("{form}'s time {datetime_text:?} is not an instant")));
    };
    let per_year = match q.opt_text("dividendFrequency")? {
        None => None,
        Some(w) => match self::per_year(w) {
            Some(n) => Some(n),
            None => return Err(q.field("dividendFrequency")?.mismatch(format!("{w:?} is not a schedule this reader knows"))),
        },
    };
    Ok(Ok(TmxQuote {
        symbol: symbol.to_string(),
        exchange_name: q.text("exchangeName")?.to_string(),
        price,
        change: q.opt_dec("priceChange")?,
        change_pct: q.opt_dec("percentChange")?,
        currency,
        datetime,
        per_year,
    }))
}

/// Read one page of a distributions reply.
pub fn parse_dividends(v: &Value, form: &str) -> Outcome<Vec<TmxDistribution>> {
    match read_dividends(v, form) {
        Ok(Ok(d)) => Outcome::Answered(d),
        Ok(Err(why)) => Outcome::Meaning(why),
        Err(m) => Outcome::Mismatch(m),
    }
}

fn read_dividends(v: &Value, form: &str) -> Result<Result<Vec<TmxDistribution>, String>, Mismatch> {
    let rows = Node::root(v).obj("data")?.obj("dividends")?.list("dividends")?;
    let mut out = Vec::new();
    for r in rows {
        let ex_date = r.day("exDate")?;
        let record_date = r.opt_day("recordDate")?;
        let pay_date = r.opt_day("payableDate")?;
        // a date or absent, read strictly; nothing a figure uses
        r.opt_day("declarationDate")?;
        let amount = r.dec("amount")?;
        if amount < Dec::ZERO {
            return Ok(Err(format!("{form} lists a distribution of {amount} going ex {ex_date}")));
        }
        if pay_date.is_some_and(|p| p < ex_date) {
            return Ok(Err(format!("{form} lists a distribution paid {} before it goes ex {ex_date}", pay_date.expect("checked"))));
        }
        let currency = match Currency::parse(r.text("currency")?) {
            Ok(c) => c,
            Err(e) => return Ok(Err(format!("{form}'s distribution currency: {e}"))),
        };
        out.push(TmxDistribution { ex_date, record_date, pay_date, amount, currency });
    }
    Ok(Ok(out))
}

/// Ask for `form`'s quote.
pub fn ask_quote(net: &Net, form: &str) -> Noted<TmxQuote> {
    let b = body("getQuoteBySymbol", &format!("{{\"symbol\":{},\"locale\":\"en\"}}", escaped(form)), QUOTE);
    match post(net, &b) {
        Ok(v) => Noted { outcome: parse_quote(&v, form), shape_change: ask::noticed(&v, &quote_shape(), &[]) },
        Err(o) => Noted { outcome: o.failed().unwrap_or(Outcome::Unreachable("no reply".into())), shape_change: None },
    }
}

/// Ask for every distribution TMX lists for `form`, page by page.
pub fn ask_dividends(net: &Net, form: &str) -> Noted<Vec<TmxDistribution>> {
    let mut all = Vec::new();
    let mut shape_change = None;
    for page in 1.. {
        let b = body("getDividendsForSymbol", &format!("{{\"symbol\":{},\"page\":{page},\"batch\":{BATCH}}}", escaped(form)), DIVIDENDS);
        let v = match post(net, &b) {
            Ok(v) => v,
            Err(o) => return Noted { outcome: o.failed().unwrap_or(Outcome::Unreachable("no reply".into())), shape_change },
        };
        shape_change = shape_change.or(ask::noticed(&v, &dividends_shape(), &[]));
        match parse_dividends(&v, form) {
            Outcome::Answered(rows) => {
                let full = rows.len() == BATCH;
                // a page that repeats the one before: TMX ignored the page asked,
                // and the pages would never end
                if page > 1 && rows.first() == all.get(all.len() - BATCH) {
                    return Noted { outcome: Outcome::Meaning(format!("TMX answered page {page} of {form}'s distributions with page {}", page - 1)), shape_change };
                }
                all.extend(rows);
                if !full {
                    break;
                }
                if page == PAGES_MAX {
                    return Noted { outcome: Outcome::Meaning(format!("{form} lists more than {} distributions", PAGES_MAX * BATCH)), shape_change };
                }
            }
            other => return Noted { outcome: other, shape_change },
        }
    }
    Noted { outcome: Outcome::Answered(all), shape_change }
}
