//! News: every per-symbol source parsed into rows, merged and kept per listing.
use bagholder_market::news;
use serde_json::{json, Value};

const SEP11_1530: i64 = 1789140600;
const SEP15_1200: i64 = 1789473600;
const SEP11_1600: i64 = 1789142400;

fn s(v: &Value) -> &str { v.as_str().unwrap_or("") }

#[test]
fn test_tmx_items_carry_an_exact_time_and_a_page_link() {
    let data = json!({"data": {"news": [{"headline": "Shopify Delivers Big: 30%+ Growth Across&#xA0;GMV", "datetime": "2026-08-05T07:00:00-04:00", "source": "GlobeNewswire via QuoteMedia", "newsid": 4883675477075330u64},
                                        {"headline": "no id", "datetime": "2026-08-05T07:00:00-04:00"},
                                        {"headline": "bad time", "datetime": "yesterday", "newsid": 5}]}});
    let rows = news::parse_tmx_news(&data, "SHOP", false);
    assert_eq!(rows, vec![json!({"id": "tmx:4883675477075330", "headline": "Shopify Delivers Big: 30%+ Growth Across GMV", "source": "GlobeNewswire",
                                  "url": "https://money.tmx.com/en/quote/SHOP/news/4883675477075330", "publishedAt": "2026-08-05T11:00:00Z", "kind": "release", "via": "tmx"})]);
}

#[test]
fn test_nasdaq_items_take_their_time_from_the_age_given() {
    let data = json!({"data": {"rows": [{"id": 28351741, "title": "Forget AMD. Here&#39;s Who Nvidia Really Needs to Be Worried About.", "publisher": "The Motley Fool", "created": "Sep 11, 2026", "ago": "17 minutes ago", "url": "/articles/forget-amd", "primarysymbol": "avgo", "related_symbols": ["avgo|stocks", "nvda|stocks"]},
                                        {"id": 2, "title": "Two hours", "publisher": "Zacks", "created": "Sep 11, 2026", "ago": "2 hours ago", "url": "https://www.nasdaq.com/articles/two", "related_symbols": ["NVDA|stocks"]},
                                        {"id": 3, "title": "Old", "publisher": "Barchart", "created": "Sep 3, 2026", "ago": "", "url": "/articles/old", "primarysymbol": "nvda"},
                                        {"id": 4, "publisher": "no title", "related_symbols": ["nvda|stocks"]},
                                        {"id": 5, "title": "Market wrap that never names it", "publisher": "Barchart", "created": "Sep 11, 2026", "ago": "3 minutes ago", "url": "/articles/wrap", "related_symbols": ["spy|etf", "aapl|stocks"]}]}});
    let rows = news::parse_nasdaq_news(&data, SEP11_1530, "NVDA", None);
    let got: Vec<[&str; 5]> = rows.iter().map(|r| [s(&r["id"]), s(&r["headline"]), s(&r["source"]), s(&r["url"]), s(&r["publishedAt"])]).collect();
    assert_eq!(got, vec![
        ["nasdaq:28351741", "Forget AMD. Here's Who Nvidia Really Needs to Be Worried About.", "The Motley Fool", "https://www.nasdaq.com/articles/forget-amd", "2026-09-11T15:13:00Z"],
        ["nasdaq:2", "Two hours", "Zacks", "https://www.nasdaq.com/articles/two", "2026-09-11T13:30:00Z"],
        ["nasdaq:3", "Old", "Barchart", "https://www.nasdaq.com/articles/old", "2026-09-03T00:00:00Z"],
    ], "an item Nasdaq does not tag with the symbol is left out");
}

#[test]
fn test_the_wire_follows_the_venue() {
    assert_eq!(news::source_for("SHOP", "TSX", "CAD"), "tmx");
    assert_eq!(news::source_for("NVDA", "NASDAQ", "USD"), "nasdaq");
    assert_eq!(news::source_for("AAPL", "", "USD"), "nasdaq");
    assert_eq!(news::source_for("QBTC", "NEO", "CAD"), "tmx");
}

#[test]
fn test_a_wires_item_is_a_release_and_a_publishers_a_story() {
    for wire in ["GlobeNewswire", "Business Wire", "PR Newswire", "ACCESS Newswire", "Accesswire", "TheNewsWire", "Canada Newswire", "TMX Newsfile", "Marketwired", "CNW Group", "NewMediaWire"] {
        assert_eq!(news::kind_of(wire), "release", "{}", wire);
    }
    for publ in ["The Motley Fool", "Zacks", "Barchart", "RTTNews", "MarketBeat", "BNK Invest", "Fintel", "", "WIRED", "MT Newswires", "Dow Jones Newswires"] {
        assert_eq!(news::kind_of(publ), "story", "{}", publ);
    }
    let tmx = news::parse_tmx_news(&json!({"data": {"news": [{"newsid": "1", "headline": "Closing", "source": "GlobeNewswire via QuoteMedia", "datetime": "2026-09-14T08:00:00-04:00"}]}}), "CH", false);
    assert_eq!((s(&tmx[0]["kind"]), s(&tmx[0]["source"])), ("release", "GlobeNewswire"));
    let press = news::parse_nasdaq_news(&json!({"data": {"rows": [{"id": 9, "title": "Shopify Delivers Big", "publisher": "", "created": "Aug 5, 2026", "ago": "Aug 5, 2026", "url": "/press-release/x", "related_symbols": ["shop|stocks"]}]}}), SEP15_1200, "SHOP", Some("release"));
    assert_eq!((s(&press[0]["kind"]), s(&press[0]["source"]), s(&press[0]["publishedAt"])), ("release", "Nasdaq", "2026-08-05T00:00:00Z"));
    let story = news::parse_nasdaq_news(&json!({"data": {"rows": [{"id": 8, "title": "Why SHOP", "publisher": "The Motley Fool", "created": "Sep 14, 2026", "ago": "1 day ago", "url": "/articles/y", "related_symbols": ["shop|stocks"]}]}}), SEP15_1200, "SHOP", None);
    assert_eq!(s(&story[0]["kind"]), "story");
}

#[test]
fn test_the_market_feed_is_a_listing_of_its_own_with_no_tag() {
    assert_eq!(news::source_for(news::MARKET.0, news::MARKET.1, news::MARKET.2), "nasdaq");
    let data = json!({"data": {"rows": [{"id": 1, "title": "Stocks Settle Lower", "publisher": "Barchart", "url": "/articles/a", "ago": "7 minutes ago", "related_symbols": ["ryam|stocks"]},
                                        {"id": 2, "title": "Value ETFs", "publisher": "Zacks", "url": "/articles/b", "ago": "2 hours ago", "related_symbols": ["mu|stocks"]}]}});
    let ids: Vec<&str> = news::parse_nasdaq_news(&data, SEP11_1600, "", None).iter().map(|r| s(&r["id"])).map(|x| Box::leak(x.to_string().into_boxed_str()) as &str).collect();
    assert_eq!(ids, ["nasdaq:1", "nasdaq:2"], "asked without a symbol, the feed keeps every item");
}

// ---------------------------------------------------------------------------
// harness: a store in a temporary home, and a network that answers from the test
// ---------------------------------------------------------------------------

