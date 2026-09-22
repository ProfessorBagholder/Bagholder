//! The news pipeline, pinned end to end: each source's parser, what a wire
//! answers for a listing, the merge of every source with what is stored, the
//! rows the store keeps and reads back, and what the notifier is handed. The
//! answers are held in `golden/news_items.json`, so a change of representation
//! must leave every item as it was.
//! After an intended change: `BAGHOLDER_BLESS=1 cargo test -p bagholder-market --test news_items`,
//! and read the diff.

use bagholder_market::news::{self, Clock, Net, NetError, Readers, WireAnswer};
use serde_json::{json, Map, Value};
use std::sync::Mutex;

fn norm(v: Value) -> Value {
    match v {
        Value::Number(n) => json!(n.as_f64().unwrap()),
        Value::Array(a) => Value::Array(a.into_iter().map(norm).collect()),
        Value::Object(m) => Value::Object(m.into_iter().map(|(k, v)| (k, norm(v))).collect::<Map<_, _>>()),
        v => v,
    }
}

fn cell<T: serde::Serialize + ?Sized>(x: &T) -> Value {
    norm(serde_json::to_value(x).unwrap())
}

fn wire(w: &Option<WireAnswer>) -> Value {
    match w {
        None => Value::Null,
        Some(w) => {
            let mut missing: Vec<&String> = w.missing.iter().collect();
            missing.sort();
            json!({"rows": cell(&w.rows), "missing": missing})
        }
    }
}

fn at(y: i64, m: u32, d: u32, h: i64, mi: i64) -> i64 {
    bagholder_model::dates::to_days(y, m, d) * 86400 + h * 3600 + mi * 60
}

fn down() -> NetError {
    NetError { code: None, text: "down".into() }
}

fn no_get(_: &str, _: &[(&str, &str)]) -> Result<String, NetError> {
    Err(down())
}

fn no_post(_: &str, _: &Value, _: &[(&str, &str)]) -> Result<Value, NetError> {
    Err(down())
}

fn tmx_releases() -> Value {
    json!({"data": {"news": [
        {"newsid": "10", "headline": "Kraken Closes $40M Financing&#xA0;Round", "source": "GlobeNewswire via QuoteMedia", "datetime": "2026-09-05T08:00:00-04:00",
         "summary": "<p>HALIFAX, Nova Scotia, Sept. 05, 2026 (GLOBE NEWSWIRE) -- Kraken Robotics closed its bought deal.</p>", "topic": "[PNG:CA]"},
        {"newsid": 12, "headline": "Kraken to Present at Conference", "source": "Newsfile", "datetime": "2026-09-01T09:30:00-04:00"},
        {"newsid": "", "headline": "no id", "datetime": "2026-09-05T08:00:00-04:00"},
        {"newsid": "13", "headline": "bad time", "datetime": "yesterday"}]}})
}

fn tmx_media() -> Value {
    json!({"data": {"news": [
        {"newsid": "11", "headline": "3 Top Canadian Defence Stocks", "source": "Motley Fool Canada via QuoteMedia", "datetime": "2026-09-02T09:00:00-04:00",
         "topic": "[PNG:CA,DEFENCE1]", "summary": "3 Top Canadian Defence Stocks"},
        {"newsid": "14", "headline": "Not about it", "source": "The Globe and Mail", "datetime": "2026-09-02T10:00:00-04:00", "topic": "[OTHER:CA]"}]}})
}

fn nasdaq_news() -> &'static str {
    r#"{"data": {"rows": [
        {"id": 1, "title": "Why SHOP Is a Buy", "publisher": "Zacks", "created": "Sep 14, 2026", "ago": "1 day ago", "url": "/articles/a", "related_symbols": ["shop|stocks"]},
        {"id": 2, "title": "Shopify Delivers Big", "publisher": "GlobeNewswire", "created": "Aug 5, 2026", "ago": "Aug 5, 2026", "url": "https://www.nasdaq.com/articles/b", "related_symbols": ["shop|stocks"]},
        {"id": 4, "title": "Market wrap", "publisher": "Barchart", "created": "Sep 14, 2026", "ago": "3 minutes ago", "url": "/articles/wrap", "related_symbols": ["spy|etf"]},
        {"id": 5, "publisher": "no title", "related_symbols": ["shop|stocks"]}]}}"#
}

fn nasdaq_press() -> &'static str {
    r#"{"data": {"rows": [
        {"id": 2, "title": "Shopify Delivers Big", "publisher": "", "created": "Aug 5, 2026", "ago": "Aug 5, 2026", "url": "/press-release/b", "related_symbols": ["shop|stocks"]},
        {"id": 3, "title": "Shopify to Announce Q3 Results", "publisher": "", "created": "Jul 8, 2026", "ago": "Jul 8, 2026", "url": "/press-release/c", "related_symbols": ["shop|stocks"]}]}}"#
}

