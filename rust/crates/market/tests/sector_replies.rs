//! Every sector name the sources send folds onto the set, or is a stated
//! non-sector (cash, other), or names a fund a fund of funds holds (#524). The
//! replies are recorded from each source the classification reads (TMX Money's
//! quote, Nasdaq's summary, Yahoo Finance's fund profile) for listings across every
//! sector, through the app's own readers, and kept in `sources/tests/replies`; a
//! name a source starts sending that nothing folds fails here.
//!
//! Recorded again with `cargo test -p bagholder-market --test sector_replies -- --ignored record`.

use std::path::PathBuf;

use serde_json::Value;

use bagholder_market::exposure;

fn replies() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sources/tests/replies")
}

/// TMX Money's keys and Nasdaq's symbols: a listing of each sector, on each venue
/// the classification reads, and one whose industry is all the sector it says.
const TMX: &[&str] = &["SHOP", "RY", "ENB", "CNR", "ATD", "DOL", "NTR", "FTS", "REI.UN", "BCE", "BHC", "CAE", "AAPL:US", "BA:US"];
const NASDAQ: &[&str] = &["AAPL", "JPM", "XOM", "UNH", "CAT", "PG", "AMZN", "LIN", "NEE", "PLD", "GOOGL", "BA", "T"];
const YAHOO_FUND: &str = "XEQT.TO";
/// Evolve's pages: a fund of funds, whose breakdown names the funds it holds (#524),
/// and a fund of shares.
const EVOLVE: &[&str] = &["EASY", "LIFE"];

#[test]
#[ignore = "reads the sources over the network; run by hand to record their replies"]
fn record() {
    let dir = replies();
    for key in TMX {
        let reply = exposure::tmx_sector_reply(key).unwrap();
        std::fs::write(dir.join("tmx").join(format!("sector-{}.json", key.replace(':', "-"))), serde_json::to_string_pretty(&reply).unwrap()).unwrap();
    }
    std::fs::create_dir_all(dir.join("nasdaq")).unwrap();
    for s in NASDAQ {
        let raw = exposure::nasdaq_summary_reply(s).unwrap();
        let v: Value = serde_json::from_str(&raw).unwrap();
        std::fs::write(dir.join("nasdaq").join(format!("summary-{s}.json")), serde_json::to_string_pretty(&v).unwrap()).unwrap();
    }
    for fund in EVOLVE {
        std::fs::write(dir.join("evolve").join(format!("breakdown-{}.html", fund.to_lowercase())), exposure::evolve_page_reply(fund).unwrap()).unwrap();
    }
    let (status, raw) = bagholder_market::shorts::yahoo_quote_summary(YAHOO_FUND, "topHoldings").unwrap().expect("the browser helper");
    assert_eq!(status, 200, "{raw}");
    let v: Value = serde_json::from_str(&raw).unwrap();
    // a fund profile is no chart: kept apart from Yahoo's chart answers, whose shape is held
    std::fs::create_dir_all(dir.join("yahoo-fund")).unwrap();
    std::fs::write(dir.join("yahoo-fund").join(format!("topholdings-{YAHOO_FUND}.json")), serde_json::to_string_pretty(&v).unwrap()).unwrap();
}

/// Industries the recorded sources give that two sectors share: no sector is
/// taken from them (`sector_of_industry` names none).
const SHARED_INDUSTRIES: &[&str] = &["agriculture", "telecommunications equipment", "computer software: programming, data processing"];

