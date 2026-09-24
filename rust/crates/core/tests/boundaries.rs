//! The boundaries of the foundation (`docs/plans/stage-1-foundation.md`, "Three
//! new crates"), held by the build's own files:
//!
//! - each crate depends only on what its column allows;
//! - none reads the machine's clock: every time is given to it, or, for the
//!   network, asked of the clock it is handed, the machine's read in one file;
//! - no float appears outside the two places allowed one: `Dec::to_f64`, the one
//!   way a decimal leaves for statistics, and the import's reading of the earlier
//!   database, which stored money as floats.
//!
//! Types hold the rest: `Dec` has no constructor from a float (a `compile_fail`
//! doctest), and the engine will depend on this crate alone, not on `rust_decimal`.
//! A text scan can be dodged by an alias, so the scan is a tripwire; review against
//! the plan is the rest.

use std::path::{Path, PathBuf};

struct Rules {
    name: &'static str,
    dir: &'static str,
    allowed: &'static [&'static str],
    /// Source files, relative to the crate, that may use `f64`: a path ending in
    /// `/` allows every file under it.
    floats_in: &'static [&'static str],
    clock: ClockRule,
}

/// How a crate may come by the time.
#[derive(Clone, Copy)]
enum ClockRule {
    /// It is handed every time as a value, and asks no clock at all.
    Given,
    /// It is handed a clock and asks it (`clock.now()`); only the files named
    /// read the machine's.
    Handed { machine_in: &'static [&'static str] },
}

impl Rules {
    fn floats_allowed(&self, path: &str) -> bool {
        self.floats_in.iter().any(|p| if p.ends_with('/') { path.starts_with(p) } else { path == *p })
    }

    /// What in a line of `path` reads the machine's clock.
    fn clock_reads(&self, path: &str) -> &'static [&'static str] {
        match self.clock {
            ClockRule::Given => &["now(", "SystemTime", "Instant"],
            ClockRule::Handed { machine_in } if machine_in.contains(&path) => &[],
            ClockRule::Handed { .. } => &["::now(", "SystemTime", "Instant", "SystemClock."],
        }
    }
}

const CRATES: [Rules; 6] = [
    Rules { name: "bagholder-core", dir: "core", allowed: &["serde", "rust_decimal", "jiff", "uuid"], floats_in: &["src/dec.rs"], clock: ClockRule::Given },
    Rules { name: "bagholder-sqlite", dir: "sqlite", allowed: &["rusqlite", "jiff"], floats_in: &[], clock: ClockRule::Given },
    Rules {
        name: "bagholder-book",
        dir: "book",
        allowed: &["bagholder-core", "bagholder-sqlite", "rusqlite", "serde", "serde_json", "uuid", "jiff"],
        floats_in: &["src/import/old.rs"],
        clock: ClockRule::Given,
    },
    // the engine (docs/plans/stage-2-engine.md): the vocabulary alone, which
    // re-exports the calendar; floats only for the statistics
    Rules { name: "bagholder-engine", dir: "engine", allowed: &["bagholder-core"], floats_in: &["src/stat/"], clock: ClockRule::Given },
    // the sources (docs/plans/stage-3a-sources.md): no float anywhere, no clock
    Rules {
        name: "bagholder-sources",
        dir: "sources",
        allowed: &["bagholder-core", "bagholder-sqlite", "bagholder-net", "bagholder-book", "rusqlite", "jiff"],
        floats_in: &[],
        clock: ClockRule::Given,
    },
    // the network: no float, and the time only from the clock it is handed, the
    // machine's read in one file
    Rules {
        name: "bagholder-net",
        dir: "net",
        allowed: &["openssl", "flate2", "serde_json", "jiff"],
        floats_in: &[],
        clock: ClockRule::Handed { machine_in: &["src/machine.rs"] },
    },
];

/// The names a manifest depends on for the crate itself: every `[dependencies]`
/// section, written any way Cargo reads it (`[dependencies]`,
/// `[dependencies.name]`, `[target.'cfg(..)'.dependencies]` and its `.name`
/// form), and build dependencies, which run code at build time. Only
/// `[dev-dependencies]` (the tests' own) are not the crate's.
fn dependencies(manifest: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in manifest.lines() {
        let line = line.trim();
        if let Some(header) = line.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
            let header = header.trim();
            // the last part of the header that names a dependency table, and a name after it
            let parts: Vec<&str> = header.split('.').collect();
            let table = parts.iter().position(|p| matches!(p.trim_matches('"'), "dependencies" | "build-dependencies"));
            match table {
                Some(i) if i + 1 < parts.len() => {
                    out.push(parts[i + 1].trim_matches('"').to_string());
                    inside = false;
                }
                Some(_) => inside = true,
                None => inside = false,
            }
            continue;
        }
        if inside && !line.is_empty() && !line.starts_with('#') {
            if let Some((name, _)) = line.split_once('=') {
                out.push(name.trim().trim_matches('"').to_string());
            }
        }
    }
    out
}

