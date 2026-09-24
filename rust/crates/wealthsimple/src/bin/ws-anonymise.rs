//! `ws-anonymise <capture.json> <out-dir>`: a capture made in the owner's own
//! browser (a list of `{operationName, variables, reply}`) written out as one
//! file per reply, `<operation>-<n>.json`, every identifying value replaced
//! (`bagholder_wealthsimple::anonymise`). Nothing else from the capture is kept:
//! not the variables, which name the owner's accounts.

use bagholder_core::json::{self, Value};
use bagholder_wealthsimple::anonymise::{leaks, Anonymiser};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, input, out] = args.as_slice() else {
        eprintln!("usage: ws-anonymise <capture.json> <out-dir>");
        std::process::exit(2);
    };
    let text = std::fs::read_to_string(input).unwrap_or_else(|e| panic!("{input}: {e}"));
    let Value::Array(items) = json::parse(&text).unwrap_or_else(|e| panic!("{input}: {e}")) else { panic!("{input}: expected a list of replies") };
    std::fs::create_dir_all(out).unwrap_or_else(|e| panic!("{out}: {e}"));
    let mut a = Anonymiser::default();
    let mut n: std::collections::BTreeMap<String, usize> = Default::default();
    for item in items {
        let Value::Object(map) = item else { panic!("{input}: an item is not an object") };
        let Some(Value::String(op)) = map.get("operationName") else { panic!("{input}: an item names no operation") };
        let reply = map.get("reply").unwrap_or_else(|| panic!("{input}: {op} has no reply"));
        let clean = a.value(reply);
        let left = leaks(&clean);
        assert!(left.is_empty(), "{op}: identifying values left at {left:?}");
        let i = n.entry(op.clone()).or_default();
        *i += 1;
        let path = format!("{out}/{op}-{i}.json");
        std::fs::write(&path, clean.canonical()).unwrap_or_else(|e| panic!("{path}: {e}"));
    }
    for (op, count) in n {
        println!("{op}: {count}");
    }
}