fn json(path: &std::path::Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The files of `dir` whose names start with `prefix`.
fn recorded(dir: &str, prefix: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(replies().join(dir)).unwrap().flatten().map(|e| e.path()).filter(|p| p.file_name().unwrap().to_string_lossy().starts_with(prefix)).collect();
    out.sort();
    assert!(!out.is_empty(), "no {prefix}* replies recorded in {dir}");
    out
}

/// A sector name a source sends: one of the set once folded, or a stated non-sector.
fn folds(name: &str) -> bool {
    !exposure::norm_sector(name).is_empty() || bagholder_model::exposure::stated_non_sector(name)
}

/// An industry a source sends: inside one sector, or one the table names as shared.
fn placed(industry: &str) -> bool {
    !bagholder_model::exposure::sector_of_industry(industry).is_empty() || SHARED_INDUSTRIES.contains(&industry.trim().to_lowercase().as_str())
}

#[test]
fn every_sector_and_industry_the_share_sources_send_is_placed() {
    let mut seen = 0;
    for f in recorded("tmx", "sector-") {
        let q = &json(&f)["data"]["getQuoteBySymbol"];
        let (sector, industry) = (q["sector"].as_str().unwrap_or(""), q["industry"].as_str().unwrap_or(""));
        assert!(folds(sector), "{}: TMX Money's sector {sector:?} folds onto nothing", f.display());
        assert!(placed(industry), "{}: TMX Money's industry {industry:?} is in no sector", f.display());
        seen += 1;
    }
    for f in recorded("nasdaq", "summary-") {
        let d = &json(&f)["data"]["summaryData"];
        let (sector, industry) = (d["Sector"]["value"].as_str().unwrap_or(""), d["Industry"]["value"].as_str().unwrap_or(""));
        assert!(folds(sector), "{}: Nasdaq's sector {sector:?} folds onto nothing", f.display());
        assert!(placed(industry), "{}: Nasdaq's industry {industry:?} is in no sector", f.display());
        seen += 1;
    }
    assert!(seen >= 20, "the share sources recorded cover every sector: {seen} replies");
}

#[test]
fn every_sector_a_fund_source_sends_folds_or_names_a_fund() {
    for f in recorded("yahoo-fund", "topholdings-") {
        let th = &json(&f)["quoteSummary"]["result"][0]["topHoldings"];
        for w in th["sectorWeightings"].as_array().unwrap() {
            for k in w.as_object().unwrap().keys() {
                // Yahoo writes its sectors as keys, words joined by underscores
                assert!(folds(&k.replace('_', " ")), "{}: Yahoo's sector {k:?} folds onto nothing", f.display());
            }
        }
    }
    let between = |html: &str, var: &str| -> Value {
        let at = html.find(&format!("var {var}")).unwrap_or_else(|| panic!("no {var}"));
        let from = at + html[at..].find('{').unwrap();
        let to = from + html[from..].find("};").unwrap() + 1;
        serde_json::from_str(&html[from..to]).unwrap()
    };
    for f in recorded("evolve", "breakdown-") {
        let html = std::fs::read_to_string(&f).unwrap();
        for r in between(&html, "portfolioBreakdownData")["data"]["sector"].as_array().unwrap() {
            let name = r["name"].as_str().unwrap();
            assert!(folds(name) || exposure::is_fund(name), "{}: Evolve's breakdown row {name:?} is neither a sector nor a fund", f.display());
        }
        for r in between(&html, "holdingsData")["data"].as_array().unwrap() {
            let name = r["gics_sector"].as_str().unwrap_or("");
            assert!(folds(name) || exposure::is_fund(name), "{}: Evolve's holding's sector {name:?} is neither a sector nor a fund", f.display());
        }
    }
}

/// The fund of funds #524 was found on, as Evolve's page states it: its breakdown
/// names no sector, so it is looked through to the funds it holds.
#[test]
fn evolves_fund_of_funds_page_states_its_funds_not_sectors() {
    let html = std::fs::read_to_string(replies().join("evolve/breakdown-easy.html")).unwrap();
    let (sectors, holdings) = exposure::parse_evolve_page(&html).unwrap();
    assert!(sectors.is_empty(), "{sectors:?}");
    assert!(holdings.iter().any(|h| h.fund), "the funds it holds are its holdings");
}

/// An industry is placed in the sector it sits wholly inside: every GICS industry
/// and every name the recorded sources give, to one of the set; a sector a share's
/// source states that the set does not hold gives way to its industry's.
#[test]
fn an_industry_is_placed_in_the_sector_it_sits_inside() {
    use bagholder_model::exposure::{sector_of_industry, SECTORS};
    for (industry, sector) in [("Aerospace & Defense", "Industrials"), ("Aerospace", "Industrials"), ("Pharmaceuticals", "Health Care"), ("REITs", "Real Estate"), ("Banking", "Financials")] {
        assert_eq!(sector_of_industry(industry), sector, "{industry}");
    }
    for shared in SHARED_INDUSTRIES {
        assert_eq!(sector_of_industry(shared), "", "{shared} is two sectors'");
    }
    for name in ["Aerospace & Defense", "Major Chemicals", "Integrated oil Companies"] {
        assert!(SECTORS.contains(&sector_of_industry(name).as_str()), "{name}");
    }
}
