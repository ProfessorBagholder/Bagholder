//! The rules of plan `market-data-keyed-by-listing`, each held over the whole
//! of what it covers rather than an example: what the market cache keeps per
//! listing is keyed by the listing (rule test 2), and no route the page calls
//! finds a listing by a bare ticker (rule test 3). Rule test 1, the property
//! over the readers, is `tests/listing_isolation.rs` in `bagholder-market`.

use std::collections::BTreeSet;

use axum::http::Method;
use rusqlite::Connection;
use serde_json::{json, Value};

/// What a market-cache table is keyed by.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Keyed {
    /// A listing's own rows: its primary key holds the listing (`ListingKey`).
    Listing,
    /// A filer's documents: the source and the filer (an SEC CIK, a SEDAR+ profile).
    Filer,
    /// A book instrument's own: its id, which follows the instrument through a
    /// ticker change (`docs/architecture.md` §5).
    Instrument,
    /// Not per listing, with the reason it may stand.
    Allowed(&'static str),
}

/// Every table the market cache holds, and what it is keyed by. A table not
/// here fails the test until it is placed in one of these, so a new table keyed
/// by a ticker cannot be added without saying so.
const TABLES: [(&str, Keyed); 27] = [
    ("price_history", Keyed::Listing),
    ("history_fetches", Keyed::Listing),
    ("price_bars", Keyed::Listing),
    ("bar_fetches", Keyed::Listing),
    ("shorts", Keyed::Listing),
    ("news", Keyed::Listing),
    ("listing_filers", Keyed::Listing),
    ("listing_filer_misses", Keyed::Listing),
    ("interlisted", Keyed::Listing),
    ("filings", Keyed::Filer),
    ("filer_fetches", Keyed::Filer),
    ("quotes", Keyed::Instrument),
    ("daily_closes", Keyed::Instrument),
    ("chains", Keyed::Instrument),
    ("quoted_ex_dividends", Keyed::Instrument),
    ("reads", Keyed::Instrument),
    ("read_failures", Keyed::Allowed("a read that failed, by what it was of: a listing by its key (`ListingKey`), or the TSX's interlisted list as a whole")),
    ("exposures", Keyed::Allowed("a broker's security id, or a share's or fund's listing built by `venues::share_key` and `fund_key`")),
    ("option_chains", Keyed::Allowed("a US option root: the US market lists one underlying per symbol, which is its `US` listing")),
    ("universes", Keyed::Allowed("a whole index's constituents, as the index's own source lists them")),
    ("gauges", Keyed::Allowed("a market-wide gauge, of no listing")),
    ("benchmark_closes", Keyed::Allowed("a benchmark's levels, of no listing")),
    ("benchmark_events", Keyed::Allowed("a benchmark's distributions, of no listing")),
    ("meta", Keyed::Allowed("its keys are held by `test_every_meta_key_that_names_a_listing_is_built_through_its_key`")),
    ("outcomes", Keyed::Allowed("what each request to a source answered, by request")),
    ("gen", Keyed::Allowed("the store's own change counters")),
    ("schema_migrations", Keyed::Allowed("the store's own migrations")),
];

fn store(migrations: usize) -> Connection {
    let c = Connection::open_in_memory().unwrap();
    for m in &bagholder_sources::cache::MIGRATIONS[..migrations] {
        c.execute_batch(m.sql).unwrap();
    }
    c
}

/// Each table's primary-key columns and every column, by name.
fn tables(c: &Connection) -> Vec<(String, BTreeSet<String>, BTreeSet<String>)> {
    let names: Vec<String> = c.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    names
        .into_iter()
        .map(|t| {
            let cols: Vec<(String, i64)> = c.prepare(&format!("PRAGMA table_info(\"{t}\")")).unwrap().query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(5)?))).unwrap().map(Result::unwrap).collect();
            let pk = cols.iter().filter(|(_, p)| *p > 0).map(|(n, _)| n.clone()).collect();
            (t, pk, cols.into_iter().map(|(n, _)| n).collect())
        })
        .collect()
}

