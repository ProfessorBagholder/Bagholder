//! The figure path imports none of the earlier crates (`docs/plans/stage-3c-switch.md`,
//! §8): what builds, sends and keeps current the figures document, and what writes
//! the book, uses the book, the engine, the sources and the broker adapters. The
//! market's context is the one door, `wire/context.rs`, until stage 5 moves it.

use std::path::PathBuf;

use crate::tests_source::production;

/// The earlier crates, by the names code uses them under.
const EARLIER: [&str; 4] = ["bagholder_model", "bagholder_store", "bagholder_ws", "bagholder_market"];

/// The figure path: what builds and sends the figures document, keeps the engine
/// current, and writes the book.
const FIGURE_PATH: [&str; 15] = [
    "wire/mod.rs", "wire/build.rs", "wire/figures.rs", "wire/filters.rs", "wire/markets.rs", "wire/news.rs",
    "figures.rs", "engine_inputs.rs", "due.rs", "read_sources.rs", "broker_reads.rs",
    "entries.rs", "csv_import.rs", "events.rs", "http/model.rs",
];

/// Each earlier crate a file's code names, outside its tests.
fn violations(text: &str) -> Vec<&'static str> {
    let code = &production(&text);
    EARLIER.iter().copied().filter(|c| code.lines().any(|l| !l.trim_start().starts_with("//") && l.contains(c))).collect()
}

#[test]
fn nothing_on_the_figure_path_uses_the_earlier_crates() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    for f in FIGURE_PATH {
        let text = std::fs::read_to_string(src.join(f)).unwrap_or_else(|e| panic!("{f}: {e}"));
        assert_eq!(violations(&text), Vec::<&str>::new(), "{f} uses an earlier crate");
    }
    // every file of the wire is on it, but the context's door
    for entry in std::fs::read_dir(src.join("wire")).unwrap() {
        let name = format!("wire/{}", entry.unwrap().file_name().to_string_lossy());
        assert!(FIGURE_PATH.contains(&name.as_str()) || name == "wire/context.rs", "{name} is in the wire and not on the figure path's list");
    }
}

#[test]
fn the_check_finds_an_earlier_crate_named_in_code_and_not_in_a_comment_or_a_test() {
    assert_eq!(violations("use bagholder_model::base::Base;\nfn f() {}"), vec!["bagholder_model"]);
    assert_eq!(violations("fn f() { bagholder_store::tables::accounts(c); bagholder_ws::x(); }"), vec!["bagholder_store", "bagholder_ws"]);
    assert!(violations("// once read bagholder_model's view\nfn f() {}").is_empty());
    assert!(violations("fn f() {}\n#[cfg(test)]\nmod tests { use bagholder_market::x; }").is_empty());
}

/// What reaches Wealthsimple's order mutations in a file's code, outside its tests: the
/// operations named, or the client's one-shot `mutate` called.
fn mutations(text: &str) -> Vec<String> {
    let code = &production(&text);
    let mut out = Vec::new();
    for l in code.lines().filter(|l| !l.trim_start().starts_with("//")) {
        for op in ["SoOrdersOrderCreate", "SoOrdersOrderCancel", "SoOrdersOrderModify", ".mutate("] {
            if l.contains(op) {
                out.push(op.to_string());
            }
        }
    }
    out
}

fn server_sources() -> Vec<(String, String)> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    let mut dirs = vec![src.clone()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") && !p.file_name().unwrap().to_string_lossy().starts_with("tests") {
                out.push((p.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/"), std::fs::read_to_string(&p).unwrap()));
            }
        }
    }
    out
}

#[test]
fn no_order_mutation_reaches_wealthsimple_but_through_the_gate() {
    for (f, text) in server_sources() {
        let found = mutations(&text);
        match f.as_str() {
            "orders/gate.rs" => assert!(!found.is_empty(), "the gate sends them"),
            // the read path names them only to refuse them
            "orders/tools.rs" => assert!(!found.contains(&".mutate(".to_string())),
            _ => assert_eq!(found, Vec::<String>::new(), "{f} reaches an order mutation outside the gate"),
        }
    }
    assert_eq!(mutations("fn f() { c.mutate(s, \"SoOrdersOrderCancel\", &v) }"), vec!["SoOrdersOrderCancel".to_string(), ".mutate(".to_string()]);
    assert!(mutations("// SoOrdersOrderCreate\nfn f() {}\n#[cfg(test)]\nmod t { fn g() { x.mutate(1) } }").is_empty());
}

