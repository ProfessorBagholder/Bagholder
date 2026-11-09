//! The ETF tax ledger held to cases worked by hand (`acb`, brief 82, PR 3;
//! `SPEC.md` `tax-cost-of-an-etf-1` to `tax-cost-of-an-etf-10`).
//!
//! Each file in `tests/acb_cases` is a small book in the engine cases' form, what
//! the tax inputs state (`tax`), and the ledger the spec requires, worked out by
//! hand from CRA's own worked examples and adjustedcostbase.ca's, or from a book
//! made up for the purpose, with the working written beside it, never produced by
//! running the ledger. An amount is compared exactly as written.

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use bagholder_engine::acb::{self, Event, GainKind, Listed, Missing, PerUnit, Slip, TaxInputs};
use common::*;

fn tax_inputs(case: &Value, ids: &mut Ids) -> TaxInputs {
    let t = case.get("tax").cloned().unwrap_or(Value::Null);
    let mut out = TaxInputs::default();
    for (f, l) in obj(&t, "funds") {
        let listed = match l.as_str().unwrap() {
            "canada" => Listed::Canada,
            "united-states" => Listed::UnitedStates,
            other => panic!("a listing not known: {other}"),
        };
        out.funds.insert(ids.instrument(&f), listed);
    }
    for (f, years) in obj(&t, "slips") {
        for (y, slips) in years.as_object().unwrap() {
            let list = slips.as_array().unwrap().iter().map(|x| Slip { return_of_capital: dec(s(x, "box42").unwrap()), income: dec(s(x, "income").unwrap_or("0")), foreign_tax: dec(s(x, "foreign_tax").unwrap_or("0")) }).collect();
            out.slips.insert((ids.instrument(&f), y.parse().unwrap()), list);
        }
    }
    for (f, days) in obj(&t, "per_unit") {
        let mut m = BTreeMap::new();
        for (d, x) in days.as_object().unwrap() {
            m.insert(day(d), PerUnit { return_of_capital: dec(s(x, "roc").unwrap()), non_cash: dec(s(x, "non_cash").unwrap_or("0")), source: s(x, "source").unwrap_or("fund").into() });
        }
        out.per_unit.insert(ids.instrument(&f), m);
    }
    out.slips_final = arr(&t, "slips_final").iter().map(|y| y.as_str().unwrap().parse().unwrap()).collect();
    for (f, years) in obj(&t, "non_cash_on") {
        for (y, d) in years.as_object().unwrap() {
            out.non_cash_on.insert((ids.instrument(&f), y.parse().unwrap()), day(d.as_str().unwrap()));
        }
    }
    out
}

fn event(e: &Event) -> &'static str {
    match e {
        Event::Bought => "bought",
        Event::Sold => "sold",
        Event::MovedIn => "moved-in",
        Event::ReturnOfCapital => "return-of-capital",
        Event::NonCash => "non-cash",
        Event::BelowZero => "below-zero",
        Event::LossAdded => "loss-added",
        Event::Split => "split",
    }
}

fn missing(m: &Missing) -> String {
    match m {
        Missing::Slip => "slip".into(),
        Missing::RecordDate(_) => "record-date".into(),
        Missing::Rate { currency, day } => format!("rate {} {day}", currency.as_str()),
        Missing::SlipDiffers(by) => format!("slip-differs {}", by.to_text()),
        Missing::MovedInCost(_) => "moved-in-cost".into(),
        Missing::MovedOut(_) => "moved-out".into(),
        Missing::PeriodOpen(d) => format!("period-open {d}"),
        Missing::AccountPlan(_) => "account-plan".into(),
        Missing::CorporateEvent(_) => "corporate-event".into(),
        Missing::Units(_) => "units".into(),
        Missing::EarlierYear(y) => format!("earlier-year {y}"),
        Missing::Calendar(y) => format!("calendar {y}"),
        Missing::Arithmetic => "arithmetic".into(),
    }
}

fn same(what: &str, want: &str, got: bagholder_core::Dec, failures: &mut Vec<String>) {
    if dec(want) != got {
        failures.push(format!("{what}: expected {want}, got {}", got.to_text()));
    }
}

