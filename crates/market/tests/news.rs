//! News: parsing and routing; fetch, store and model tests live elsewhere.
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
    let rows = news::parse_tmx_news(&data, "SHOP");
    assert_eq!(rows, vec![json!({"id": "tmx:4883675477075330", "headline": "Shopify Delivers Big: 30%+ Growth Across GMV", "source": "GlobeNewswire",
                                  "url": "https://money.tmx.com/en/quote/SHOP/news/4883675477075330", "publishedAt": "2026-08-05T11:00:00Z", "kind": "release"})]);
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
    for wire in ["GlobeNewswire", "Business Wire", "PR Newswire", "ACCESS Newswire", "TheNewsWire", "Canada Newswire", "TMX Newsfile", "Marketwired", "CNW Group"] {
        assert_eq!(news::kind_of(wire), "release", "{}", wire);
    }
    for publ in ["The Motley Fool", "Zacks", "Barchart", "RTTNews", "MarketBeat", "BNK Invest", "Fintel", ""] {
        assert_eq!(news::kind_of(publ), "story", "{}", publ);
    }
    let tmx = news::parse_tmx_news(&json!({"data": {"news": [{"newsid": "1", "headline": "Closing", "source": "GlobeNewswire via QuoteMedia", "datetime": "2026-09-14T08:00:00-04:00"}]}}), "CH");
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
