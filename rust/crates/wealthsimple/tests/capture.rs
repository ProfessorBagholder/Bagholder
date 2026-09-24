//! Every row of a whole capture of the owner's history, mapped: run by hand
//! against a capture kept outside the repository (it is the owner's data), with
//! `BAGHOLDER_WS_CAPTURE=<dir> cargo test -p bagholder-wealthsimple --test capture -- --ignored --nocapture`.
//! It prints what each kind of row became and every problem, counted.

mod common;

use std::collections::BTreeMap;

use bagholder_sources::reply::Node;
use common::*;

#[test]
#[ignore = "reads the owner's capture, kept outside the repository"]
fn every_row_of_the_capture_maps() {
    let dir = std::env::var("BAGHOLDER_WS_CAPTURE").expect("BAGHOLDER_WS_CAPTURE names the capture's directory");
    let dir = std::path::Path::new(&dir);
    let mapped = map_all(&mut replay(dir));
    let rows: Vec<_> = mapped.iter().map(|(r, _)| r.clone()).collect();
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut problems: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut legs = 0;
    for (row, m) in &mapped {
        let n = Node::root(row);
        let key = format!("{} {} {}", n.text("type").unwrap(), n.opt_text("subType").unwrap().unwrap_or("-"), n.text("unifiedStatus").unwrap());
        legs += m.legs.len();
        let what = m.legs.iter().map(|l| l.kind.to_string()).collect::<Vec<_>>().join("+");
        *kinds.entry(format!("{key} -> {}", if what.is_empty() { "nothing".into() } else { what })).or_default() += 1;
        for p in &m.problems {
            let e = problems.entry(format!("{} ({key})", p.code)).or_insert((0, p.detail.clone()));
            e.0 += 1;
        }
    }
    println!("{} rows, {legs} transactions", rows.len());
    for (k, n) in &kinds {
        println!("{n:6} {k}");
    }
    println!("problems:");
    for (k, (n, d)) in &problems {
        println!("{n:6} {k}: {d}");
    }
}
