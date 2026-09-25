//! Daily bars for the chart.
//!
//! An instrument's bars can come from more than one place, and which place has
//! them depends on the instrument. The candidates are asked in order and the
//! first whose bars reach back to the start of the span wins; a source with a
//! late start never beats one that has the earlier days. The winner is
//! remembered for the symbol, so the next span asks it first.

use serde_json::{json, Value};
use std::collections::BTreeMap;

use crate::http::{post_json, TMX_HEADERS};
use crate::parse::{parse_coinbase_candles, parse_tmx_history};
use crate::quotes::{yahoo_forms, tmx_quote_symbol};
use crate::http::FetchError;
use crate::tmx::{tmx_lookup, tmx_lookup_try, TMX_URL};
use bagholder_model::input::Listing;
use bagholder_model::value::field_s;
use bagholder_store::bars::{day_of_epoch, Bar, ChartBars, DayBar, Ohlcv, SourceBar, TimeBar};

/// A source covers a span when its first bar is
/// within this of the span's start.
pub const COVERAGE_SLACK_DAYS: i64 = 7;

pub const HISTORY_STALE_HOURS: f64 = 20.0;

const TMX_HISTORY_QUERY: &str = "query getTimeSeriesData($symbol: String!, $freq: String, $interval: Int, $start: String, $end: String) { getTimeSeriesData(symbol: $symbol, freq: $freq, interval: $interval, start: $start, end: $end) { dateTime open high low close volume } }";

pub const COINBASE_CANDLES_URL: &str = "https://api.exchange.coinbase.com/products/{}/candles?granularity={}&start={}&end={}";
pub const COINBASE_PRODUCT_URL: &str = "https://api.exchange.coinbase.com/products/{}";
pub const YAHOO_CHART_RANGE_URL: &str = "https://query1.finance.yahoo.com/v8/finance/chart/{}?period1={}&period2={}&interval={}";

/// Where an instrument's bars can come from, in
/// order of preference.
///
/// Shares and ETFs: TMX Money under the venue's form, then Yahoo under each
/// venue suffix of the currency. Crypto: the Coinbase market and the Yahoo
/// pair in the position's own currency, then the USD market and pair, which
/// are converted at the Bank of Canada rate. Nothing for an option contract
/// itself -- the chart maps it to its underlying first.
pub fn history_candidates(rec: &Listing) -> Vec<(String, String)> {
    let kind = { let k = rec.kind.clone(); if k.is_empty() { "Shares".to_string() } else { k } };
    let sym = bagholder_model::venues::tmx_symbol(&rec.symbol);
    let ccy = { let c = rec.currency.clone(); if c.is_empty() { "CAD".to_string() } else { c.trim().to_uppercase() } };
    if sym.is_empty() {
        return vec![];
    }
    if kind == "Crypto" {
        let mut out = vec![
            ("coinbase".to_string(), format!("{}-{}", sym, ccy)),
            ("yahoo".to_string(), format!("{}-{}", sym, ccy)),
        ];
        if ccy != "USD" {
            out.push(("coinbase".to_string(), format!("{}-USD", sym)));
            out.push(("yahoo".to_string(), format!("{}-USD", sym)));
        }
        return out;
    }
    if kind != "Shares" {
        return vec![];
    }
    let mut out = Vec::new();
    if let Some(k) = tmx_quote_symbol(&rec.symbol, &rec.exchange, &ccy) {
        out.push(("tmx".to_string(), k));
    }
    out.extend(yahoo_forms(rec).into_iter().map(|f| ("yahoo".to_string(), f)));
    out
}

fn bars_meta_key(rec: &Listing) -> String {
    format!("bars_source:{}", bagholder_model::venues::tmx_symbol(&rec.symbol))
}

/// The candidates with the remembered winner
/// first.
pub fn ordered_candidates(conn: &rusqlite::Connection, rec: &Listing) -> Vec<(String, String)> {
    let cands = history_candidates(rec);
    if cands.is_empty() {
        return cands;
    }
    let v = bagholder_store::tables::get_meta(conn, &bars_meta_key(rec), "").unwrap_or_default();
    if let Some((s, k)) = v.split_once('|') {
        let win = (s.to_string(), k.to_string());
        if cands.contains(&win) {
            let mut out = vec![win.clone()];
            out.extend(cands.into_iter().filter(|c| *c != win));
            return out;
        }
    }
    cands
}

fn remember_winner(conn: &rusqlite::Connection, rec: &Listing, source: &str, key: &str) {
    let _ = bagholder_store::tables::set_meta(conn, &bars_meta_key(rec), &format!("{}|{}", source, key));
}

/// The currency a candidate's bars are quoted in -- a
/// crypto pair's quote currency, otherwise the listing's own.
pub fn bar_currency(source_kind: &str, key: &str, rec: &Listing) -> String {
    let _ = source_kind;
    if rec.kind.clone() == "Crypto" {
        if let Some((_, q)) = key.split_once('-') {
            return q.to_string();
        }
    }
    let c = rec.currency.clone();
    if c.is_empty() { "CAD".into() } else { c.to_uppercase() }
}

/// The published rate for a day, or the most
/// recent one within a week.
fn rate_on_or_before(fx: &BTreeMap<String, f64>, day: &str, days: i64) -> Option<f64> {
    for i in 0..days {
        let d = bagholder_model::dates::shift_date(day, -i);
        if let Some(r) = fx.get(&d) {
            if *r > 0.0 {
                return Some(*r);
            }
        }
    }
    None
}

/// Bars in the position's currency, with the
/// rates read from the store.
///
/// USD bars become CAD at the Bank of Canada rate of the bar's own day. A bar
/// whose day has no published rate within a week is dropped, never guessed;
/// anything else cannot be converted and yields nothing at all.
pub fn in_position_currency<B: Bar + Clone>(conn: &rusqlite::Connection, bars: &[B], quoted_in: &str, currency: &str) -> Vec<B> {
    let quote = quoted_in.to_uppercase();
    let ccy = { let c = currency; if c.is_empty() { "CAD".to_string() } else { c.to_uppercase() } };
    if quote == ccy {
        return bars.to_vec();
    }
    if !(quote == "USD" && ccy == "CAD") {
        return vec![];
    }
    let fx = read_fx(conn).unwrap_or_default();
    in_position_currency_with(bars, quoted_in, currency, &fx)
}

/// The same, with the rates the caller has already read.
pub fn in_position_currency_with<B: Bar + Clone>(bars: &[B], quoted_in: &str, currency: &str, fx: &BTreeMap<String, f64>) -> Vec<B> {
    let quote = quoted_in.to_uppercase();
    let ccy = { let c = currency; if c.is_empty() { "CAD".to_string() } else { c.to_uppercase() } };
    if quote == ccy {
        return bars.to_vec();
    }
    if !(quote == "USD" && ccy == "CAD") {
        return vec![];
    }
    let mut out = Vec::new();
    for b in bars {
        let day = b.day();
        let rate = match rate_on_or_before(fx, &day, 7) { Some(r) => r, None => continue };
        let mut nb = b.clone();
        let px = nb.px_mut();
        if let Some(v) = px.open.as_mut() { *v *= rate; }
        if let Some(v) = px.high.as_mut() { *v *= rate; }
        if let Some(v) = px.low.as_mut() { *v *= rate; }
        px.close *= rate;
        out.push(nb);
    }
    out
}