/// A list the order code acts on is never cut at a count: `LIMIT` only in the page's own
/// paging of the book's orders and brackets.
fn limits(text: &str) -> usize {
    production(text).lines().filter(|l| !l.trim_start().starts_with("//") && l.contains(" LIMIT ")).count()
}

#[test]
fn no_list_the_order_code_acts_on_is_cut_at_a_count() {
    for (f, text) in server_sources().into_iter().filter(|(f, _)| f.starts_with("orders/")) {
        assert_eq!(limits(&text), 0, "{f} cuts a list at a count");
    }
    let book = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../book/src/orders.rs")).unwrap();
    // `orders_before` and `brackets_where`'s paged arm: the page's paging, nothing else
    assert_eq!(limits(&book), 2, "a LIMIT in the book's orders beyond the page's own paging");
    assert_eq!(limits("fn f() { q(\"SELECT * FROM orders LIMIT 200\") }"), 1);
}

#[test]
fn the_order_path_reads_nothing_leniently_and_nothing_from_the_earlier_store() {
    for (f, text) in server_sources().into_iter().filter(|(f, _)| f.starts_with("orders/") || f == "http/orders.rs") {
        let code = &production(&text);
        for word in ["lenient", "page_num", "bagholder_store"] {
            assert!(!code.lines().any(|l| !l.trim_start().starts_with("//") && l.contains(word)), "{f} uses {word}");
        }
    }
}

/// What reads or writes the earlier store's quotes, in a file's code outside its tests.
fn earlier_quotes(text: &str) -> Vec<&'static str> {
    let code = &production(&text);
    const NAMES: [&str; 11] = [
        "upsert_quote", "refresh_quotes", "refresh_distributions", "refresh_all", "refresh_periodic", "quote_loop",
        "quote_symbols_needing_refresh", "peek_quote", "store::market::quotes", "fetch_yahoo_quote", "fetch_cboe_ca_quote",
    ];
    NAMES.iter().copied().filter(|n| code.lines().any(|l| !l.trim_start().starts_with("//") && l.contains(n))).collect()
}

/// Every quote the app reads goes into the market cache through the sources' quote
/// readers (`docs/plans/stage-5-interface-and-running.md`, A2): nothing in the server
/// writes or reads the earlier store's `quotes` table.
#[test]
fn nothing_reads_or_writes_the_earlier_store_s_quotes() {
    for (f, text) in server_sources() {
        assert_eq!(earlier_quotes(&text), Vec::<&str>::new(), "{f} reaches the earlier store's quotes");
    }
    assert_eq!(earlier_quotes("fn f() { bagholder_store::market::upsert_quote(c, s, &q, \"tmx\", n) }"), vec!["upsert_quote"]);
    assert_eq!(earlier_quotes("fn f() { spawn(quote_loop); bagholder_market::quotes::refresh_quotes(c) }"), vec!["refresh_quotes", "quote_loop"]);
    assert!(earlier_quotes("// once upsert_quote\nfn f() {}\n#[cfg(test)]\nmod t { fn g() { peek_quote(c) } }").is_empty());
}

/// What reaches the earlier store (`bagholder.db`) in a file's code, outside its
/// tests: its file or its connections, its schema and repairs, the tables and
/// keys that stayed in it (the rest moved to the book and the market cache,
/// `docs/plans/stage-6-cutover.md`, 6a), and the functions that read those.
fn earlier_store(text: &str) -> Vec<&'static str> {
    let code = &production(&text);
    const NAMES: [&str; 22] = [
        "OLD_FILE", "bagholder.db", "bagholder_store::pool", "bagholder_store::connect", "bagholder_store::open_db", "bagholder_store::schema",
        "bagholder_store::relabel", "gens::install", "bagholder_store::activities", "bagholder_store::rows", "bagholder_store::csvimport",
        "list_watchlist", "tables::fx_rates", "benchmark_days", "benchmark_prices", "FROM fx_rates", "FROM activities", "FROM securities",
        "synced_at", "notify_settings", "notify_seen:", ".open()",
    ];
    NAMES.iter().copied().filter(|n| code.lines().any(|l| !l.trim_start().starts_with("//") && l.contains(n))).collect()
}

