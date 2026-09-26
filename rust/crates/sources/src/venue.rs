//! How each source writes a listing's symbol, by the venue the book's record
//! names (moved from `bagholder-market`'s TMX and Yahoo readers, the logic the
//! design review kept).
//!
//! A ticker is written differently by each source: TMX bare for the TSX and the
//! Venture, `:CNX` for the CSE, `:AQL` for Cboe Canada and `:US` for a US listing;
//! Yahoo with `.TO`, `.V`, `.CN`, `.NE`, or bare for a US listing, and a class
//! written with a dash (`BBD-A.TO`). A record's symbol may carry a Yahoo suffix of
//! its own (`ENB.TO` from a file import); the venue the record names decides the
//! form, never that suffix.
//!
//! A listing whose venue the record does not name, or names by a code or words
//! no table here knows (an OTC venue, a file-imported row), follows its currency
//! (`SPEC.md` §2, TMX Money; §4, Short interest): a CAD listing is asked as a
//! Canadian one, from TMX's bare form and Yahoo's `.TO`, a USD listing as a US one.
//! Each source is then asked the other forms of that market in turn, so a venue
//! the record names wrongly still finds the listing.

use bagholder_core::Currency;

use crate::contract::Market;

/// The market of a venue, by its market identifier code. A venue this table
/// does not name is covered by no source here, and says so.
pub fn market_of(mic: Option<&str>) -> Option<Market> {
    match mic? {
        // the TSX, the TSX Venture, the CSE, and Alpha (TSX Alpha Exchange, a
        // venue trading both the TSX's and the TSX Venture's issues)
        "XTSE" | "XTSX" | "XCNQ" | "XATS" => Some(Market::Canada),
        "NEOE" => Some(Market::CboeCanada),
        // Nasdaq, the NYSE, NYSE Arca, NYSE American, Cboe's US equities (BZX), IEX
        "XNAS" | "XNYS" | "ARCX" | "XASE" | "BATS" | "IEXG" => Some(Market::UnitedStates),
        _ => None,
    }
}

/// The market of a listing: its venue's, else its currency's (CAD, Canada; USD,
/// the US), else none.
pub fn market_of_listing(mic: Option<&str>, currency: Currency) -> Option<Market> {
    market_of(mic).or(match currency {
        Currency::CAD => Some(Market::Canada),
        Currency::USD => Some(Market::UnitedStates),
        _ => None,
    })
}

/// The venue a record names, as a code this module knows: its code, else its
/// words (`TSX Venture Exchange`, `CBOE`); none where neither is known.
pub fn known_mic(mic: Option<&str>, words: Option<&str>) -> Option<&'static str> {
    mic.and_then(mic_of).or_else(|| words.and_then(mic_of))
}

/// The market identifier code of a venue as a broker, a directory or a person
/// writes it in words (`TSX-V`, `NASDAQ`, `Cboe Canada`); none for words no venue
/// here is known by.
pub fn mic_of(words: &str) -> Option<&'static str> {
    match words.trim().to_ascii_uppercase().as_str() {
        "TSX" | "TORONTO" | "TORONTO STOCK EXCHANGE" | "XTSE" => Some("XTSE"),
        // `XTSV` is how some records write the TSX Venture's code
        "TSX-V" | "TSXV" | "TSX VENTURE" | "TSX VENTURE EXCHANGE" | "VENTURE" | "CDNX" | "XTSX" | "XTSV" => Some("XTSX"),
        "CSE" | "CANADIAN SECURITIES EXCHANGE" | "XCNQ" => Some("XCNQ"),
        "CBOE CANADA" | "CBOE CA" | "NEO" | "NEOE" => Some("NEOE"),
        "ALPHA" | "ALPHA EXCHANGE" | "XATS" => Some("XATS"),
        "NASDAQ" | "XNAS" => Some("XNAS"),
        "NYSE" | "XNYS" => Some("XNYS"),
        "NYSE ARCA" | "ARCA" | "ARCX" => Some("ARCX"),
        "NYSE AMERICAN" | "AMEX" | "XASE" => Some("XASE"),
        // `CBOE` alone is Cboe's US equities venue; Cboe Canada is named as such
        "BATS" | "CBOE BZX" | "CBOE" => Some("BATS"),
        "IEX" | "IEXG" => Some("IEXG"),
        _ => None,
    }
}