fn epoch_of_day(day: &str) -> i64 {
    match bagholder_model::dates::parse_iso(day) {
        Some((y, m, d)) => bagholder_model::dates::to_days(y, m, d) * 86400,
        None => 0,
    }
}

/// Only bars with an open, a high and a low -- the chart
/// draws candlesticks or nothing.
fn whole_bars<B: Bar>(bars: Vec<B>) -> Vec<B> {
    bars.into_iter().filter(|b| { let px = b.px(); px.open.is_some() && px.high.is_some() && px.low.is_some() }).collect()
}

/// The Coinbase Exchange market for a pair, or
/// nothing when it does not trade there. Remembered; a miss for a day.
pub fn coinbase_market(conn: &rusqlite::Connection, pair: &str, today: &str) -> String {
    let pair = pair.trim().to_uppercase();
    if !pair.contains('-') {
        return String::new();
    }
    let meta_key = format!("coinbase_product:{}", pair);
    let v = bagholder_store::tables::get_meta(conn, &meta_key, "").unwrap_or_default();
    if let Some(rest) = v.strip_prefix('@') {
        return rest.to_string();
    }
    if let Some(when) = v.strip_prefix("none@") {
        if when > bagholder_model::dates::shift_date(today, -crate::tmx::RESOLVE_RETRY_DAYS).as_str() {
            return String::new();
        }
    }
    let url = COINBASE_PRODUCT_URL.replace("{}", &pair);
    let id = match crate::http::get_text(&url, &[("User-Agent", crate::http::UA), ("Accept", "text/csv,application/json,*/*;q=0.8")]) {
        Ok(t) => serde_json::from_str::<Value>(if t.is_empty() { "{}" } else { &t }).ok().map(|d| field_s(&d, "id").to_uppercase()).unwrap_or_default(),
        Err(_) => String::new(),
    };
    if id == pair {
        let _ = bagholder_store::tables::set_meta(conn, &meta_key, &format!("@{}", pair));
        return pair;
    }
    let _ = bagholder_store::tables::set_meta(conn, &meta_key, &format!("none@{}", today));
    String::new()
}

pub const COINBASE_CANDLE_LIMIT: i64 = 300;

/// Candles of `granularity` seconds over the
/// span, three hundred at a time, a few spans in parallel.
pub fn fetch_coinbase_candles(product: &str, granularity: i64, start_ts: i64, end_ts: i64) -> Vec<TimeBar> {
    let span = COINBASE_CANDLE_LIMIT * granularity;
    let mut chunks: Vec<(i64, i64)> = Vec::new();
    let mut cur = start_ts.div_euclid(granularity) * granularity;
    while cur < end_ts {
        chunks.push((cur, (cur + span).min(end_ts)));
        cur += span;
    }
    let one = |c: (i64, i64)| -> Vec<TimeBar> {
        let url = COINBASE_CANDLES_URL
            .replacen("{}", product, 1)
            .replacen("{}", &granularity.to_string(), 1)
            .replacen("{}", &iso_instant(c.0), 1)
            .replacen("{}", &iso_instant(c.1), 1);
        match crate::http::get_text(&url, &[]) {
            Ok(t) => parse_coinbase_candles(&t),
            Err(_) => vec![],
        }
    };
    let answers = parallel(chunks, 4, one);
    let mut by_time: BTreeMap<i64, TimeBar> = BTreeMap::new();
    for bars in answers {
        for b in bars {
            by_time.insert(b.time, b);
        }
    }
    by_time.into_values().collect()
}

/// `ThreadPoolExecutor(max_workers).map`: the answers in the order asked.
fn parallel<T: Send + Sync + Clone, R: Send, F: Fn(T) -> R + Sync>(items: Vec<T>, workers: usize, f: F) -> Vec<R> {
    let n = items.len();
    let mut out: Vec<Option<R>> = (0..n).map(|_| None).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let slots = std::sync::Mutex::new(&mut out);
    std::thread::scope(|s| {
        for _ in 0..workers.min(n.max(1)) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if i >= n {
                    return;
                }
                let r = f(items[i].clone());
                slots.lock().unwrap()[i] = Some(r);
            });
        }
    });
    out.into_iter().map(|r| r.expect("every item answered")).collect()
}

fn iso_instant(ts: i64) -> String {
    let days = ts.div_euclid(86400);
    let rem = ts.rem_euclid(86400);
    let (y, m, d) = bagholder_model::dates::from_days(days);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, rem / 3600, (rem % 3600) / 60, rem % 60)
}

/// A symbol Yahoo says it does not carry is remembered
/// for the day and not asked again; any other failure is the chain's to
/// record.
pub fn fetch_yahoo(conn: &rusqlite::Connection, symbol: &str, start_ts: i64, end_ts: i64, interval: &str, today: &str) -> Result<Vec<SourceBar>, FetchError> {
    let miss_key = format!("yahoo_miss:{}", symbol);
    if bagholder_store::tables::get_meta(conn, &miss_key, "").unwrap_or_default() == today {
        return Ok(vec![]);
    }
    let url = YAHOO_CHART_RANGE_URL
        .replacen("{}", symbol, 1)
        .replacen("{}", &start_ts.to_string(), 1)
        .replacen("{}", &end_ts.to_string(), 1)
        .replacen("{}", interval, 1);
    match crate::quotes::yahoo_get_result(&url) {
        Ok(text) => Ok(crate::quotes::parse_yahoo_chart(&text)),
        Err(e) if e.code() == Some(404) => {
            let _ = bagholder_store::tables::set_meta(conn, &miss_key, today);
            Ok(vec![])
        }
        Err(e) => Err(e),
    }
}

/// One candidate's daily bars over a span, oldest
/// first. A failure is returned, so the chain can say what failed.
pub fn fetch_daily_from(
    conn: &rusqlite::Connection,
    source: &str,
    key: &str,
    rec: &Listing,
    start: &str,
    end: &str,
    today: &str,
) -> Result<Vec<DayBar>, FetchError> {
    let start_ts = epoch_of_day(start);
    let end_ts = epoch_of_day(end) + 86400;
    match source {
        "tmx" => {
            let daily = |form: &str| -> Result<Option<Vec<DayBar>>, FetchError> {
                let payload = json!({
                    "operationName": "getTimeSeriesData",
                    "variables": {"symbol": form, "freq": "day", "interval": 1, "start": start, "end": end},
                    "query": TMX_HISTORY_QUERY,
                });
                let data = post_json(TMX_URL, &payload, &TMX_HEADERS)?;
                let bars = parse_tmx_history(&data);
                Ok(if bars.is_empty() { None } else { Some(bars) })
            };
            let got = tmx_lookup_try(conn, key, today, daily)?.0;
            Ok(whole_bars(got.unwrap_or_default()))
        }
        "coinbase" => {
            if coinbase_market(conn, key, today).is_empty() {
                return Ok(vec![]);
            }
            let days: Vec<DayBar> = fetch_coinbase_candles(key, 86400, start_ts, end_ts).into_iter().map(|b| DayBar { date: b.day(), px: b.px }).collect();
            Ok(in_position_currency(conn, &days, &bar_currency(source, key, rec), &rec.currency))
        }
        "yahoo" => {
            let days: Vec<DayBar> = fetch_yahoo(conn, key, start_ts, end_ts, "1d", today)?.into_iter().map(|b| b.on_day()).collect();
            Ok(in_position_currency(conn, &whole_bars(days), &bar_currency(source, key, rec), &rec.currency))
        }
        _ => Ok(vec![]),
    }
}

