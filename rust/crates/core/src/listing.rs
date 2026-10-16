//! A listing: a symbol on a listing venue (`docs/architecture.md` §5; plan
//! `market-data-keyed-by-listing`, Approach, 1). What the market cache keeps of a
//! listing (its chart, bars, chosen source, short interest, breakdowns, news
//! marks) is found under this key and no other, so two listings that share a
//! ticker never share what is kept for them.
//!
//! A key cannot be made without its venue. The venue is the market identifier
//! code in Canada, where each exchange lists its own issues, and `US` for any US
//! exchange, where one symbol names one listing across every exchange under one
//! national plan and the sources state the NYSE group differently (FIGI's
//! composite: one listing "within the same country or market"). A coin is
//! `<asset>@CRYPTO`, a directory instrument `<symbol>@INDEX`, an option contract
//! its OCC symbol `@OCC`.

use std::fmt;

/// The market identifier codes (ISO 10383) of the Canadian venues: the TSX, the
/// TSX Venture, the CSE, Alpha, and Cboe Canada.
pub const CANADA_MICS: [&str; 5] = ["XTSE", "XTSX", "XCNQ", "XATS", "NEOE"];
/// The market identifier codes of the US venues: Nasdaq, the NYSE, NYSE Arca, NYSE
/// American, Cboe's US equities (BZX), IEX.
pub const US_MICS: [&str; 6] = ["XNAS", "XNYS", "ARCX", "XASE", "BATS", "IEXG"];

/// The venue a listing of a coin is on.
pub const CRYPTO: &str = "CRYPTO";
/// The venue of an instrument from Bagholder's own directory (an index, a future,
/// a rate, a currency pair).
pub const INDEX: &str = "INDEX";
/// The venue of an option contract, named by its OCC symbol.
pub const OCC: &str = "OCC";
/// The venue every US exchange's listings are keyed under.
pub const US: &str = "US";
/// The market's own news, which no one listing carries, is kept under `*@MARKET`.
pub const MARKET: &str = "MARKET";

/// The market identifier code of a venue as a broker, a directory or a person
/// writes it in words or as a code (`TSX-V`, `NASDAQ`, `Cboe Canada`, `XNYS`);
/// none for words no venue here is known by.
pub fn mic_of(words: &str) -> Option<&'static str> {
    let words = words.trim().to_ascii_uppercase();
    VENUE_WORDS.iter().find(|(_, w)| w.contains(&words.as_str())).map(|(mic, _)| *mic)
}

/// Each venue's code and the words it is written in (upper case), the code
/// itself among them: the one table `mic_of` reads, and the one the page's key
/// is made from (generated into `web/src/lib/generated/venues.ts`).
pub const VENUE_WORDS: [(&str, &[&str]); 11] = [
    ("XTSE", &["TSX", "TORONTO", "TORONTO STOCK EXCHANGE", "XTSE"]),
    // `XTSV` is how some records write the TSX Venture's code
    ("XTSX", &["TSX-V", "TSXV", "TSX VENTURE", "TSX VENTURE EXCHANGE", "VENTURE", "CDNX", "XTSX", "XTSV"]),
    ("XCNQ", &["CSE", "CANADIAN SECURITIES EXCHANGE", "XCNQ", "CNSX"]),
    ("NEOE", &["CBOE CANADA", "CBOE CA", "NEO", "NEOE", "AQL"]),
    ("XATS", &["ALPHA", "ALPHA EXCHANGE", "XATS"]),
    ("XNAS", &["NASDAQ", "XNAS"]),
    ("XNYS", &["NYSE", "XNYS"]),
    ("ARCX", &["NYSE ARCA", "ARCA", "ARCX"]),
    ("XASE", &["NYSE AMERICAN", "AMEX", "XASE"]),
    // `CBOE` alone is Cboe's US equities venue; Cboe Canada is named as such
    ("BATS", &["BATS", "CBOE BZX", "CBOE"]),
    ("IEXG", &["IEX", "IEXG"]),
];

