//! The shared model cases in tests/cases, run through this model: the same
//! files the Swift and Kotlin implementations run through theirs, so none can
//! disagree without a failing test. Regenerate with `make-cases` after an
//! intended model change and review the diff.

use serde_json::Value;
use std::path::PathBuf;

use bagholder_model::cases::{case_doc, cases, expect, expect_from, to_text};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/cases")
}

/// Equal as the JSON values are equal: 2 and 2.0 are the same number.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q)),
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).map_or(false, |w| same(v, w)))
        }
        _ => a == b,
    }
}

fn read(path: &PathBuf) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn test_every_case_matches() {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir()).unwrap().map(|e| e.unwrap().path())
        .filter(|p| p.extension().map_or(false, |x| x == "json")).collect();
    paths.sort();
    assert!(!paths.is_empty(), "no cases found");
    for path in paths {
        let doc = read(&path);
        let got = expect_from(&doc["snapshot"], &doc["market"], doc["today"].as_str().unwrap(), &doc["filters"], doc.get("journal"));
        assert!(same(&got, &doc["expect"]), "{}", path.display());
    }
}

#[test]
fn test_cases_are_current() {
    for (name, case) in cases() {
        let path = dir().join(format!("{}.json", name));
        assert!(same(&read(&path)["expect"], &expect(&case)), "{}", name);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), to_text(&case_doc(&case)), "{} is not what make-cases writes", name);
    }
}
