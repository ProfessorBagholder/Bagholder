//! The TSX's list of its inter-listed securities: each TSX or TSX Venture listing
//! that is also listed on a US national securities exchange, and its US symbol.
//! A recognized exchange must keep and publish this list (National Instrument
//! 23-101, s. 6.6.1(3)–(4)); the TSX publishes it as a text file, compiled "on a
//! best efforts basis from information received from issuers". It is the one
//! statement the app reads that a Canadian listing and a US one are the same
//! company's, so a listing it pairs shows both regulators' filings (plan
//! `market-data-keyed-by-listing`, Approach, 2).

use bagholder_core::ListingKey;
use crate::stored::feeds::Interlisted;

use crate::http::FetchError;

pub const URL: &str = "https://www.tsx.com/files/trading/interlisted-companies.txt";
/// Read at most this often: the TSX states the list as of a quarter's end.
pub const READ_EVERY_HOURS: f64 = 24.0;

/// The words the list's `International Market` column names a US national
/// securities exchange by. Any other market (the ASX, AIM, the LSE, …) is no US
/// listing, and its row pairs nothing.
const US_MARKETS: [&str; 7] = ["NYSE", "NYSE MKT", "NYSE ARCA", "NASDAQ", "NASDAQCM", "NASDAQGS", "NASDAQGM"];

const MONTHS: [&str; 12] = ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];

/// The day the list states it is as of (`As of October 1, 2026`), as `YYYY-MM-DD`.
fn as_of(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("As of ")?;
    let (month_day, year) = rest.rsplit_once(',')?;
    let (month, day) = month_day.trim().split_once(' ')?;
    let m = MONTHS.iter().position(|x| x.eq_ignore_ascii_case(month))? + 1;
    let d: u32 = day.trim().parse().ok()?;
    let y: u32 = year.trim().parse().ok()?;
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

/// The list read: the day it is as of, and each row that pairs a TSX or TSX
/// Venture listing with a listing on a US exchange. A reply that is not this
/// list's shape (no `As of` line, a header other than the one read) is a reply
/// that does not read, never an empty list.
pub fn parse(text: &str) -> Result<(String, Vec<Interlisted>), String> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let day = lines.next().and_then(as_of).ok_or("the TSX's interlisted list names no day it is as of")?;
    let header: Vec<&str> = lines.next().ok_or("the TSX's interlisted list has no header")?.split('\t').map(str::trim).collect();
    if header != ["Symbol", "Name", "US Symbol", "Sector", "International Market"] {
        return Err(format!("the TSX's interlisted list has the columns {header:?}, not the ones read"));
    }
    let mut out = Vec::new();
    for line in lines {
        let cells: Vec<&str> = line.split('\t').map(str::trim).collect();
        let [symbol, name, us_symbol, _, market] = cells[..] else {
            return Err(format!("a row of the TSX's interlisted list does not read: {line:?}"));
        };
        if us_symbol.is_empty() || !US_MARKETS.contains(&market.to_ascii_uppercase().as_str()) {
            continue;
        }
        let (root, venue) = symbol.rsplit_once(':').ok_or_else(|| format!("{symbol:?} in the TSX's interlisted list names no venue"))?;
        let listing = ListingKey::on(root, venue).map_err(|e| format!("{symbol:?} in the TSX's interlisted list: {e}"))?;
        let us_listing = ListingKey::on(us_symbol, "US").map_err(|e| format!("{us_symbol:?} in the TSX's interlisted list: {e}"))?;
        out.push(Interlisted { listing, us_listing, name: name.to_string() });
    }
    Ok((day, out))
}

/// The list as the TSX publishes it now.
pub fn fetch() -> Result<(String, Vec<Interlisted>), FetchError> {
    let text = crate::http::get_text(URL, &[("User-Agent", crate::http::UA), ("Accept", "text/plain,*/*")])?;
    parse(&text).map_err(FetchError::Transport)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_on_a_us_exchange_pairs_its_listings_and_every_other_pairs_none() {
        let text = "As of October 1, 2026\n\nSymbol\tName\tUS Symbol\tSector\tInternational Market\nT:TSX\tTELUS Corporation\tTU\tComm. & Media\tNYSE\nFURY:TSX\tFury Gold Mines Limited\tFURY\tMining\tNYSE Mkt\nAFE:TSXV\tAfrica Energy Corp.\t\tOil & Gas\tNasdaq Nordic\nXX:TSXV\tX Corp.\tXX\tMining\tASX\nBRK.B:TSX\tY Inc.\tY-B\tMining\tNasdaqGS\n";
        let (day, rows) = parse(text).unwrap();
        assert_eq!(day, "2026-10-01");
        let pairs: Vec<(String, String)> = rows.iter().map(|r| (r.listing.to_string(), r.us_listing.to_string())).collect();
        assert_eq!(pairs, vec![("T@XTSE".into(), "TU@US".into()), ("FURY@XTSE".into(), "FURY@US".into()), ("BRK.B@XTSE".into(), "Y.B@US".into())]);
    }

    #[test]
    fn a_reply_that_is_not_the_list_does_not_read() {
        for text in ["", "<html>busy</html>", "As of October 1, 2026\nSymbol\tName\n", "As of Octember 1, 2026\nSymbol\tName\tUS Symbol\tSector\tInternational Market\n"] {
            assert!(parse(text).is_err(), "{text:?}");
        }
        assert!(parse("As of October 1, 2026\nSymbol\tName\tUS Symbol\tSector\tInternational Market\nT:TSX\tTELUS\n").is_err(), "a short row");
    }
}