use bagholder_market::news::{Ask, Clock, NetError, Net, Readers, WireAnswer};
use rusqlite::Connection;
use std::collections::HashSet;
use std::sync::Mutex;

struct Db {
    dir: tempfile::TempDir,
    conn: Connection,
}

fn db() -> Db {
    let dir = tempfile::tempdir().unwrap();
    let conn = bagholder_store::connect(dir.path()).unwrap();
    bagholder_store::relabel::ensure(&conn).unwrap();
    Db { dir, conn }
}

fn at(y: i64, m: u32, d: u32, h: i64, mi: i64) -> i64 {
    bagholder_model::dates::to_days(y, m, d) * 86400 + h * 3600 + mi * 60
}

fn no_get(_: &str, _: &[(&str, &str)]) -> Result<String, NetError> {
    panic!("nothing is fetched")
}

fn no_post(_: &str, _: &Value, _: &[(&str, &str)]) -> Result<Value, NetError> {
    panic!("nothing is posted")
}

fn down() -> NetError {
    NetError { code: None, text: "down".into() }
}

fn row(id: &str, headline: &str, when: &str, source: &str, kind: &str) -> Value {
    json!({"id": id, "headline": headline, "source": source, "url": format!("u-{}", id), "publishedAt": when, "kind": kind})
}

fn ids(rows: &[Value]) -> Vec<String> {
    rows.iter().map(|r| s(&r["id"]).to_string()).collect()
}

fn listing(sym: &str, ex: &str, ccy: &str, name: &str) -> news::Listing {
    (sym.into(), ex.into(), ccy.into(), name.into())
}

// ---------------------------------------------------------------------------
// TMX
// ---------------------------------------------------------------------------

#[test]
fn test_tmx_is_asked_under_the_code_the_quote_uses_and_resolves_a_wrong_venue() {
    let d = db();
    let item = json!({"newsid": "7", "headline": "QIMC Engages Echo Seismic", "source": "TMX Newsfile via QuoteMedia", "datetime": "2026-09-14T09:13:00-04:00"});
    let asked = Mutex::new(Vec::<String>::new());
    let post = |_: &str, body: &Value, _: &[(&str, &str)]| -> Result<Value, NetError> {
        let form = s(&body["variables"]["symbol"]).to_string();
        asked.lock().unwrap().push(form.clone());
        Ok(if form == "QIMC:CNX" || form == "CH" { json!({"data": {"news": [item]}}) } else { json!({"data": {"news": []}}) })
    };
    let net = Net { get: &no_get, post: &post, pace: false };
    let clock = Clock::at(at(2026, 9, 16, 12, 0));
    let (src, rows) = news::fetch_symbol(&d.conn, &net, "QIMC", "CSE", "CAD", &clock);
    news::fetch_symbol(&d.conn, &net, "CH", "TSX-V", "CAD", &clock);
    assert_eq!((src.as_str(), asked.lock().unwrap().clone()), ("tmx", vec!["QIMC:CNX".to_string(), "QIMC:CNX".into(), "CH".into(), "CH".into()]),
               "each listing under the code its quote uses, once for each of TMX's two tabs");
    let rows = rows.unwrap().rows;
    assert_eq!((s(&rows[0]["kind"]), s(&rows[0]["url"])), ("release", "https://money.tmx.com/en/quote/QIMC:CNX/news/7"));
    // the record names the wrong venue: the lookup resolves the form that answers, as it does for a quote
    // (the form TMX's resolver verified, remembered as a miss would be, so nothing is asked of the network)
    bagholder_store::tables::set_meta(&d.conn, "tmx_form:QIMC", "none@2026-09-16").unwrap();
    asked.lock().unwrap().clear();
    let (_, found) = news::fetch_symbol(&d.conn, &net, "QIMC", "TSX-V", "CAD", &clock);
    assert_eq!((asked.lock().unwrap().clone(), ids(&found.unwrap().rows)), (vec!["QIMC".to_string(), "QIMC".into()], Vec::<String>::new()),
               "a form TMX cannot place answers nothing, under both tabs");
}

#[test]
fn test_tmx_names_a_listing_by_its_own_topic_codes() {
    let topic = "[ABHI:AQL,ABHI:CA,ART00001,CCHI:AQL,CCHI:CA,DIVIDEND]";
    assert!(news::tmx_names(topic, "CCHI"));
    assert!(news::tmx_names("[HG:CNX,MINING01]", "HG:CNX"));
    assert!(news::tmx_names("[ASTS,SPACE001]", "ASTS:US"), "a US listing's code is its bare ticker");
    assert!(!news::tmx_names(topic, "CCH"), "a code is a whole ticker, not a prefix of one");
    assert!(!news::tmx_names("[T,VZ,TMUS]", "T"), "AT&T's bare code is not Telus, a Canadian listing named `T:CA`");
    assert!(news::tmx_names("[T:CA,BCE:CA]", "T"));
    assert!(!news::tmx_names("[HG,INSURE01]", "HG:CNX"), "the NYSE's HG is not the CSE's");
    assert!(!news::tmx_names("[ASTS:CA]", "ASTS:US"));
    assert!(!news::tmx_names("", "PNG"));
}

#[test]
fn test_a_publishers_story_is_kept_only_where_tmx_tags_the_listing() {
    let data = json!({"data": {"news": [
        {"newsid": "1", "headline": "Kraken Robotics: Undersea Batteries Drive Solid Revenue Growth", "source": "SeekingAlpha via QuoteMedia",
         "datetime": "2026-09-05T10:00:00-04:00", "topic": "[PNG:CA,TECH0001]"},
        {"newsid": "2", "headline": "Most shorted stocks on Wall Street", "source": "SeekingAlpha via QuoteMedia",
         "datetime": "2026-09-05T11:00:00-04:00", "topic": "[ASTS,NBIS]"}]}});
    let rows = news::parse_tmx_news(&data, "PNG", true);
    let got: Vec<[&str; 3]> = rows.iter().map(|r| [s(&r["id"]), s(&r["kind"]), s(&r["source"])]).collect();
    assert_eq!(got, vec![["tmx:1", "story", "SeekingAlpha"]], "a story TMX tags with another listing is not this one's");
    let wire = news::parse_tmx_news(&json!({"data": {"news": [{"newsid": "3", "headline": "Kraken closes financing", "source": "GlobeNewswire via QuoteMedia",
                                                              "datetime": "2026-09-05T08:00:00-04:00"}]}}), "PNG", false);
    assert_eq!(s(&wire[0]["kind"]), "release", "the press releases tab reads as it always did");
}

fn kraken_release() -> Value {
    json!({"newsid": "10", "headline": "Kraken closes financing", "source": "GlobeNewswire via QuoteMedia", "datetime": "2026-09-05T08:00:00-04:00", "topic": "[PNG:CA]"})
}

fn kraken_story() -> Value {
    json!({"newsid": "11", "headline": "3 Top Canadian Defence Stocks", "source": "Motley Fool Canada via QuoteMedia", "datetime": "2026-09-02T09:00:00-04:00", "topic": "[PNG:CA,DEFENCE1]"})
}