/// The venue a listing on `mic` is keyed under: the code in Canada, `US` in the
/// US; none for a code no venue here is.
pub fn listing_venue(mic: &str) -> Option<&str> {
    if US_MICS.contains(&mic) {
        Some(US)
    } else if CANADA_MICS.contains(&mic) {
        Some(mic)
    } else {
        None
    }
}

/// Each venue a key holds, by the name a person reads it under (`TD · TSX`):
/// the one table the server's sentences and the page's labels name venues from.
pub const VENUE_NAMES: [(&str, &str); 9] = [
    ("XTSE", "TSX"),
    ("XTSX", "TSX-V"),
    ("XCNQ", "CSE"),
    ("NEOE", "Cboe Canada"),
    ("XATS", "Alpha"),
    (US, "US"),
    (CRYPTO, "crypto"),
    (INDEX, "index"),
    (OCC, "option"),
];

/// The market cache's meta keys that name a listing, each built by
/// `ListingKey::meta` and by nothing else (held by
/// `test_every_meta_key_that_names_a_listing_is_built_through_its_key`).
pub mod meta {
    /// The source that last answered for a listing's bars.
    pub const BARS_SOURCE: &str = "bars_source";
    /// A timeframe of a listing's bars no source had, `|<tf>` after the key.
    pub const BARS_MISS: &str = "bars_miss";
    /// When a listing's wire was last read.
    pub const NEWS_FETCHED: &str = "news_fetched";
    /// When a listing's news was last read from one source (`news_source_fetched:<source>:<key>`).
    pub const NEWS_SOURCE_FETCHED: &str = "news_source_fetched";
    /// What a listing's regulators last answered.
    pub const FILINGS_SOURCES: &str = "filings_sources";
    /// Every one, for a store cleared of them.
    pub const ALL: [&str; 5] = [BARS_SOURCE, BARS_MISS, NEWS_FETCHED, NEWS_SOURCE_FETCHED, FILINGS_SOURCES];
}

/// The venue a share or fund on the venue `venue` names is keyed under: the code
/// in Canada, `US` in the US, from words, a code or the key's own `US`; none for
/// a venue no share is keyed by.
pub fn key_venue(venue: &str) -> Option<&'static str> {
    if venue.trim().eq_ignore_ascii_case(US) {
        return Some(US);
    }
    let mic = mic_of(venue)?;
    if US_MICS.contains(&mic) {
        Some(US)
    } else {
        CANADA_MICS.iter().find(|m| **m == mic).copied()
    }
}

/// The venue suffixes a Yahoo-style symbol carries, which a key's spelling drops.
pub const VENUE_SUFFIXES: [&str; 4] = [".TO", ".V", ".CN", ".NE"];

/// A symbol in the one spelling a key holds: upper case, a Yahoo venue suffix a
/// record may carry dropped (`ENB.TO` is `ENB` on the venue its record names),
/// and a class written with a dot whichever way a source writes it (`BRK-B`,
/// `BRK/B` and `BRK.B` are one symbol; `BRK` is another).
pub fn spelling(symbol: &str) -> String {
    let s = symbol.trim().to_ascii_uppercase();
    let s = VENUE_SUFFIXES.iter().find_map(|suffix| s.strip_suffix(suffix)).map_or(s.clone(), str::to_string);
    s.split_whitespace().collect::<Vec<_>>().join("").replace(['-', '/'], ".")
}

/// A listing: a symbol on a listing venue.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ListingKey {
    symbol: String,
    venue: String,
}

/// Why text or a venue is not a listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotAListing(pub String);

impl fmt::Display for NotAListing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NotAListing {}

impl ListingKey {
    /// A listing on the venue `venue` names, in words or as a code, or as a key
    /// writes it (`US`, `CRYPTO` for a coin, `INDEX`, `OCC`).
    pub fn on(symbol: &str, venue: &str) -> Result<ListingKey, NotAListing> {
        match venue.trim().to_ascii_uppercase().as_str() {
            // the US market itself, as a key names it
            US => return Self::new(symbol, US),
            CRYPTO => return Self::coin(symbol),
            INDEX => return Self::index(symbol),
            OCC => return Self::option(symbol),
            _ => {}
        }
        let mic = mic_of(venue).ok_or_else(|| NotAListing(format!("{venue:?} is no venue a listing is keyed by")))?;
        let venue = listing_venue(mic).ok_or_else(|| NotAListing(format!("{mic} is no venue a listing is keyed by")))?;
        Self::new(symbol, venue)
    }

