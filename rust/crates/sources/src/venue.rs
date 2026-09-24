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

use crate::contract::Market;

/// The market of a venue, by its market identifier code. A venue this table
/// does not name is covered by no source here, and says so.
pub fn market_of(mic: Option<&str>) -> Option<Market> {
    match mic? {
        // the TSX, the TSX Venture, the CSE, and Alpha (TSX Alpha Exchange, an
        // ATS whose listings are the TSX's)
        "XTSE" | "XTSX" | "XCNQ" | "XATS" => Some(Market::Canada),
        "NEOE" => Some(Market::CboeCanada),
        // Nasdaq, the NYSE, NYSE Arca, NYSE American, Cboe's US equities (BZX)
        "XNAS" | "XNYS" | "ARCX" | "XASE" | "BATS" => Some(Market::UnitedStates),
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
        "XNAS" | "XNYS" | "ARCX" | "XASE" | "BATS" => Some(":US"),
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

/// Yahoo's suffix for a venue.
pub fn yahoo_suffix(mic: &str) -> Option<&'static str> {
    match mic {
        "XTSE" | "XATS" => Some(".TO"),
        "XTSX" => Some(".V"),
        "XCNQ" => Some(".CN"),
        "NEOE" => Some(".NE"),
        "XNAS" | "XNYS" | "ARCX" | "XASE" | "BATS" => Some(""),
        _ => None,
    }
}

/// The Yahoo form of a listing: its root with a class written with a dash, and
/// its venue's suffix.
pub fn yahoo_form(symbol: &str, mic: &str) -> Option<String> {
    let r = root(symbol).replace('.', "-");
    if r.is_empty() || r.contains(' ') {
        return None;
    }
    yahoo_suffix(mic).map(|s| format!("{r}{s}"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(yahoo_form("BBD.A", "XTSE").as_deref(), Some("BBD-A.TO"));
        assert_eq!(yahoo_form("QNC.TO", "XTSX").as_deref(), Some("QNC.V"));
        assert_eq!(yahoo_form("BRK.B", "XNYS").as_deref(), Some("BRK-B"));
        assert_eq!(yahoo_form("MAXQ", "NEOE").as_deref(), Some("MAXQ.NE"));
        assert_eq!(tmx_form("X", "XLON"), None);
        assert_eq!(yahoo_form("TWO WORDS", "XTSE"), None);
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
    fn a_tmx_answer_counts_only_on_the_venue_asked() {
        assert!(tmx_venue_matches("", "Toronto Stock Exchange"));
        assert!(tmx_venue_matches("", "TSX Venture Exchange"));
        assert!(!tmx_venue_matches("", "Canadian Securities Exchange"));
        assert!(tmx_venue_matches(":AQL", "Cboe Canada"));
    }
}