/// The index of the first answer whose bars reach
/// back to the span's start, else the one reaching furthest back. The winner
/// is remembered.
fn pick_covering<B, F: Fn(&B) -> i64>(
    conn: &rusqlite::Connection,
    rec: &Listing,
    answers: &[(String, String, Vec<B>)],
    span_start: i64,
    first_of: F,
) -> Option<usize> {
    let slack = COVERAGE_SLACK_DAYS * 86400;
    let mut best: Option<(i64, usize)> = None;
    for (i, (source, key, bars)) in answers.iter().enumerate() {
        if bars.is_empty() {
            continue;
        }
        let first = first_of(&bars[0]);
        if first <= span_start + slack {
            remember_winner(conn, rec, source, key);
            return Some(i);
        }
        if best.map(|(f, _)| first < f).unwrap_or(true) {
            best = Some((first, i));
        }
    }
    let (_, i) = best?;
    remember_winner(conn, rec, &answers[i].0, &answers[i].1);
    Some(i)
}

/// What one candidate did on the last chain that came back empty: its bar
/// count, or its failure.
#[derive(Clone)]
enum Note {
    Bars,
    Failed(FetchError),
}

fn chart_notes() -> &'static std::sync::Mutex<std::collections::HashMap<(String, &'static str), Vec<(String, String, Note)>>> {
    static N: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<(String, &'static str), Vec<(String, String, Note)>>>> = std::sync::OnceLock::new();
    N.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn remember_notes(rec: &Listing, which: &'static str, notes: Vec<(String, String, Note)>) {
    let sym = bagholder_model::venues::tmx_symbol(&rec.symbol);
    chart_notes().lock().unwrap().insert((sym, which), notes);
}

/// Why a chart has no bars, in one sentence -- what
/// failed, or which sources were asked and had none.
pub fn chart_reason(rec: &Listing, tf: &str) -> String {
    let which: &'static str = if INTRADAY.contains(&tf) { "hourly" } else { "daily" };
    let sym = bagholder_model::venues::tmx_symbol(&rec.symbol);
    let notes = chart_notes().lock().unwrap().get(&(sym, which)).cloned().unwrap_or_default();
    let label = |s: &str| -> String {
        crate::http::SOURCE_LABELS.iter().find(|(k, _)| *k == s).map(|(_, v)| v.to_string()).unwrap_or_else(|| s.to_string())
    };
    let mut failed: Vec<String> = Vec::new();
    for (source, _, outcome) in &notes {
        if let Note::Failed(e) = outcome {
            let line = format!("{} {}", label(source), crate::http::describe_failure(e));
            if !failed.contains(&line) {
                failed.push(line);
            }
        }
    }
    if !failed.is_empty() {
        return format!("{}.", failed.join("; "));
    }
    let mut names: Vec<String> = Vec::new();
    for (source, _) in history_candidates(rec) {
        let n = label(&source);
        if !names.contains(&n) {
            names.push(n);
        }
    }
    if names.is_empty() {
        return "No price source covers this instrument.".into();
    }
    let listed = if names.len() <= 2 {
        names.join(" or ")
    } else {
        format!("{} or {}", names[..names.len() - 1].join(", "), names[names.len() - 1])
    };
    format!("No bars for this span from {}.", listed)
}

/// The chain, stopping as soon as one candidate covers
/// the span.
pub fn fetch_history(conn: &rusqlite::Connection, rec: &Listing, start: &str, end: &str, today: &str) -> (Vec<DayBar>, String) {
    let span_start = epoch_of_day(start);
    let mut answers: Vec<(String, String, Vec<DayBar>)> = Vec::new();
    let mut notes: Vec<(String, String, Note)> = Vec::new();
    for (source, key) in ordered_candidates(conn, rec) {
        let bars = match fetch_daily_from(conn, &source, &key, rec, start, end, today) {
            Ok(b) => {
                notes.push((source.clone(), key.clone(), Note::Bars));
                b
            }
            Err(e) => {
                notes.push((source.clone(), key.clone(), Note::Failed(e)));
                vec![]
            }
        };
        let covered = bars.first().map(|b| epoch_of_day(&b.date) <= span_start + COVERAGE_SLACK_DAYS * 86400).unwrap_or(false);
        answers.push((source, key, bars));
        if covered {
            break;
        }
    }
    match pick_covering(conn, rec, &answers, span_start, |b: &DayBar| epoch_of_day(&b.date)) {
        Some(i) => (answers[i].2.clone(), answers[i].0.clone()),
        None => {
            remember_notes(rec, "daily", notes);
            (vec![], String::new())
        }
    }
}

/// Whether a span's daily bars are due a read: never read from its start, or
/// stale where the span reaches the present, and not a read that came back empty
/// a few minutes ago (then it waits, as an intraday miss does).
pub fn daily_due(conn: &rusqlite::Connection, rec: &Listing, start: &str, end: &str, today: &str, now_unix: f64) -> bool {
    let sym = bagholder_model::venues::tmx_symbol(&rec.symbol);
    let start: String = start.chars().take(10).collect();
    let end: String = end.chars().take(10).collect();
    if sym.is_empty() || start.len() != 10 || end.len() != 10 || intraday_missed_recently(conn, &sym, "1d", now_unix) {
        return false;
    }
    let last = bagholder_store::market::history_fetch(conn, &sym).unwrap_or(None);
    let covered = last.as_ref().map_or(false, |l| l.start <= start);
    let fresh = last
        .as_ref()
        .and_then(|l| crate::quotes::instant_secs_public(&l.fetched_at))
        .map_or(false, |then| (now_unix - then) < HISTORY_STALE_HOURS * 3600.0);
    !covered || (end >= bagholder_model::dates::shift_date(today, -3) && !fresh)
}

/// Read a span's daily bars from the sources and store them; a read that finds
/// nothing is remembered so the next few minutes do not ask again.
fn fill_daily(conn: &rusqlite::Connection, rec: &Listing, start: &str, today: &str, now_stamp: &str) -> rusqlite::Result<()> {
    let sym = bagholder_model::venues::tmx_symbol(&rec.symbol);
    let start: String = start.chars().take(10).collect();
    let last_start = bagholder_store::market::history_fetch(conn, &sym)?.map(|l| l.start).unwrap_or_default();
    // a span already read from an earlier day is read again from that day, so the stamp stays true
    let fetch_from = if !last_start.is_empty() && last_start < start { last_start } else { start };
    let (bars, source) = fetch_history(conn, rec, &fetch_from, today, today);
    if bars.is_empty() {
        record_intraday_miss(conn, &sym, "1d", now_stamp);
        return Ok(());
    }
    bagholder_store::market::upsert_price_history(conn, &sym, &bars, &source)?;
    // the stamp says what is covered: when the bars begin well after the day
    // asked for, only from their first day, so an earlier span asks again
    let got_from = bars[0].date.clone();
    let slack = bagholder_model::dates::shift_date(&fetch_from, COVERAGE_SLACK_DAYS);
    let covered_from = if got_from <= slack { fetch_from } else { got_from };
    bagholder_store::market::mark_history_fetched(conn, &sym, &covered_from, now_stamp)
}

/// The stored bars for a span, fetching first when it was never fetched or the
/// copy is stale and the span reaches the present. What a background job calls;
/// a chart asked for by a page reads what is stored and has the read done in the
/// background (`ensure_daily_in_background`).
pub fn ensure_history(
    conn: &rusqlite::Connection,
    rec: &Listing,
    start: &str,
    end: &str,
    today: &str,
    now_unix: f64,
    now_stamp: &str,
) -> rusqlite::Result<Vec<DayBar>> {
    if daily_due(conn, rec, start, end, today, now_unix) {
        fill_daily(conn, rec, start, today, now_stamp)?;
    }
    stored_daily(conn, rec, start, end)
}

/// The daily bars stored for a span, asking nothing.
pub fn stored_daily(conn: &rusqlite::Connection, rec: &Listing, start: &str, end: &str) -> rusqlite::Result<Vec<DayBar>> {
    let sym = bagholder_model::venues::tmx_symbol(&rec.symbol);
    let start: String = start.chars().take(10).collect();
    let end: String = end.chars().take(10).collect();
    if sym.is_empty() || start.len() != 10 || end.len() != 10 {
        return Ok(vec![]);
    }
    bagholder_store::market::price_history(conn, &sym, &start, &end)
}

fn daily_pending_set() -> &'static std::sync::Mutex<Vec<String>> {
    static PENDING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    &PENDING
}

/// Whether a daily read for this listing is under way now.
pub fn daily_pending(rec: &Listing) -> bool {
    let sym = bagholder_model::venues::tmx_symbol(&rec.symbol);
    daily_pending_set().lock().unwrap_or_else(|e| e.into_inner()).contains(&sym)
}

/// Start the daily read for a span, once per listing at a time, and return at
/// once; `done` is called when it ends, whatever it found, so whoever showed the
/// stored bars meanwhile is told to look again.
pub fn ensure_daily_in_background(pool: std::sync::Arc<bagholder_store::pool::Pool>, rec: Listing, start: String, done: impl FnOnce() + Send + 'static) {
    let sym = bagholder_model::venues::tmx_symbol(&rec.symbol);
    {
        let mut p = daily_pending_set().lock().unwrap_or_else(|e| e.into_inner());
        if p.contains(&sym) {
            return;
        }
        p.push(sym.clone());
    }
    let left = sym.clone();
    let run = move || {
        if let Ok(conn) = pool.get() {
            let (today, _, stamp) = crate::clock_now();
            let _ = fill_daily(&conn, &rec, &start, &today, &stamp);
        }
        daily_pending_set().lock().unwrap_or_else(|e| e.into_inner()).retain(|s| *s != sym);
        done();
    };
    if std::thread::Builder::new().name("bagholder-daily".into()).spawn(run).is_err() {
        // no thread to read on: nothing is under way, so nothing is left pending
        daily_pending_set().lock().unwrap_or_else(|e| e.into_inner()).retain(|s| *s != left);
    }
}

fn read_fx(conn: &rusqlite::Connection) -> rusqlite::Result<BTreeMap<String, f64>> {
    bagholder_store::tables::fx_rates(conn, bagholder_store::tables::FX_PAIR)
}

/// Weekly (Monday start) or monthly bars from daily
/// ones -- the period's first open, highest high, lowest low, last close and
/// summed volume.
pub fn aggregate_daily(bars: &[DayBar], tf: &str) -> Vec<DayBar> {
    let mut out: Vec<DayBar> = Vec::new();
    let mut cur_key = String::new();
    for b in bars {
        let (y, m, d) = match bagholder_model::dates::parse_iso(&b.date) { Some(p) => p, None => continue };
        let key = if tf == "1w" {
            let days = bagholder_model::dates::to_days(y, m, d);
            // Monday is 0: 1970-01-01 was a Thursday
            let weekday = ((days + 3) % 7 + 7) % 7;
            let (wy, wm, wd) = bagholder_model::dates::from_days(days - weekday);
            bagholder_model::dates::fmt(wy, wm, wd)
        } else {
            bagholder_model::dates::fmt(y, m, 1)
        };
        if out.is_empty() || cur_key != key {
            cur_key = key.clone();
            out.push(DayBar { date: key, px: b.px });
            continue;
        }
        let cur = out.last_mut().unwrap();
        cur.px.close = b.px.close;
        if let Some(h) = b.px.high {
            cur.px.high = Some(match cur.px.high { Some(x) => x.max(h), None => h });
        }
        if let Some(l) = b.px.low {
            cur.px.low = Some(match cur.px.low { Some(x) => x.min(l), None => l });
        }
        if let Some(v) = b.px.volume {
            let now = cur.px.volume.unwrap_or(0.0);
            cur.px.volume = Some(now + v);
        }
    }
    out
}

pub const TIMEFRAMES: [&str; 5] = ["1h", "4h", "1d", "1w", "1M"];
/// The two timeframes that need minute data.
pub const INTRADAY: [&str; 2] = ["1h", "4h"];
pub const INTRADAY_SECONDS: [(&str, i64); 2] = [("1h", 3600), ("4h", 14400)];

/// What the chart draws for an instrument -- itself,
/// or for an option contract its underlying, since no source keeps contract
/// history.
pub fn chart_instrument(rec: &Listing) -> Listing {
    if rec.kind == "Options" {
        let under = bagholder_model::symbols::underlying_symbol(&rec.symbol);
        if !under.is_empty() && under != "—" {
            let ccy = if rec.currency.is_empty() { "USD" } else { &rec.currency };
            return Listing::new(under, rec.exchange.clone(), ccy, "Shares");
        }
    }
    rec.clone()
}

/// The preferred source for an instrument's bars.
pub fn history_source(rec: &Listing) -> Option<(String, String)> {
    history_candidates(rec).into_iter().next()
}

// --------------------------------------------------------------------------
// intraday bars
// --------------------------------------------------------------------------

pub const TMX_CHART_QUERY: &str = "query getCompanyChart($symbol: String!, $from: String!, $to: String!) { intraday: getChartDataBySymbol(symbol: $symbol, fromDate: $from, toDate: $to) { dateTime open high low close volume } }";
/// 9:30 exchange time.
pub const SESSION_OPEN_MINUTES: i64 = 9 * 60 + 30;
pub const TMX_INTRADAY_DAYS: i64 = 365;
pub const YAHOO_INTRADAY_DAYS: i64 = 729;
pub const COINBASE_EXCHANGE_START: &str = "2015-01-01";
/// Rate-limited: asked for a chart someone opens, never by the background
/// sweep.
pub const ON_DEMAND_ONLY_SOURCES: [&str; 1] = ["yahoo"];
pub const INTRADAY_RETRY_MINUTES: f64 = 10.0;
pub const ARCHIVE_BATCH: usize = 12;
pub const ARCHIVE_TOPUP_HOURS: f64 = 20.0;
/// No daily source forgets its bars today; one that does goes here.
pub const SHORT_DAILY_SOURCES: [&str; 0] = [];

/// (epoch, day, minute of day, offset) from
/// `2026-09-02T09:30:00-04:00`, None where it is not a time.
pub fn minute_stamp(text: &str) -> Option<(i64, String, i64, i64)> {
    let b = text.as_bytes();
    if b.len() == 25 && b[4] == b'-' && b[10] == b'T' && b[13] == b':' && b[22] == b':' && (b[19] == b'+' || b[19] == b'-') {
        let int = |s: &str| bagholder_model::textrules::parse_int(s);
        if let (Some(hh), Some(mm), Some(ss), Some(oh), Some(om)) = (int(&text[11..13]), int(&text[14..16]), int(&text[17..19]), int(&text[20..22]), int(&text[23..25])) {
            let day = &text[..10];
            if let Some((y, m, d)) = bagholder_model::dates::parse_iso(day) {
                let minute = hh * 60 + mm;
                let offset = (oh * 3600 + om) * if b[19] == b'-' { -1 } else { 1 };
                let epoch = bagholder_model::dates::to_days(y, m, d) * 86400 + minute * 60 + ss - offset;
                return Some((epoch, day.to_string(), minute, offset));
            }
        }
    }
    // the long way round: an ISO time with or without its offset
    let (d, t) = text.split_once('T').or_else(|| text.split_once(' '))?;
    let (y, m, dd) = bagholder_model::dates::parse_iso(d)?;
    let mut offset = 0i64;
    let mut clock = t;
    let mut naive = true;
    if let Some(pos) = t.rfind(['+', '-']) {
        if pos > 0 {
            let sign = if t.as_bytes()[pos] == b'-' { -1 } else { 1 };
            let off = &t[pos + 1..];
            let (oh, om) = off.split_once(':').unwrap_or((off, "0"));
            offset = sign * (oh.parse::<i64>().ok()? * 3600 + om.parse::<i64>().ok()? * 60);
            clock = &t[..pos];
            naive = false;
        }
    } else if let Some(stripped) = t.strip_suffix('Z') {
        clock = stripped;
        naive = false;
    }
    let parts: Vec<&str> = clock.split(':').collect();
    let hh: i64 = parts.first()?.parse().ok()?;
    let mm: i64 = parts.get(1).map(|x| x.parse().ok()).unwrap_or(Some(0))?;
    let ss: i64 = parts.get(2).map(|x| x.split('.').next().unwrap_or("0").parse().ok()).unwrap_or(Some(0))?;
    let wall = bagholder_model::dates::to_days(y, m, dd) * 86400 + hh * 3600 + mm * 60 + ss;
    // a time with no offset is the exchange's own wall clock (TMX: Toronto),
    // never the machine's, which can be anywhere
    let offset = if naive { bagholder_model::clock::offset_for_wall(TMX_ZONE, wall)? } else { offset };
    Some((wall - offset, bagholder_model::dates::fmt(y, m, dd), hh * 60 + mm, offset))
}

/// TMX's exchange zone: a chart time it sends without an offset is read here.
const TMX_ZONE: &str = "America/Toronto";

fn opt_num(v: Option<&Value>) -> Option<f64> {
    match v {
        None | Some(Value::Null) => None,
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) if s.is_empty() => None,
        Some(Value::String(s)) => bagholder_model::textrules::parse_float(s),
        Some(Value::Bool(b)) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// One-minute bars from TMX's chart feed, with the
/// exchange-local minute of day, oldest first.
pub fn parse_tmx_minutes(data: &Value) -> Vec<SourceBar> {
    let rows = data.get("data").and_then(|d| d.get("intraday")).and_then(|r| r.as_array()).cloned().unwrap_or_default();
    let mut out: Vec<SourceBar> = Vec::new();
    for r in rows {
        let when = field_s(&r, "dateTime");
        if !r.is_object() || when.is_empty() {
            continue;
        }
        let (stamp, day, minute, offset) = match minute_stamp(&when) { Some(s) => s, None => continue };
        let close = match opt_num(r.get("close")) { Some(c) if c > 0.0 => c, _ => continue };
        out.push(SourceBar {
            time: stamp,
            day,
            minute,
            offset,
            px: Ohlcv { open: opt_num(r.get("open")), high: opt_num(r.get("high")), low: opt_num(r.get("low")), close, volume: opt_num(r.get("volume")) },
        });
    }
    out.sort_by_key(|b| b.time);
    out
}

/// Bars of `bucket_minutes` aligned to the session
/// open, the bucket's first open, highest high, lowest low, last close and
/// summed volume; the bar's time is the bucket's start.
pub fn aggregate_session(minutes: &[SourceBar], bucket_minutes: i64) -> Vec<TimeBar> {
    let mut order: Vec<(String, i64)> = Vec::new();
    let mut out: std::collections::HashMap<(String, i64), TimeBar> = std::collections::HashMap::new();
    for m in minutes {
        let rel = (m.minute - SESSION_OPEN_MINUTES).max(0);
        let idx = rel.div_euclid(bucket_minutes);
        let start_minute = SESSION_OPEN_MINUTES + idx * bucket_minutes;
        let key = (m.day.clone(), idx);
        let close = m.px.close;
        let or_close = |v: Option<f64>| v.unwrap_or(close);
        match out.get_mut(&key) {
            None => {
                let start = epoch_of_day(&m.day) + start_minute * 60 - m.offset;
                out.insert(
                    key.clone(),
                    TimeBar {
                        time: start,
                        px: Ohlcv {
                            open: Some(or_close(m.px.open)),
                            high: Some(or_close(m.px.high)),
                            low: Some(or_close(m.px.low)),
                            close,
                            volume: Some(m.px.volume.unwrap_or(0.0)),
                        },
                    },
                );
                order.push(key);
            }
            Some(b) => {
                b.px.close = close;
                if let Some(h) = m.px.high {
                    if h > b.px.high.unwrap_or(f64::MIN) {
                        b.px.high = Some(h);
                    }
                }
                if let Some(l) = m.px.low {
                    if l < b.px.low.unwrap_or(f64::MAX) {
                        b.px.low = Some(l);
                    }
                }
                b.px.volume = Some(b.px.volume.unwrap_or(0.0) + m.px.volume.unwrap_or(0.0));
            }
        }
    }
    let mut rows: Vec<TimeBar> = order.into_iter().map(|k| out.remove(&k).unwrap()).collect();
    rows.sort_by_key(|b| b.time);
    rows
}

/// One-minute bars over [start, end], a month at a
/// time, a few months in parallel. A month that fails is a month with none.
/// The gap the archive leaves between months it asks TMX for. A chart someone is
/// waiting on is not paced; the archive's own backfill has all day.
pub const ARCHIVE_TMX_GAP: std::time::Duration = std::time::Duration::from_millis(250);

pub fn fetch_tmx_minutes(key: &str, start: &str, end: &str, on_demand: bool) -> Vec<SourceBar> {
    let mut chunks: Vec<(String, String)> = Vec::new();
    let mut cur = match bagholder_model::dates::parse_iso(&start.chars().take(10).collect::<String>()) { Some(d) => d, None => return vec![] };
    let last = match bagholder_model::dates::parse_iso(&end.chars().take(10).collect::<String>()) { Some(d) => d, None => return vec![] };
    let day_n = |d: (i64, u32, u32)| bagholder_model::dates::to_days(d.0, d.1, d.2);
    while day_n(cur) <= day_n(last) {
        let month_end = (cur.0, cur.1, bagholder_model::dates::days_in_month(cur.0, cur.1));
        let stop = if day_n(month_end) < day_n(last) { month_end } else { last };
        chunks.push((bagholder_model::dates::fmt(cur.0, cur.1, cur.2), bagholder_model::dates::fmt(stop.0, stop.1, stop.2)));
        cur = bagholder_model::dates::from_days(day_n(stop) + 1);
    }
    let one = |span: (String, String)| -> Vec<SourceBar> {
        if !on_demand {
            crate::http::pace_host("app-money.tmx.com", ARCHIVE_TMX_GAP);
        }
        let payload = json!({"operationName": "getCompanyChart", "variables": {"symbol": key, "from": span.0, "to": span.1}, "query": TMX_CHART_QUERY});
        match post_json(TMX_URL, &payload, &TMX_HEADERS) {
            Ok(d) => parse_tmx_minutes(&d),
            Err(_) => vec![],
        }
    };
    let mut out: Vec<SourceBar> = parallel(chunks, if on_demand { 4 } else { 1 }, one).into_iter().flatten().collect();
    out.sort_by_key(|b| b.time);
    out
}

/// Hourly bars onto a coarser grid aligned to the
/// clock.
pub fn aggregate_hourly(bars: &[TimeBar], seconds: i64) -> Vec<TimeBar> {
    let mut out: BTreeMap<i64, TimeBar> = BTreeMap::new();
    for b in bars {
        let k = b.time.div_euclid(seconds) * seconds;
        let close = b.px.close;
        let pick = |v: Option<f64>| v.unwrap_or(close);
        let (hi, lo) = (pick(b.px.high), pick(b.px.low));
        match out.get_mut(&k) {
            None => {
                out.insert(
                    k,
                    TimeBar { time: k, px: Ohlcv { open: Some(pick(b.px.open)), high: Some(hi), low: Some(lo), close, volume: Some(b.px.volume.unwrap_or(0.0)) } },
                );
            }
            Some(cur) => {
                if hi > cur.px.high.unwrap_or(f64::MIN) {
                    cur.px.high = Some(hi);
                }
                if lo < cur.px.low.unwrap_or(f64::MAX) {
                    cur.px.low = Some(lo);
                }
                cur.px.close = close;
                cur.px.volume = Some(cur.px.volume.unwrap_or(0.0) + b.px.volume.unwrap_or(0.0));
            }
        }
    }
    out.into_values().collect()
}

/// The earliest date a source has intraday bars
/// for.
pub fn source_intraday_reach(source: &str, today: &str) -> String {
    match source {
        "coinbase" => COINBASE_EXCHANGE_START.to_string(),
        "tmx" => bagholder_model::dates::shift_date(today, -TMX_INTRADAY_DAYS),
        "yahoo" => bagholder_model::dates::shift_date(today, -YAHOO_INTRADAY_DAYS),
        _ => String::new(),
    }
}

/// The earliest date intraday bars exist for across
/// the instrument's sources, or "".
pub fn intraday_reach(rec: &Listing, today: &str) -> String {
    history_candidates(rec)
        .iter()
        .map(|(s, _)| source_intraday_reach(s, today))
        .filter(|r| !r.is_empty())
        .min()
        .unwrap_or_default()
}

/// What the chart can show for a trade starting
/// on `start`.
pub fn available_timeframes(rec: &Listing, start: &str, today: &str) -> Vec<&'static str> {
    if history_candidates(rec).is_empty() {
        return vec![];
    }
    let mut out = Vec::new();
    let reach = intraday_reach(rec, today);
    let start10: String = start.chars().take(10).collect();
    if !reach.is_empty() && start10 >= reach {
        out.extend(["1h", "4h"]);
    }
    out.extend(["1d", "1w", "1M"]);
    out
}

fn miss_key(symbol: &str, tf: &str) -> String {
    format!("bars_miss:{}|{}", bagholder_model::venues::tmx_symbol(symbol), tf)
}

pub fn record_intraday_miss(conn: &rusqlite::Connection, symbol: &str, tf: &str, now_stamp: &str) {
    let _ = bagholder_store::tables::set_meta(conn, &miss_key(symbol, tf), now_stamp);
}

pub fn intraday_missed_recently(conn: &rusqlite::Connection, symbol: &str, tf: &str, now_unix: f64) -> bool {
    let v = bagholder_store::tables::get_meta(conn, &miss_key(symbol, tf), "").unwrap_or_default();
    if v.is_empty() {
        return false;
    }
    match crate::quotes::instant_secs_public(&v) {
        Some(then) => now_unix - then < INTRADAY_RETRY_MINUTES * 60.0,
        None => false,
    }
}

/// The available timeframes less an intraday one
/// a recent fetch could not supply and nothing is stored for.
pub fn offered_timeframes(conn: &rusqlite::Connection, rec: &Listing, start: &str, today: &str, now_unix: f64) -> Vec<&'static str> {
    let sym = bagholder_model::venues::tmx_symbol(&rec.symbol);
    available_timeframes(rec, start, today)
        .into_iter()
        .filter(|tf| {
            !INTRADAY.contains(tf)
                || !intraday_missed_recently(conn, &sym, tf, now_unix)
                || !bagholder_store::market::price_bars(conn, &sym, tf, 0, 1 << 40).unwrap_or_default().is_empty()
        })
        .collect()
}

