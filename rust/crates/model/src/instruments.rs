//! The market instruments the watchlist can follow beside listings: indices,
//! futures, rates and currency pairs, each with the code Yahoo's chart
//! endpoint quotes it under.
//!
//! Nothing here is traded from the app. The directory exists so the ⌘K list
//! finds `WTI`, `NDX` or `VIX` and the watchlist can quote them.

use serde_json::{json, Value};

pub const KIND_LABEL: [(&str, &str); 5] = [
    ("Index", "Indices"),
    ("Future", "Futures"),
    ("Commodity", "Commodities"),
    ("Rate", "Rates"),
    ("Currency", "Currencies"),
];

pub struct Instrument {
    pub symbol: &'static str,
    pub name: &'static str,
    pub kind: &'static str,
    pub exchange: &'static str,
    pub currency: &'static str,
    pub yahoo: &'static str,
    pub aliases: &'static [&'static str],
}

macro_rules! inst {
    ($s:expr, $n:expr, $k:expr, $v:expr, $c:expr, $y:expr, [$($a:expr),*]) => {
        Instrument { symbol: $s, name: $n, kind: $k, exchange: $v, currency: $c, yahoo: $y, aliases: &[$($a),*] }
    };
}

pub static INSTRUMENTS: &[Instrument] = &[
    inst!("SPX", "S&P 500", "Index", "Index", "USD", "^GSPC", ["S&P", "S&P500", "SP500", "GSPC"]),
    inst!("NDX", "Nasdaq 100", "Index", "Index", "USD", "^NDX", ["NASDAQ100", "NASDAQ 100"]),
    inst!("IXIC", "Nasdaq Composite", "Index", "Index", "USD", "^IXIC", ["NASDAQ", "COMP"]),
    inst!("DJI", "Dow Jones Industrial Average", "Index", "Index", "USD", "^DJI", ["DJIA", "DOW", "DOW JONES"]),
    inst!("RUT", "Russell 2000", "Index", "Index", "USD", "^RUT", ["RUSSELL", "RUSSELL 2000"]),
    inst!("VIX", "CBOE Volatility Index", "Index", "Index", "USD", "^VIX", ["VOLATILITY"]),
    inst!("TSX", "S&P/TSX Composite", "Index", "Index", "CAD", "^GSPTSE", ["GSPTSE", "TSX COMPOSITE", "S&P/TSX"]),
    inst!("FTSE", "FTSE 100", "Index", "Index", "GBP", "^FTSE", ["FTSE 100"]),
    inst!("DAX", "DAX", "Index", "Index", "EUR", "^GDAXI", ["GDAXI"]),
    inst!("N225", "Nikkei 225", "Index", "Index", "JPY", "^N225", ["NIKKEI", "NIKKEI 225"]),
    inst!("HSI", "Hang Seng", "Index", "Index", "HKD", "^HSI", ["HANG SENG"]),
    inst!("STOXX50E", "Euro Stoxx 50", "Index", "Index", "EUR", "^STOXX50E", ["STOXX", "EURO STOXX"]),
    inst!("DXY", "US Dollar Index", "Index", "Index", "USD", "DX-Y.NYB", ["DOLLAR INDEX"]),
    // the equity index futures trade nearly around the clock: the read on the
    // market after hours
    inst!("ES", "S&P 500 E-mini futures", "Future", "CME", "USD", "ES=F",
          ["ES=F", "S&P FUTURES", "S&P 500 FUTURES", "SPX FUTURES", "ES FUTURES", "FUTURES"]),
    inst!("NQ", "Nasdaq 100 E-mini futures", "Future", "CME", "USD", "NQ=F",
          ["NQ=F", "NASDAQ FUTURES", "NASDAQ 100 FUTURES", "NQ FUTURES"]),
    inst!("YM", "Dow E-mini futures", "Future", "CBOT", "USD", "YM=F", ["YM=F", "DOW FUTURES", "YM FUTURES"]),
    inst!("RTY", "Russell 2000 E-mini futures", "Future", "CME", "USD", "RTY=F",
          ["RTY=F", "RUSSELL FUTURES", "RTY FUTURES"]),
    inst!("CL", "Crude Oil (WTI)", "Commodity", "NYMEX", "USD", "CL=F", ["WTI", "CRUDE", "OIL", "CRUDE OIL"]),
    inst!("BZ", "Brent Crude Oil", "Commodity", "ICE", "USD", "BZ=F", ["BRENT"]),
    inst!("NG", "Natural Gas", "Commodity", "NYMEX", "USD", "NG=F", ["NATGAS", "NATURAL GAS", "GAS"]),
    inst!("GC", "Gold", "Commodity", "COMEX", "USD", "GC=F", ["GOLD"]),
    inst!("SI", "Silver", "Commodity", "COMEX", "USD", "SI=F", ["SILVER"]),
    inst!("HG", "Copper", "Commodity", "COMEX", "USD", "HG=F", ["COPPER"]),
    inst!("PL", "Platinum", "Commodity", "NYMEX", "USD", "PL=F", ["PLATINUM"]),
    inst!("ZC", "Corn", "Commodity", "CBOT", "USD", "ZC=F", ["CORN"]),
    inst!("ZW", "Wheat", "Commodity", "CBOT", "USD", "ZW=F", ["WHEAT"]),
    inst!("TNX", "US 10-Year Treasury Yield", "Rate", "Index", "USD", "^TNX",
          ["10Y", "10-YEAR", "10 YEAR", "TREASURY", "YIELD"]),
    // the two contracts the market prices policy with, quoted as 100 minus the
    // rate they settle against
    inst!("ZQ", "30-Day Federal Funds futures", "Rate", "CBOT", "USD", "ZQ=F",
          ["FED", "FED FUNDS", "FED FUNDS FUTURES", "FEDERAL FUNDS", "FOMC", "POLICY RATE", "ZQ=F"]),
    inst!("SR3", "Three-Month SOFR futures", "Rate", "CME", "USD", "SR3=F",
          ["SOFR", "SOFR FUTURES", "THREE-MONTH SOFR", "SR3=F"]),
    inst!("USDCAD", "US Dollar / Canadian Dollar", "Currency", "FX", "CAD", "CAD=X", ["USD/CAD", "CAD", "LOONIE"]),
    inst!("EURUSD", "Euro / US Dollar", "Currency", "FX", "USD", "EURUSD=X", ["EUR/USD", "EURO"]),
    inst!("GBPUSD", "British Pound / US Dollar", "Currency", "FX", "USD", "GBPUSD=X", ["GBP/USD", "POUND"]),
    inst!("USDJPY", "US Dollar / Japanese Yen", "Currency", "FX", "JPY", "JPY=X", ["USD/JPY", "YEN"]),
    inst!("BTCUSD", "Bitcoin / US Dollar", "Currency", "FX", "USD", "BTC-USD", ["BITCOIN", "BTC"]),
];

