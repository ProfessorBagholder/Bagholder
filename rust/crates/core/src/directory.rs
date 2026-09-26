//! The app's own directory of market instruments the watchlist and the Markets
//! tab's tiles follow beside listings (`SPEC.md` §4 Markets): indices, futures,
//! commodities, rates and currency pairs, each with its usual short symbol, its
//! venue, its currency, the code Yahoo's chart quotes it under and the aliases
//! people type. Nothing here is traded from the app.

pub const KIND_LABEL: [(&str, &str); 5] = [
    ("Index", "Indices"),
    ("Future", "Futures"),
    ("Commodity", "Commodities"),
    ("Rate", "Rates"),
    ("Currency", "Currencies"),
];

#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
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
        Entry { symbol: $s, name: $n, kind: $k, exchange: $v, currency: $c, yahoo: $y, aliases: &[$($a),*] }
    };
}

pub static INSTRUMENTS: &[Entry] = &[
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
pub const RATE_FROM_PRICE: [&str; 2] = ["ZQ", "SR3"];

/// What a market tile calls the instrument: the symbol, unless people know it
/// by a name.
pub fn label(symbol: &str) -> String {
    let sym = symbol.trim().to_uppercase();
    LABELS.iter().find(|(k, _)| *k == sym).map(|(_, v)| (*v).to_string()).unwrap_or(sym)
}

/// The heading the heatmap files an instrument of this kind under.
pub fn kind_label(kind: &str) -> String {
    KIND_LABEL.iter().find(|(k, _)| *k == kind).map(|(_, v)| (*v).to_string()).unwrap_or_else(|| kind.to_string())
}

/// The instrument a watched row or a tile is, by its symbol and venue; nothing
/// for a listing.
pub fn find(symbol: &str, exchange: &str) -> Option<&'static Entry> {
    let sym = symbol.trim().to_uppercase();
    let ex = exchange.trim().to_uppercase();
    INSTRUMENTS.iter().find(|r| r.symbol == sym && r.exchange.to_uppercase() == ex)
}

impl Entry {
    /// What kind of instrument it is in Bagholder's terms: a commodity is quoted
    /// by its front future, a pair is a currency pair.
    pub fn instrument_kind(&self) -> crate::instrument::InstrumentKind {
        use crate::instrument::InstrumentKind;
        match self.kind {
            "Index" => InstrumentKind::Index,
            "Rate" => InstrumentKind::Rate,
            "Currency" => InstrumentKind::CurrencyPair,
            _ => InstrumentKind::Future,
        }
    }

    /// The directory's own key for it, as its `directory` reference: `SPX@INDEX`.
    pub fn key(&self) -> String {
        format!("{}@{}", self.symbol, self.exchange.to_uppercase())
    }

    /// Whether it is quoted as 100 minus the rate it settles against.
    pub fn priced_as_rate(&self) -> bool {
        RATE_FROM_PRICE.contains(&self.symbol)
    }

    /// Its prices' scale on a tile: none for Bitcoin, three for a rate, four for a
    /// pair, two for the rest.
    pub fn decimals(&self) -> u32 {
        if self.symbol == "BTCUSD" {
            return 0;
        }
        match self.kind {
            "Rate" => 3,
            "Currency" => 4,
            _ => 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_entry_is_found_by_its_own_symbol_and_venue_once() {
        for e in INSTRUMENTS {
            assert!(std::ptr::eq(find(&e.symbol.to_lowercase(), e.exchange).unwrap(), e));
            assert_eq!(INSTRUMENTS.iter().filter(|x| x.key() == e.key()).count(), 1, "{}", e.key());
        }
        assert!(find("SPX", "NASDAQ").is_none());
    }
}