/// Nothing reads or writes the earlier store after the carry: only the carry and
/// the one-time imports it runs open it (`carry.rs`, `legacy_import.rs`,
/// `legacy_orders.rs`), and Clear data, which empties a cleared kind from it
/// through the carry's own `carry::clear_old_store`, and the tools that read a copy of one the person names
/// (`compare.rs`, `compare-figures`; `demo_facts.rs`, the demo's). The figure
/// path names its file only to import it into a new book. The market readers
/// convert and count days from the book's rates and the market cache's tracker
/// closes, not the earlier store's tables. An update that is rolled back names
/// its file to put it back as the update found it (`update.rs`).
#[test]
fn nothing_reads_the_earlier_store_but_the_carry() {
    const CARRY: [&str; 5] = ["carry.rs", "legacy_import.rs", "legacy_orders.rs", "compare.rs", "demo_facts.rs"];
    for (f, text) in server_sources() {
        let found = earlier_store(&text);
        match f.as_str() {
            f if CARRY.contains(&f) => {}
            "figures.rs" => assert_eq!(found, vec!["OLD_FILE", "bagholder.db"], "the figure path names the earlier store only to import it"),
            // an update rolled back puts the stores back as it found them, the earlier
            // one among them: the carry brings it to its own schema at a start
            "update.rs" => assert_eq!(found, vec!["OLD_FILE"], "the update names the earlier store only to put it back"),
            _ => assert_eq!(found, Vec::<&str>::new(), "{f} reaches the earlier store"),
        }
    }
    let market = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../market/src");
    for f in ["history.rs", "shorts.rs"] {
        let text = std::fs::read_to_string(market.join(f)).unwrap();
        let code = &production(&text);
        for dropped in ["read_fx", "tables::fx_rates", "benchmark_days", "benchmark_prices"] {
            // the earlier quote readers keep their own rates, in `quotes.rs`
            assert!(!code.lines().any(|l| !l.trim_start().starts_with("//") && l.contains(dropped)), "market/{f} reads {dropped}");
        }
    }
    assert_eq!(earlier_store("fn f() { let c = bagholder_store::connect(&home)?; }"), vec!["bagholder_store::connect"]);
    assert_eq!(earlier_store("fn f() { get_meta(&c, \"synced_at\", \"\") }"), vec!["synced_at"]);
    assert!(earlier_store("// bagholder.db once\nfn f() {}\n#[cfg(test)]\nmod t { fn g() { app.open() } }").is_empty());
}

/// The whole readers: every quote, close and broker statement of the book at once.
const WHOLE_READERS: [&str; 3] = ["market", "market_from_cache", "brokers"];

/// The only places that may read the market and every account's statements whole
/// (`docs/plans/stage-3-the-brokers-statements.md`, brief 22, 11): the engine built
/// at a start (`figures::build`, whose `market` reads the cache and the statements),
/// the sources' own build (`read_sources::engine`) and the comparison with the
/// earlier app (`compare.rs`). A change handler reads only what its change named;
/// a call anywhere else fails here.
const WHOLE_READS: [(&str, &str); 6] = [
    ("compare.rs", "market_from_cache"),
    ("compare.rs", "brokers"),
    ("figures.rs", "market_from_cache"),
    ("figures.rs", "brokers"),
    ("figures.rs", "market"),
    ("read_sources.rs", "market_from_cache"),
];

/// Each whole reader a file's code calls, outside its tests and comments, once per call.
fn whole_reads(text: &str) -> Vec<&'static str> {
    let code = &production(&text);
    let call = regex::Regex::new(r"(^|[^A-Za-z0-9_.])(market|market_from_cache|brokers)\(").unwrap();
    let mut out = Vec::new();
    for l in code.lines().filter(|l| !l.trim_start().starts_with("//")) {
        for c in call.captures_iter(l) {
            let name = c.get(2).unwrap();
            if l[..name.start()].trim_end().ends_with("fn") {
                continue;
            }
            out.push(*WHOLE_READERS.iter().find(|r| **r == name.as_str()).unwrap());
        }
    }
    out
}