#[test]
fn test_both_of_tmxs_tabs_are_read_and_a_failing_stories_tab_keeps_the_releases() {
    let d = db();
    let clock = Clock::at(at(2026, 9, 16, 12, 0));
    let tabs = Mutex::new(Vec::<bool>::new());
    let post = |_: &str, body: &Value, _: &[(&str, &str)]| -> Result<Value, NetError> {
        let media = body["variables"]["companyInNews"].as_bool().unwrap_or(false);
        tabs.lock().unwrap().push(media);
        Ok(json!({"data": {"news": [if media { kraken_story() } else { kraken_release() }]}}))
    };
    let (src, rows) = news::fetch_symbol(&d.conn, &Net { get: &no_get, post: &post, pace: false }, "PNG", "TSX-V", "CAD", &clock);
    assert_eq!((src.as_str(), tabs.lock().unwrap().clone()), ("tmx", vec![false, true]));
    let mut got: Vec<(String, String)> = rows.unwrap().rows.iter().map(|r| (s(&r["id"]).to_string(), s(&r["kind"]).to_string())).collect();
    got.sort();
    assert_eq!(got, vec![("tmx:10".to_string(), "release".to_string()), ("tmx:11".into(), "story".into())]);
    let failing = |_: &str, body: &Value, _: &[(&str, &str)]| -> Result<Value, NetError> {
        if body["variables"]["companyInNews"].as_bool().unwrap_or(false) {
            return Err(down());
        }
        Ok(json!({"data": {"news": [kraken_release()]}}))
    };
    let (_, rows) = news::fetch_symbol(&d.conn, &Net { get: &no_get, post: &failing, pace: false }, "PNG", "TSX-V", "CAD", &clock);
    assert_eq!(ids(&rows.unwrap().rows), vec!["tmx:10"], "the releases still arrive when the stories tab fails");
}

// ---------------------------------------------------------------------------
// the sources beside the wire
// ---------------------------------------------------------------------------

fn yahoo_asset(uuid: &str, title: &str, tickers: &[&str], provider: &str, when: &str) -> Value {
    json!({"node": {"asset": {"id": uuid, "title": title,
        "contentAttributes": {"pubDate": when, "provider": {"displayName": provider}, "canonicalUrl": format!("https://finance.yahoo.com/news/{}", uuid)},
        "finance": {"stockTickers": tickers.iter().map(|t| json!({"symbol": t})).collect::<Vec<_>>()}}}})
}

#[test]
fn test_yahoo_keeps_what_its_ticker_tags_name() {
    let t = "2026-09-14T13:13:00Z";
    let data = json!({"data": {"lightyearList": {"main": {"edges": [
        yahoo_asset("a1", "Kraken Robotics Announces Q2 Results", &["PNG.V", "KRKNF"], "Newsfile", t),
        yahoo_asset("a2", "3 Defence Stocks To Watch", &["LMT", "RTX"], "Motley Fool", t),
        yahoo_asset("a3", "Kraken Wins Navy Contract", &["PNG.V"], "The Globe and Mail", "2026-09-15T10:00:00.000Z"),
        yahoo_asset("a4", "no date", &["PNG.V"], "Newsfile", "")]}}}});
    let rows = news::parse_yahoo_news(&data, "PNG.V", "", "");
    let got: Vec<[&str; 4]> = rows.iter().map(|r| [s(&r["id"]), s(&r["kind"]), s(&r["source"]), s(&r["publishedAt"])]).collect();
    assert_eq!(got, vec![["yahoo:a1", "release", "Newsfile", "2026-09-14T13:13:00Z"], ["yahoo:a3", "story", "The Globe and Mail", "2026-09-15T10:00:00Z"]],
               "an item Yahoo tags with other tickers is theirs; a wire's item is a release");
    let d = db();
    bagholder_store::tables::set_meta(&d.conn, "tmx_form:QIMC", "@:CNX").unwrap();
    let forms: Vec<String> = [("PNG", "TSX-V", "CAD"), ("HG", "CSE", "CAD"), ("HBIX", "Cboe Canada", "CAD"), ("ASTS", "NASDAQ", "USD"),
                              ("LUNR", "NASDAQ", ""), ("QIMC", "", "CAD"), ("VEQT", "", "CAD"), ("F", "", "")]
        .iter().map(|(a, b, c)| news::yahoo_form(&d.conn, a, b, c)).collect();
    assert_eq!(forms, ["PNG.V", "HG.CN", "HBIX.NE", "ASTS", "LUNR", "QIMC.CN", "VEQT.TO", ""],
               "the venue decides before the currency: a US listing with no currency is not `LUNR.TO`, another company");
}

#[test]
fn test_yahoo_keeps_a_canadian_companys_items_tagged_with_its_us_twin() {
    let a = |uuid: &str, title: &str, tickers: &[&str]| yahoo_asset(uuid, title, tickers, "PR Newswire", "2026-09-14T13:13:00Z");
    let data = json!({"data": {"lightyearList": {"main": {"edges": [
        a("a1", "CHARBONE Announces Closing of $1.5M Drawdown", &["CH.V", "CHHYF"]),
        a("a2", "Charbone Announces Its First Hydrogen Supply Hub", &["CHHYF"]),
        a("a3", "ESGFIRE Initiates Coverage on Charbone Corporation", &["CHHYF", "PLUG", "FCEL"]),
        a("a4", "Presenting on Emerging Growth Conference 90 Day 1", &["ASPI", "IBX.AX", "STLNF"]),
        a("a5", "CHARBONE to Present at the Hydrogen East Conference", &[]),
        a("a6", "Hydrogen prices climb", &[])]}}}});
    assert_eq!(ids(&news::parse_yahoo_news(&data, "CH.V", "CH", "Charbone Hydrogen Corp")), vec!["yahoo:a1", "yahoo:a2", "yahoo:a3", "yahoo:a5"],
               "the twin Yahoo tags beside the listing names it; an untagged item counts where its headline names the listing");
    let partner = json!({"data": {"lightyearList": {"main": {"edges": [
        a("p1", "Kraken and Saab sign sonar partnership", &["PNG.V", "KRKNF", "SAABF"]),
        a("p2", "Kraken Robotics orders", &["PNG.V", "KRKNF"]),
        a("p3", "Saab raises its outlook", &["SAABF"])]}}}});
    assert_eq!(ids(&news::parse_yahoo_news(&partner, "PNG.V", "PNG", "Kraken Robotics Inc.")), vec!["yahoo:p1", "yahoo:p2"],
               "a partner's symbol on an item naming several is not the listing's twin");
    let us = json!({"data": {"lightyearList": {"main": {"edges": [a("b1", "Palantir wins Army deal", &["PLTR"]), a("b2", "Kraken Robotics orders", &["KRKNF"])]}}}});
    assert_eq!(ids(&news::parse_yahoo_news(&us, "PLTR", "PLTR", "Palantir Technologies Inc")), vec!["yahoo:b1"], "a US listing has no twin");
}