/// Whether the stored bars already cover [start, now].
pub fn intraday_ready(conn: &rusqlite::Connection, rec: &Listing, tf: &str, start: &str, today: &str, now_unix: f64) -> bool {
    let sym = bagholder_model::venues::tmx_symbol(&rec.symbol);
    let reach = intraday_reach(rec, today);
    if sym.is_empty() || reach.is_empty() || !INTRADAY.contains(&tf) {
        return true;
    }
    if intraday_missed_recently(conn, &sym, tf, now_unix) {
        // nothing to wait for: the last try produced nothing
        return true;
    }
    let start10: String = start.chars().take(10).collect();
    let start_day = if start10 > reach { start10 } else { reach };
    let start_ts = epoch_of_day(&start_day);
    let last = bagholder_store::market::bar_fetch(conn, &sym, tf).unwrap_or(None);
    last.and_then(|l| l.start_ts).map(|s| s <= start_ts).unwrap_or(false)
}

/// Start the fetch for a span not
/// stored yet, once per instrument, and return at once.
pub fn ensure_intraday_in_background(pool: std::sync::Arc<bagholder_store::pool::Pool>, rec: Listing, tf: String, start: String, end: String) {
    static PENDING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let sym = bagholder_model::venues::tmx_symbol(&rec.symbol);
    {
        let mut p = PENDING.lock().unwrap();
        if p.contains(&sym) {
            return;
        }
        p.push(sym.clone());
    }
    let _ = std::thread::Builder::new().name(format!("bagholder-intraday-{}", sym)).spawn(move || {
        if let Ok(conn) = pool.get() {
            let (today, now_unix, stamp) = crate::clock_now();
            let _ = ensure_intraday(&conn, &rec, &tf, &start, &end, &today, now_unix, &stamp, 1.0, true);
        }
        PENDING.lock().unwrap().retain(|s| *s != sym);
    });
}