fn check(case: &Value) -> Vec<String> {
    let name = s(case, "name").unwrap().to_string();
    let mut b = build(case);
    let tax = tax_inputs(case, &mut b.ids);
    let got = acb::holdings(&b.inputs, &tax);
    let mut f = Vec::new();
    let expect = case.get("expect").unwrap();
    let want = arr(expect, "acb");
    if want.len() != got.len() {
        f.push(format!("{} holdings expected, {} got", want.len(), got.len()));
    }
    for w in want {
        let fund = b.ids.instrument(s(&w, "fund").unwrap());
        let Some(h) = got.iter().find(|h| h.fund == fund) else {
            f.push(format!("no holding of {}", s(&w, "fund").unwrap()));
            continue;
        };
        let label = s(&w, "fund").unwrap();
        same(&format!("{label} units"), s(&w, "units").unwrap(), h.units, &mut f);
        same(&format!("{label} cost"), s(&w, "cost").unwrap(), h.cost, &mut f);
        if let Some(rows) = w.get("rows").and_then(Value::as_array) {
            let had: Vec<String> = h.rows.iter().map(|r| format!("{} {} {} {} {}", r.day, event(&r.event), r.units.to_text(), r.cad.to_text(), r.cost_after.to_text())).collect();
            let wanted: Vec<String> = rows.iter().map(|r| r.as_str().unwrap().to_string()).collect();
            // compared as numbers: each field is re-read from its text
            let norm = |line: &str| -> Vec<String> { line.split(' ').map(|p| bagholder_core::Dec::parse(p).map(|d| d.to_text()).unwrap_or_else(|_| p.to_string())).collect() };
            let a: Vec<Vec<String>> = wanted.iter().map(|l| norm(l)).collect();
            let g: Vec<Vec<String>> = had.iter().map(|l| norm(l)).collect();
            if a != g {
                f.push(format!("{label} rows:\n  expected\n    {}\n  got\n    {}", wanted.join("\n    "), had.join("\n    ")));
            }
        }
        for (y, wy) in obj(&w, "years") {
            let year: i16 = y.parse().unwrap();
            let Some(gy) = h.years.get(&year) else {
                f.push(format!("{label} {y}: no such year"));
                continue;
            };
            let norm = |line: &str| -> String { line.split(' ').map(|p| bagholder_core::Dec::parse(p).map(|d| d.to_text()).unwrap_or_else(|_| p.to_string())).collect::<Vec<_>>().join(" ") };
            let want_missing: Vec<String> = arr(&wy, "missing").iter().map(|m| norm(m.as_str().unwrap())).collect();
            let got_missing: Vec<String> = gy.missing.iter().map(|m| norm(&missing(m))).collect();
            if want_missing != got_missing {
                f.push(format!("{label} {y} missing: expected {want_missing:?}, got {got_missing:?}"));
            }
            let wg = arr(&wy, "gains");
            if wg.len() != gy.gains.len() {
                f.push(format!("{label} {y}: {} gains expected, {} got: {:?}", wg.len(), gy.gains.len(), gy.gains));
                continue;
            }
            for (n, (a, g)) in wg.iter().zip(&gy.gains).enumerate() {
                let kind = match g.kind {
                    GainKind::Sale => "sale",
                    GainKind::BelowZero => "below-zero",
                };
                if s(a, "kind").unwrap_or("sale") != kind || day(s(a, "day").unwrap()) != g.day {
                    f.push(format!("{label} {y} gain {n}: expected {} on {}, got {kind} on {}", s(a, "kind").unwrap_or("sale"), s(a, "day").unwrap(), g.day));
                }
                for (k, v) in [("units", g.units), ("proceeds", g.proceeds), ("cost", g.cost), ("outlays", g.outlays), ("gain", g.gain), ("denied", g.denied)] {
                    if let Some(want) = s(a, k) {
                        same(&format!("{label} {y} gain {n} {k}"), want, v, &mut f);
                    }
                }
            }
        }
        if let Some(c) = s(&w, "complete_through") {
            let got = h.complete_through().map(|y| y.to_string()).unwrap_or_else(|| "none".into());
            if got != c {
                f.push(format!("{label} complete through: expected {c}, got {got}"));
            }
        }
    }
    f.into_iter().map(|x| format!("{name}: {x}")).collect()
}

#[test]
fn every_case_holds() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/acb_cases");
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    assert!(!files.is_empty());
    let mut failures = Vec::new();
    let mut ran = 0;
    for file in files {
        let cases: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        for case in cases.as_array().unwrap() {
            ran += 1;
            failures.extend(check(case));
        }
    }
    assert!(failures.is_empty(), "{} of {ran} cases failed:\n{}", failures.len(), failures.join("\n"));
}

