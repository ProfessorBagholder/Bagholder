//! SEDAR+ filings from a shell, as JSON.
//!
//!     sedar resolve <name|number>              profiles matching an issuer
//!     sedar filings <name|number> [n]          an issuer's filings
//!     sedar newest [n]                         newest filings across SEDAR+
//!     sedar get <profileNo> <id> <dest.pdf>    download one document

use bagholder_market::disclosures::{repr_quoted, SourceError};
use bagholder_market::sedar;
use serde_json::{json, Value};

const USAGE: &str = "sedar — SEDAR+ filings over HTTP\n  sedar resolve <name|number>       profiles matching an issuer\n  sedar filings <name|number> [n]   an issuer's filings (JSON)\n  sedar newest [n]                  newest filings across SEDAR+\n  sedar get <profileNo> <id> <dest.pdf>   download one document\n";

fn fail(msg: String) -> i32 {
    println!("{}", json!({"ok": false, "error": msg}));
    1
}

fn run(argv: &[String]) -> Result<i32, SourceError> {
    let arg = |i: usize| argv.get(i).cloned().unwrap_or_default();
    let count = |i: usize, d: usize| -> Result<usize, i32> {
        match argv.get(i) {
            None => Ok(d),
            Some(t) => t.trim().parse().map_err(|_| {
                eprintln!("not a number: {}", t);
                2
            }),
        }
    };
    match argv[0].as_str() {
        "resolve" => match sedar::resolve_profile(&arg(1))? {
            sedar::Lookup::Found(rows) => println!("{}", serde_json::to_string_pretty(&json!({"ok": true, "profiles": rows})).unwrap()),
            sedar::Lookup::NotFound => {
                let q = arg(1).trim().to_string();
                return Ok(fail(if q.is_empty() { "empty query".into() } else { format!("no SEDAR+ profile matched {}", repr_quoted(&q)) }));
            }
        },
        "filings" => {
            let limit = match count(2, sedar::SEARCH_LIMIT) { Ok(n) => n, Err(c) => return Ok(c) };
            match sedar::list_filings(Some(&arg(1)), None, limit)? {
                Some(Value::Object(m)) => {
                    let mut out = serde_json::Map::new();
                    out.insert("ok".into(), json!(true));
                    out.extend(m);
                    println!("{}", serde_json::to_string_pretty(&Value::Object(out)).unwrap());
                }
                _ => return Ok(fail(format!("no SEDAR+ profile matched {}", repr_quoted(arg(1).trim())))),
            }
        }
        "newest" => {
            let limit = match count(1, 30) { Ok(n) => n, Err(c) => return Ok(c) };
            println!("{}", serde_json::to_string_pretty(&json!({"ok": true, "filings": sedar::newest(limit)?})).unwrap());
        }
        "get" => {
            if argv.len() < 4 {
                eprintln!("usage: sedar get <profileNo> <id> <dest.pdf>");
                return Ok(2);
            }
            match sedar::download_bytes(&argv[1], &argv[2], None)? {
                Some((data, ct)) => {
                    if let Err(e) = std::fs::write(&argv[3], &data) {
                        return Ok(fail(e.to_string()));
                    }
                    println!("{}", json!({"ok": true, "path": argv[3], "contentType": ct, "bytes": data.len()}));
                }
                None => return Ok(fail(format!("no document {} in profile {}", repr_quoted(&argv[2]), argv[1]))),
            }
        }
        other => {
            eprintln!("unknown command {}", repr_quoted(other));
            return Ok(2);
        }
    }
    Ok(0)
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() || matches!(argv[0].as_str(), "-h" | "--help" | "help") {
        eprint!("{}", USAGE);
        std::process::exit(2);
    }
    if !sedar::available() {
        std::process::exit(fail("SEDAR+ needs the bagholder-browser helper beside this program".into()));
    }
    let code = run(&argv).unwrap_or_else(|e| fail(e.to_string()));
    std::process::exit(code);
}