/// The bars of a fetch, per intraday timeframe: one fetch covers both, so
/// the caller never asks a source twice for the same span.
#[derive(Clone, Debug, Default)]
pub struct ByTimeframe {
    pub h1: Vec<TimeBar>,
    pub h4: Vec<TimeBar>,
}

impl ByTimeframe {
    pub fn get(&self, tf: &str) -> &[TimeBar] {
        match tf {
            "1h" => &self.h1,
            "4h" => &self.h4,
            _ => &[],
        }
    }
    pub fn is_empty(&self) -> bool {
        self.h1.is_empty() && self.h4.is_empty()
    }
}

/// {tf: bars} of one candidate over the span.
pub fn fetch_intraday_from(
    conn: &rusqlite::Connection,
    source: &str,
    key: &str,
    rec: &Listing,
    start_ts: i64,
    end_ts: i64,
    today: &str,
    on_demand: bool,
) -> Result<ByTimeframe, FetchError> {
    let crypto = rec.kind.clone() == "Crypto";
    let mut out = ByTimeframe::default();
    match source {
        "tmx" => {
            let start = day_of_epoch(start_ts);
            let end = day_of_epoch(end_ts);
            let minutes = tmx_lookup(conn, key, today, |form| {
                let bars = fetch_tmx_minutes(form, &start, &end, on_demand);
                if bars.is_empty() { None } else { Some(bars) }
            })
            .0
            .unwrap_or_default();
            if !minutes.is_empty() {
                out.h1 = aggregate_session(&minutes, 60);
                out.h4 = aggregate_session(&minutes, 240);
            }
        }
        "coinbase" => {
            if coinbase_market(conn, key, today).is_empty() {
                return Ok(out);
            }
            let hourly = in_position_currency(conn, &fetch_coinbase_candles(key, 3600, start_ts, end_ts), &bar_currency(source, key, rec), &rec.currency);
            if !hourly.is_empty() {
                out.h4 = aggregate_hourly(&hourly, 14400);
                out.h1 = hourly;
            }
        }
        "yahoo" => {
            let fetched = whole_bars(fetch_yahoo(conn, key, start_ts, end_ts, "60m", today)?);
            let hourly = in_position_currency(conn, &fetched, &bar_currency(source, key, rec), &rec.currency);
            if hourly.is_empty() {
                return Ok(out);
            }
            if crypto {
                let as_time: Vec<TimeBar> = hourly.iter().map(|b| b.at_time()).collect();
                out.h4 = aggregate_hourly(&as_time, 14400);
                out.h1 = as_time;
            } else {
                out.h1 = aggregate_session(&hourly, 60);
                out.h4 = aggregate_session(&hourly, 240);
            }
        }
        _ => {}
    }
    Ok(out)
}

