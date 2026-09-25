//! The Markets tab: `markets_view` and everything it reads.
//!
//! The tile row, the watchlist with its quotes, the heatmap of what the book
//! holds, and the news items each tagged with the listings they were read for.

use crate::activity::Kind;
use crate::context::MarketBase as Base;
use crate::exposure::{norm_sector, underlying_exposure, Exposure, Exposures, UNCLASSIFIED};
use crate::input::Listing;
use crate::instruments;
use crate::venues::{tmx_symbol, watch_exposure_key};
use crate::wire::{HeldTile, MarketInstrument, MarketTile, Markets, NewsItem, NewsTag, Ordered, Position, UniverseTile, WatchItem};

/// The feed whose items belong to the market rather than to a listing.
const MARKET_FEED: (&str, &str) = ("*", "MARKET");

/// The Markets tab's tile row when the user has never changed it.
const DEFAULT_TILES: [(&str, &str); 6] = [("SPX", "INDEX"), ("NDX", "INDEX"), ("DJI", "INDEX"), ("VIX", "INDEX"), ("GC", "COMEX"), ("BTCUSD", "FX")];
const TILES_MAX: usize = 12;

/// Where a watched listing's quote is kept -- its symbol and venue, so a listing
/// the book also holds elsewhere keeps its own.
pub fn watch_quote_key(symbol: &str, exchange: &str) -> String {
    format!("{}@{}", symbol.trim().to_uppercase(), exchange.trim().to_uppercase())
}

/// The saved set, else the default; only what the directory knows, twelve at most.
fn tile_list(base: &Base) -> Vec<&'static instruments::Instrument> {
    let rows: Vec<(String, String)> = match base.tiles.as_ref() {
        // never saved is not the same as saved empty
        Some(saved) => saved.iter().map(|t| (t.symbol.clone(), t.exchange.clone())).collect(),
        None => DEFAULT_TILES.iter().map(|(s, e)| (s.to_string(), e.to_string())).collect(),
    };
    let mut out: Vec<&'static instruments::Instrument> = Vec::new();
    for (sym, ex) in rows {
        if let Some(inst) = instruments::find(&sym, &ex) {
            if !out.iter().any(|known| known.symbol == inst.symbol) {
                out.push(inst);
            }
        }
    }
    out.truncate(TILES_MAX);
    out
}

fn instrument_listing(i: &instruments::Instrument) -> Listing {
    Listing { symbol: i.symbol.into(), exchange: i.exchange.into(), currency: i.currency.into(), kind: "Instrument".into(), quote_key: Some(watch_quote_key(i.symbol, i.exchange)), yahoo: Some(i.yahoo.into()), start: None }
}

/// The tile row keyed as a watched instrument is.
pub fn tile_symbols(base: &Base) -> Vec<Listing> {
    tile_list(base).into_iter().map(instrument_listing).collect()
}

/// The instrument's own price scale.
fn tile_decimals(inst: &instruments::Instrument) -> i64 {
    if inst.symbol == "BTCUSD" {
        return 0;
    }
    match inst.kind {
        "Rate" => 3,
        "Currency" => 4,
        _ => 2,
    }
}

pub fn tile_rows(base: &Base) -> Vec<MarketTile> {
    tile_list(base)
        .into_iter()
        .map(|inst| {
            let quote = base.quotes.get(&watch_quote_key(inst.symbol, inst.exchange));
            let last = quote.and_then(|q| q.price);
            let change = quote.and_then(|q| q.price_change);
            // A contract quoted as 100 minus the rate carries that rate beside its
            // published price: the price is what the exchange gives, the rate is
            // the contract's own definition of it, and a day that moves the price
            // down has moved the rate it prices up.
            let (rate, rate_change) = MarketTile::implied(instruments::implied_rate(inst.symbol, last), change);
            MarketTile {
                symbol: inst.symbol,
                exchange: inst.exchange,
                label: instruments::label(inst.symbol),
                name: inst.name,
                kind: inst.kind,
                last,
                change,
                percent_change: quote.and_then(|q| q.percent_change),
                decimals: tile_decimals(inst),
                rate,
                rate_change,
            }
        })
        .collect()
}