/// What breaks the rule in a schema: each table not keyed as its kind says.
fn violations(c: &Connection) -> Vec<String> {
    let mut out = Vec::new();
    for (t, pk, cols) in tables(c) {
        let Some((_, kind)) = TABLES.iter().find(|(n, _)| *n == t) else {
            out.push(format!("{t} is not placed in TABLES"));
            continue;
        };
        let holds = |c: &str| pk.contains(c);
        let ok = match kind {
            Keyed::Listing => holds("listing"),
            // a document once, by its own id at its source, beside the filer it belongs to
            Keyed::Filer => holds("source") && (holds("filer") || (holds("id") && cols.contains("filer"))),
            Keyed::Instrument => holds("instrument_id") || holds("subject"),
            Keyed::Allowed(_) => true,
        };
        if !ok {
            out.push(format!("{t} ({kind:?}) is keyed by {pk:?}"));
        }
    }
    out
}

/// Rule test 2, the tables: every market-cache table with per-listing or
/// per-filer rows has its listing, or its source and filer, in its key; every
/// other is a book instrument's or allowed with its reason. The same check over
/// the store before migration 6 finds the tables that were keyed by a ticker.
#[test]
fn test_every_market_cache_table_is_keyed_by_its_listing_its_filer_or_its_instrument() {
    let now = store(bagholder_sources::cache::MIGRATIONS.len());
    assert_eq!(violations(&now), Vec::<String>::new());
    let before = store(5);
    let found = violations(&before);
    for t in ["price_history", "price_bars", "shorts", "news", "filings"] {
        assert!(found.iter().any(|v| v.starts_with(t)), "the store before migration 6 keyed {t} by a ticker: {found:?}");
    }
}

/// The meta keys a source keeps under its own name for one listing, or no
/// listing's, with the reason each may stand.
const META_ALLOWED: [(&str, &str); 2] = [
    ("yahoo_miss", "Yahoo's own form for one listing, which names its venue (`ENB.TO`)"),
    ("coinbase_product", "a coin pair, which is one coin's market"),
];

/// Every `.rs` file under the workspace's crates, less tests and generated code.
fn sources() -> Vec<(String, String)> {
    let crates = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut out = Vec::new();
    let mut dirs = vec![crates.clone()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            if p.is_dir() {
                if name != "tests" && name != "target" && name != "migrations" {
                    dirs.push(p);
                }
            } else if name.ends_with(".rs") && !name.starts_with("tests") {
                // a file's own tests are not the code that writes the store
                let code = crate::tests_source::production(&std::fs::read_to_string(&p).unwrap());
                out.push((p.strip_prefix(&crates).unwrap().to_string_lossy().replace('\\', "/"), code));
            }
        }
    }
    out
}

