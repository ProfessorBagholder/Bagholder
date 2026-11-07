//! TMX Money's GraphQL service (`app-money.tmx.com/graphql`): the quote of a
//! Canadian listing, and the exchange's record of distributions: the market's
//! record for every Canadian payer no company reader serves (`payers::exchange`).
//!
//! The service wants its own site's origin and a locale. An unknown symbol
//! answers with an error naming code 404 and no data: not carried. A form of the
//! symbol answered on another venue than the one asked is another listing, and is
//! refused by the caller against the venue the book names.
//!
//! **Quotes.** Up to twenty listings are asked in one request, each its own
//! `getQuoteBySymbol` under an alias of its place (`q0`, `q1`, …), read as one
//! listing's quote. An alias TMX does not know answers null with an error naming
//! its alias and code 404: not carried. Any other error naming an alias fails
//! that listing alone; the others still answer.
//!
//! A halted listing is answered with a null price: TMX states it has
//! no price now, and the price read last stands. The schedule the quote states
//! (`dividendFrequency`) is read for the payer's record alone: a word this reader
//! does not know is that record's mismatch, and leaves the price standing. The
//! ex-dividend date the quote states (`exDividendDate`, written
//! `2026-08-14 00:00:00.0`, and `""` where TMX states none) is the Ex-Div of a
//! payer no declared record serves (`SPEC.md` §5, Cashflow Positions); a form
//! this reader does not know is its mismatch, and leaves the price standing too.
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
/// TMX's pace for every request to it, quotes included: the gap the heatmap's
/// and the news's readers already kept, set before anything asks it.
pub const PACE: bagholder_net::Pace = bagholder_net::Pace { gap: std::time::Duration::from_millis(600), rest: std::time::Duration::from_secs(60) };
const URL: &str = "https://app-money.tmx.com/graphql";
const HEADERS: [(&str, &str); 5] = [("Content-Type", "application/json"), ("locale", "en"), ("Origin", "https://money.tmx.com"), ("Referer", "https://money.tmx.com/"), ("User-Agent", "Mozilla/5.0")];

/// The fields one listing's quote is asked for.
const QUOTE_FIELDS: &str = "symbol name exchangeName exchangeCode price priceChange percentChange prevClose currency datetime dividendFrequency exDividendDate";
/// The most listings one quote request asks for. TMX states no limit: twenty,
/// the spark's, keeps each request's query near 4 KB and a pass's TMX requests at
/// one for every twenty Canadian listings.
pub const QUOTES_MOST: usize = 20;
const DIVIDENDS: &str = "query getDividendsForSymbol($symbol: String!, $page: Int, $batch: Int) { dividends: getDividendsForSymbol(symbol: $symbol, page: $page, batch: $batch) { dividends { exDate recordDate payableDate declarationDate amount currency } } }";
/// Rows per page of distributions; a full page means another is asked.
const BATCH: usize = 100;
/// The most pages asked: a century of monthly distributions.
const PAGES_MAX: usize = 12;

pub fn source() -> SourceName {
    SourceName::named(SOURCE)
}

pub fn quotes_shape() -> RecordedShape {
    ask::recorded_shape(include_str!("../../shapes/tmx-quotes.paths"))
}

/// A quote reply's `data` is keyed by the aliases asked.
pub const QUOTES_KEYED: &[crate::reply::Keyed] = &[crate::reply::Keyed { parent: "data", fields: &[] }];

pub fn dividends_shape() -> RecordedShape {
    ask::recorded_shape(include_str!("../../shapes/tmx-dividends.paths"))
}


/// A listing's quote as TMX states it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TmxQuote {
    pub symbol: String,
    /// The venue the answer is for, as TMX names it.
    pub exchange_name: String,
    /// None where TMX states no price now (a halted listing).
    pub price: Option<Dec>,
    pub change: Option<Dec>,
    pub change_pct: Option<Dec>,
    pub currency: Currency,
    /// When TMX served the quote, its only time: in session and out it equals the
    /// request's time within seconds. It says the price is current then, never
    /// when the last trade was.
    pub datetime: Timestamp,
    /// How often the listing pays, where TMX states it; a word this reader does
    /// not know, the mismatch naming it.
    pub per_year: Result<Option<u32>, Mismatch>,
    /// The ex-dividend date TMX reports on the quote, where it states one; a
    /// form this reader does not know, the mismatch naming it.
    pub ex_dividend: Result<Option<Date>, Mismatch>,
}