#[test]
fn only_a_start_reads_the_market_and_every_account_whole() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found: Vec<(String, &str)> = Vec::new();
    let mut dirs = vec![src.clone()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap().map(Result::unwrap) {
            let p = e.path();
            if p.is_dir() {
                dirs.push(p);
                continue;
            }
            let rel = p.strip_prefix(&src).unwrap().to_string_lossy().replace('\\', "/");
            if !rel.ends_with(".rs") || rel.starts_with("tests_") {
                continue;
            }
            for r in whole_reads(&std::fs::read_to_string(&p).unwrap()) {
                found.push((rel.clone(), r));
            }
        }
    }
    found.sort();
    let mut want: Vec<(String, &str)> = WHOLE_READS.iter().map(|(f, r)| (f.to_string(), *r)).collect();
    want.sort();
    assert_eq!(found, want, "a whole read of the market or every account's statements was added or removed: a change reads only what it changed");
}

#[test]
fn the_whole_read_check_finds_a_call_and_not_a_definition_a_method_a_comment_or_a_test() {
    assert_eq!(whole_reads("let m = market(book, cache)?;"), vec!["market"]);
    assert_eq!(whole_reads("m.brokers = engine_inputs::brokers(&book)?;"), vec!["brokers"]);
    assert!(whole_reads("pub fn market(book: &Book) {}\nlet x = adapter.brokers(1);").is_empty());
    assert!(whole_reads("// market(book, cache)\nfn f() {}\n#[cfg(test)]\nmod t { fn g() { market(b, c); } }").is_empty());
    // a test-only field or import near the top does not hide the code after it (brief 25)
    let planted = "#[cfg(test)]\nuse crate::fake;\nstruct S {\n    #[cfg(test)]\n    seen: u8,\n}\nfn sweep() { let m = crate::figures::market(&book, &cache)?; }\n#[cfg(test)]\nmod t {}";
    assert_eq!(whole_reads(planted), vec!["market"]);
}

/// Each line of a file's code, outside its tests and comments, that names one of
/// Wealthsimple's hosts.
fn wealthsimple_hosts(text: &str) -> Vec<String> {
    let code = &production(text);
    code.lines().filter(|l| !l.trim_start().starts_with("//") && l.contains("wealthsimple.com")).map(|l| l.trim().to_string()).collect()
}

/// Every Wealthsimple address the app asks comes from one place that the stand-in's
/// variable (`BAGHOLDER_WS_BASE`) redirects: the newer client's `hosts` module, and
/// the older client's own reading of it. Any other crate's code naming a host would
/// reach Wealthsimple itself from a test's signed-in server (brief 37, 6).
#[test]
fn every_wealthsimple_address_comes_from_the_one_place_a_stand_in_redirects() {
    let crates = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    // the one place each client names its hosts: private constants, read only by the
    // functions that honour the stand-in's variable (each file's own test holds that)
    const ALLOWED: [(&str, &[&str]); 2] = [("wealthsimple/src/hosts.rs", &["const WEB: &str", "const TOKEN: &str"]), ("ws/src/session.rs", &["const GRAPHQL: &str", "const OAUTH: &str", "const WEB: &str"])];
    fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    for c in std::fs::read_dir(&crates).unwrap().flatten() {
        let src = c.path().join("src");
        if src.is_dir() {
            walk(&src, &mut files);
        }
    }
    assert!(files.len() > 100, "every crate's source is read");
    let mut found = Vec::new();
    for f in files {
        let rel = f.strip_prefix(&crates).unwrap().to_string_lossy().replace('\\', "/");
        // a module that is only a test's (`tests_*.rs`) holds no code the app runs
        let test_only = f.file_name().is_some_and(|n| n.to_string_lossy().starts_with("tests_"));
        if test_only {
            continue;
        }
        let allowed: &[&str] = ALLOWED.iter().find(|(file, _)| *file == rel).map_or(&[], |(_, lines)| *lines);
        for l in wealthsimple_hosts(&std::fs::read_to_string(&f).unwrap()) {
            if !allowed.iter().any(|a| l.starts_with(a)) {
                found.push(format!("{rel}: {l}"));
            }
        }
    }
    assert!(found.is_empty(), "Wealthsimple named outside its hosts module:\n{}", found.join("\n"));
    assert_eq!(wealthsimple_hosts("const X: &str = \"https://my.wealthsimple.com/x\";"), vec!["const X: &str = \"https://my.wealthsimple.com/x\";"]);
    assert!(wealthsimple_hosts("/// help.wealthsimple.com\nfn f() {}").is_empty());
}