#[test]
fn test_seeking_alpha_keeps_what_its_symbol_tags_name() {
    let xml = r#"<rss><channel>
          <item><title>Kraken Robotics: Undersea Batteries Drive Growth</title><link>https://seekingalpha.com/article/1</link>
            <guid isPermaLink="false">Article:1</guid><pubDate>Fri, 05 Sep 2026 10:00:00 -0400</pubDate><sa:symbol>PNG:CA</sa:symbol><sa:symbol>KRKNF</sa:symbol></item>
          <item><title>Most shorted stocks</title><link>https://seekingalpha.com/news/2</link><guid>MarketCurrent:2</guid>
            <pubDate>Fri, 05 Sep 2026 11:00:00 -0400</pubDate><sa:symbol>ASTS</sa:symbol></item>
        </channel></rss>"#;
    let rows = news::parse_sa_news(xml, "PNG:CA");
    let got: Vec<[&str; 4]> = rows.iter().map(|r| [s(&r["headline"]), s(&r["source"]), s(&r["url"]), s(&r["publishedAt"])]).collect();
    assert_eq!(got, vec![["Kraken Robotics: Undersea Batteries Drive Growth", "Seeking Alpha", "https://seekingalpha.com/article/1", "2026-09-05T14:00:00Z"]]);
    let forms: Vec<String> = [("PNG", "TSX-V", "CAD"), ("VEQT", "TSX", "CAD"), ("ASTS", "NASDAQ", "USD"), ("HG", "CSE", "CAD"), ("HBIX", "Cboe Canada", "CAD")]
        .iter().map(|(a, b, c)| news::sa_form(a, b, c)).collect();
    assert_eq!(forms, ["PNG:CA", "VEQT:CA", "ASTS", "", ""], "Seeking Alpha has no form for the CSE or Cboe Canada");
}

#[test]
fn test_a_name_is_searched_as_the_press_writes_it() {
    let cases = [("Harvest Reddit Enhanced High Income Shares ETF (the “ETF”)", "Harvest Reddit Enhanced High Income Shares ETF"),
                 ("Harvest Diversified High Income Shares ETF - Class A", "Harvest Diversified High Income Shares ETF"),
                 ("Ninepoint Partners LP - Cameco Highshares ETF", "Ninepoint Cameco Highshares ETF"),
                 ("Vanguard All-Equity ETF Portfolio - ETF", "Vanguard All-Equity ETF Portfolio"),
                 ("Palantir Technologies Inc (Class A)", "Palantir Technologies"),
                 ("Nebius Group N.V. Class A", "Nebius"),
                 ("Micron Technology, Inc.", "Micron Technology"),
                 ("Charbone Hydrogen Corp", "Charbone Hydrogen"),
                 ("", "")];
    for (raw, want) in cases {
        assert_eq!(news::search_name(raw), want, "{}", raw);
    }
    assert_eq!(news::google_queries("HG", "CSE", "CAD", "Hydrograph Clean Power Inc."), ["\"Hydrograph Clean Power\"", "\"CSE:HG\""]);
    assert_eq!(news::google_queries("CH", "TSX-V", "CAD", "Charbone Hydrogen Corp"), ["\"Charbone Hydrogen\"", "\"TSXV:CH\""]);
    assert_eq!(news::google_queries("HBIX", "Cboe Canada", "CAD", ""), ["\"NEO:HBIX\""], "no name: the ticker alone");
    assert_eq!(news::google_queries("MU", "NASDAQ", "USD", "Micron Technology, Inc."), ["\"Micron Technology\"", "\"NASDAQ:MU\""]);
}

#[test]
fn test_google_keeps_a_headline_only_where_it_names_the_listing() {
    // headlines Google News returned for the searches, each with what it must be
    let cases: &[(&str, &str, bool, &[(&str, bool)])] = &[
        ("PLTE", "Harvest Palantir Enhanced High Income Shares ETF - Class A", false, &[
            ("(PLTE) Equity Market Report (PLTE:CA)", true),
            ("Canadian ETF Express | Harvest Palantir Enhanced High Income Shares ETF Was the Top Gainer, Rising 32.29%", true),
            ("Harvest High Income Shares ETFs Announces August 2026 Distributions", true),
            ("The Ultimate Investor Guide to High-Income TSX ETFs Generating Monthly Cash Flow", false),
            ("Canadian ETF Express | GLOBAL X INVESTMENTS CANADA INC. BETAPRO NATURAL GAS LEVERAGED DAILY BULL Was the Top Gainer, Rising 3.65%", false)]),
        ("CH", "Charbone Hydrogen Corp", false, &[
            ("CHARBONE Announces Change of Corporate Name and Registered Address", true),
            ("Charbone Reports Q2 2026 Financial Results, Confirming 155% Gas Income Growth", true),
            ("Boeing Announces Second Quarter Deliveries", false)]),
        ("HG", "Hydrograph Clean Power Inc.", false, &[
            ("HydroGraph Announces Change of Auditor", true),
            ("Is HydroGraph Clean Power (CNSX:HG) Fully Valued After Wider Losses And Fresh Funding?", true),
            ("HydroGraph Clean Power (HG.C): A year ago this thing looked insane. Then it got bigger.", true),
            ("Widespread intensification of global river hydrograph flashiness under climate change", false)]),
        ("YES", "Char Technologies Ltd.", false, &[
            ("CHAR Tech Receives Patent Notice of Allowance for Pyrogas Treatment to Syngas", true),
            ("CHAR Technologies Ltd. (CVE:YES): Are Analysts Optimistic?", true),
            ("UW Works with Wyoming DEQ-AML, UR Energy on Soil Reclamation Project Using Coal Char", false),
            ("Canada’s Energy Trade Is Alive Again — Why NG Energy International Corp (TSXV:GASX) Matters Now", false)]),
        ("QIMC", "Quebec Innovative Materials Corp", false, &[
            ("Québec Innovative Materials Corp. Engages Echo Seismic and Strum Consulting", true),
            ("QIMC launches 78-km natural hydrogen survey", true),
            ("The hunt is on for natural 'white' hydrogen in Nova Scotia’s underground", false)]),
        ("SXHI", "Ninepoint SpaceX HighShares ETF", false, &[
            ("SXHI: SpaceX High-Income ETF's 9.17% Screener Yield Puts Private-Space Exposure in the Spotlight", true),
            ("Ninepoint Partners Announces June 2026 Cash Distributions", true),
            ("Retail investors can now buy Canadian and US IPOs at offering price", false)]),
        ("EASY", "Evolve All-in-One UltraYield ETF", false, &[
            ("EASY WAYS TO RETIRE EARLY", false),
            ("Evolve Sets September 2026 Distributions Across UltraYield ETFs and Income Funds", true)]),
        ("QNC", "Quantum Emotion Corp", false, &[
            ("Quantum eMotion Submits Quantum Entropy Source for NIST Validation", true),
            ("$Xanadu Quantum Technologies (XNDU.US)$", false),
            ("Why investors are watching Quantum stocks", false)]),
        ("HG", "Hydrograph Clean Power Inc.", false, &[
            ("MDI joins HydroGraph partner network", true),
            ("Sparc reports positive results using HydroGraph's Fractal Graphene in solvent-based coatings", true)]),
        ("CH", "Charbone Hydrogen Corp", false, &[
            ("The Supply Gap No One Is Filling: How CHARBONE Is Building the UHP Industrial Gas Platform", true)]),
        ("VEQT", "Vanguard All-Equity ETF Portfolio - ETF", false, &[
            ("Vanguard Investments Canada Announces Final 2025 Annual Capital Gains Distributions for the Vanguard ETFs", true),
            ("15 cheap, but well-rated ETFs", false),
            ("No Time to Invest? Buy Any of These 3 Vanguard ETF Portfolios to Be Set for Life", false)]),
        ("NA", "National Bank of Canada", false, &[
            ("National Bank of Canada Reports Record Quarter", true),
            ("National Bank Financial raises its target on Cameco", true),
            ("National Bank of Greece posts record profit", false)]),
        ("RY", "Royal Bank of Canada", false, &[
            ("Royal Bank of Scotland to cut jobs", false)]),
        ("EASY", "Evolve All-in-One UltraYield ETF", false, &[
            ("How Canada's ETF Industry Continues to Evolve", false)]),
        ("HHIS", "Harvest Diversified High Income Shares ETF - Class A", false, &[
            ("Investors Rush to Harvest Tax Losses Before Year End", false),
            ("Harvest ETFs Announces August 2026 Distributions", true)]),
        ("CH", "Charbone Hydrogen Corp", false, &[
            ("Why Charbone shares jumped 30%", true)]),
        ("BMO", "Bank of Montreal", false, &[
            ("Bank of Montreal Reports Third Quarter Results", true),
            ("Bank of Canada holds rates steady", false)]),
        ("CNQ", "Canadian Natural Resources Limited", false, &[
            ("Canadian Natural Resources to buy oil sands assets", true),
            ("Canadian dollar weakens as oil slides", false),
            ("Canadian natural gas prices slump", false),
            ("Canadian stocks close higher", false)]),
        ("HXS", "Global X S&P 500 Index Corporate Class ETF", false, &[
            ("Global stocks slide on rate fears", false)]),
        ("CH", "", true, &[
            ("Chile ETF (CH) hits a new high", true),
            ("NYSE:CH moves", true),
            ("TSXV:CH moves", false)]),
    ];
    for (sym, name, us, heads) in cases {
        for (head, want) in heads.iter() {
            assert_eq!(news::names_listing(head, sym, name, *us), *want, "{}: {}", sym, head);
        }
    }
}