fn yahoo_asset(uuid: &str, title: &str, tickers: &[&str], provider: &str, when: &str, attrs: Value) -> Value {
    let mut ca = json!({"pubDate": when, "provider": {"displayName": provider}});
    for (k, v) in attrs.as_object().unwrap() {
        ca[k] = v.clone();
    }
    json!({"node": {"asset": {"id": uuid, "title": title, "contentAttributes": ca,
        "finance": {"stockTickers": tickers.iter().map(|t| json!({"symbol": t})).collect::<Vec<_>>()}}}})
}

fn yahoo_news() -> Value {
    json!({"data": {"lightyearList": {"main": {"edges": [
        yahoo_asset("a1", "Kraken Robotics Announces Q2 Results", &["PNG.V", "KRKNF"], "Newsfile", "2026-09-04T13:13:00Z",
                    json!({"canonicalUrl": "https://finance.yahoo.com/news/a1", "summary": "Kraken reported record revenue. It raised guidance."})),
        yahoo_asset("a2", "3 Defence Stocks To Watch", &["LMT", "RTX"], "Motley Fool", "2026-09-04T13:13:00Z", json!({})),
        yahoo_asset("a3", "Kraken Wins Navy Contract", &["PNG.V"], "", "2026-09-03T10:00:00.000Z",
                    json!({"canonicalUrl": "", "clickthroughUrl": "https://example.test/a3", "description": "A navy contract for sonar."})),
        yahoo_asset("a4", "no date", &["PNG.V"], "Newsfile", "", json!({}))]}}}})
}

fn sa_news() -> &'static str {
    r#"<rss><channel>
      <item><title>Kraken Robotics: Undersea Batteries Drive Growth</title><link>https://seekingalpha.com/article/1</link>
        <guid isPermaLink="false">Article:1</guid><pubDate>Fri, 06 Sep 2026 10:00:00 -0400</pubDate><description>Batteries lead.</description><sa:symbol>PNG:CA</sa:symbol></item>
      <item><title>Most shorted stocks</title><link>https://seekingalpha.com/news/2</link><guid>MarketCurrent:2</guid>
        <pubDate>Fri, 06 Sep 2026 11:00:00 -0400</pubDate><sa:symbol>ASTS</sa:symbol></item>
    </channel></rss>"#
}

fn google_news() -> String {
    let item = |title: &str, source: &str, when: &str, link: &str| {
        format!("<item><title>{}</title><link>{}</link><pubDate>{}</pubDate><source url=\"x\">{}</source></item>", title, link, when, source)
    };
    format!("<rss><channel>{}</channel></rss>", [
        item("Kraken Robotics Closes $40M Financing Round - The Globe and Mail", "The Globe and Mail", "Fri, 05 Sep 2026 13:00:00 GMT", "https://news.google.com/rss/articles/a"),
        item("Kraken Robotics lands sonar order - BNN Bloomberg", "BNN Bloomberg", "Sun, 07 Sep 2026 12:00:00 GMT", "https://news.google.com/rss/articles/b"),
        item("PNG Stock Price and Chart — TSXV:PNG - tradingview.com", "tradingview.com", "Fri, 31 Jul 2026 00:00:00 GMT", "https://news.google.com/rss/articles/c"),
    ].concat())
}

fn tmx_post(media_fails: bool) -> impl Fn(&str, &Value, &[(&str, &str)]) -> Result<Value, NetError> + Sync {
    move |_: &str, body: &Value, _: &[(&str, &str)]| {
        let media = body["variables"]["companyInNews"].as_bool().unwrap_or(false);
        if media && media_fails {
            return Err(down());
        }
        Ok(if media { tmx_media() } else { tmx_releases() })
    }
}

fn nasdaq_get(url: &str, _: &[(&str, &str)]) -> Result<String, NetError> {
    Ok(if url.contains("press_release") { nasdaq_press() } else { nasdaq_news() }.to_string())
}

