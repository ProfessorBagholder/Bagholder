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
        // a positions reply names neither its account nor its day: the file does,
        // from what was asked (the account's stand-in and the day)
        let path = match (op.as_str(), map.get("variables")) {
            ("FetchHoldingsExportPositionsAsOfDate", Some(Value::Object(vars))) => {
                let account = match vars.get("accountIds") {
                    Some(Value::Array(ids)) if ids.len() == 1 => {
                        let mut one = std::collections::BTreeMap::new();
                        one.insert("accountId".to_string(), ids[0].clone());
                        match a.value(&Value::Object(one)) {
                            Value::Object(m) => match m.get("accountId") {
                                Some(Value::String(s)) => s.clone(),
                                _ => panic!("{op}: an account id that is not text"),
                            },
                            _ => unreachable!("an object stays an object"),
                        }
                    }
                    // several accounts in one reply: one file per account, each
                    // the reply with that account alone
                    _ => {
                        let Some(Value::String(day)) = vars.get("asOf") else { panic!("{op}: positions asked with no day") };
                        let Value::Object(root) = &clean else { panic!("{op}: a reply that is not an object") };
                        let Some(Value::Object(data)) = root.get("data") else { panic!("{op}: no data") };
                        let Some(Value::Array(accounts)) = data.get("accounts") else { panic!("{op}: no accounts") };
                        for acc in accounts {
                            let Value::Object(am) = acc else { continue };
                            let Some(Value::String(id)) = am.get("id") else { continue };
                            let mut d = std::collections::BTreeMap::new();
                            d.insert("accounts".to_string(), Value::Array(vec![acc.clone()]));
                            let mut r = std::collections::BTreeMap::new();
                            r.insert("data".to_string(), Value::Object(d));
                            let path = format!("{out}/positions@{id}@{day}.json");
                            std::fs::write(&path, Value::Object(r).canonical()).unwrap_or_else(|e| panic!("{path}: {e}"));
                        }
                        continue;
                    }
                };
                let Some(Value::String(day)) = vars.get("asOf") else { panic!("{op}: positions asked with no day") };
                format!("{out}/positions@{account}@{day}.json")
            }
            _ => format!("{out}/{op}-{i}.json"),
        };
        std::fs::write(&path, clean.canonical()).unwrap_or_else(|e| panic!("{path}: {e}"));
    }
    for (op, count) in n {
        println!("{op}: {count}");
    }
}
