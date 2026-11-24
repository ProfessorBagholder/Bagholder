//! The Options card on a US listing's page (brief 83; `SPEC.md` markets-123 to markets-130): for
//! each contract Cboe's delayed chain lists on 100 of the listing's shares, what
//! one contract puts in play (its Cost), the premium it would earn (its
//! Premium) and the one over the other (its Yield); and a contract's own premium
//! each day since it began trading, from Yahoo's daily closes.
//!
//! The chain is read when the card shows and every fifteen minutes while it
//! shows: the page asks, and an ask within a minute of the last read shares it.
//! Each read sends back the held chain's `Last-Modified`, so Cboe answers a bare
//! 304 where it has nothing newer. A contract opened from the card is figured
//! from the chain the card read; its chart is read from Yahoo on opening it. No
//! chain and no chart is kept past the process.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use bagholder_core::instrument::OptionRight;
use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::Timestamp;
use bagholder_core::{Dec, DecError, ListingKey, Rounding};
use bagholder_sources::adapters::cboe_options::{self, Chain, ChainContract, ChainReply};
use bagholder_sources::adapters::yahoo;
use bagholder_sources::outcome::Outcome;
use bagholder_diff::Diff;
use bagholder_diff_derive::Diff;
use serde::Serialize;
use ts_rs::TS;

use crate::app::App;
use crate::http::OkOr;

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// How long a read of a chain answers every ask that follows it: the card's
/// asks of one showing, and two pages open on one listing.
const SHARED: Duration = Duration::from_secs(60);

/// How long the chain a card read figures a contract opened from it, and how
/// often a card shown is read again.
pub const HELD: Duration = Duration::from_secs(15 * 60);

/// One contract's row on the card: a contract is on 100 shares.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
#[diff(key = occ)]
pub struct OptionRow {
    /// Its OCC symbol, as Cboe states it (`KO261211P00066000`).
    pub occ: String,
    /// As the card titles it opened (`KO Dec 11 66 put`, `KO Jan 15 '27 66 put`).
    pub title: String,
    pub strike: crate::wire::Dec,
    /// What one contract puts in play: the strike × 100 for a put, 100 × the
    /// listing's current price for a call; none for a call while the listing's
    /// price is not read.
    pub cost: Option<crate::wire::Dec>,
    /// The bid × 100: a contract no one bids on is not listed.
    pub premium: crate::wire::Dec,
    /// The Premium over the Cost, in percent, to two decimals.
    #[serde(rename = "yield")]
    pub yield_pct: Option<crate::wire::Dec>,
}

/// One side of one expiry: every strike the chain lists, highest first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct OptionSide {
    pub rows: Vec<OptionRow>,
    /// The row whose strike is nearest the listing's current price, which the
    /// card opens in view; none while the price is not read.
    pub near: Option<usize>,
}

/// One expiry the chain lists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
#[diff(key = day)]
pub struct OptionExpiry {
    /// Its day, `YYYY-MM-DD`.
    pub day: String,
    /// As its button says it: `Dec 11`, and `Jan 15 '27` in a later year.
    pub label: String,
    pub puts: OptionSide,
    pub calls: OptionSide,
}

/// `options:<symbol=…&exchange=…&currency=…>`, the card's document, and
/// `GET /api/options`, which agents read.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(untagged)]
pub enum OptionsAnswer {
    Ok {
        #[ts(type = "true")]
        ok: bool,
        /// Nearest first. None for a listing with no options: the page then
        /// shows no card.
        expiries: Vec<OptionExpiry>,
    },
    Refused(OkOr),
}

impl Diff for OptionsAnswer {
    fn diff(&self, new: &Self, path: &mut Vec<serde_json::Value>, ops: &mut Vec<serde_json::Value>) {
        match (self, new) {
            (OptionsAnswer::Ok { expiries: a, .. }, OptionsAnswer::Ok { expiries: b, .. }) => bagholder_diff::field(a, b, "expiries", path, ops),
            _ => bagholder_diff::as_json(self, new, path, ops),
        }
    }
    fn keys(path: &mut Vec<String>, out: &mut Vec<(String, &'static str)>) {
        bagholder_diff::field_keys::<Vec<OptionExpiry>>("expiries", path, out)
    }
}

/// One day's point on a contract's chart.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PremiumPoint {
    /// `YYYY-MM-DD`; the last point is today's.
    pub day: String,
    pub premium: crate::wire::Dec,
}

/// A contract's premium each day since it began trading.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PremiumChart {
    /// Oldest first; the last is today's Premium.
    pub points: Vec<PremiumPoint>,
    /// The axis's top, middle and bottom lines, in dollars.
    pub top: crate::wire::Dec,
    pub middle: crate::wire::Dec,
    pub bottom: crate::wire::Dec,
    /// The first day, as the axis says it (`Nov 3`).
    pub from: String,
}