/// What a market tile calls the instrument: the symbol, unless people know it
/// by a name.
const LABELS: [(&str, &str); 17] = [
    ("ZQ", "FED FUNDS"), ("SR3", "SOFR"), ("CL", "WTI"), ("BZ", "BRENT"), ("NG", "NATGAS"), ("GC", "GOLD"),
    ("SI", "SILVER"), ("HG", "COPPER"), ("PL", "PLATINUM"), ("ZC", "CORN"), ("ZW", "WHEAT"), ("TNX", "10Y"),
    ("USDCAD", "USD/CAD"), ("EURUSD", "EUR/USD"), ("GBPUSD", "GBP/USD"), ("USDJPY", "USD/JPY"),
    ("BTCUSD", "BITCOIN"),
];

/// A contract quoted as 100 minus the rate it settles against. Nothing else in
/// the directory carries a rate under its price.
const RATE_FROM_PRICE: [&str; 2] = ["ZQ", "SR3"];

/// `implied_rate`: the rate such a contract is pricing -- the
/// contract's own definition, not a reading of it.
pub fn implied_rate(symbol: &str, price: Option<f64>) -> Option<f64> {
    let sym = symbol.trim().to_uppercase();
    if !RATE_FROM_PRICE.contains(&sym.as_str()) {
        return None;
    }
    let p = price?;
    Some(((100.0 - p) * 1e4).round() / 1e4)
}