/// Rule test 2, the meta keys: a meta key that names a listing is built by
/// `ListingKey::meta` alone, under one of `bagholder_core::listing::meta`'s
/// prefixes, so its listing is its key and never its ticker; no code writes one of
/// those prefixes by hand. The other prefixes a meta key is written under are the
/// allowed ones, each with its reason, and each is still in use.
#[test]
fn test_every_meta_key_that_names_a_listing_is_built_through_its_key() {
    let code = sources();
    for prefix in bagholder_core::listing::meta::ALL {
        for quote in ['"', '\''] {
            let by_hand = format!("{quote}{prefix}:");
            let at: Vec<&str> = code.iter().filter(|(_, t)| t.contains(&by_hand)).map(|(f, _)| f.as_str()).collect();
            assert!(at.is_empty(), "{prefix} is written by hand in {at:?}: build it with `ListingKey::meta`");
        }
    }
    for (prefix, why) in META_ALLOWED {
        let written = |t: &str| ["\"", "'"].iter().any(|q| t.contains(&format!("{q}{prefix}:")) || t.contains(&format!("{q}{prefix}{q}")));
        assert!(code.iter().any(|(_, t)| written(t)), "{prefix} ({why}) is no longer written: drop it from META_ALLOWED");
    }
    // the builder: two listings of one ticker, on every pair of venues, are two keys under every prefix
    let venues: Vec<&str> = bagholder_core::listing::CANADA_MICS.iter().chain(bagholder_core::listing::US_MICS.iter()).copied().collect();
    for a in &venues {
        for b in &venues {
            let (x, y) = (bagholder_core::ListingKey::on("ZQ", a).unwrap(), bagholder_core::ListingKey::on("ZQ", b).unwrap());
            for prefix in bagholder_core::listing::meta::ALL {
                assert_eq!(x.meta(prefix, &["s"]) == y.meta(prefix, &["s"]), x == y, "{prefix}: {a} and {b}");
                assert!(x.meta(prefix, &[]).ends_with(&format!(":{x}")));
            }
        }
    }
}

/// The routes that take a symbol with no venue by design, or carry one only as a
/// label: what they answer is every listing of it, each with its venue, or the
/// listing the broker's own id names.
const BARE_SYMBOL_ROUTES: [(&str, &str); 5] = [
    ("POST /api/tiles/set", "a tile is an instrument of the app's own directory, found by its symbol and venue together (`directory::find`); no listing is ever a tile"),
    ("GET /api/symbols/search", "the exchanges' directories searched by text: each match carries its venue"),
    ("GET /api/listings", "the resolver: every listing of the symbol, each with its venue, none picked"),
    ("POST /api/order", "an order names its listing by the broker's own security id; the symbol is its label"),
    ("POST /api/order/preview", "a preview names its listing by the broker's own security id; the symbol is its label"),
];

/// The request a route that holds its listing inside another field is driven
/// with: the listing by its symbol and no venue.
fn shaped(route: &str) -> Option<Value> {
    match route {
        "POST /api/order/listing" => Some(json!({"of": {"kind": "venue", "symbol": "ZZQQ", "venue": ""}})),
        "POST /api/tiles/set" => Some(json!({"tiles": [{"symbol": "ZZQQ", "exchange": ""}]})),
        "POST /api/watchlist/add" => Some(json!({"symbol": "ZZQQ", "exchange": "", "name": "", "currency": "CAD"})),
        "POST /api/entries" => Some(json!({"entry": "trade", "account": "", "instrument": null, "symbol": "ZZQQ", "exchange": "", "currency": "CAD", "day": "2026-01-05", "side": "buy", "quantity": "1", "price": "1", "fee": ""})),
        _ => None,
    }
}