#[test]
fn test_google_items_lose_the_publisher_suffix_quote_pages_and_undated_pages() {
    let item = |title: &str, source: &str, when: &str, link: &str| format!("<item><title>{}</title><link>{}</link><pubDate>{}</pubDate><source url=\"x\">{}</source></item>", title, link, when, source);
    let xml = format!("<rss><channel>{}</channel></rss>", [
        item("HydroGraph Announces Change of Auditor - Investing News Network", "Investing News Network", "Mon, 31 Aug 2026 12:00:00 GMT", "https://news.google.com/rss/articles/a"),
        item("HG Stock Price and Chart — CSE:HG - tradingview.com", "tradingview.com", "Thu, 01 Jan 1970 00:00:00 GMT", "https://news.google.com/rss/articles/b"),
        item("HydroGraph Clean Power Stock Price, News, Quote &amp; History - Investing News Network", "Investing News Network", "Tue, 13 Jan 2026 00:00:00 GMT", "https://news.google.com/rss/articles/c"),
        item("Hydrograph Clean Power Inc Revenue Breakdown – CSE:HG - tradingview.com", "tradingview.com", "Fri, 31 Jul 2026 00:00:00 GMT", "https://news.google.com/rss/articles/d"),
        item("HG Forecast — Price Target — Prediction for 2027 - TradingView", "TradingView", "Fri, 31 Jul 2026 00:00:00 GMT", "https://news.google.com/rss/articles/e"),
        item("What is HydroGraph Clean Power rStock | How RHG Works - MEXC", "MEXC", "Fri, 31 Jul 2026 00:00:00 GMT", "https://news.google.com/rss/articles/f"),
        item("$HydroGraph Clean Power (HGRAF.US)$ - Moomoo", "Moomoo", "Fri, 31 Jul 2026 00:00:00 GMT", "https://news.google.com/rss/articles/g"),
    ].concat());
    let rows = news::parse_google_news(&xml, "HG", "Hydrograph Clean Power Inc.", false);
    let got: Vec<[&str; 4]> = rows.iter().map(|r| [s(&r["headline"]), s(&r["source"]), s(&r["publishedAt"]), s(&r["kind"])]).collect();
    assert_eq!(got, vec![["HydroGraph Announces Change of Auditor", "Investing News Network", "2026-08-31T12:00:00Z", "story"]]);
    assert!(s(&rows[0]["id"]).starts_with("gnews:"));
}

// ---------------------------------------------------------------------------
// the merge
// ---------------------------------------------------------------------------

fn wire_of(answer: Option<Vec<Value>>) -> impl Fn(&Connection, &str, &str, &str, &Clock) -> (String, Option<WireAnswer>) + Sync {
    move |_, _, _, _, _| ("tmx".to_string(), answer.clone().map(WireAnswer::of))
}

fn stored(conn: &Connection, sym: &str, ex: &str) -> Vec<Value> {
    bagholder_store::feeds::news_for(conn, sym, ex).unwrap()
}

#[test]
fn test_every_source_is_merged_one_row_per_story_the_wires_copy_first() {
    let d = db();
    let clock = Clock::at(at(2026, 9, 16, 12, 0));
    let wire = wire_of(Some(vec![row("tmx:1", "Charbone Closes Loan", "2026-09-08T12:00:00Z", "TheNewsWire", "release")]));
    let asked = Mutex::new(Vec::<(String, String)>::new());
    let extra = |key: &str, ask: &Ask| -> Result<Option<Vec<Value>>, NetError> {
        asked.lock().unwrap().push((key.to_string(), ask.name.clone()));
        Ok(Some(match key {
            "yahoo" => vec![row("yahoo:u1", "CHARBONE closes loan.", "2026-09-08T12:00:00Z", "TheNewsWire", "release"),
                            row("yahoo:u2", "Charbone delivers electrolyzer", "2026-09-09T12:00:00Z", "BNN Bloomberg", "story")],
            "sa" => vec![row("sa:1", "Charbone: a hydrogen story", "2026-09-10T12:00:00Z", "Seeking Alpha", "story")],
            _ => vec![row("gnews:1", "Charbone delivers electrolyzer", "2026-09-09T12:05:00Z", "The Globe and Mail", "story"),
                      row("gnews:2", "Charbone Reports Q2 2026 Financial Results", "2026-08-27T12:00:00Z", "The Globe and Mail", "story")],
        }))
    };
    let (src, rows) = news::read_listing(&d.conn, &Readers { wire: &wire, extra: &extra }, "CH", "TSX-V", "CAD", "Charbone Hydrogen Corp", false, &clock, None).unwrap();
    let mut a = asked.lock().unwrap().clone();
    a.sort();
    let name = "Charbone Hydrogen Corp".to_string();
    assert_eq!(a, vec![("gnews".to_string(), name.clone()), ("sa".into(), name.clone()), ("yahoo".into(), name)]);
    assert_eq!((src.as_str(), ids(&rows.unwrap())), ("tmx", vec!["sa:1".to_string(), "yahoo:u2".into(), "tmx:1".into(), "gnews:2".into()]),
               "newest first; the same headline from a later source is the earlier source's row");
    let got: Vec<[&str; 3]> = stored(&d.conn, "CH", "TSX-V").iter().map(|r| [s(&r["id"]), s(&r["source"]), s(&r["wire"])]).map(|x| x.map(|v| Box::leak(v.to_string().into_boxed_str()) as &str)).collect();
    assert_eq!(got, vec![["sa:1", "sa", "Seeking Alpha"], ["yahoo:u2", "yahoo", "BNN Bloomberg"], ["tmx:1", "tmx", "TheNewsWire"], ["gnews:2", "gnews", "The Globe and Mail"]],
               "each row is stored under the source it was read from");
}