/// One contract as the card opens it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OptionContract {
    pub occ: String,
    /// `KO Dec 11 66 put`.
    pub title: String,
    pub cost: Option<crate::wire::Dec>,
    pub premium: crate::wire::Dec,
    /// None where Yahoo has no bars for it.
    pub chart: Option<PremiumChart>,
    /// Why its chart was not read, where it was not.
    pub chart_failed: Option<String>,
}

/// `GET /api/options/contract`.
#[derive(Serialize, TS)]
#[serde(untagged)]
pub enum OptionContractAnswer {
    Ok {
        #[ts(type = "true")]
        ok: bool,
        contract: OptionContract,
    },
    Refused(OkOr),
}

fn hundred() -> Dec {
    Dec::from_int(100)
}

/// A day as the card says it: `Dec 11`.
fn day_label(d: Date) -> String {
    format!("{} {}", MONTHS[(d.month() - 1) as usize], d.day())
}

/// An expiry as the card names it: `Dec 11` this year, and with its year in the
/// app's short form in a later one (`Jan 15 '27`, `SPEC.md` formatting-10).
fn expiry_label(d: Date, today: Date) -> String {
    if d.year() > today.year() {
        format!("{} '{:02}", day_label(d), d.year() % 100)
    } else {
        day_label(d)
    }
}

/// A strike as the card writes it: whole, or to two decimals (`66`, `62.50`).
fn strike_text(k: Dec) -> String {
    if k.round(0, Rounding::HalfEven) == k {
        return k.round(0, Rounding::HalfEven).to_text();
    }
    let t = k.round(2, Rounding::HalfEven).to_text();
    match t.split_once('.') {
        Some((whole, part)) => format!("{whole}.{part:0<2}"),
        None => t,
    }
}

/// The OCC root a listing's own contracts carry: its symbol without the point
/// of a class (`BRK.B`'s are `BRKB`).
fn root_of(symbol: &str) -> String {
    symbol.chars().filter(|c| *c != '.').collect::<String>().to_ascii_uppercase()
}

/// Whether a contract is on 100 of the listing's shares: an adjusted contract,
/// whose deliverable is something else, carries another root (`KO1`).
fn on_the_listing(c: &ChainContract, root: &str) -> bool {
    c.occ.len() > 15 && c.occ[..c.occ.len() - 15].eq_ignore_ascii_case(root)
}

fn row(symbol: &str, c: &ChainContract, price: Option<Dec>, today: Date) -> Result<OptionRow, String> {
    let e = |e: DecError| format!("{}'s figures: {e}", c.occ);
    let premium = c.bid.checked_mul(hundred()).map_err(e)?;
    let cost = match c.right {
        OptionRight::Put => Some(c.strike.checked_mul(hundred()).map_err(e)?),
        OptionRight::Call => price.map(|p| p.checked_mul(hundred())).transpose().map_err(e)?,
    };
    let yield_pct = match cost {
        Some(k) if k.is_positive() => Some(premium.mul_div_rounded(hundred(), k, 2, Rounding::HalfUp).map_err(e)?),
        _ => None,
    };
    Ok(OptionRow { occ: c.occ.clone(), title: title(symbol, c, today), strike: c.strike.into(), cost: cost.map(Into::into), premium: premium.into(), yield_pct: yield_pct.map(Into::into) })
}

fn side(symbol: &str, mut list: Vec<&ChainContract>, price: Option<Dec>, today: Date) -> Result<OptionSide, String> {
    list.sort_by(|a, b| b.strike.cmp(&a.strike));
    let rows = list.iter().map(|c| row(symbol, c, price, today)).collect::<Result<Vec<_>, _>>()?;
    let mut near: Option<(usize, Dec)> = None;
    if let Some(p) = price {
        for (i, c) in list.iter().enumerate() {
            let off = c.strike.checked_sub(p).map_err(|e| format!("{}'s strike: {e}", c.occ))?.abs();
            if near.is_none_or(|(_, best)| off < best) {
                near = Some((i, off));
            }
        }
    }
    Ok(OptionSide { rows, near: near.map(|(i, _)| i) })
}

/// The card for one listing on `today`: each expiry the chain lists, nearest
/// first, its puts and its calls, each figured on the listing's current price
/// where it is read. Only a contract one could sell is listed: one someone bids
/// on, on 100 of the listing's shares. A strike no one bids on puts nothing in play
/// and earns nothing, and an expiry with no strike bid on either side is not listed.
pub fn card(chain: &Chain, symbol: &str, price: Option<Dec>, today: Date) -> Result<Vec<OptionExpiry>, String> {
    let root = root_of(symbol);
    let mut by: BTreeMap<Date, (Vec<&ChainContract>, Vec<&ChainContract>)> = BTreeMap::new();
    for c in chain.contracts.iter().filter(|c| on_the_listing(c, &root) && c.bid.is_positive()) {
        let e = by.entry(c.expiry).or_default();
        match c.right {
            OptionRight::Put => e.0.push(c),
            OptionRight::Call => e.1.push(c),
        }
    }
    by.into_iter().map(|(day, (puts, calls))| Ok(OptionExpiry { day: day.to_string(), label: expiry_label(day, today), puts: side(symbol, puts, price, today)?, calls: side(symbol, calls, price, today)? })).collect()
}