/// The currency a venue's listings trade in, where the venue says it.
pub fn currency_of(mic: &str) -> Option<bagholder_core::Currency> {
    match market_of(Some(mic))? {
        Market::Canada | Market::CboeCanada => Some(bagholder_core::Currency::CAD),
        Market::UnitedStates => Some(bagholder_core::Currency::USD),
        _ => None,
    }
}

/// The symbol without a Yahoo venue suffix a record may carry (`ENB.TO` → `ENB`).
/// A class or unit suffix (`BBD.A`, `BEP.UN`) is part of the symbol and stays.
pub fn root(symbol: &str) -> String {
    let s = symbol.trim().to_ascii_uppercase();
    for suffix in [".TO", ".V", ".CN", ".NE"] {
        if let Some(r) = s.strip_suffix(suffix) {
            return r.to_string();
        }
    }
    s
}

/// TMX's suffix for a venue.
pub fn tmx_suffix(mic: &str) -> Option<&'static str> {
    match mic {
        "XTSE" | "XTSX" | "XATS" => Some(""),
        "XCNQ" => Some(":CNX"),
        "NEOE" => Some(":AQL"),
        "XNAS" | "XNYS" | "ARCX" | "XASE" | "BATS" | "IEXG" => Some(":US"),
        _ => None,
    }
}

/// The words the venue TMX's reply names must contain for its answer to be the
/// listing asked about (TMX answers a form on another venue with that venue's
/// listing, which is another security).
pub fn tmx_venue_words(suffix: &str) -> &'static [&'static str] {
    match suffix {
        "" => &["TORONTO STOCK EXCHANGE", "TSX VENTURE"],
        ":CNX" => &["CANADIAN SECURITIES EXCHANGE"],
        ":AQL" => &["CBOE", "NEO"],
        ":US" => &["NYSE", "NASDAQ", "NEW YORK", "CBOE"],
        _ => &[],
    }
}

/// Whether the venue TMX's reply names is the one a form asks for.
pub fn tmx_venue_matches(suffix: &str, exchange_name: &str) -> bool {
    let up = exchange_name.to_ascii_uppercase();
    tmx_venue_words(suffix).iter().any(|w| up.contains(w))
}

/// The TMX form of a listing: its root and its venue's suffix.
pub fn tmx_form(symbol: &str, mic: &str) -> Option<String> {
    let r = root(symbol);
    if r.is_empty() || r.contains(' ') {
        return None;
    }
    tmx_suffix(mic).map(|s| format!("{r}{s}"))
}

/// The Canadian venues, the likelier first: the forms asked after a listing's
/// own, when its venue is wrong or missing.
const CANADIAN_VENUES: [&str; 4] = ["XTSE", "XTSX", "XCNQ", "NEOE"];
/// The US venues asked after a listing's own: Yahoo writes every US listing bare.
const US_VENUES: [&str; 1] = ["XNAS"];

/// The venues a listing is asked on, in order: its own, then the other venues
/// of its market (by its currency where its venue is none known here).
fn venues_of(mic: Option<&str>, currency: Currency) -> Vec<&str> {
    let own = mic.filter(|m| market_of(Some(m)).is_some());
    let others: &[&str] = match market_of_listing(own, currency) {
        Some(Market::Canada | Market::CboeCanada) => &CANADIAN_VENUES,
        Some(Market::UnitedStates) => &US_VENUES,
        _ => &[],
    };
    own.into_iter().chain(others.iter().copied()).collect()
}