/// Every watched listing, with what a quote source needs to price it.
pub fn watch_symbols(base: &Base) -> Vec<Listing> {
    base.watchlist
        .iter()
        .map(|w| match instruments::find(&w.symbol, &w.exchange) {
            Some(i) => Listing { symbol: w.symbol.clone(), exchange: w.exchange.clone(), currency: w.currency.clone(), quote_key: Some(watch_quote_key(&w.symbol, &w.exchange)), ..instrument_listing(i) },
            None => {
                // a watched coin is the USD pair, whatever currency the book holds it in
                let crypto = w.exchange.to_uppercase() == "CRYPTO";
                Listing {
                    symbol: w.symbol.clone(),
                    exchange: w.exchange.clone(),
                    currency: if crypto { "USD".into() } else { w.currency.clone() },
                    kind: if crypto { "Crypto" } else { "Shares" }.into(),
                    quote_key: Some(watch_quote_key(&w.symbol, &w.exchange)),
                    yahoo: None,
                    start: None,
                }
            }
        })
        .collect()
}

/// The watched listings, then the tile row's instruments not already among them.
pub fn quote_symbols(base: &Base) -> Vec<Listing> {
    let mut out = watch_symbols(base);
    for tile in tile_symbols(base) {
        if !out.iter().any(|known| known.quote_key == tile.quote_key) {
            out.push(tile);
        }
    }
    out
}

/// The sector a record gives most weight to; the first of equals.
pub fn dominant_sector(record: Option<&Exposure>) -> String {
    let mut best = String::new();
    let mut heaviest = 0.0_f64;
    for (name, weight) in record.map(|r| r.sectors.as_slice()).unwrap_or(&[]) {
        if *weight > heaviest {
            let known = norm_sector(name);
            best = if known.is_empty() { name.clone() } else { known };
            heaviest = *weight;
        }
    }
    if best.is_empty() { UNCLASSIFIED.to_string() } else { best }
}

/// A listing is one listing whether the book names it `QNC.TO` or the
/// watchlist `QNC`: the bare ticker and the venue.
fn lk(symbol: &str, exchange: &str) -> (String, String) {
    (tmx_symbol(symbol), exchange.trim().to_uppercase())
}

pub fn watch_rows(base: &Base, positions: &[&Position]) -> Vec<WatchItem> {
    base.watchlist
        .iter()
        .map(|w| {
            let quote = base.quotes.get(&watch_quote_key(&w.symbol, &w.exchange));
            let held = positions.iter().find(|p| p.symbol == w.symbol && p.exchange.to_uppercase() == w.exchange.to_uppercase());
            let inst = instruments::find(&w.symbol, &w.exchange);
            let crypto = w.exchange.to_uppercase() == "CRYPTO";
            let (exchange, sector, kind) = match inst {
                Some(i) => (i.exchange.to_string(), instruments::kind_label(i.kind), i.kind.to_string()),
                None if crypto => ("Crypto".to_string(), "Digital assets".to_string(), "Crypto".to_string()),
                None => (w.exchange.clone(), dominant_sector(base.exposures.get(&watch_exposure_key(&w.symbol, &w.exchange, &w.currency))), "Shares".to_string()),
            };
            WatchItem {
                symbol: w.symbol.clone(),
                exchange,
                name: w.name.clone(),
                currency: if crypto { "USD".to_string() } else { w.currency.clone() },
                last: quote.and_then(|q| q.price),
                price_change: quote.and_then(|q| q.price_change),
                percent_change: quote.and_then(|q| q.percent_change),
                sector,
                kind,
                position_id: held.map(|p| p.id.clone()),
            }
        })
        .collect()
}

/// One tile per symbol held, its market value in CAD summed over the accounts
/// holding it.
pub fn heatmap_items(positions: &[&Position], exposures: &Exposures, cad: &dyn Fn(f64, &str) -> f64) -> Vec<HeldTile> {
    let mut out: Vec<HeldTile> = Vec::new();
    for p in positions {
        let value = cad(p.mv, &p.currency);
        if !(value > 0.0) {
            continue;
        }
        if let Some(tile) = out.iter_mut().find(|t| t.symbol == p.symbol && t.exchange.to_uppercase() == p.exchange.to_uppercase()) {
            tile.value += value;
            continue;
        }
        let sector = match p.kind {
            Kind::Crypto => "Digital assets".to_string(),
            // a contract counts under its underlying's record
            Kind::Options => dominant_sector(underlying_exposure(exposures, &p.underlying, &p.currency)),
            _ => dominant_sector(exposures.get(&p.security_id)),
        };
        out.push(HeldTile { id: p.id.clone(), symbol: p.symbol.clone(), exchange: p.exchange.clone(), value, percent_change: p.percent_change, sector });
    }
    out
}