/// A contract's title on `today`: `KO Dec 11 66 put`, `KO Jan 15 '27 66 put`.
pub fn title(symbol: &str, c: &ChainContract, today: Date) -> String {
    let right = match c.right {
        OptionRight::Put => "put",
        OptionRight::Call => "call",
    };
    format!("{} {} {} {right}", symbol.to_ascii_uppercase(), expiry_label(c.expiry, today), strike_text(c.strike))
}

/// A contract's chart: each past day's close × 100, and today's Premium last.
/// The axis runs from the least to the most, and spans at least four dollars:
/// a narrower run is widened two dollars each way, never below zero. None
/// without a past day.
pub fn premium_chart(closes: &[(Date, Dec)], today: Date, premium: Option<Dec>) -> Result<Option<PremiumChart>, String> {
    let e = |e: DecError| format!("a contract's chart: {e}");
    let mut points: Vec<(Date, Dec)> = Vec::new();
    for (d, close) in closes.iter().filter(|(d, _)| *d < today) {
        points.push((*d, close.checked_mul(hundred()).map_err(e)?));
    }
    if points.is_empty() {
        return Ok(None);
    }
    if let Some(p) = premium {
        points.push((today, p));
    }
    let lo = points.iter().map(|p| p.1).min().expect("a point");
    let hi = points.iter().map(|p| p.1).max().expect("a point");
    let four = Dec::from_int(4);
    let two = Dec::from_int(2);
    let (lo, hi) = if hi.checked_sub(lo).map_err(e)? < four { (lo.checked_sub(two).map_err(e)?.max(Dec::ZERO), hi.checked_add(two).map_err(e)?) } else { (lo, hi) };
    let middle = hi.checked_add(lo).map_err(e)?.div_rounded(two, 2, Rounding::HalfEven).map_err(e)?;
    Ok(Some(PremiumChart {
        from: day_label(points[0].0),
        points: points.into_iter().map(|(d, p)| PremiumPoint { day: d.to_string(), premium: p.into() }).collect(),
        top: hi.into(),
        middle: middle.into(),
        bottom: lo.into(),
    }))
}

/// The chain last read of each symbol: when, Cboe's `Last-Modified` for it, and
/// the chain, none where Cboe lists none.
struct Held {
    at: Instant,
    last_modified: Option<String>,
    chain: Option<Chain>,
}

fn memory() -> &'static Mutex<HashMap<String, Held>> {
    static MEMORY: OnceLock<Mutex<HashMap<String, Held>>> = OnceLock::new();
    MEMORY.get_or_init(Default::default)
}

/// `symbol`'s chain: the one held where it was read within `fresh`, else read
/// again; none where Cboe lists no chain for it.
fn chain_of(net: &bagholder_net::Net, symbol: &str, fresh: Duration) -> Result<Option<Chain>, String> {
    let held = memory().lock().unwrap_or_else(|e| e.into_inner()).get(symbol).map(|h| (h.at, h.last_modified.clone(), h.chain.clone()));
    if let Some((at, _, chain)) = &held {
        if at.elapsed() < fresh {
            return Ok(chain.clone());
        }
    }
    // the held chain's stamp, so Cboe answers a bare 304 where it has nothing newer
    let (since, kept) = match held {
        Some((_, Some(stamp), Some(chain))) => (Some(stamp), Some(chain)),
        _ => (None, None),
    };
    let noted = match stand_in() {
        Some(base) => cboe_options::ask_of(net, &base, symbol, since.as_deref()),
        None => cboe_options::ask(net, symbol, since.as_deref()),
    };
    let (chain, last_modified) = match noted.outcome {
        Outcome::Answered(ChainReply::Chain { chain, last_modified }) => (Some(chain), last_modified),
        Outcome::Answered(ChainReply::NotModified) => (kept, since),
        Outcome::NotCarried(_) => (None, None),
        // told to stay off the network: no card, and nothing said, since nothing was asked
        Outcome::Offline => return Ok(None),
        other => return Err(format!("Cboe's option chain of {symbol} could not be read: {} ({})", other.kind(), other.detail())),
    };
    memory().lock().unwrap_or_else(|e| e.into_inner()).insert(symbol.to_string(), Held { at: Instant::now(), last_modified, chain: chain.clone() });
    Ok(chain)
}

/// Cboe's and Yahoo's stand-in on loopback, where `BAGHOLDER_OPTIONS_BASE` names
/// one (the browser tests' `e2e/serve.mjs`), as `BAGHOLDER_WS_BASE` names
/// Wealthsimple's: the card's chain and charts are asked of it alone.
fn stand_in() -> Option<String> {
    std::env::var("BAGHOLDER_OPTIONS_BASE").ok().map(|b| b.trim_end_matches('/').to_string()).filter(|b| !b.is_empty())
}

/// Today in New York, where US options trade: what makes an expiry's year a later one.
fn market_today() -> Date {
    TimeZone::get("America/New_York").map(|z| Timestamp::now().to_zoned(z).date()).unwrap_or_else(|_| Timestamp::now().to_zoned(TimeZone::UTC).date())
}