/// {tf: bars} over the span from the first candidate
/// whose reach covers it and that has bars for it; the remembered winner is
/// tried first. The background sweep leaves the rate-limited sources alone.
pub fn fetch_intraday(
    conn: &rusqlite::Connection,
    rec: &Listing,
    start_ts: i64,
    end_ts: i64,
    today: &str,
    on_demand: bool,
) -> (ByTimeframe, String) {
    let start_day = day_of_epoch(start_ts);
    let mut answers: Vec<(String, String, Vec<TimeBar>)> = Vec::new();
    let mut by: Vec<ByTimeframe> = Vec::new();
    let mut notes: Vec<(String, String, Note)> = Vec::new();
    for (source, key) in ordered_candidates(conn, rec) {
        let reach = source_intraday_reach(&source, today);
        if reach.is_empty() || start_day < reach || (!on_demand && ON_DEMAND_ONLY_SOURCES.contains(&source.as_str())) {
            continue;
        }
        let by_tf = match fetch_intraday_from(conn, &source, &key, rec, start_ts, end_ts, today, on_demand) {
            Ok(m) => {
                notes.push((source.clone(), key.clone(), Note::Bars));
                m
            }
            Err(e) => {
                notes.push((source.clone(), key.clone(), Note::Failed(e)));
                ByTimeframe::default()
            }
        };
        let hourly = by_tf.h1.clone();
        let covered = hourly.first().map(|b| b.time <= start_ts + COVERAGE_SLACK_DAYS * 86400).unwrap_or(false);
        answers.push((source, key, hourly));
        by.push(by_tf);
        if covered {
            break;
        }
    }
    match pick_covering(conn, rec, &answers, start_ts, |b: &TimeBar| b.time) {
        Some(i) => (by[i].clone(), answers[i].0.clone()),
        None => {
            remember_notes(rec, "hourly", notes);
            (ByTimeframe::default(), String::new())
        }
    }
}