/// Every route whose request names a listing by its symbol (the page's
/// generated route table, read against the generated declarations), driven with
/// a symbol and no venue: each refuses and says the venue is missing, or holds
/// nothing and says the listing's data waits for its venue, except the two that
/// answer every listing of a symbol by design. The routes that name a
/// listing otherwise (an order by the broker's id or the book's instrument, a
/// contract by its terms) carry no symbol to find one by.
#[test]
fn test_no_route_finds_a_listing_by_a_bare_symbol() {
    let _g = crate::tests_common::guard();
    let generated = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../web/src/lib/generated");
    let decls: String = std::fs::read_dir(&generated).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "ts")).map(|e| std::fs::read_to_string(e.path()).unwrap()).collect();
    // every declared type, and those that carry a symbol: by a field of their own,
    // or through a type they hold that does (`TilesSet` holds `TileRef`s)
    let declared: Vec<(String, String)> = decls
        .split("export type ")
        .skip(1)
        .filter_map(|d| {
            let (name, body) = d.split_once(" = ")?;
            let body = body.split("\n\n").next().unwrap_or(body);
            // its fields and the types they hold, its doc comments left out
            let mut code = String::new();
            let mut rest = body;
            while let Some(i) = rest.find("/**") {
                code.push_str(&rest[..i]);
                rest = rest[i..].find("*/").map_or("", |j| &rest[i + j + 2..]);
            }
            code.push_str(rest);
            // and its string literals: a kind named `"Options"` is not the type `Options`
            let code: String = code.split('"').step_by(2).collect::<Vec<_>>().join(" ");
            Some((name.trim().to_string(), code))
        })
        .collect();
    let mut with_symbol: BTreeSet<String> = declared.iter().filter(|(_, b)| b.contains("symbol: string") || b.contains("symbol?: string")).map(|(n, _)| n.clone()).collect();
    loop {
        let names = |b: &str| b.split(|c: char| !c.is_alphanumeric() && c != '_').map(str::to_string).collect::<BTreeSet<_>>();
        let more: Vec<String> = declared.iter().filter(|(n, b)| !with_symbol.contains(n) && names(b).iter().any(|w| with_symbol.contains(w))).map(|(n, _)| n.clone()).collect();
        if more.is_empty() {
            break;
        }
        with_symbol.extend(more);
    }
    let carries_symbol = |name: &str| with_symbol.contains(name);
    let app = crate::tests_routes_golden::isolated();
    let rt = crate::tests_routes_golden::runtime();
    let mut driven = Vec::new();
    for e in crate::http::route_table() {
        let route = format!("{} {}", e.method.to_uppercase(), e.path);
        let Some(ty) = e.query.as_deref().or(e.body.as_deref()).filter(|t| carries_symbol(t)) else { continue };
        let fields = json!({"symbol": "ZZQQ", "currency": "CAD", "kind": "Shares", "entry": "trade", "account": "", "day": "2026-01-05", "side": "buy", "quantity": "1", "price": "1", "fee": "", "tiles": [{"symbol": "ZZQQ"}], "from": "2026-01-01", "to": "2026-01-05", "tf": "1d"});
        let req = if let Some(body) = shaped(&route) {
            crate::tests_routes_golden::from_the_page(&app, Method::POST, e.path, Some(body))
        } else if e.method == "get" {
            let q: Vec<String> = fields.as_object().unwrap().iter().filter_map(|(k, v)| v.as_str().map(|s| format!("{k}={s}"))).collect();
            crate::tests_routes_golden::from_the_page(&app, Method::GET, &format!("{}?{}", e.path, q.join("&")), None)
        } else {
            crate::tests_routes_golden::from_the_page(&app, Method::POST, e.path, Some(fields.clone()))
        };
        let got: Value = rt.block_on(crate::tests_routes_golden::json_of(app.clone(), req));
        driven.push(route.clone());
        if let Some((_, why)) = BARE_SYMBOL_ROUTES.iter().find(|(r, _)| *r == route) {
            assert_ne!(got["status"], 500, "{route} ({why}): {got}");
            continue;
        }
        // a refusal that names the missing venue, or an answer that holds nothing and says its data waits for one
        let error = got["body"]["error"].as_str().unwrap_or_default();
        let refused = got["body"]["ok"] == false && (error.contains("exchange") || error.contains("venue"));
        let waits = got["body"]["ok"] == true && got["body"].to_string().contains("waits for its venue") && got["body"]["bars"].as_array().is_none_or(|b| b.is_empty());
        assert!(refused || waits, "{route} ({ty}) answered a bare symbol: {got}");
    }
    // the routes that take a listing, so the rule is known to have been driven over them
    for want in ["GET /api/listing", "GET /api/symbols/quote", "GET /api/news/symbol", "GET /api/filings", "GET /api/shorts", "GET /api/history", "POST /api/watchlist/add", "POST /api/tiles/set", "POST /api/entries"] {
        assert!(driven.iter().any(|d| d == want), "{want} carries a symbol and was not driven: {driven:?}");
    }
}