/// The listing's current price, as its page shows it: the book's quote where the
/// book holds it (a holding's page), else the glance a listing's own page reads;
/// none while neither is read.
fn price_of(app: &Arc<App>, key: &ListingKey, exchange: &str, currency: &str) -> Option<Dec> {
    let held = app.figures.get().and_then(|f| {
        f.read(|e| {
            let inputs = e.inputs();
            let held = inputs.ledger.instruments.values().find(|i| i.current_name().and_then(|n| ListingKey::of(i.instrument.kind, n).ok()).as_ref() == Some(key))?;
            inputs.market.quotes.get(&held.instrument.id).map(|q| q.price.amount)
        })
        .flatten()
    });
    held.or_else(|| crate::following::glance(app, key.symbol(), exchange, currency).ok().map(|g| g.price.0))
}

/// One listing's card, the chain read as the card shows.
pub fn options_payload(app: &Arc<App>, key: &ListingKey, exchange: &str, currency: &str) -> OptionsAnswer {
    let none = || OptionsAnswer::Ok { ok: true, expiries: vec![] };
    // a US listing's: Canadian-listed options are not on the card
    if key.venue() != bagholder_core::listing::US {
        return none();
    }
    let chain = match chain_of(&app.net, key.symbol(), SHARED) {
        Ok(Some(c)) => c,
        Ok(None) => return none(),
        Err(e) => return OptionsAnswer::Refused(OkOr::err(e)),
    };
    match card(&chain, key.symbol(), price_of(app, key, exchange, currency), market_today()) {
        Ok(expiries) => OptionsAnswer::Ok { ok: true, expiries },
        Err(e) => OptionsAnswer::Refused(OkOr::err(e)),
    }
}

/// One contract of a listing's card, as the card opens it: its figures from the
/// chain the card read, its chart read from Yahoo.
pub fn contract_payload(app: &Arc<App>, key: &ListingKey, exchange: &str, currency: &str, occ: &str) -> OptionContractAnswer {
    let refused = |e: String| OptionContractAnswer::Refused(OkOr::err(e));
    let occ = occ.split_whitespace().collect::<String>().to_ascii_uppercase();
    if key.venue() != bagholder_core::listing::US {
        return refused(format!("{} is not a US listing, whose options the card shows", key.named()));
    }
    let chain = match chain_of(&app.net, key.symbol(), HELD) {
        Ok(Some(c)) => c,
        Ok(None) => return refused(format!("Cboe lists no options of {}", key.symbol())),
        Err(e) => return refused(e),
    };
    let Some(c) = chain.contracts.iter().find(|c| c.occ == occ && on_the_listing(c, &root_of(key.symbol()))) else {
        return refused(format!("{occ} is not one of {}'s contracts the chain lists", key.symbol()));
    };
    let r = match row(key.symbol(), c, price_of(app, key, exchange, currency), market_today()) {
        Ok(r) => r,
        Err(e) => return refused(e),
    };
    let now = Timestamp::now();
    let today = now.to_zoned(TimeZone::UTC).date();
    let asked = match stand_in() {
        Some(base) => yahoo::ask_span_of(&app.net, &format!("{base}/v8/finance/chart/"), &occ, Date::constant(2000, 1, 3), today, now),
        None => yahoo::ask_span(&app.net, &occ, Date::constant(2000, 1, 3), today, now),
    };
    let (closes, chart_failed) = match asked.outcome {
        Outcome::Answered(chart) => (chart.closes, None),
        // a contract never traded: no bars; told to stay off the network: none asked for
        Outcome::NotCarried(_) | Outcome::Offline => (vec![], None),
        other => (vec![], Some(format!("Yahoo's chart of {occ} could not be read: {} ({})", other.kind(), other.detail()))),
    };
    let chart = match premium_chart(&closes, chain.session, Some(r.premium.0)) {
        Ok(c) => c,
        Err(e) => return refused(e),
    };
    OptionContractAnswer::Ok { ok: true, contract: OptionContract { occ: occ.clone(), title: r.title, cost: r.cost, premium: r.premium, chart, chart_failed } }
}