    /// A coin, by its asset.
    pub fn coin(asset: &str) -> Result<ListingKey, NotAListing> {
        Self::new(asset, CRYPTO)
    }

    /// An instrument of Bagholder's own directory, by the symbol it names it.
    pub fn index(symbol: &str) -> Result<ListingKey, NotAListing> {
        let symbol = symbol.trim().to_ascii_uppercase();
        if symbol.is_empty() {
            return Err(NotAListing("a directory instrument with no symbol".into()));
        }
        Ok(ListingKey { symbol, venue: INDEX.into() })
    }

    /// An option contract, by its OCC symbol (spaces as the OCC writes them dropped).
    pub fn option(occ: &str) -> Result<ListingKey, NotAListing> {
        let symbol: String = occ.split_whitespace().collect::<Vec<_>>().join("").to_ascii_uppercase();
        if symbol.is_empty() {
            return Err(NotAListing("an option contract with no OCC symbol".into()));
        }
        Ok(ListingKey { symbol, venue: OCC.into() })
    }

    fn new(symbol: &str, venue: &str) -> Result<ListingKey, NotAListing> {
        let symbol = spelling(symbol);
        if symbol.is_empty() {
            return Err(NotAListing(format!("a listing on {venue} with no symbol")));
        }
        Ok(ListingKey { symbol, venue: venue.into() })
    }

    /// The key a book instrument's market data is kept under, from what it is and
    /// what it is called now: a coin by its asset, a directory instrument (an
    /// index, a future, a rate, a currency pair) by its symbol, a share or fund by
    /// its symbol on the venue its records name. None for an option contract
    /// (charted as its underlying) or a prediction market's contract, and for an
    /// instrument whose records name no venue a key is made from: it gets no
    /// market data until a record states one.
    pub fn of(kind: crate::instrument::InstrumentKind, name: &crate::instrument::Name) -> Result<ListingKey, NotAListing> {
        use crate::instrument::InstrumentKind as K;
        match kind {
            K::Crypto => ListingKey::coin(&name.symbol),
            K::Index | K::Future | K::Rate | K::CurrencyPair => ListingKey::index(&name.symbol),
            K::Security => {
                let venue = name.venue_mic.as_deref().or(name.venue_name.as_deref()).ok_or_else(|| NotAListing(format!("{} names no venue", name.symbol)))?;
                ListingKey::on(&name.symbol, venue)
            }
            K::OptionContract | K::EventContract => Err(NotAListing(format!("{} is not a listing market data is kept for", name.symbol))),
        }
    }

    /// The key the market's own news is kept under (`*@MARKET`).
    pub fn market_feed() -> ListingKey {
        ListingKey { symbol: "*".into(), venue: MARKET.into() }
    }

    /// Whether this is the market's own news rather than a listing's.
    pub fn is_market_feed(&self) -> bool {
        self.venue == MARKET
    }

    /// The symbol, in the key's one spelling.
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    /// The venue: a Canadian venue's code, `US`, `CRYPTO`, `INDEX` or `OCC`.
    pub fn venue(&self) -> &str {
        &self.venue
    }

    /// The listing as a person reads it: its symbol and its venue in words
    /// (`TD · TSX`, `TD · US`).
    pub fn named(&self) -> String {
        let venue = VENUE_NAMES.iter().find(|(v, _)| *v == self.venue).map_or(self.venue.as_str(), |(_, n)| *n);
        format!("{} · {venue}", self.symbol)
    }

    /// The market cache's meta key for what is kept of this listing under
    /// `prefix` (one of `meta`'s), its parts joined by `:` and the key last: the
    /// one way such a key is built, so no meta key names a listing by its ticker.
    pub fn meta(&self, prefix: &str, parts: &[&str]) -> String {
        let mut out = String::from(prefix);
        for p in parts {
            out.push(':');
            out.push_str(p);
        }
        out.push(':');
        out.push_str(&self.to_string());
        out
    }