/// Settlement: as the row states it, else T+3 before 2017-09-05, T+2 from then to
/// 2024-05-27 in Canada and 2024-05-28 in the US, T+1 from then, on each market's
/// own settlement calendar; none in a year no calendar the app holds covers
/// (`tax-cost-of-an-etf-2`).
#[test]
fn settlement_follows_each_markets_cycle_and_calendar() {
    use bagholder_core::transaction::{Kind, Transaction};
    let t = |d: &str| Transaction {
        id: bagholder_core::TransactionId::new(bagholder_core::RecordId::parse("01900000-0000-7000-8000-0000000000f1").unwrap(), bagholder_core::Leg::parse("trade").unwrap()),
        mapping: bagholder_core::MappingVersion { source: bagholder_core::SourceName::named("case"), version: 1 },
        account: bagholder_core::AccountId::parse("01900000-0000-7000-8000-0000000000f2").unwrap(),
        occurred_at: None,
        trade_date: day(d),
        settle_date: None,
        kind: Kind::Buy,
        effect: None,
        instrument: None,
        quantity: None,
        price: None,
        cash: None,
        fee: None,
        fx_rate: None,
        paid_on: None,
        value: None,
    };
    let ca = |d: &str| acb::settlement(&t(d), Listed::Canada, None).map(|x| x.to_string()).unwrap_or_else(|| "none".into());
    let us = |d: &str| acb::settlement(&t(d), Listed::UnitedStates, None).map(|x| x.to_string()).unwrap_or_else(|| "none".into());
    // the move to T+2: Friday 2017-09-01 was T+3 over Labour Day, Tuesday 09-05 T+2,
    // both settling Thursday 09-07
    assert_eq!(ca("2017-09-01"), "2017-09-07");
    assert_eq!(ca("2017-09-05"), "2017-09-07");
    assert_eq!(us("2017-09-01"), "2017-09-07");
    assert_eq!(us("2017-09-05"), "2017-09-07");
    // Friday 2024-05-24: T+2 in both; the US's Memorial Day 05-27 skipped there
    assert_eq!(ca("2024-05-24"), "2024-05-28");
    assert_eq!(us("2024-05-24"), "2024-05-29");
    // Monday 2024-05-27: Canada's first T+1 day; the US's first, Tuesday 05-28
    assert_eq!(ca("2024-05-27"), "2024-05-28");
    assert_eq!(us("2024-05-28"), "2024-05-29");
    // US Thanksgiving is no business day for a US fund and is one for a Canadian
    // fund, 2025 as 2026; Canada Day the other way round
    assert_eq!(us("2025-11-26"), "2025-11-28");
    assert_eq!(ca("2025-11-26"), "2025-11-27");
    assert_eq!(us("2026-11-25"), "2026-11-27");
    assert_eq!(ca("2026-11-25"), "2026-11-26");
    assert_eq!(ca("2025-06-30"), "2025-07-02");
    assert_eq!(us("2025-06-30"), "2025-07-01");
    // days the exchange trades and its market does not settle are skipped: Columbus
    // Day for a US fund, Remembrance Day for a Canadian one, also under T+3
    assert_eq!(us("2025-10-10"), "2025-10-14");
    assert_eq!(ca("2025-11-10"), "2025-11-12");
    assert_eq!(ca("2015-11-10"), "2015-11-16");
    // the NYSE's unscheduled closure for President Carter: DTC settled that day
    assert_eq!(us("2025-01-08"), "2025-01-09");
    // the venue its listing names, where it names one
    use bagholder_core::sessions::Venue;
    assert_eq!(acb::settlement(&t("2026-11-25"), Listed::Canada, Some(Venue::UsEquities)).unwrap().to_string(), "2026-11-27");
    // a year no calendar the app holds covers: none, never a guess
    assert_eq!(ca("2009-06-01"), "none");
    assert_eq!(us("2009-06-01"), "none");
    // a stated settlement stands, in any year
    let mut stated = t("2009-06-01");
    stated.settle_date = Some(day("2009-06-04"));
    assert_eq!(acb::settlement(&stated, Listed::Canada, None).unwrap().to_string(), "2009-06-04");
}