/// What is wrong with one crate: its manifest and its sources (path, text).
fn violations(rules: &Rules, manifest: &str, sources: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    for dep in dependencies(manifest) {
        if !rules.allowed.contains(&dep.as_str()) {
            out.push(format!("{} depends on {dep}", rules.name));
        }
    }
    for (path, text) in sources {
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            for clock in rules.clock_reads(path) {
                if code.contains(clock) {
                    out.push(format!("{}:{}: reads the clock ({clock})", path, n + 1));
                }
            }
            if code.contains("f64") && !rules.floats_allowed(path) {
                out.push(format!("{}:{}: uses a float", path, n + 1));
            }
        }
    }
    out
}

fn sources(dir: &Path) -> Vec<(String, String)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                out.push((rel, std::fs::read_to_string(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &dir.join("src"), &mut out);
    out
}

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

#[test]
fn the_foundation_keeps_to_its_boundaries() {
    let mut all = Vec::new();
    for rules in &CRATES {
        let dir = crates_dir().join(rules.dir);
        let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        // the unit tests inside a crate's `src` are held the same way
        all.extend(violations(rules, &manifest, &sources(&dir)));
    }
    assert!(all.is_empty(), "{}", all.join("\n"));
}

#[test]
fn the_checker_catches_each_kind_of_violation() {
    let rules = &CRATES[0];
    let clean = "[package]\nname = \"x\"\n[dependencies]\nserde = \"1\"\n# a comment\n[dev-dependencies]\ntempfile = \"3\"\n";
    assert!(violations(rules, clean, &[]).is_empty(), "a dev-dependency is not the crate's");
    let networked = "[dependencies]\nserde = \"1\"\nreqwest = \"0.12\"\n";
    assert_eq!(violations(rules, networked, &[]), vec!["bagholder-core depends on reqwest"]);
    for sneaky in [
        "[dependencies.reqwest]\nversion = \"0.12\"\n",
        "[target.'cfg(unix)'.dependencies]\nreqwest = \"0.12\"\n",
        "[target.'cfg(unix)'.dependencies.reqwest]\nversion = \"0.12\"\n",
        "[build-dependencies]\nreqwest = \"0.12\"\n",
    ] {
        assert_eq!(violations(rules, sneaky, &[]), vec!["bagholder-core depends on reqwest"], "{sneaky}");
    }
    let clock = vec![("src/a.rs".to_string(), "let t = jiff::Timestamp::now();".to_string())];
    assert_eq!(violations(rules, clean, &clock).len(), 1);
    let std_clock = vec![("src/a.rs".to_string(), "let t = std::time::SystemTime::UNIX_EPOCH;".to_string())];
    assert_eq!(violations(rules, clean, &std_clock).len(), 1);
    let float = vec![("src/money.rs".to_string(), "fn from(v: f64) -> Dec".to_string())];
    assert_eq!(violations(rules, clean, &float), vec!["src/money.rs:1: uses a float"]);
    let allowed = vec![("src/dec.rs".to_string(), "pub fn to_f64(self) -> f64".to_string())];
    assert!(violations(rules, clean, &allowed).is_empty());
    let commented = vec![("src/a.rs".to_string(), "let x = 1; // never an f64, never now()".to_string())];
    assert!(violations(rules, clean, &commented).is_empty());

    // the engine: the vocabulary alone, floats only for the statistics
    let engine = &CRATES[3];
    let on_core = "[dependencies]\nbagholder-core = { path = \"../core\" }\n";
    assert!(violations(engine, on_core, &[]).is_empty());
    let on_decimal = "[dependencies]\nbagholder-core = { path = \"../core\" }\nrust_decimal = \"1\"\n";
    assert_eq!(violations(engine, on_decimal, &[]), vec!["bagholder-engine depends on rust_decimal"]);
    let on_store = "[dependencies]\nbagholder-store = { path = \"../store\" }\n";
    assert_eq!(violations(engine, on_store, &[]), vec!["bagholder-engine depends on bagholder-store"]);
    let stat = vec![("src/stat/returns.rs".to_string(), "fn r(a: f64) -> f64".to_string())];
    assert!(violations(engine, on_core, &stat).is_empty());
    let figure = vec![("src/trades.rs".to_string(), "let pnl: f64 = 0.0;".to_string())];
    assert_eq!(violations(engine, on_core, &figure), vec!["src/trades.rs:1: uses a float"]);
    let clock = vec![("src/fx.rs".to_string(), "let now = Timestamp::now();".to_string())];
    assert_eq!(violations(engine, on_core, &clock).len(), 1);

    // the sources: no float, no clock, and not the old market code
    let sources = &CRATES[4];
    let on_book = "[dependencies]\nbagholder-core = { path = \"../core\" }\nbagholder-book = { path = \"../book\" }\n";
    assert!(violations(sources, on_book, &[]).is_empty());
    let on_market = "[dependencies]\nbagholder-market = { path = \"../market\" }\n";
    assert_eq!(violations(sources, on_market, &[]), vec!["bagholder-sources depends on bagholder-market"]);
    let price = vec![("src/adapters/cboe.rs".to_string(), "let close: f64 = 1.0;".to_string())];
    assert_eq!(violations(sources, on_book, &price), vec!["src/adapters/cboe.rs:1: uses a float"]);
    let clock = vec![("src/due.rs".to_string(), "let t = jiff::Timestamp::now();".to_string())];
    assert_eq!(violations(sources, on_book, &clock).len(), 1);

    // the network: the clock it is handed, the machine's in one file
    let net = &CRATES[5];
    let deps = "[dependencies]\nopenssl = \"0.10\"\njiff = \"0.2\"\n";
    let handed = vec![("src/limiter.rs".to_string(), "let now = clock.now();".to_string())];
    assert!(violations(net, deps, &handed).is_empty(), "asking the clock handed in is not reading the machine's");
    for read in ["let t = Timestamp::now();", "let t = std::time::Instant::now();", "global().refused(h, SystemClock.now(), None)"] {
        let v = vec![("src/net.rs".to_string(), read.to_string())];
        assert!(!violations(net, deps, &v).is_empty(), "{read}");
    }
    let machine = vec![("src/machine.rs".to_string(), "fn now(&self) -> Timestamp { Timestamp::now() }".to_string())];
    assert!(violations(net, deps, &machine).is_empty(), "the one file that reads the machine's clock");
    let on_book = "[dependencies]\nbagholder-book = { path = \"../book\" }\n";
    assert_eq!(violations(net, on_book, &[]), vec!["bagholder-net depends on bagholder-book"]);
    let float = vec![("src/browser.rs".to_string(), "let t = timeout.as_secs_f64();".to_string())];
    assert_eq!(violations(net, deps, &float), vec!["src/browser.rs:1: uses a float"]);
}

/// The code moved out of `bagholder-market` (`docs/plans/stage-3a-sources.md`,
/// "One copy") is there no more: the client, the browser session, the pacing
/// the never-read health record, the HTML table reader with its character
/// references, TMX's venue forms, Yahoo's venue suffixes and the option
/// midpoint rule each live in one place; and every request the old readers make
/// through `http.rs` takes its turn on the one limiter, not the bare client.
#[test]
fn the_moved_code_has_one_copy() {
    let market = crates_dir().join("market");
    for gone in ["src/client.rs", "src/browser.rs", "src/pace.rs", "src/htmltables.rs", "src/entities.rs"] {
        assert!(!market.join(gone).exists(), "bagholder-market still holds {gone}");
    }
    let old_definitions = [
        "fn note_source",
        "fn source_health",
        "struct YahooGate",
        "fn request_any",
        "fn html_tables",
        "fn unescape(",
        "fn charref",
        // TMX's venue forms and the venue each must name
        "FORMS_CAD",
        "FORMS_USD",
        "VENUE_OF_FORM",
        // Yahoo's venue suffixes
        "YAHOO_SUFFIX",
        "YAHOO_FORMS_CAD",
        "YAHOO_FORMS_USD",
        "fn yahoo_root",
        // the option midpoint rule
        "(bid + ask) / 2",
    ];
    for (path, text) in sources(&market) {
        for old in old_definitions {
            assert!(!text.contains(old), "{path} still defines {old}");
        }
    }
    let http = std::fs::read_to_string(market.join("src/http.rs")).expect("market's http.rs");
    assert!(!http.contains("client::request"), "market's http.rs asks the bare client, past the one limiter");
}