/// Stored bars of an intraday timeframe for the
/// span, fetched from its start when never fetched and topped up when the span
/// reaches the present and the copy is older than `max_age_hours`. Bars once
/// stored are kept for good.
#[allow(clippy::too_many_arguments)]
pub fn ensure_intraday(
    conn: &rusqlite::Connection,
    rec: &Listing,
    tf: &str,
    start: &str,
    end: &str,
    today: &str,
    now_unix: f64,
    now_stamp: &str,
    max_age_hours: f64,
    on_demand: bool,
) -> rusqlite::Result<Vec<TimeBar>> {
    let sym = bagholder_model::venues::tmx_symbol(&rec.symbol);
    let reach = intraday_reach(rec, today);
    if sym.is_empty() || reach.is_empty() || !INTRADAY.contains(&tf) {
        return Ok(vec![]);
    }
    let start10: String = start.chars().take(10).collect();
    let start_day = if start10 > reach { start10 } else { reach };
    let start_ts = epoch_of_day(&start_day);
    let now_i = now_unix as i64;
    let end_ts = (epoch_of_day(&end.chars().take(10).collect::<String>()) + 86400).min(now_i);
    let last = bagholder_store::market::bar_fetch(conn, &sym, tf)?;
    let last_start = last.as_ref().and_then(|l| l.start_ts);
    let covered = last.is_some() && last_start.map(|s| s <= start_ts).unwrap_or(false);
    let fresh = last.is_some()
        && match crate::quotes::instant_secs_public(last.as_ref().map(|l| l.fetched_at.as_str()).unwrap_or("")) {
            Some(then) => now_unix - then < max_age_hours * 3600.0,
            None => false,
        };
    let needs_recent = end_ts >= now_i - 3 * 86400;
    let fetch_from = if !covered {
        Some(start_ts)
    } else if needs_recent && !fresh {
        let newest = bagholder_store::market::last_bar_time(conn, &sym, tf)?;
        Some(start_ts.max(newest.unwrap_or(start_ts) - 2 * 86400))
    } else {
        None
    };
    if let Some(from) = fetch_from {
        let (by_tf, source) = fetch_intraday(conn, rec, from, now_i, today, on_demand);
        // one fetch fills every intraday timeframe, so a miss covers them all
        for (k, _) in INTRADAY_SECONDS {
            let bars = by_tf.get(k);
            if bars.is_empty() {
                record_intraday_miss(conn, &sym, k, now_stamp);
                continue;
            }
            bagholder_store::market::upsert_price_bars(conn, &sym, k, bars, &source)?;
            let first = bars[0].time;
            let covered_from = if first <= from + COVERAGE_SLACK_DAYS * 86400 { from } else { first };
            let stamp_from = last_start.map_or(covered_from, |s| covered_from.min(s));
            bagholder_store::market::mark_bars_fetched(conn, &sym, k, stamp_from, now_stamp)?;
        }
    }
    bagholder_store::market::price_bars(conn, &sym, tf, start_ts, end_ts)
}