/// TMX's ex-dividend date on a quote: `""` where it states none, else a day at
/// midnight written `YYYY-MM-DD 00:00:00.0`; anything else is not a form this
/// reader knows.
fn ex_dividend(q: &Node) -> Result<Option<Date>, Mismatch> {
    let n = q.field("exDividendDate")?;
    let text = n.as_text()?;
    if text.is_empty() {
        return Ok(None);
    }
    match text.strip_suffix(" 00:00:00.0").map(crate::reply::day_from) {
        Some(Ok(d)) => Ok(Some(d)),
        Some(Err(why)) => Err(n.mismatch(why)),
        None => Err(n.mismatch(format!("{text:?} is not a day written YYYY-MM-DD 00:00:00.0"))),
    }
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


fn post(net: &Net, body: &[u8]) -> Result<Value, Outcome<()>> {
    let reply = match ask::send(net, &Ask::post(URL, &HEADERS, body), &[]) {
        Outcome::Answered(r) => r,
        other => return Err(other.failed().expect("not answered")),
    };
    ask::json(&reply.body).map_err(Outcome::Mismatch)
}

/// The alias the listing in place `i` of a quote request is asked under.
fn alias(i: usize) -> String {
    format!("q{i}")
}

/// The quote request for `forms`, each under its alias.
fn quotes_body(forms: &[String]) -> Vec<u8> {
    let asked: Vec<String> = forms.iter().enumerate().map(|(i, f)| format!("{}: getQuoteBySymbol(symbol: {}, locale: \"en\") {{ {QUOTE_FIELDS} }}", alias(i), escaped(f))).collect();
    body("getQuotes", "{}", &format!("query getQuotes {{ {} }}", asked.join(" ")))
}

/// Read a quote reply for `forms`, asked in that order: what came of each.
pub fn parse_quotes(v: &Value, forms: &[String]) -> Outcome<Vec<Outcome<TmxQuote>>> {
    let root = Node::root(v);
    // each error that names the alias it is for, by that alias: (code, message)
    let mut named: Vec<(String, String, String)> = Vec::new();
    let mut whole: Option<String> = None;
    if let Ok(errs) = root.list("errors") {
        for e in errs {
            let message = e.text("message").unwrap_or("").to_string();
            let code = e.text("code").unwrap_or("").to_string();
            match e.list("path").ok().and_then(|p| p.first().and_then(|a| a.as_text().ok().map(str::to_string))) {
                Some(a) => named.push((a, code, message)),
                None => whole = Some(message),
            }
        }
    }
    let data = match root.obj("data") {
        Ok(d) => d,
        // no answer for any listing: an error of the request's own, or a reply this reader cannot read
        Err(m) => return whole.map_or(Outcome::Mismatch(m), |why| Outcome::Unreachable(format!("TMX: {why}"))),
    };
    let each = forms
        .iter()
        .enumerate()
        .map(|(i, form)| {
            let a = alias(i);
            match named.iter().find(|(n, _, _)| *n == a) {
                Some((_, code, _)) if code == "404" => Outcome::NotCarried(format!("TMX does not know {form}")),
                Some((_, code, message)) => Outcome::Unreachable(format!("TMX could not answer {form}: {message} (code {code})")),
                None => match data.obj(&a).and_then(|q| read_quote(&q, form)) {
                    // TMX answers a form with the listing it knows by that symbol, which can
                    // be on another venue than the form asks: that listing is not this one
                    Ok(Ok(q)) if !crate::venue::tmx_venue_matches(form.find(':').map_or("", |i| &form[i..]), &q.exchange_name) => {
                        Outcome::NotCarried(format!("TMX answers {form} on {}, not the venue asked", q.exchange_name))
                    }
                    Ok(Ok(q)) => Outcome::Answered(q),
                    Ok(Err(why)) => Outcome::Meaning(why),
                    Err(m) => Outcome::Mismatch(m),
                },
            }
        })
        .collect();
    Outcome::Answered(each)
}

fn read_quote(q: &Node, form: &str) -> Result<Result<TmxQuote, String>, Mismatch> {
    let symbol = q.text("symbol")?;
    // a TSX or TSX-V listing is answered by its bare ticker, any other venue's by
    // the form itself (`QIMC:CNX`)
    let bare = form.split(':').next().unwrap_or(form);
    if !symbol.eq_ignore_ascii_case(bare) && !symbol.eq_ignore_ascii_case(form) {
        return Ok(Err(format!("TMX answered {symbol} for {form}")));
    }
    // null: no price now (a halted listing)
    let price = q.opt_dec("price")?;
    if let Some(p) = price.filter(|p| *p <= Dec::ZERO) {
        return Ok(Err(format!("{form}'s price is {p}")));
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
        None => Ok(None),
        Some(w) => match self::per_year(w) {
            Some(n) => Ok(Some(n)),
            None => Err(q.field("dividendFrequency")?.mismatch(format!("{w:?} is not a schedule this reader knows"))),
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
        ex_dividend: ex_dividend(&q),
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

/// Ask for the quotes of `forms`, at most `QUOTES_MOST` of them, in one request:
/// what came of each, in the order asked. A failure of the request is the outcome
/// itself.
pub fn ask_quotes(net: &Net, forms: &[String]) -> Noted<Vec<Outcome<TmxQuote>>> {
    if forms.len() > QUOTES_MOST {
        return Noted { outcome: Outcome::Meaning(format!("{} listings asked of TMX at once, more than its {QUOTES_MOST}", forms.len())), shape_change: None };
    }
    match post(net, &quotes_body(forms)) {
        Ok(v) => Noted { outcome: parse_quotes(&v, forms), shape_change: ask::noticed(&v, &quotes_shape(), QUOTES_KEYED) },
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