fn first_answers(key: &str) -> Vec<Value> {
    match key {
        "yahoo" => vec![row("yahoo:u1", "Yahoo story", "2026-09-10T12:00:00Z", "Pub", "story")],
        "sa" => vec![row("sa:1", "SA story", "2026-09-11T12:00:00Z", "Pub", "story")],
        _ => vec![row("gnews:1", "Google story", "2026-09-12T12:00:00Z", "Pub", "story")],
    }
}

#[test]
fn test_a_source_that_fails_or_is_not_due_keeps_its_stored_stories() {
    let d = db();
    let now = at(2026, 9, 16, 12, 0);
    let first = |k: &str, _: &Ask| -> Result<Option<Vec<Value>>, NetError> { Ok(Some(first_answers(k))) };
    let wire1 = wire_of(Some(vec![row("tmx:1", "Wire item", "2026-09-09T12:00:00Z", "Pub", "story")]));
    news::read_listing(&d.conn, &Readers { wire: &wire1, extra: &first }, "CH", "TSX-V", "CAD", "", false, &Clock::at(now), None).unwrap();
    let asked = Mutex::new(Vec::<String>::new());
    let later = |k: &str, _: &Ask| -> Result<Option<Vec<Value>>, NetError> {
        asked.lock().unwrap().push(k.to_string());
        if k == "yahoo" { Err(down()) } else { Ok(Some(vec![])) }
    };
    // fifteen minutes on: the wire and Yahoo are due, Seeking Alpha and Google are not; Yahoo fails
    let wire2 = wire_of(Some(vec![row("tmx:2", "New wire item", "2026-09-16T12:10:00Z", "Pub", "story")]));
    let (_, rows) = news::read_listing(&d.conn, &Readers { wire: &wire2, extra: &later }, "CH", "TSX-V", "CAD", "", false, &Clock::at(now + 16 * 60), None).unwrap();
    assert_eq!(asked.lock().unwrap().clone(), vec!["yahoo"], "Seeking Alpha and Google are read every thirty minutes");
    assert_eq!(ids(&rows.unwrap()), vec!["tmx:2", "gnews:1", "sa:1", "yahoo:u1"], "the wire's item is replaced; the failing and the resting sources keep what they had");
    // nothing answers at all: the stored list stands untouched
    let none = wire_of(None);
    let failing = |_: &str, _: &Ask| -> Result<Option<Vec<Value>>, NetError> { Err(down()) };
    let got = news::read_listing(&d.conn, &Readers { wire: &none, extra: &failing }, "CH", "TSX-V", "CAD", "", true, &Clock::at(now + 45 * 60), None).unwrap();
    assert_eq!((got.0.as_str(), got.1), ("tmx", None));
    assert_eq!(ids(&stored(&d.conn, "CH", "TSX-V")), vec!["tmx:2", "gnews:1", "sa:1", "yahoo:u1"]);
    // every source answers, Yahoo with nothing at all: a list it had does not vanish on one empty answer
    let empty_yahoo = |k: &str, _: &Ask| -> Result<Option<Vec<Value>>, NetError> { Ok(Some(if k == "yahoo" { vec![] } else { first_answers(k) })) };
    let (_, rows) = news::read_listing(&d.conn, &Readers { wire: &wire2, extra: &empty_yahoo }, "CH", "TSX-V", "CAD", "", true, &Clock::at(now + 60 * 60), None).unwrap();
    assert!(ids(&rows.unwrap()).contains(&"yahoo:u1".to_string()));
}

#[test]
fn test_a_release_is_new_once_whichever_source_carries_it_and_a_first_read_source_is_history() {
    let d = db();
    let now = at(2026, 9, 16, 12, 0);
    let told = Mutex::new(Vec::<Vec<String>>::new());
    let on_new = |_: &Connection, _: &str, _: &str, _: &[Value], new: &[String]| {
        let mut n = new.to_vec();
        n.sort();
        told.lock().unwrap().push(n);
    };
    let release = |i: &str| row(i, "Charbone Closes Loan", "2026-09-16T11:00:00Z", "TheNewsWire", "release");
    // the wire is read, Yahoo for the first time: Yahoo's whole list is history
    let wire = wire_of(Some(vec![row("tmx:1", "Old wire item", "2026-09-01T12:00:00Z", "Pub", "story")]));
    let old = |k: &str, _: &Ask| -> Result<Option<Vec<Value>>, NetError> {
        Ok(if k == "yahoo" { Some(vec![row("yahoo:old", "Charbone old release", "2026-09-10T12:00:00Z", "NewMediaWire", "release")]) } else { None })
    };
    news::read_listing(&d.conn, &Readers { wire: &wire, extra: &old }, "CH", "TSX-V", "CAD", "", false, &Clock::at(now), Some(&on_new)).unwrap();
    assert_eq!(told.lock().unwrap().last().unwrap().clone(), vec!["tmx:1"], "a source met for the first time brings history, not news (the wire's own first read is the notifier's to judge)");
    // TMX fails this pass and Yahoo carries a new release: it is new
    let none = wire_of(None);
    let fresh = |k: &str, _: &Ask| -> Result<Option<Vec<Value>>, NetError> { Ok(if k == "yahoo" { Some(vec![release("yahoo:new")]) } else { None }) };
    news::read_listing(&d.conn, &Readers { wire: &none, extra: &fresh }, "CH", "TSX-V", "CAD", "", false, &Clock::at(now + 16 * 60), Some(&on_new)).unwrap();
    assert_eq!(told.lock().unwrap().last().unwrap().clone(), vec!["yahoo:new"]);
    // TMX answers again with the same release under its own id: not new a second time
    let again = wire_of(Some(vec![release("tmx:999")]));
    let (_, rows) = news::read_listing(&d.conn, &Readers { wire: &again, extra: &fresh }, "CH", "TSX-V", "CAD", "", false, &Clock::at(now + 32 * 60), Some(&on_new)).unwrap();
    assert!(ids(&rows.unwrap()).contains(&"tmx:999".to_string()), "the wire's copy is the row");
    assert_eq!(told.lock().unwrap().last().unwrap().clone(), Vec::<String>::new(), "the same headline under the wire's id is the story already told");
}

#[test]
fn test_a_headline_repeated_months_later_is_a_new_release() {
    let d = db();
    let now = at(2026, 9, 16, 12, 0);
    let told = Mutex::new(Vec::<Vec<String>>::new());
    let halt = |i: &str, when: &str| row(i, "IIROC Trading Halt - QNC", when, "TMX Newsfile", "release");
    let nothing = |_: &str, _: &Ask| -> Result<Option<Vec<Value>>, NetError> { Ok(None) };
    let first = wire_of(Some(vec![halt("tmx:1", "2026-06-10T14:00:00Z")]));
    news::read_listing(&d.conn, &Readers { wire: &first, extra: &nothing }, "QNC", "TSX-V", "CAD", "", false, &Clock::at(now), None).unwrap();
    let second = wire_of(Some(vec![halt("tmx:2", "2026-09-16T13:00:00Z"), halt("tmx:1", "2026-06-10T14:00:00Z")]));
    let on_new = |_: &Connection, _: &str, _: &str, _: &[Value], new: &[String]| told.lock().unwrap().push(new.to_vec());
    let (_, rows) = news::read_listing(&d.conn, &Readers { wire: &second, extra: &nothing }, "QNC", "TSX-V", "CAD", "", false, &Clock::at(now + 16 * 60), Some(&on_new)).unwrap();
    assert_eq!((ids(&rows.unwrap()), told.lock().unwrap().clone()), (vec!["tmx:2".to_string(), "tmx:1".into()], vec![vec!["tmx:2".to_string()]]),
               "the same words months apart are two halts, the second one new");
}