// --------------------------------------------------------------------------
// news
// --------------------------------------------------------------------------

/// `news_text_key`: a headline as one story -- letters and digits only,
/// one case, one space between words.
pub fn news_text_key(headline: &str) -> String {
    let lower = headline.to_lowercase();
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    for c in lower.chars() {
        if c.is_ascii_alphanumeric() {
            cur.push(c);
        } else if !cur.is_empty() {
            words.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words.join(" ")
}

const FRENCH_WORDS: [&str; 20] = [
    "annonce", "annoncent", "ses", "du", "des", "une", "pour", "avec", "sur", "résultats", "clôture",
    "croissance", "les", "et", "au", "aux", "dans", "son", "sa", "le",
];
const FRENCH_LETTERS: [char; 14] = ['à', 'â', 'ç', 'é', 'è', 'ê', 'ë', 'î', 'ï', 'ô', 'û', 'ù', 'ü', 'ÿ'];

/// `looks_french`: accented letters or French function words, two or
/// more.
pub fn looks_french(headline: &str) -> bool {
    let t = headline.to_lowercase();
    let accents = t.chars().filter(|c| FRENCH_LETTERS.contains(c) || *c == 'œ').count();
    if accents >= 2 {
        return true;
    }
    // whole words, bounded by \b
    let words: Vec<&str> = t.split(|c: char| !(c.is_alphanumeric() || c == '_')).collect();
    let hits = words.iter().filter(|w| FRENCH_WORDS.contains(w) || **w == "la").count();
    hits >= 2
}

/// Minutes since the epoch for an ISO instant, or nothing when it cannot be
/// read.
fn when_minutes(iso: &str) -> Option<f64> {
    let s = iso.replace('Z', "+00:00");
    let (d, t) = s.split_once('T')?;
    let (y, m, day) = crate::dates::parse_iso(d)?;
    let hhmm: Vec<&str> = t.split(|c| c == ':' || c == '+' || c == '-').collect();
    let hh: f64 = hhmm.first()?.parse().ok()?;
    let mm: f64 = hhmm.get(1).and_then(|x| x.parse().ok()).unwrap_or(0.0);
    let ss: f64 = hhmm.get(2).and_then(|x| x.parse().ok()).unwrap_or(0.0);
    // the offset, when the instant carries one
    let mut offset_minutes = 0.0;
    if let Some(pos) = t.rfind(['+', '-']) {
        if pos > 0 {
            let sign = if t.as_bytes()[pos] == b'-' { -1.0 } else { 1.0 };
            let off = &t[pos + 1..];
            let (oh, om) = off.split_once(':').unwrap_or((off, "0"));
            offset_minutes = sign * (oh.parse::<f64>().unwrap_or(0.0) * 60.0 + om.parse::<f64>().unwrap_or(0.0));
        }
    }
    let days = crate::dates::to_days(y, m, day) as f64;
    Some(days * 1440.0 + hh * 60.0 + mm + ss / 60.0 - offset_minutes)
}

/// A release posted in French beside its English original -- the same wire, a
/// listing in common, within three hours -- is one story. The English row stays.
fn drop_translations(rows: Vec<NewsItem>) -> Vec<NewsItem> {
    let keys = |r: &NewsItem| -> Vec<(String, String)> { r.tags.iter().map(|t| lk(&t.symbol, &t.exchange)).collect() };
    let french: Vec<bool> = rows.iter().map(|r| looks_french(&r.headline)).collect();
    let whens: Vec<Option<f64>> = rows.iter().map(|r| when_minutes(&r.published_at)).collect();
    let is_twin = |i: usize| {
        let listed = keys(&rows[i]);
        rows.iter().enumerate().any(|(j, other)| {
            j != i && !french[j] && other.source == rows[i].source && keys(other).iter().any(|k| listed.contains(k)) && matches!((whens[i], whens[j]), (Some(a), Some(b)) if (b - a).abs() <= 180.0)
        })
    };
    let twins: Vec<bool> = (0..rows.len()).map(|i| french[i] && is_twin(i)).collect();
    rows.into_iter().zip(twins).filter(|(_, twin)| !twin).map(|(r, _)| r).collect()
}

/// Every item kept, newest first, each tagged with the listings it was read for.
/// An item two listings share is one row with two tags.
pub fn news_rows(base: &Base, positions: &[&Position], watch: &[WatchItem]) -> Vec<NewsItem> {
    let mut items: Vec<&crate::input::NewsRow> = base.news.iter().collect();
    items.sort_by(|a, b| b.published_at.cmp(&a.published_at));

    let mut rows: Vec<NewsItem> = Vec::new();
    // one story is one row: the same wire id, or the same headline under
    // another id (a release carried by several wires, a story republished per
    // symbol, an update)
    let mut by_id: Vec<(String, usize)> = Vec::new();
    let mut by_text: Vec<(String, usize)> = Vec::new();

    for n in items {
        // the market feed's items carry no tag: they are the market's
        let is_market = n.symbol == MARKET_FEED.0 && n.exchange.to_uppercase() == MARKET_FEED.1;
        let key = lk(&n.symbol, &n.exchange);
        let tag = (!is_market).then(|| {
            let held = positions.iter().find(|p| lk(&p.symbol, &p.exchange) == key);
            let watched = watch.iter().find(|w| lk(&w.symbol, &w.exchange) == key);
            NewsTag {
                symbol: key.0.clone(),
                exchange: n.exchange.clone(),
                held: held.is_some(),
                watched: watched.is_some(),
                percent_change: match (held, watched) {
                    (Some(p), _) => p.percent_change,
                    (None, Some(w)) => w.percent_change,
                    (None, None) => None,
                },
                position_id: held.map(|p| p.id.clone()),
            }
        });
        let text = news_text_key(&n.headline);
        let known = by_id.iter().find(|(k, _)| *k == n.id).map(|(_, i)| *i).or_else(|| if text.is_empty() { None } else { by_text.iter().find(|(k, _)| *k == text).map(|(_, i)| *i) });

        if let Some(i) = known {
            match tag {
                None => rows[i].market = true,
                Some(tag) => {
                    if !rows[i].tags.iter().any(|t| lk(&t.symbol, &t.exchange) == key) {
                        rows[i].tags.push(tag);
                    }
                }
            }
            // the same text on a wire and in a publisher's column is the release
            if n.kind == "release" {
                rows[i].kind = "release".into();
            }
            if !by_id.iter().any(|(k, _)| *k == n.id) {
                by_id.push((n.id.clone(), i));
            }
            continue;
        }

        rows.push(NewsItem {
            id: n.id.clone(),
            headline: n.headline.clone(),
            source: n.wire.clone(),
            url: n.url.clone(),
            published_at: n.published_at.clone(),
            market: is_market,
            tags: tag.into_iter().collect(),
            kind: if n.kind.is_empty() { "story".to_string() } else { n.kind.clone() },
        });
        let i = rows.len() - 1;
        by_id.push((n.id.clone(), i));
        if !text.is_empty() && !by_text.iter().any(|(k, _)| *k == text) {
            by_text.push((text, i));
        }
    }
    rows.sort_by(|a, b| b.published_at.cmp(&a.published_at));
    drop_translations(rows)
}

pub fn markets_view(base: &Base, positions: &[&Position]) -> Markets {
    // a holding's value is already in CAD
    let cad = |amount: f64, _currency: &str| amount;
    let watchlist = watch_rows(base, positions);
    let universes = base
        .universes
        .0
        .iter()
        .map(|(key, rows)| {
            let tiles = rows
                .iter()
                .map(|r| UniverseTile { id: None, symbol: r.symbol.clone(), name: r.name.clone(), value: r.value, percent_change: r.percent_change, sector: if r.sector.is_empty() { UNCLASSIFIED.to_string() } else { r.sector.clone() }, country: r.country.clone() })
                .collect();
            (key.clone(), tiles)
        })
        .collect();
    Markets {
        holdings: heatmap_items(positions, &base.exposures, &cad),
        news: news_rows(base, positions, &watchlist),
        watchlist,
        universes: Ordered(universes),
        tiles: tile_rows(base),
        instruments: instruments::INSTRUMENTS.iter().map(|r| MarketInstrument { symbol: r.symbol, label: instruments::label(r.symbol), name: r.name, exchange: r.exchange, kind: r.kind, aliases: r.aliases }).collect(),
    }
}