/// Every chain held, let go: a test's stand-in network is asked afresh.
#[cfg(test)]
pub(crate) fn forget() {
    memory().lock().unwrap_or_else(|e| e.into_inner()).clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn d(s: &str) -> Dec {
        Dec::parse(s).unwrap()
    }

    /// The day the recorded chain was made.
    fn today() -> Date {
        "2026-11-11".parse().unwrap()
    }

    const CHAIN: &str = include_str!("../tests/replies/options-card/chain-KO.json");
    const CHART: &str = include_str!("../tests/replies/options-card/chart-KO261211P00066000.json");

    fn chain() -> Chain {
        match cboe_options::parse(&bagholder_core::json::parse(CHAIN).unwrap(), "KO") {
            Outcome::Answered(c) => c,
            other => panic!("the recorded chain: {} ({})", other.kind(), other.detail()),
        }
    }

    fn expiry<'a>(card: &'a [OptionExpiry], day: &str) -> &'a OptionExpiry {
        card.iter().find(|e| e.day == day).unwrap_or_else(|| panic!("no {day} expiry"))
    }

    fn of<'a>(side: &'a OptionSide, strike: &str) -> &'a OptionRow {
        side.rows.iter().find(|r| r.strike.0 == d(strike)).unwrap_or_else(|| panic!("no {strike} strike"))
    }

    fn figures(r: &OptionRow) -> (Option<String>, Option<String>, Option<String>) {
        let t = |v: Option<crate::wire::Dec>| v.map(|v| v.0.to_text());
        (t(r.cost), t(Some(r.premium)), t(r.yield_pct))
    }

    fn some(cost: &str, premium: &str, yield_pct: &str) -> (Option<String>, Option<String>, Option<String>) {
        (Some(d(cost).to_text()), Some(d(premium).to_text()), Some(d(yield_pct).to_text()))
    }

    /// The three figures, worked by hand from the recorded chain (every figure in
    /// it made up), the listing at $68.42: a put's Cost is its strike × 100, a
    /// call's 100 × the price; the Premium is the bid × 100; the Yield the one
    /// over the other, in percent to two decimals.
    #[test]
    fn a_contract_s_cost_premium_and_yield_are_its_strike_or_price_its_bid_and_their_ratio() {
        let card = card(&chain(), "KO", Some(d("68.42")), today()).unwrap();
        let dec11 = expiry(&card, "2026-12-11");
        // bid 0.24: 24 / 6,600 = 0.3636…%
        assert_eq!(figures(of(&dec11.puts, "66")), some("6600", "24", "0.36"));
        // bid 0.11: 11 / 6,250 = 0.176%
        assert_eq!(figures(of(&dec11.puts, "62.5")), some("6250", "11", "0.18"));
        // bid 2.66 on 100 × 68.42: 266 / 6,842 = 3.8877…%
        assert_eq!(figures(of(&dec11.calls, "66")), some("6842", "266", "3.89"));
        // bid 0.20 on 6,842: 0.2923…%
        assert_eq!(figures(of(&dec11.calls, "72")), some("6842", "20", "0.29"));
        // a call's Cost is the price's whatever its strike; a put's, its own strike's
        assert!(dec11.calls.rows.iter().all(|r| r.cost == Some(d("6842").into())));
        assert!(dec11.puts.rows.iter().all(|r| r.cost.map(|c| c.0) == r.strike.0.checked_mul(hundred()).ok()));
    }

    /// A strike no one bids on cannot be sold: it is not listed, so every row is a
    /// contract one could put in play, with its real figures; and an expiry with no
    /// strike bid on either side is not listed at all.
    #[test]
    fn a_strike_no_one_bids_on_is_not_listed() {
        let card = card(&chain(), "KO", Some(d("68.42")), today()).unwrap();
        let nov13 = expiry(&card, "2026-11-13");
        let strikes = |s: &OptionSide| s.rows.iter().map(|r| r.strike.0.to_text()).collect::<Vec<_>>();
        assert!(!strikes(&nov13.puts).contains(&"60".to_string()), "{:?}", strikes(&nov13.puts));
        assert!(!strikes(&nov13.calls).contains(&"76".to_string()), "{:?}", strikes(&nov13.calls));
        assert!(card.iter().flat_map(|e| e.puts.rows.iter().chain(&e.calls.rows)).all(|r| r.premium.0.is_positive()));
        // a chain whose one expiry no one bids on, on either side: no expiry at all
        let mut none = chain();
        for c in &mut none.contracts {
            if c.expiry.to_string() == "2026-11-13" {
                c.bid = Dec::ZERO;
            }
        }
        let card = super::card(&none, "KO", Some(d("68.42")), today()).unwrap();
        assert!(!card.iter().any(|e| e.day == "2026-11-13"), "{:?}", card.iter().map(|e| &e.day).collect::<Vec<_>>());
    }

    /// A call is figured on the listing's price: while it is not read a call has
    /// no Cost and no Yield, and the card opens on no row.
    #[test]
    fn a_call_has_no_cost_while_the_listing_s_price_is_not_read() {
        let card = card(&chain(), "KO", None, today()).unwrap();
        let dec11 = expiry(&card, "2026-12-11");
        assert_eq!(figures(of(&dec11.calls, "66")), (None, Some(d("266").to_text()), None));
        assert_eq!(figures(of(&dec11.puts, "66")), some("6600", "24", "0.36"));
        assert_eq!((dec11.puts.near, dec11.calls.near), (None, None));
    }

    /// An adjusted contract, whose OCC root is not the listing's symbol, is on
    /// something other than 100 shares: it is not listed.
    #[test]
    fn an_adjusted_contract_is_not_listed() {
        let chain = chain();
        assert!(chain.contracts.iter().any(|c| c.occ.starts_with("KO1")), "the recorded chain lists one");
        let card = card(&chain, "KO", Some(d("68.42")), today()).unwrap();
        let occs: Vec<&str> = card.iter().flat_map(|e| e.puts.rows.iter().chain(e.calls.rows.iter())).map(|r| r.occ.as_str()).collect();
        assert!(!occs.is_empty() && occs.iter().all(|o| !o.starts_with("KO1")));
        // every other contract someone bids on is
        assert_eq!(occs.len(), chain.contracts.iter().filter(|c| !c.occ.starts_with("KO1") && c.bid.is_positive()).count());
    }

    /// Expiries nearest first, each labelled as its button says it; strikes
    /// highest first; the card opens on the strike nearest the price.
    #[test]
    fn expiries_run_nearest_first_and_strikes_highest_first() {
        let card = card(&chain(), "KO", Some(d("68.42")), today()).unwrap();
        // this year's by month and day; a later year's with its year (`SPEC.md` formatting-10)
        assert_eq!(card.iter().map(|e| e.label.as_str()).collect::<Vec<_>>(), vec!["Nov 13", "Nov 20", "Dec 11", "Jan 15 '27"]);
        let dec11 = expiry(&card, "2026-12-11");
        let strikes: Vec<Dec> = dec11.puts.rows.iter().map(|r| r.strike.0).collect();
        assert!(strikes.windows(2).all(|w| w[0] > w[1]), "{strikes:?}");
        assert_eq!(strikes.first().copied(), Some(d("76")));
        assert_eq!(dec11.puts.near.map(|i| dec11.puts.rows[i].strike.0), Some(d("68")));
        assert_eq!(dec11.calls.near.map(|i| dec11.calls.rows[i].strike.0), Some(d("68")));
    }

    #[test]
    fn a_contract_s_title_names_its_listing_expiry_strike_and_side() {
        let chain = chain();
        let find = |occ: &str| chain.contracts.iter().find(|c| c.occ == occ).unwrap();
        assert_eq!(title("KO", find("KO261211P00066000"), today()), "KO Dec 11 66 put");
        assert_eq!(title("KO", find("KO261211C00062500"), today()), "KO Dec 11 62.50 call");
        assert_eq!(title("ko", find("KO270115P00072500"), today()), "KO Jan 15 '27 72.50 put");
        // seen in the expiry's own year, it is this year's
        let new_year: Date = "2027-01-04".parse().unwrap();
        assert_eq!(title("KO", find("KO270115P00072500"), new_year), "KO Jan 15 72.50 put");
        assert_eq!(card(&chain, "KO", None, new_year).unwrap().iter().map(|e| e.label.as_str()).collect::<Vec<_>>(), vec!["Nov 13", "Nov 20", "Dec 11", "Jan 15"]);
    }

    /// The chart: each past day's close × 100 as Yahoo states it (a day with no
    /// close has no point), today's bar left for today's Premium, which is last;
    /// the axis from the least to the most.
    #[test]
    fn a_contract_s_chart_is_its_past_closes_and_today_s_premium() {
        let now: Timestamp = "2026-11-12T03:00:00Z".parse().unwrap();
        let v = bagholder_core::json::parse(CHART).unwrap();
        let yahoo = match yahoo::parse(&v, "KO261211P00066000", now) {
            Outcome::Answered(c) => c,
            other => panic!("the recorded chart: {} ({})", other.kind(), other.detail()),
        };
        let today: Date = "2026-11-11".parse().unwrap();
        let chart = premium_chart(&yahoo.closes, today, Some(d("24"))).unwrap().unwrap();
        let got: Vec<(String, String)> = chart.points.iter().map(|p| (p.day.clone(), p.premium.0.to_text())).collect();
        // 32 sessions from Sep 29, two of them with no close; today's close (0.25) gives way to today's Premium
        assert_eq!(got.len(), 30);
        assert_eq!(got[0], ("2026-09-29".to_string(), d("62").to_text()));
        assert_eq!(got[1], ("2026-09-30".to_string(), d("61").to_text()));
        assert_eq!(got[3], ("2026-10-05".to_string(), d("58").to_text()), "Oct 2 states no close");
        assert_eq!(got.last().cloned(), Some(("2026-11-11".to_string(), d("24").to_text())));
        assert_eq!(chart.from, "Sep 29");
        assert_eq!((chart.top.0, chart.middle.0, chart.bottom.0), (d("64"), d("44"), d("24")));
        // no Premium today: the past days alone
        let past = premium_chart(&yahoo.closes, today, None).unwrap().unwrap();
        assert_eq!(past.points.last().map(|p| p.day.as_str()), Some("2026-11-10"));
        // no past day: no chart
        assert_eq!(premium_chart(&[], today, Some(d("24"))).unwrap(), None);
        assert_eq!(premium_chart(&[(today, d("0.25"))], today, Some(d("24"))).unwrap(), None);
    }

    /// An axis spans at least four dollars: a narrower run is widened two each
    /// way, never below zero.
    #[test]
    fn a_narrow_run_s_axis_is_widened_and_never_below_zero() {
        let day = |s: &str| s.parse::<Date>().unwrap();
        let today = day("2026-11-11");
        let flat = premium_chart(&[(day("2026-11-09"), d("0.5")), (day("2026-11-10"), d("0.51"))], today, Some(d("50"))).unwrap().unwrap();
        assert_eq!((flat.top.0, flat.middle.0, flat.bottom.0), (d("53"), d("50.5"), d("48")));
        let low = premium_chart(&[(day("2026-11-10"), d("0.01"))], today, Some(d("2"))).unwrap().unwrap();
        assert_eq!((low.top.0, low.middle.0, low.bottom.0), (d("4"), d("2"), d("0")));
    }

    #[test]
    fn a_strike_is_written_whole_or_to_two_decimals() {
        assert_eq!(strike_text(d("66")), "66");
        assert_eq!(strike_text(d("66.000")), "66");
        assert_eq!(strike_text(d("62.5")), "62.50");
        assert_eq!(strike_text(d("2.25")), "2.25");
    }

    /// The network as the tests answer it: Cboe's chain of KO and Yahoo's chart of
    /// one of its puts from the recorded captures, 304 to a chain asked again
    /// with its stamp, 403 to a symbol Cboe lists no options for, and 404 to
    /// everything else (the listing's quote among them).
    struct Answers(Mutex<Vec<String>>);
    struct Shared(Arc<Answers>);
    impl bagholder_net::Transport for Shared {
        fn answer(&self, ask: &bagholder_net::Ask) -> Result<bagholder_net::Answer, bagholder_net::NetError> {
            let url = ask.url.to_string();
            self.0 .0.lock().unwrap().push(url.clone());
            let since = ask.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("if-modified-since"));
            let (status, headers, body): (u16, Vec<(String, String)>, &str) = if url.ends_with("/delayed_quotes/options/KO.json") {
                if since { (304, vec![], "") } else { (200, vec![("last-modified".into(), "Wed, 11 Nov 2026 21:40:12 GMT".into())], CHAIN) }
            } else if url.ends_with("/delayed_quotes/options/ZZZQ.json") {
                (403, vec![], "")
            } else if url.contains("/v8/finance/chart/KO261211P00066000?") {
                (200, vec![], CHART)
            } else {
                (404, vec![], "")
            };
            Ok((status, url, headers, body.as_bytes().to_vec()))
        }
    }

    fn app_answered() -> (tempfile::TempDir, Arc<App>, Arc<Answers>) {
        let asked = Arc::new(Answers(Mutex::new(vec![])));
        let dir = tempfile::tempdir().unwrap();
        let net = bagholder_net::Net::answered_by(Arc::new(bagholder_net::SystemClock), Arc::new(bagholder_net::Limiter::new()), Box::new(Shared(asked.clone())));
        let app = App::with_net(dir.path().to_path_buf(), std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into(), net);
        (dir, app, asked)
    }

    fn asked_of(asked: &Answers, part: &str) -> usize {
        asked.0.lock().unwrap().iter().filter(|u| u.contains(part)).count()
    }

    /// The card as the page asks for it: the chain read when the card shows and
    /// shared by an ask within the minute; a contract opened from it figured from
    /// that chain and charted from Yahoo; no card, and nothing asked, for a
    /// Canadian listing; no card for a listing Cboe lists no options for.
    #[test]
    fn the_card_reads_the_chain_as_it_shows_and_a_contract_reads_its_chart() {
        let _g = crate::tests_common::guard();
        forget();
        let (_dir, app, asked) = app_answered();
        let ko = ListingKey::on("KO", "NYSE").unwrap();
        let OptionsAnswer::Ok { expiries, .. } = options_payload(&app, &ko, "NYSE", "USD") else { panic!("the card was refused") };
        assert_eq!(expiries.len(), 4);
        // the quote not read: a put stands on its strike, a call has no Cost
        let dec11 = expiry(&expiries, "2026-12-11");
        assert_eq!(figures(of(&dec11.puts, "66")), some("6600", "24", "0.36"));
        assert_eq!(figures(of(&dec11.calls, "66")), (None, Some(d("266").to_text()), None));
        assert_eq!(asked_of(&asked, "/delayed_quotes/options/KO.json"), 1);
        // asked again within the minute: the same read
        assert!(matches!(options_payload(&app, &ko, "NYSE", "USD"), OptionsAnswer::Ok { .. }));
        assert_eq!(asked_of(&asked, "/delayed_quotes/options/KO.json"), 1);
        // a contract opened from it: the chain the card read, Yahoo's chart
        let OptionContractAnswer::Ok { contract, .. } = contract_payload(&app, &ko, "NYSE", "USD", "KO261211P00066000") else { panic!("the contract was refused") };
        assert_eq!(asked_of(&asked, "/delayed_quotes/options/KO.json"), 1);
        assert_eq!(asked_of(&asked, "/v8/finance/chart/KO261211P00066000?"), 1);
        assert_eq!((contract.title.as_str(), contract.cost.map(|c| c.0), contract.premium.0), ("KO Dec 11 66 put", Some(d("6600")), d("24")));
        let chart = contract.chart.expect("a chart");
        assert_eq!((chart.points.len(), chart.points.last().map(|p| p.premium.0)), (30, Some(d("24"))));
        assert_eq!(contract.chart_failed, None);
        // a contract Yahoo has no bars for: no chart, and no failure
        let OptionContractAnswer::Ok { contract, .. } = contract_payload(&app, &ko, "NYSE", "USD", "KO261211C00066000") else { panic!("the contract was refused") };
        assert_eq!((contract.chart, contract.chart_failed), (None, None));
        // an adjusted contract is not one the card opens
        assert!(matches!(contract_payload(&app, &ko, "NYSE", "USD", "KO1261211P00066000"), OptionContractAnswer::Refused(_)));
        // a listing Cboe lists no options for: no card
        let none = ListingKey::on("ZZZQ", "NASDAQ").unwrap();
        assert!(matches!(options_payload(&app, &none, "NASDAQ", "USD"), OptionsAnswer::Ok { expiries, .. } if expiries.is_empty()));
        // a Canadian listing: no card, and Cboe is not asked
        let before = asked.0.lock().unwrap().len();
        let enb = ListingKey::on("ENB", "TSX").unwrap();
        assert!(matches!(options_payload(&app, &enb, "TSX", "CAD"), OptionsAnswer::Ok { expiries, .. } if expiries.is_empty()));
        assert_eq!(asked.0.lock().unwrap().len(), before);
        forget();
    }

    /// A chain read again after the minute sends back its stamp, and Cboe's bare
    /// 304 keeps the chain held.
    #[test]
    fn a_chain_read_again_keeps_the_one_held_where_cboe_has_nothing_newer() {
        let _g = crate::tests_common::guard();
        forget();
        let (_dir, app, asked) = app_answered();
        let first = chain_of(&app.net, "KO", SHARED).unwrap().expect("a chain");
        let again = chain_of(&app.net, "KO", Duration::ZERO).unwrap().expect("the chain held");
        assert_eq!(first, again);
        assert_eq!(asked_of(&asked, "/delayed_quotes/options/KO.json"), 2);
        forget();
    }

    fn wait_for(what: &str, done: impl Fn() -> bool) {
        let until = Instant::now() + Duration::from_secs(20);
        while !done() {
            assert!(Instant::now() < until, "{what} did not come to pass");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The card's document as a page watching it is sent it (`SPEC.md`
    /// markets-129): nothing until the first read, then the card; let go once no
    /// page shows it; read again when a page shows it again.
    #[test]
    fn the_card_s_document_is_read_while_a_page_shows_it() {
        let _g = crate::tests_common::guard();
        forget();
        let (_dir, app, asked) = app_answered();
        let key = "options:symbol=KO&exchange=NYSE&currency=USD";
        assert!(crate::docs::read(&app, key).is_none(), "nothing until the first read");
        let feed = crate::events::Feed::open(app.clone());
        let want = |k: &str| [(k.to_string(), crate::events::Want { params: json!({}), have: None })].into_iter().collect();
        let shown = || matches!(crate::docs::read(&app, key), Some(crate::docs::Doc::Options(OptionsAnswer::Ok { ref expiries, .. })) if expiries.len() == 4);
        assert!(app.events.watch(&app, feed.id(), want(key)));
        wait_for("the card read as it shows", shown);
        assert_eq!(asked_of(&asked, "/delayed_quotes/options/KO.json"), 1);
        // no page shows it: let go
        assert!(app.events.watch(&app, feed.id(), Default::default()));
        wait_for("the card let go", || crate::docs::read(&app, key).is_none());
        // shown again: read again
        forget();
        assert!(app.events.watch(&app, feed.id(), want(key)));
        wait_for("the card read as it shows again", shown);
        assert_eq!(asked_of(&asked, "/delayed_quotes/options/KO.json"), 2);
        drop(feed);
        forget();
    }

    /// Told to stay off the network, Cboe is not asked: no card, and no failure said.
    #[test]
    fn offline_there_is_no_card_and_nothing_is_said() {
        let _g = crate::tests_common::guard();
        forget();
        // the test home tells the process to stay off the network
        crate::tests_common::home();
        let net = bagholder_net::Net::new(Arc::new(bagholder_net::SystemClock), Arc::new(bagholder_net::Limiter::new()));
        assert_eq!(chain_of(&net, "KO", SHARED), Ok(None));
        forget();
    }

    /// A row is sent as the card reads it, `yield` by that name.
    #[test]
    fn a_row_is_sent_with_its_yield_by_that_name() {
        let card = card(&chain(), "KO", Some(d("68.42")), today()).unwrap();
        let row = of(&expiry(&card, "2026-12-11").puts, "66");
        assert_eq!(serde_json::to_value(row).unwrap(), json!({"occ": "KO261211P00066000", "title": "KO Dec 11 66 put", "strike": "66", "cost": "6600", "premium": "24", "yield": "0.36"}));
    }
}