/// TMX's forms of a Canadian listing, its venue's own first (bare where the
/// venue is none TMX names), then the other Canadian venues' (bare, `:CNX`,
/// `:AQL`). TMX answers each form on the venue it names, which the reader
/// checks, so at most the listing's own venue answers for it.
pub fn tmx_forms(symbol: &str, mic: Option<&str>, currency: Currency) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for v in venues_of(mic, currency) {
        if matches!(market_of(Some(v)), Some(Market::UnitedStates)) {
            continue;
        }
        if let Some(f) = tmx_form(symbol, v) {
            if !out.contains(&f) {
                out.push(f);
            }
        }
    }
    out
}

/// Yahoo's suffixes for a venue, the likelier first. Alpha (TSX Alpha
/// Exchange) lists nothing: it trades both the TSX's and the TSX Venture's
/// issues, so a trade there names either (01 Communique, filled on Alpha, is
/// `ONE.V`); TMX gives its issues one set of symbols across both, so at most one
/// of the two answers.
pub fn yahoo_suffixes(mic: &str) -> &'static [&'static str] {
    match mic {
        "XTSE" => &[".TO"],
        "XATS" => &[".TO", ".V"],
        "XTSX" => &[".V"],
        "XCNQ" => &[".CN"],
        "NEOE" => &[".NE"],
        "XNAS" | "XNYS" | "ARCX" | "XASE" | "BATS" | "IEXG" => &[""],
        _ => &[],
    }
}

/// The Yahoo forms of a listing, the likelier first: its root with a class
/// written with a dash, and each of its venue's suffixes.
pub fn yahoo_forms(symbol: &str, mic: &str) -> Vec<String> {
    let r = root(symbol).replace('.', "-");
    if r.is_empty() || r.contains(' ') {
        return vec![];
    }
    yahoo_suffixes(mic).iter().map(|s| format!("{r}{s}")).collect()
}