pub fn label(symbol: &str) -> String {
    let sym = symbol.trim().to_uppercase();
    LABELS.iter().find(|(k, _)| *k == sym).map(|(_, v)| (*v).to_string()).unwrap_or(sym)
}

pub fn kind_label(kind: &str) -> String {
    KIND_LABEL.iter().find(|(k, _)| *k == kind).map(|(_, v)| (*v).to_string()).unwrap_or_else(|| kind.to_string())
}

pub fn rows() -> Vec<Value> {
    INSTRUMENTS
        .iter()
        .map(|r| {
            json!({
                "symbol": r.symbol, "name": r.name, "kind": r.kind, "exchange": r.exchange,
                "currency": r.currency, "yahoo": r.yahoo, "aliases": r.aliases,
            })
        })
        .collect()
}

/// `find`: the instrument a watched row is, by symbol and venue,
/// or nothing when the row is a listing.
pub fn find(symbol: &str, exchange: &str) -> Option<&'static Instrument> {
    let sym = symbol.trim().to_uppercase();
    let ex = exchange.trim().to_uppercase();
    INSTRUMENTS.iter().find(|r| r.symbol == sym && r.exchange.to_uppercase() == ex)
}

/// `search`: an exact symbol or alias first, then one starting
/// with the text, then one with a word starting with it.
///
/// A single letter matches only an exact symbol, so `V` finds Visa's listings
/// and not every index with a V in it.
pub fn search(text: &str) -> Vec<crate::wire::SymbolMatch> {
    let q = text.trim().to_uppercase();
    if q.is_empty() {
        return vec![];
    }
    let mut out: Vec<(u8, crate::wire::SymbolMatch)> = Vec::new();
    for r in INSTRUMENTS {
        let mut names: Vec<String> = vec![r.symbol.to_string()];
        names.extend(r.aliases.iter().map(|a| a.to_uppercase()));
        let upper_name = r.name.to_uppercase();

        let mut words: Vec<String> = Vec::new();
        for x in names.iter().cloned().chain(std::iter::once(upper_name.clone())) {
            for w in x.replace('/', " ").split_whitespace() {
                words.push(w.to_string());
            }
        }

        let rank: i8 = if names.contains(&q) {
            0
        } else if q.chars().count() < 2 {
            -1
        } else if names.iter().chain(std::iter::once(&upper_name)).any(|x| x.starts_with(&q)) {
            1
        } else if words.iter().any(|w| w.starts_with(&q)) {
            2
        } else {
            -1
        };
        if rank >= 0 {
            out.push((
                rank as u8,
                crate::wire::SymbolMatch {
                    symbol: r.symbol.to_string(), name: r.name.to_string(), exchange: r.exchange.to_string(),
                    currency: r.currency.to_string(), kind: Some(r.kind.to_string()), rank: Some(rank as f64),
                },
            ));
        }
    }
    // a stable sort on the rank alone
    out.sort_by_key(|(rank, _)| *rank);
    out.into_iter().map(|(_, r)| r).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `crates/model/tests/instruments.rs` covers `search`'s ranking by symbol;
    /// this is the one thing it does not check, the numeric rank itself.
    #[test]
    fn test_search_carries_the_rank_an_exact_match_beats_a_prefix_beats_a_word() {
        assert_eq!(search("VIX")[0].rank, Some(0.0));
        assert_eq!(search("VOLAT")[0].rank, Some(1.0), "an alias VIX has no symbol prefix for, but starts with it");
    }
}