    /// Whether the listing is on a Canadian venue.
    pub fn canadian(&self) -> bool {
        CANADA_MICS.contains(&self.venue.as_str())
    }

    /// Whether the listing is on the US market.
    pub fn us(&self) -> bool {
        self.venue == US
    }
}

/// `SYMBOL@VENUE`, the form a key is stored and sent in.
impl fmt::Display for ListingKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.symbol, self.venue)
    }
}

impl std::str::FromStr for ListingKey {
    type Err = NotAListing;
    /// A key as `Display` writes it; a venue it does not name is refused.
    fn from_str(s: &str) -> Result<ListingKey, NotAListing> {
        let (symbol, venue) = s.rsplit_once('@').ok_or_else(|| NotAListing(format!("{s:?} names no venue")))?;
        match venue {
            CRYPTO => ListingKey::coin(symbol),
            INDEX => ListingKey::index(symbol),
            OCC => ListingKey::option(symbol),
            MARKET if symbol == "*" => Ok(ListingKey::market_feed()),
            US => ListingKey::new(symbol, US),
            mic if CANADA_MICS.contains(&mic) => ListingKey::new(symbol, mic),
            other => Err(NotAListing(format!("{other:?} is no venue a listing is keyed by"))),
        }
    }
}

impl serde::Serialize for ListingKey {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for ListingKey {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<ListingKey, D::Error> {
        let text = String::deserialize(d)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every venue the tables know keys as itself in Canada and as the one US
    /// market in the US, round-trips through its text, and two venues of one
    /// country never share a key, whatever the symbol.
    #[test]
    fn every_venue_keys_its_listings_apart_and_round_trips() {
        let symbols = ["A", "RY", "BRK.B", "BEP.UN", "X1", "ZZZZZ"];
        for s in symbols {
            for mic in CANADA_MICS.iter().chain(US_MICS.iter()) {
                let k = ListingKey::on(s, mic).unwrap();
                assert_eq!(k.to_string().parse::<ListingKey>().unwrap(), k);
                assert_eq!(k.venue(), if US_MICS.contains(mic) { US } else { mic });
            }
            for a in CANADA_MICS {
                for b in CANADA_MICS.iter().chain([US].iter()) {
                    if a != *b {
                        assert_ne!(ListingKey::on(s, a).unwrap(), ListingKey::on(s, b).unwrap(), "{s} on {a} and {b}");
                    }
                }
            }
            // every US exchange is one market
            let us: Vec<ListingKey> = US_MICS.iter().map(|m| ListingKey::on(s, m).unwrap()).collect();
            assert!(us.windows(2).all(|w| w[0] == w[1]));
            // a coin, a directory instrument and a share with one symbol are three listings
            let three = [ListingKey::coin(s).unwrap(), ListingKey::index(s).unwrap(), ListingKey::on(s, "XNYS").unwrap()];
            assert!(three[0] != three[1] && three[1] != three[2] && three[0] != three[2]);
        }
    }

    #[test]
    fn a_class_is_one_spelling_and_a_yahoo_suffix_is_the_venues_own() {
        for written in ["BRK.B", "BRK-B", "BRK/B", "brk.b", " BRK.B "] {
            assert_eq!(ListingKey::on(written, "NYSE").unwrap().symbol(), "BRK.B");
        }
        assert_ne!(ListingKey::on("BRK", "NYSE").unwrap(), ListingKey::on("BRK.B", "NYSE").unwrap());
        assert_eq!(ListingKey::on("ENB.TO", "TSX").unwrap(), ListingKey::on("ENB", "TSX").unwrap());
        assert_eq!(ListingKey::on("BEP.UN", "TSX").unwrap().symbol(), "BEP.UN");
    }

    #[test]
    fn no_key_without_a_venue_or_a_symbol() {
        for venue in ["", "OTC", "PINK", "LSE", "unknown"] {
            assert!(ListingKey::on("RY", venue).is_err(), "{venue}");
        }
        assert!(ListingKey::on("", "TSX").is_err());
        for text in ["RY", "RY@", "RY@XLON", "@US", "RY@NYSE"] {
            assert!(text.parse::<ListingKey>().is_err(), "{text}");
        }
    }
}