#[test]
fn test_a_wire_feed_that_fails_keeps_its_stored_items() {
    let d = db();
    let now = at(2026, 9, 16, 12, 0);
    let both = |_: &str, body: &Value, _: &[(&str, &str)]| -> Result<Value, NetError> {
        Ok(json!({"data": {"news": [if body["variables"]["companyInNews"].as_bool().unwrap_or(false) { kraken_story() } else { kraken_release() }]}}))
    };
    let stories_down = |_: &str, body: &Value, _: &[(&str, &str)]| -> Result<Value, NetError> {
        if body["variables"]["companyInNews"].as_bool().unwrap_or(false) {
            return Err(down());
        }
        Ok(json!({"data": {"news": [kraken_release()]}}))
    };
    let nothing = |_: &str, _: &Ask| -> Result<Option<Vec<Value>>, NetError> { Ok(None) };
    let read = |post: &news::PostFn<'_>, minutes: i64| {
        let net = Net { get: &no_get, post, pace: false };
        let wire = |c: &Connection, sy: &str, ex: &str, cc: &str, cl: &Clock| news::fetch_symbol(c, &net, sy, ex, cc, cl);
        news::read_listing(&d.conn, &Readers { wire: &wire, extra: &nothing }, "PNG", "TSX-V", "CAD", "", false, &Clock::at(now + minutes * 60), None).unwrap().1.unwrap()
    };
    read(&both, 0);
    let mut got = ids(&read(&stories_down, 16));
    got.sort();
    assert_eq!(got, vec!["tmx:10", "tmx:11"], "In The Media failing leaves the stories it had");
    let by: std::collections::BTreeMap<String, String> = stored(&d.conn, "PNG", "TSX-V").iter().map(|r| (s(&r["id"]).to_string(), s(&r["source"]).to_string())).collect();
    assert_eq!(by, [("tmx:10".to_string(), "tmx".to_string()), ("tmx:11".into(), "tmx-media".into())].into_iter().collect());
}

#[test]
fn test_a_source_with_nothing_to_ask_does_not_count_as_an_answer() {
    // a CSE listing: Seeking Alpha has no feed for it; everything that can be asked fails
    let d = db();
    let asked = Mutex::new(HashSet::<String>::new());
    let failing = |k: &str, _: &Ask| -> Result<Option<Vec<Value>>, NetError> {
        asked.lock().unwrap().insert(k.to_string());
        Err(down())
    };
    let none = wire_of(None);
    let got = news::read_listing(&d.conn, &Readers { wire: &none, extra: &failing }, "HG", "CSE", "CAD", "Hydrograph Clean Power Inc.", true, &Clock::at(at(2026, 9, 16, 12, 0)), None).unwrap();
    assert_eq!((got.0.as_str(), got.1), ("tmx", None), "nothing answered, so the listing is asked again next pass");
    assert!(!asked.lock().unwrap().contains("sa"));
    assert!(news::fetch_sa(&Net { get: &no_get, post: &no_post, pace: false }, "HG", "CSE", "CAD").unwrap().is_none());
}

#[test]
fn test_a_listing_is_due_while_any_of_its_sources_is() {
    // Freshness is each source's own: a wire read moments ago by a copy of the app that asked
    // nothing else leaves the other sources to read, and a source with nothing to ask keeps nothing due.
    let d = db();
    let now = at(2026, 9, 16, 12, 0);
    let stamp = Clock::at(now).stamp();
    bagholder_store::feeds::replace_news(&d.conn, "CH", "TSX-V", "tmx", &[row("tmx:1", "Wire item", "2026-09-16T11:00:00Z", "Pub", "story")], &stamp).unwrap();
    bagholder_store::feeds::replace_news(&d.conn, "HG", "CSE", "tmx", &[row("tmx:2", "Wire item", "2026-09-16T11:00:00Z", "Pub", "story")], &stamp).unwrap();
    let listings = vec![listing("CH", "TSX-V", "CAD", "Charbone Hydrogen Corp"), listing("HG", "CSE", "CAD", "Hydrograph Clean Power Inc.")];
    let syms = |ls: Vec<news::Listing>| ls.into_iter().map(|l| l.0).collect::<Vec<_>>();
    assert_eq!(syms(news::stale(&d.conn, &listings, now + 60, 15).unwrap()), ["CH", "HG"], "the wire is fresh, the other sources never read");
    assert!(!news::sources_for(&d.conn, "HG", "CSE", "CAD", "Hydrograph Clean Power Inc.").contains(&"sa".to_string()), "Seeking Alpha has no CSE feed");
    let (started, landed) = (Mutex::new(Vec::<String>::new()), Mutex::new(Vec::<(String, bool)>::new()));
    let wire = wire_of(Some(vec![]));
    let empty = |_: &str, _: &Ask| -> Result<Option<Vec<Value>>, NetError> { Ok(Some(vec![])) };
    let path = d.dir.path().to_path_buf();
    let open = move || bagholder_store::connect(&path).ok();
    let on_start = |due: &[news::Listing]| started.lock().unwrap().extend(due.iter().map(|l| l.0.clone()));
    let on_done = |l: &news::Listing, ok: bool| landed.lock().unwrap().push((l.0.clone(), ok));
    let n = news::refresh(&open, &Readers { wire: &wire, extra: &empty }, &listings, &Clock::at(now + 60), None, Some(&on_start), Some(&on_done), news::LISTINGS_AT_ONCE).unwrap();
    assert_eq!(n, 2);
    let (mut st, mut la) = (started.lock().unwrap().clone(), landed.lock().unwrap().clone());
    st.sort();
    la.sort();
    assert_eq!((st, la), (vec!["CH".to_string(), "HG".into()], vec![("CH".to_string(), true), ("HG".into(), true)]));
    assert_eq!(news::stale(&d.conn, &listings, now + 120, 15).unwrap(), vec![], "every source read: nothing due, a source with nothing to ask included");
    assert_eq!(syms(news::stale(&d.conn, &listings, now + 17 * 60, 15).unwrap()), ["CH", "HG"]);
}

#[test]
fn test_a_listing_with_no_venue_is_left_to_the_wire() {
    let d = db();
    let wire = |_: &Connection, _: &str, _: &str, _: &str, _: &Clock| ("nasdaq".to_string(), Some(WireAnswer::default()));
    let extra = |_: &str, _: &Ask| -> Result<Option<Vec<Value>>, NetError> { panic!("no other source is asked") };
    news::read_listing(&d.conn, &Readers { wire: &wire, extra: &extra }, "F", "", "", "", false, &Clock::at(at(2026, 9, 16, 12, 0)), None).unwrap();
}

