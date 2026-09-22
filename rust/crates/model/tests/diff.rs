//! The typed differ (`patch::typed`, the stream's) against the JSON one
//! (`patch::diff`) on real views: every shared case turned into every other, and
//! each case turned by the changes the stream carries -- a price moving, a grade
//! set, a headline arriving, a listing watched, an activity arriving, another
//! filter, a trade opened. Both must give the same operations, and those applied to
//! the old view's JSON must give the new one's.

use serde_json::{json, Value};
use std::path::PathBuf;

use bagholder_model::base::build_base;
use bagholder_model::patch::{apply, diff, typed};
use bagholder_model::view::{view_of, Detail};
use bagholder_model::wire::View;

fn cases() -> Vec<Value> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../tests/cases");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().map_or(false, |x| x == "json")).collect();
    paths.sort();
    paths.iter().map(|p| serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()).collect()
}

fn view(doc: &Value, filters: &Value, detail: Option<&str>) -> View {
    let journal = doc.get("journal").and_then(|j| j.as_object()).cloned().unwrap_or_default();
    let base = build_base(&doc["snapshot"], &doc["market"], &journal, doc["today"].as_str());
    view_of(&base, Some(filters), Detail::Only(detail))
}

/// The two differs on one pair; a disagreement is described, not panicked on, so a
/// run lists them all.
fn check(label: &str, a: &View, b: &View, failures: &mut Vec<String>) -> usize {
    let (ja, jb) = (a.to_value(), b.to_value());
    let t = typed(a, b);
    let g = diff(&ja, &jb);
    let mut got = ja.clone();
    apply(&mut got, &t);
    if got != jb {
        failures.push(format!("{label}: the typed patch does not give the new view"));
    }
    if t != g {
        let first = t.iter().zip(&g).position(|(x, y)| x != y).unwrap_or(t.len().min(g.len()));
        let show = |ops: &[Value]| ops.get(first).map(|o| { let s = o.to_string(); s.chars().take(300).collect::<String>() }).unwrap_or_default();
        failures.push(format!("{label}: typed {} ops, json {} ops; first difference at {first}:\n  typed {}\n  json  {}", t.len(), g.len(), show(&t), show(&g)));
    }
    t.len()
}

fn edited(doc: &Value, f: impl Fn(&mut Value)) -> Value {
    let mut d = doc.clone();
    f(&mut d);
    d
}

#[test]
fn test_the_typed_differ_sends_what_the_json_one_sends() {
    let docs = cases();
    assert!(docs.len() >= 30);
    let mut failures = Vec::new();
    let mut ops = 0;
    let views: Vec<View> = docs.iter().map(|d| view(d, &d["filters"], None)).collect();
    for (i, a) in views.iter().enumerate() {
        assert_eq!(check(&format!("case {i} to itself"), a, a, &mut failures), 0, "nothing moved is nothing sent");
        for (j, b) in views.iter().enumerate() {
            ops += check(&format!("case {i} to case {j}"), a, b, &mut failures);
        }
    }
    for (i, doc) in docs.iter().enumerate() {
        let before = &views[i];
        let filters = &doc["filters"];
        let changes: Vec<(&str, Value)> = vec![
            ("every price up a cent", edited(doc, |d| {
                if let Some(q) = d["market"]["quotes"].as_object_mut() {
                    for v in q.values_mut() {
                        if let Some(p) = v["price"].as_f64() {
                            v["price"] = json!(p + 0.01);
                        }
                    }
                }
            })),
            ("a headline arriving", edited(doc, |d| {
                let row = json!({"id": "n-new", "symbol": "*", "exchange": "MARKET", "headline": "new", "wire": "Wire", "url": "", "publishedAt": "2099-01-01T00:00:00Z", "kind": "story"});
                match d["snapshot"]["news"].as_array_mut() {
                    Some(a) => a.insert(0, row),
                    None => d["snapshot"]["news"] = json!([row]),
                }
            })),
            ("a listing watched", edited(doc, |d| {
                let row = json!({"symbol": "ZZZ", "exchange": "TSX", "name": "Zed", "currency": "CAD"});
                match d["snapshot"]["watchlist"].as_array_mut() {
                    Some(a) => a.push(row),
                    None => d["snapshot"]["watchlist"] = json!([row]),
                }
            })),
            ("the last activity gone", edited(doc, |d| {
                if let Some(a) = d["snapshot"]["activities"].as_array_mut() {
                    a.pop();
                }
            })),
            ("the first activity gone", edited(doc, |d| {
                if let Some(a) = d["snapshot"]["activities"].as_array_mut() {
                    if !a.is_empty() {
                        a.remove(0);
                    }
                }
            })),
        ];
        for (what, changed) in changes {
            check(&format!("case {i}: {what}"), before, &view(&changed, filters, None), &mut failures);
        }
        if let Some(t) = before.trades.first() {
            let graded = edited(doc, |d| {
                let journal = d.as_object_mut().unwrap().entry("journal").or_insert(json!({}));
                journal[t.id.as_str()] = json!({"thesis": "", "tags": ["x"], "grade": if t.grade == "B" { "C" } else { "B" }});
            });
            check(&format!("case {i}: a grade set"), before, &view(&graded, filters, None), &mut failures);
            check(&format!("case {i}: a trade opened"), before, &view(doc, filters, Some(&t.id)), &mut failures);
        }
        check(&format!("case {i}: the filters cleared"), before, &view(doc, &json!({}), None), &mut failures);
    }
    assert!(ops > 1000, "the pairs moved something to compare");
    assert!(failures.is_empty(), "{} disagreements:\n{}", failures.len(), failures.iter().take(15).cloned().collect::<Vec<_>>().join("\n"));
}