fn answers() -> Value {
    let mut out = Map::new();
    let now = at(2026, 9, 15, 12, 0);

    // each source's parser
    out.insert("tmx_releases".into(), cell(&news::parse_tmx_news(&tmx_releases(), "PNG", false)));
    out.insert("tmx_media".into(), cell(&news::parse_tmx_news(&tmx_media(), "PNG", true)));
    out.insert("nasdaq".into(), cell(&news::parse_nasdaq_news(&serde_json::from_str(nasdaq_news()).unwrap(), now, "SHOP", None)));
    out.insert("nasdaq_press".into(), cell(&news::parse_nasdaq_news(&serde_json::from_str(nasdaq_press()).unwrap(), now, "SHOP", Some("release"))));
    out.insert("nasdaq_market".into(), cell(&news::parse_nasdaq_news(&serde_json::from_str(nasdaq_news()).unwrap(), now, "", None)));
    out.insert("yahoo".into(), cell(&news::parse_yahoo_news(&yahoo_news(), "PNG.V", "PNG", "Kraken Robotics Inc.")));
    out.insert("sa".into(), cell(&news::parse_sa_news(sa_news(), "PNG:CA")));
    out.insert("google".into(), cell(&news::parse_google_news(&google_news(), "PNG", "Kraken Robotics Inc.", false)));

    // what a wire answers for a listing
    let d = tempfile::tempdir().unwrap();
    let conn = bagholder_store::connect(d.path()).unwrap();
    bagholder_store::relabel::ensure(&conn).unwrap();
    let clock = Clock::at(now);
    let both = tmx_post(false);
    let one = tmx_post(true);
    let tmx_net = Net { get: &no_get, post: &both, pace: false };
    let tmx_half = Net { get: &no_get, post: &one, pace: false };
    let us_net = Net { get: &nasdaq_get, post: &no_post, pace: false };
    let dead = Net { get: &no_get, post: &no_post, pace: false };
    out.insert("wire_tmx".into(), wire(&news::fetch_symbol(&conn, &tmx_net, "PNG", "TSX-V", "CAD", &clock).1));
    out.insert("wire_tmx_media_down".into(), wire(&news::fetch_symbol(&conn, &tmx_half, "PNG", "TSX-V", "CAD", &clock).1));
    out.insert("wire_us".into(), wire(&news::fetch_symbol(&conn, &us_net, "SHOP", "NASDAQ", "USD", &clock).1));
    out.insert("wire_us_down".into(), wire(&news::fetch_symbol(&conn, &dead, "SHOP", "NASDAQ", "USD", &clock).1));

    // the merge, over three passes of one listing
    let told = Mutex::new(Vec::<Value>::new());
    let on_new = |_: &rusqlite::Connection, sym: &str, ex: &str, rows: &[_], ids: &[String]| {
        told.lock().unwrap().push(json!({"symbol": sym, "exchange": ex, "rows": cell(rows), "new": ids}));
    };
    let stored = |conn: &rusqlite::Connection| cell(&bagholder_store::feeds::news_for(conn, "PNG", "TSX-V").unwrap());
    let pass = |conn: &rusqlite::Connection, readers: &Readers, clock: &Clock| {
        let (src, rows) = news::read_listing(conn, readers, "PNG", "TSX-V", "CAD", "Kraken Robotics Inc.", false, clock, Some(&on_new)).unwrap();
        json!({"source": src, "rows": rows.as_ref().map(|r| cell(r)), "stored": stored(conn)})
    };

    // first: every source answers
    let full_wire = |c: &rusqlite::Connection, s: &str, e: &str, cc: &str, cl: &Clock| news::fetch_symbol(c, &tmx_net, s, e, cc, cl);
    let all_extra = |key: &str, _: &news::Ask| {
        Ok(Some(match key {
            "yahoo" => news::parse_yahoo_news(&yahoo_news(), "PNG.V", "PNG", "Kraken Robotics Inc."),
            "sa" => news::parse_sa_news(sa_news(), "PNG:CA"),
            _ => news::parse_google_news(&google_news(), "PNG", "Kraken Robotics Inc.", false),
        }))
    };
    out.insert("merge_first".into(), pass(&conn, &Readers { wire: &full_wire, extra: &all_extra }, &clock));

    // sixteen minutes on: the stories tab fails, Yahoo fails, the rest are not due
    let half_wire = |c: &rusqlite::Connection, s: &str, e: &str, cc: &str, cl: &Clock| news::fetch_symbol(c, &tmx_half, s, e, cc, cl);
    let failing = |_: &str, _: &news::Ask| Err(down());
    out.insert("merge_stand_in".into(), pass(&conn, &Readers { wire: &half_wire, extra: &failing }, &Clock::at(now + 16 * 60)));

    // an hour on: nothing answers at all, and the stored list stands
    let dead_wire = |c: &rusqlite::Connection, s: &str, e: &str, cc: &str, cl: &Clock| news::fetch_symbol(c, &dead, s, e, cc, cl);
    out.insert("merge_none".into(), pass(&conn, &Readers { wire: &dead_wire, extra: &failing }, &Clock::at(now + 60 * 60)));

    out.insert("told".into(), Value::Array(told.into_inner().unwrap()));
    Value::Object(out)
}

#[test]
fn test_every_news_item_is_what_it_was() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/news_items.json");
    let have = norm(answers());
    if std::env::var("BAGHOLDER_BLESS").map_or(false, |v| v == "1") {
        std::fs::write(&path, serde_json::to_string_pretty(&have).unwrap() + "\n").unwrap();
        return;
    }
    let want: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("tests/golden/news_items.json")).unwrap();
    for (k, v) in want.as_object().unwrap() {
        assert_eq!(&have[k], v, "{} is not what it was", k);
    }
    assert_eq!(have.as_object().unwrap().len(), want.as_object().unwrap().len());
}