#[test]
fn test_refresh_reads_only_stale_listings_and_replaces_their_rows() {
    let d = db();
    let now = at(2026, 9, 11, 15, 30);
    let answers = Mutex::new(std::collections::HashMap::from([
        ("SHOP".to_string(), vec![json!({"id": "tmx:1", "headline": "One", "source": "GlobeNewswire", "url": "u1", "publishedAt": "2026-09-11T14:00:00Z"})]),
        ("NVDA".to_string(), vec![json!({"id": "nasdaq:9", "headline": "Nine", "source": "Zacks", "url": "u9", "publishedAt": "2026-09-11T15:00:00Z"})]),
    ]));
    let calls = Mutex::new(Vec::<String>::new());
    let fake = |_: &Connection, symbol: &str, exchange: &str, _: &str, _: &Clock| {
        calls.lock().unwrap().push(symbol.to_string());
        ((if exchange == "TSX" { "tmx" } else { "nasdaq" }).to_string(), answers.lock().unwrap().get(symbol).cloned().map(WireAnswer::of))
    };
    let others = |_: &str, ask: &Ask| -> Result<Option<Vec<Value>>, NetError> { if ask.symbol == "BROKEN" { Err(down()) } else { Ok(Some(vec![])) } };
    let readers = Readers { wire: &fake, extra: &others };
    let listings = vec![listing("SHOP", "TSX", "CAD", ""), listing("NVDA", "NASDAQ", "USD", ""), listing("BROKEN", "TSX", "CAD", "")];
    let path = d.dir.path().to_path_buf();
    let open = move || bagholder_store::connect(&path).ok();
    assert_eq!(news::refresh(&open, &readers, &listings, &Clock::at(now), None, None, None, news::LISTINGS_AT_ONCE).unwrap(), 2,
               "a listing no source answers for leaves nothing behind and is asked again next time");
    let mut c = calls.lock().unwrap().clone();
    c.sort();
    assert_eq!(c, ["BROKEN", "NVDA", "SHOP"], "listings are read side by side");
    calls.lock().unwrap().clear();
    assert_eq!(news::refresh(&open, &readers, &listings, &Clock::at(now), None, None, None, news::LISTINGS_AT_ONCE).unwrap(), 0);
    assert_eq!(calls.lock().unwrap().clone(), ["BROKEN"], "fresh listings are not asked again within fifteen minutes");
    answers.lock().unwrap().insert("SHOP".into(), vec![json!({"id": "tmx:2", "headline": "Two", "source": "CNW", "url": "u2", "publishedAt": "2026-09-11T16:00:00Z"})]);
    let later = at(2026, 9, 11, 16, 0);
    news::refresh(&open, &readers, &listings, &Clock::at(later), None, None, None, news::LISTINGS_AT_ONCE).unwrap();
    let snap = bagholder_store::snapshot::snapshot(&d.conn, false).unwrap();
    let rows: Vec<[String; 3]> = snap["news"].as_array().unwrap().iter().map(|r| [s(&r["id"]).to_string(), s(&r["symbol"]).to_string(), s(&r["wire"]).to_string()]).collect();
    assert_eq!(rows, vec![["tmx:2".to_string(), "SHOP".into(), "CNW".into()], ["nasdaq:9".to_string(), "NVDA".into(), "Zacks".into()]],
               "newest first; a listing's rows are replaced by its wire's latest");
    bagholder_store::feeds::forget_news(&d.conn, "SHOP", "TSX").unwrap();
    let snap = bagholder_store::snapshot::snapshot(&d.conn, false).unwrap();
    assert_eq!(ids(snap["news"].as_array().unwrap()), vec!["nasdaq:9"]);
    assert_eq!(news::stale(&d.conn, &[listing("SHOP", "TSX", "CAD", "")], later, 15).unwrap(), vec![listing("SHOP", "TSX", "CAD", "")], "forgotten means stale");
}

#[test]
fn test_a_ticker_with_no_venue_is_never_asked_of_tmx() {
    // TMX's news answers on the bare ticker whatever venue it is asked under, so a name with no
    // venue would come back as another company's. Only Nasdaq, whose items name their symbols.
    let d = db();
    let get = |_: &str, _: &[(&str, &str)]| -> Result<String, NetError> {
        Ok(r#"{"data": {"rows": [{"id": 5, "title": "Ford declares dividend", "publisher": "PR Newswire", "created": "Sep 14, 2026", "ago": "1 day ago", "url": "/a", "related_symbols": ["f|stocks"]}]}}"#.into())
    };
    let (src, rows) = news::fetch_symbol(&d.conn, &Net { get: &get, post: &no_post, pace: false }, "F", "", "", &Clock::at(at(2026, 9, 15, 12, 0)));
    assert_eq!(src, "nasdaq", "no venue: TMX is never asked");
    let got: Vec<[&str; 2]> = rows.as_ref().unwrap().rows.iter().map(|r| [s(&r["kind"]), s(&r["headline"])]).collect();
    assert_eq!(got, vec![["release", "Ford declares dividend"]]);
}

#[test]
fn test_a_us_listing_reads_its_releases_beside_its_news_each_once() {
    let d = db();
    let asked = Mutex::new(Vec::<String>::new());
    let get = |url: &str, _: &[(&str, &str)]| -> Result<String, NetError> {
        asked.lock().unwrap().push(url.to_string());
        Ok(if url.contains("press_release") {
            r#"{"data": {"rows": [{"id": 2, "title": "Shopify Delivers Big", "publisher": "", "created": "Aug 5, 2026", "ago": "Aug 5, 2026", "url": "/press-release/b", "related_symbols": ["shop|stocks"]}, {"id": 3, "title": "Shopify to Announce", "publisher": "", "created": "Jul 8, 2026", "ago": "Jul 8, 2026", "url": "/press-release/c", "related_symbols": ["shop|stocks"]}]}}"#
        } else {
            r#"{"data": {"rows": [{"id": 1, "title": "Why SHOP", "publisher": "Zacks", "created": "Sep 14, 2026", "ago": "1 day ago", "url": "/articles/a", "related_symbols": ["shop|stocks"]}, {"id": 2, "title": "Shopify Delivers Big", "publisher": "GlobeNewswire", "created": "Aug 5, 2026", "ago": "Aug 5, 2026", "url": "/articles/b", "related_symbols": ["shop|stocks"]}]}}"#
        }.into())
    };
    let (src, rows) = news::fetch_symbol(&d.conn, &Net { get: &get, post: &no_post, pace: false }, "SHOP", "NASDAQ", "USD", &Clock::at(at(2026, 9, 15, 12, 0)));
    assert_eq!(src, "nasdaq");
    let got: Vec<[&str; 3]> = rows.as_ref().unwrap().rows.iter().map(|r| [s(&r["id"]), s(&r["kind"]), s(&r["source"])]).collect();
    assert_eq!(got, vec![["nasdaq:1", "story", "Zacks"], ["nasdaq:2", "release", "GlobeNewswire"], ["nasdaq:3", "release", "Nasdaq"]],
               "the news feed's own wire item is a release; the press feed adds what the news feed lacks, each once");
    assert_eq!(asked.lock().unwrap().len(), 2);
}