fn age_hours(fetched_at: Option<&str>, now_unix: f64) -> Option<f64> {
    crate::quotes::instant_secs_public(fetched_at?).map(|then| (now_unix - then) / 3600.0)
}

/// Hours since the archive last asked for `sym`'s intraday bars, whether it got any
/// or not. A listing its source has no bars for is never stamped as fetched -- only
/// as missed -- and one that is asked again on every pass for that reason is asked
/// without end: a miss is a read, and the next is due when any other would be.
fn archive_read_age_hours(conn: &rusqlite::Connection, sym: &str, now_unix: f64) -> Option<f64> {
    let last = bagholder_store::market::bar_fetch(conn, sym, "1h").unwrap_or(None);
    let fetched = age_hours(last.as_ref().map(|l| l.fetched_at.as_str()), now_unix);
    let missed = crate::quotes::instant_secs_public(&bagholder_store::tables::get_meta(conn, &miss_key(sym, "1h"), "").unwrap_or_default())
        .map(|then| (now_unix - then) / 3600.0);
    match (fetched, missed) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Keep the intraday bars of recently traded or
/// held instruments for good, a few per call -- those never fetched first,
/// then those whose copy is older than a day. Returns the symbols worked.
pub fn archive_intraday(conn: &rusqlite::Connection, recs: &[Listing], today: &str, now_unix: f64, now_stamp: &str, limit: usize) -> Vec<String> {
    let mut todo = archive_intraday_due(conn, recs, today, now_unix);
    todo.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    let mut done = Vec::new();
    for (_, sym, rec) in todo.into_iter().take(limit) {
        let start = rec.start.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| today.to_string());
        let _ = ensure_intraday(conn, &rec, "1h", &start, today, today, now_unix, now_stamp, ARCHIVE_TOPUP_HOURS, false);
        // however that went, it was asked: never the same listing again on the next pass
        if archive_read_age_hours(conn, &sym, now_unix).map_or(true, |age| age > ARCHIVE_TOPUP_HOURS) {
            record_intraday_miss(conn, &sym, "1h", now_stamp);
        }
        done.push(sym);
    }
    done
}

/// The listings whose intraday bars the archive should ask for now: never asked
/// first (0), then those last asked more than `ARCHIVE_TOPUP_HOURS` ago (1).
pub fn archive_intraday_due(conn: &rusqlite::Connection, recs: &[Listing], today: &str, now_unix: f64) -> Vec<(u8, String, Listing)> {
    let mut todo: Vec<(u8, String, Listing)> = Vec::new();
    for rec in recs {
        let sym = bagholder_model::venues::tmx_symbol(&rec.symbol);
        if sym.is_empty() || intraday_reach(rec, today).is_empty() {
            continue;
        }
        match archive_read_age_hours(conn, &sym, now_unix) {
            None => todo.push((0, sym, rec.clone())),
            Some(age) if age > ARCHIVE_TOPUP_HOURS => todo.push((1, sym, rec.clone())),
            Some(_) => {}
        }
    }
    todo
}

/// Seconds until the archive next has something to top up: the moment the oldest
/// stored read among `recs` passes `ARCHIVE_TOPUP_HOURS`. `None` when nothing is
/// archived at all -- then only a change to the book can make work. What the
/// archive waits for, in place of asking every five minutes.
pub fn archive_next_due_secs(conn: &rusqlite::Connection, recs: &[Listing], today: &str, now_unix: f64) -> Option<f64> {
    let mut soonest: Option<f64> = None;
    for rec in recs {
        let sym = bagholder_model::venues::tmx_symbol(&rec.symbol);
        if sym.is_empty() || intraday_reach(rec, today).is_empty() {
            continue;
        }
        let left = match archive_read_age_hours(conn, &sym, now_unix) {
            Some(age) => ((ARCHIVE_TOPUP_HOURS - age) * 3600.0).max(0.0),
            None => 0.0,
        };
        soonest = Some(soonest.map_or(left, |s: f64| s.min(left)));
    }
    soonest
}

/// Keep daily bars for instruments whose source forgets
/// them. No current source does, so this is idle until one is added.
pub fn archive_daily(conn: &rusqlite::Connection, recs: &[Listing], today: &str, now_unix: f64, now_stamp: &str, limit: usize) -> Vec<String> {
    let mut todo: Vec<(u8, String, Listing)> = Vec::new();
    for rec in recs {
        let src = history_source(rec);
        let sym = bagholder_model::venues::tmx_symbol(&rec.symbol);
        let short = src.as_ref().map(|(s, _)| SHORT_DAILY_SOURCES.contains(&s.as_str())).unwrap_or(false);
        if !short || sym.is_empty() {
            continue;
        }
        let last = bagholder_store::market::history_fetch(conn, &sym).unwrap_or(None);
        if last.is_none() {
            todo.push((0, sym, rec.clone()));
        } else if age_hours(last.as_ref().map(|l| l.fetched_at.as_str()), now_unix).map(|a| a > ARCHIVE_TOPUP_HOURS).unwrap_or(true) {
            todo.push((1, sym, rec.clone()));
        }
    }
    todo.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    let mut done = Vec::new();
    for (_, sym, rec) in todo.into_iter().take(limit) {
        let start = rec.start.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| today.to_string());
        let _ = ensure_history(conn, &rec, &start, today, today, now_unix, now_stamp);
        done.push(sym);
    }
    done
}

/// Bars for one timeframe over a span -- daily from the
/// daily store, weekly and monthly aggregated from it, 1h and 4h from the
/// intraday store.
#[allow(clippy::too_many_arguments)]
pub fn ensure_bars(
    conn: &rusqlite::Connection,
    rec: &Listing,
    tf: &str,
    start: &str,
    end: &str,
    today: &str,
    now_unix: f64,
    now_stamp: &str,
) -> rusqlite::Result<ChartBars> {
    if INTRADAY.contains(&tf) {
        return Ok(ChartBars::Hours(ensure_intraday(conn, rec, tf, start, end, today, now_unix, now_stamp, 1.0, true)?));
    }
    let daily = ensure_history(conn, rec, start, end, today, now_unix, now_stamp)?;
    Ok(ChartBars::Days(if tf == "1d" { daily } else { aggregate_daily(&daily, tf) }))
}