/// The Yahoo forms a listing is asked under, in order: its venue's own, then
/// the other venues' of its market (`.TO`, `.V`, `.CN`, `.NE` for a Canadian
/// listing; bare for a US one), its currency's where its venue is none known.
pub fn yahoo_forms_of(symbol: &str, mic: Option<&str>, currency: Currency) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for v in venues_of(mic, currency) {
        for f in yahoo_forms(symbol, v) {
            if !out.contains(&f) {
                out.push(f);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_venue_in_words_is_its_code_and_its_market_s_currency() {
        assert_eq!(mic_of("TSX-V"), Some("XTSX"));
        assert_eq!(mic_of("Cboe Canada"), Some("NEOE"));
        assert_eq!(mic_of(" nasdaq "), Some("XNAS"));
        assert_eq!(mic_of("CRYPTO"), None);
        assert_eq!(currency_of("XCNQ"), Some(bagholder_core::Currency::CAD));
        assert_eq!(currency_of("BATS"), Some(bagholder_core::Currency::USD));
        assert_eq!(currency_of("XLON"), None);
    }

    #[test]
    fn a_record_suffix_is_dropped_and_a_class_kept() {
        assert_eq!(root("ENB.TO"), "ENB");
        assert_eq!(root("BBD.A"), "BBD.A");
        assert_eq!(root("BEP.UN"), "BEP.UN");
        assert_eq!(root("nkw.h"), "NKW.H");
    }

    #[test]
    fn each_source_writes_the_venue_its_own_way() {
        assert_eq!(tmx_form("ENB.TO", "XTSE").as_deref(), Some("ENB"));
        assert_eq!(tmx_form("BBD.A", "XTSE").as_deref(), Some("BBD.A"));
        assert_eq!(tmx_form("ABC", "XCNQ").as_deref(), Some("ABC:CNX"));
        assert_eq!(tmx_form("WQTM", "BATS").as_deref(), Some("WQTM:US"));
        assert_eq!(yahoo_forms("BBD.A", "XTSE"), ["BBD-A.TO"]);
        assert_eq!(yahoo_forms("QNC.TO", "XTSX"), ["QNC.V"]);
        assert_eq!(yahoo_forms("BRK.B", "XNYS"), ["BRK-B"]);
        assert_eq!(yahoo_forms("MAXQ", "NEOE"), ["MAXQ.NE"]);
        // a fill on Alpha names a TSX or a TSX Venture issue
        assert_eq!(yahoo_forms("ONE", "XATS"), ["ONE.TO", "ONE.V"]);
        assert_eq!(tmx_form("X", "XLON"), None);
        assert!(yahoo_forms("TWO WORDS", "XTSE").is_empty());
        assert!(yahoo_forms("X", "XLON").is_empty());
    }

    #[test]
    fn the_venue_named_decides_the_market() {
        assert_eq!(market_of(Some("XATS")), Some(Market::Canada));
        assert_eq!(market_of(Some("NEOE")), Some(Market::CboeCanada));
        assert_eq!(market_of(Some("BATS")), Some(Market::UnitedStates));
        // a listing whose record names no venue is covered by no source
        assert_eq!(market_of(None), None);
    }

    #[test]
    fn the_old_words_for_each_venue_are_known() {
        for (words, mic) in [("CBOE", "BATS"), ("IEX", "IEXG"), ("CDNX", "XTSX"), ("VENTURE", "XTSX"), ("TSX Venture Exchange", "XTSX"), ("XTSV", "XTSX"), ("TORONTO", "XTSE"), ("Toronto Stock Exchange", "XTSE"), ("Canadian Securities Exchange", "XCNQ")] {
            assert_eq!(mic_of(words), Some(mic), "{words}");
        }
        assert_eq!(market_of(Some("IEXG")), Some(Market::UnitedStates));
        // a code no table knows gives way to the venue's words, then to nothing
        assert_eq!(known_mic(Some("XTSV"), None), Some("XTSX"));
        assert_eq!(known_mic(Some("OTCM"), Some("NYSE")), Some("XNYS"));
        assert_eq!(known_mic(Some(""), Some("TSX Venture Exchange")), Some("XTSX"));
        assert_eq!(known_mic(Some("OTCM"), Some("OTC Markets")), None);
    }

    #[test]
    fn a_listing_whose_venue_is_none_known_follows_its_currency() {
        use bagholder_core::Currency;
        assert_eq!(market_of_listing(None, Currency::CAD), Some(Market::Canada));
        assert_eq!(market_of_listing(Some("OTCM"), Currency::USD), Some(Market::UnitedStates));
        assert_eq!(market_of_listing(Some("NEOE"), Currency::USD), Some(Market::CboeCanada));
        // TMX's forms: the venue's own first, then the other Canadian venues'
        assert_eq!(tmx_forms("ENB", None, Currency::CAD), ["ENB", "ENB:CNX", "ENB:AQL"]);
        assert_eq!(tmx_forms("ENB", Some("XCNQ"), Currency::CAD), ["ENB:CNX", "ENB", "ENB:AQL"]);
        assert_eq!(tmx_forms("HBIX", Some("NEOE"), Currency::CAD), ["HBIX:AQL", "HBIX", "HBIX:CNX"]);
        assert!(tmx_forms("SPY", Some("ARCX"), Currency::USD).is_empty());
        // Yahoo's: the venue's own, then its market's others
        assert_eq!(yahoo_forms_of("ONE", Some("XATS"), Currency::CAD), ["ONE.TO", "ONE.V", "ONE.CN", "ONE.NE"]);
        assert_eq!(yahoo_forms_of("BRK.B", None, Currency::USD), ["BRK-B"]);
        assert!(yahoo_forms_of("X", None, Currency::parse("EUR").unwrap()).is_empty());
    }

    #[test]
    fn a_tmx_answer_counts_only_on_the_venue_asked() {
        assert!(tmx_venue_matches("", "Toronto Stock Exchange"));
        assert!(tmx_venue_matches("", "TSX Venture Exchange"));
        assert!(!tmx_venue_matches("", "Canadian Securities Exchange"));
        assert!(tmx_venue_matches(":AQL", "Cboe Canada"));
    }
}
