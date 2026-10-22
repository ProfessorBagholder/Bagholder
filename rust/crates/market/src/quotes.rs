//! What the market readers share for asking Yahoo and naming a listing: Yahoo's
//! pace on the one limiter, its chart read as bars, and the forms a listing is
//! asked under at Yahoo and at TMX.

use serde_json::{json, Value};
use std::time::Duration;

use crate::http::get_text;
use bagholder_model::input::Listing;
use bagholder_model::value::{field_s, get, num};
use crate::stored::bars::{Ohlcv, SourceBar};

const YAHOO_HEADERS: [(&str, &str); 2] = [("User-Agent", "Mozilla/5.0"), ("Accept", "application/json")];

/// Yahoo rate-limits bursts: one request at a time, well spaced, and after a
/// 429 nothing is asked of it for ten minutes. That is its pace on the one
/// limiter every host goes through.
pub const YAHOO_HOST: &str = "query1.finance.yahoo.com";
pub const YAHOO_PACE: bagholder_net::Pace = bagholder_net::Pace { gap: Duration::from_millis(2000), rest: Duration::from_secs(600) };

/// Yahoo's pace, set on the one limiter.
fn yahoo_paced() {
    bagholder_net::machine::global().configure(YAHOO_HOST, YAHOO_PACE);
}

/// A turn at Yahoo for a request not made through `crate::http` (the float's
/// browser session), which takes its own.
fn yahoo_turn() -> Result<(), crate::http::FetchError> {
    yahoo_paced();
    bagholder_net::machine::global()
        .turn(YAHOO_HOST, &bagholder_net::SystemClock)
        .map_err(|r| crate::http::FetchError::Transport(format!("yahoo: refused a request; not asked again before {}", r.until)))
}

/// A Yahoo request for the chart path: the body, or the failure -- a rest in force reads as one, and a 404 keeps its
/// code so the symbol can be remembered as one Yahoo does not carry.
pub fn yahoo_get_result(url: &str) -> Result<String, crate::http::FetchError> {
    // the request takes its turn, and a 429 rests the host, in `crate::http`
    yahoo_paced();
    get_text(url, &YAHOO_HEADERS)
}

/// An instant as epoch seconds, for callers outside this module.
pub fn instant_secs_public(s: &str) -> Option<f64> {
    instant_secs(s)
}

/// Bars in the exchange's own local day and
/// minute, oldest first; a row with no close is dropped.
///
/// Yahoo's `gmtoffset` is the offset today, not the bar's, so where the
/// exchange names its zone each bar is given its own standard or daylight
/// offset instead.
pub fn parse_yahoo_chart(text: &str) -> Vec<SourceBar> {
    let d: Value = match serde_json::from_str(if text.is_empty() { "{}" } else { text }) { Ok(v) => v, Err(_) => return vec![] };
    let results = match d.get("chart").and_then(|c| c.get("result")).and_then(|r| r.as_array()) {
        Some(r) if !r.is_empty() => r,
        _ => return vec![],
    };
    let r = &results[0];
    let ts = r.get("timestamp").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let q = r
        .get("indicators")
        .and_then(|i| i.get("quote"))
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let meta = r.get("meta").cloned().unwrap_or_else(|| json!({}));
    let zone = field_s(&meta, "exchangeTimezoneName");
    let fixed = num(get(&meta, "gmtoffset"), 0.0) as i64;

    let col = |k: &str, i: usize| -> Option<f64> {
        q.get(k).and_then(|v| v.as_array()).and_then(|a| a.get(i)).and_then(|x| x.as_f64())
    };

    let mut out: Vec<SourceBar> = Vec::new();
    for (i, t) in ts.iter().enumerate() {
        let t = match t.as_i64() { Some(t) => t, None => continue };
        let close = match col("close", i) { Some(c) if c > 0.0 => c, _ => continue };
        let (day, minute, off) = match bagholder_model::clock::local_at(&zone, t) {
            Some((d, mi, o)) => (d, mi, o),
            None => {
                let local = t + fixed;
                let days = local.div_euclid(86400);
                let rem = local.rem_euclid(86400);
                let (y, m, dd) = bagholder_model::dates::from_days(days);
                (bagholder_model::dates::fmt(y, m, dd), (rem / 60) as i64, fixed)
            }
        };
        out.push(SourceBar {
            time: t,
            day,
            minute,
            offset: off,
            px: Ohlcv { open: col("open", i), high: col("high", i), low: col("low", i), close, volume: col("volume", i) },
        });
    }
    out.sort_by_key(|b| b.time);
    out
}

/// Seconds since the epoch for an ISO instant.
fn instant_secs(s: &str) -> Option<f64> {
    if s.is_empty() {
        return None;
    }
    let t = s.replace('Z', "+00:00");
    let (d, rest) = t.split_once('T')?;
    let (y, m, day) = bagholder_model::dates::parse_iso(d)?;
    let clock: Vec<&str> = rest.split(|c| c == ':' || c == '+' || c == '-').collect();
    let hh: f64 = clock.first()?.parse().ok()?;
    let mm: f64 = clock.get(1).and_then(|x| x.parse().ok()).unwrap_or(0.0);
    let ss: f64 = clock.get(2).and_then(|x| x.split('.').next()?.parse().ok()).unwrap_or(0.0);
    Some(bagholder_model::dates::to_days(y, m, day) as f64 * 86400.0 + hh * 3600.0 + mm * 60.0 + ss)
}

pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// The market identifier code of a venue as this reader's records name it, in
/// words or as a code (`bagholder_core::listing::mic_of`, the one table), for the
/// venue rules in `bagholder_sources::venue`; a key's `US` names the US market,
/// whose every listing Yahoo writes bare, as it writes Nasdaq's.
pub fn venue_mic(exchange: &str) -> Option<&'static str> {
    bagholder_core::listing::mic_of(exchange).or_else(|| (bagholder_core::listing::key_venue(exchange) == Some(bagholder_core::listing::US)).then_some("XNAS"))
}

/// The Yahoo forms of a listing: its own venue's alone (`bagholder_sources::venue::yahoo_forms`).
/// A listing whose venue is none known here has none: a currency never stands in
/// for a venue, and another venue's form is another listing (`PLTR.TO` is
/// Palantir's Canadian depositary receipt, not the stock).
pub fn yahoo_forms(rec: &Listing) -> Vec<String> {
    venue_mic(&rec.exchange).map(|mic| bagholder_sources::venue::yahoo_forms(&rec.symbol, mic)).unwrap_or_default()
}

/// The form a listing's quote is filed under, or
/// nothing when TMX does not carry it -- crypto, options, unknown venues.
pub fn tmx_quote_symbol(symbol: &str, exchange: &str) -> Option<String> {
    let s = bagholder_model::venues::tmx_symbol(symbol);
    if s.is_empty() || s.contains(' ') {
        return None;
    }
    bagholder_model::venues::tmx_form(exchange).map(|f| format!("{}{}", s, f))
}

/// Yahoo's pace, for a caller that makes its own request: false while its rest
/// after a 429 stands; otherwise waits its turn and takes it.
pub fn yahoo_may_ask() -> bool {
    yahoo_turn().is_ok()
}

/// Yahoo turned a request away with a 429: nothing is asked of it for its rest.
pub fn yahoo_back_off() {
    bagholder_net::machine::refused_now(YAHOO_HOST, None);
}
