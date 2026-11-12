//! CDS's tax forms read as the readers read them (`cds_tax`, brief 82, PR 2).
//! Every form, list and figure under `tests/replies/cds` is made up, in the CRA
//! template's own layout: no filing's content is in the repository.

use bagholder_core::jiff::civil::Date;
use bagholder_core::Dec;
use bagholder_market::cds_tax::{self, Filing, FormRef, Found, PerUnit};

fn reply(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/replies/cds").join(name)).unwrap()
}
fn d(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}
fn day(s: &str) -> Date {
    s.parse().unwrap()
}
fn workbook(name: &str) -> cds_tax::Form {
    let t = bagholder_market::xls::table(&reply(name)).unwrap();
    cds_tax::form_from_grid(&cds_tax::grid_of(&t)).unwrap()
}

/// A workbook form, read through the app's own workbook reader: its SYMBOL, CUSIP
/// and method, and each record date's return of capital and non-cash distribution
/// per unit, worked by hand: 60% of 0.20 income is 0.12; December's 60% of 0.25 is
/// 0.15, beside its 0.05 non-cash.
#[test]
fn a_workbook_form_gives_each_record_dates_figures() {
    let f = workbook("made-up-percent.xls");
    assert_eq!((f.symbol.as_str(), f.cusip.as_str(), f.percent), ("ZZB", "000000ZZ1", true));
    let got = cds_tax::per_unit(&f).unwrap();
    let want = [("2024-03-27", "0.12", "0"), ("2024-06-26", "0.12", "0"), ("2024-09-26", "0.12", "0"), ("2024-12-30", "0.15", "0.05")];
    assert_eq!(got.len(), 4);
    for (g, (r, roc, nc)) in got.iter().zip(want) {
        assert_eq!(g, &PerUnit { record: day(r), return_of_capital: d(roc), non_cash: d(nc) });
    }
}

/// A form laid out by `pdftotext -layout`: each label's figures on the line after
/// it, under its distribution's heading, an empty row read as none. By hand: 50% of
/// 0.30 is 0.15; the third's 25% of 0.40 is 0.10, beside its 0.10 non-cash.
#[test]
fn a_laid_out_pdf_form_gives_each_record_dates_figures() {
    let f = cds_tax::form_from_layout(&String::from_utf8(reply("made-up-layout.txt")).unwrap()).unwrap();
    assert_eq!((f.symbol.as_str(), f.cusip.as_str(), f.percent), ("ZZC", "000000ZZ2", true));
    assert!(f.distributions.iter().all(|x| !x.boxes.contains_key("23") && !x.boxes.contains_key("33")), "an empty row is none, never the row under it");
    let got = cds_tax::per_unit(&f).unwrap();
    assert_eq!(got, vec![
        PerUnit { record: day("2025-04-28"), return_of_capital: d("0.15"), non_cash: d("0") },
        PerUnit { record: day("2025-08-27"), return_of_capital: d("0.15"), non_cash: d("0") },
        PerUnit { record: day("2025-12-29"), return_of_capital: d("0.10"), non_cash: d("0.10") },
    ]);
}

/// A form whose boxes do not add to what was paid on a record date, within a cent,
/// is set aside whole: its 30% and 60% of 0.20 come to 0.18 against 0.20 paid.
#[test]
fn a_form_whose_boxes_do_not_add_to_what_was_paid_is_set_aside() {
    let f = workbook("made-up-unbalanced.xls");
    let why = cds_tax::per_unit(&f).unwrap_err();
    assert!(why.contains("2024-03-27"), "{why}");
}

/// The breakdown service's list and a year's list on www.cds.ca, each row read.
#[test]
fn both_lists_are_read_row_by_row() {
    let ctbs = cds_tax::ctbs_list(&String::from_utf8(reply("made-up-ctbs-list.html")).unwrap());
    assert_eq!(ctbs.len(), 3);
    assert_eq!(ctbs[1], Filing { received: "2026-03-02 09:00:00".into(), cusip: "000000ZZ2".into(), revised: true, name: "MADE UP COVERED CALL UTILITIES ETF".into(), kind: "T3R16".into(), year: 2025, form: FormRef::Ctbs("T00000000002".into()) });
    let year = cds_tax::year_list(&String::from_utf8(reply("made-up-2024-list.html")).unwrap(), 2024);
    assert_eq!(year.len(), 2);
    assert_eq!((year[0].cusip.as_str(), year[0].name.as_str(), year[0].kind.as_str()), ("000000ZZ1", "MADE UP COVERED CALL BANKS ETF", "T3R16"));
    assert_eq!(year[0].form, FormRef::Xls("0/AAAA/$File/MADE-UP_T3_R16_TY2024.xls".into()));
}

/// A fund named apart from its filing (a class suffix; `and` for `&`) still finds
/// it by the words they share, the latest of a revised filing first, a T5 never,
/// and a CUSIP already known is the only candidate.
#[test]
fn candidates_are_found_by_shared_words_and_a_known_cusip() {
    let ctbs = cds_tax::ctbs_list(&String::from_utf8(reply("made-up-ctbs-list.html")).unwrap());
    let c = cds_tax::candidates(&ctbs, "Made Up Covered Call Utilities ETF - Class E Units", 2025, None);
    assert_eq!(c.iter().map(|f| f.form.clone()).collect::<Vec<_>>(), vec![FormRef::Ctbs("T00000000002".into())], "the revised filing, once, and no T5");
    let amp = vec![Filing { received: "1".into(), cusip: "000000ZZ5".into(), revised: false, name: "MADE UP BANKS & INSURERS ETF".into(), kind: "T3R16".into(), year: 2024, form: FormRef::Xls("x".into()) }];
    assert_eq!(cds_tax::candidates(&amp, "Made Up Banks and Insurers ETF", 2024, None).len(), 1);
    let year = cds_tax::year_list(&String::from_utf8(reply("made-up-2024-list.html")).unwrap(), 2024);
    let known = cds_tax::candidates(&year, "anything at all", 2024, Some("000000ZZ7"));
    assert_eq!(known.iter().map(|f| f.cusip.as_str()).collect::<Vec<_>>(), vec!["000000ZZ7"]);
}

/// A fund's form is the first candidate whose own SYMBOL is the listing's: a
/// candidate with another symbol is passed over, and a form found but unbalanced
/// is kept as set aside, never read.
#[test]
fn the_forms_own_symbol_decides_and_an_unbalanced_one_is_set_aside() {
    let year = cds_tax::year_list(&String::from_utf8(reply("made-up-2024-list.html")).unwrap(), 2024);
    let fetch = |f: &Filing| -> Result<Vec<u8>, String> {
        Ok(match &f.form {
            FormRef::Xls(h) if h.contains("AAAA") => reply("made-up-percent.xls"),
            _ => reply("made-up-unbalanced.xls"),
        })
    };
    match cds_tax::find(&year, "Made Up Covered Call Banks ETF", "ZZB", 2024, None, &fetch).unwrap() {
        Found::Read { cusip, per_unit, .. } => {
            assert_eq!(cusip, "000000ZZ1");
            assert_eq!(per_unit.len(), 4);
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(cds_tax::find(&year, "Made Up Banks ETF", "ZZQ", 2024, None, &fetch).unwrap(), Found::SetAside { .. }));
    assert_eq!(cds_tax::find(&year, "Made Up Banks ETF", "NONE", 2024, None, &fetch).unwrap(), Found::None);
}

/// Read as the slips are: a past year once; the year just ended once a day from
/// February until a read on or after April 1; the year running, never.
#[test]
fn forms_are_read_once_a_past_year_and_daily_through_march_for_the_last() {
    let t = |s: &str| day(s);
    assert!(!cds_tax::due(2026, None, t("2026-03-10")), "the year running");
    assert!(cds_tax::due(2024, None, t("2026-03-10")));
    assert!(!cds_tax::due(2024, Some(t("2025-04-01")), t("2026-03-10")), "a past year once");
    assert!(!cds_tax::due(2025, None, t("2026-01-15")), "before February");
    assert!(cds_tax::due(2025, None, t("2026-02-01")));
    assert!(cds_tax::due(2025, Some(t("2026-03-09")), t("2026-03-10")), "once a day");
    assert!(!cds_tax::due(2025, Some(t("2026-03-10")), t("2026-03-10")));
    assert!(cds_tax::due(2025, Some(t("2026-03-31")), t("2026-04-02")), "a read on or after April 1 is still owed");
    assert!(!cds_tax::due(2025, Some(t("2026-04-01")), t("2026-04-02")));
}

/// What a read found is kept, and the CUSIP the latest form stated is the one later
/// years are matched by.
#[test]
fn what_a_read_found_is_kept_with_its_cusip() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    for m in &bagholder_sources::cache::MIGRATIONS {
        conn.execute_batch(m.sql).unwrap();
    }
    use bagholder_market::stored::fund_tax;
    let read = Found::Read { cusip: "000000ZZ1".into(), form: "f1".into(), per_unit: vec![PerUnit { record: day("2024-03-27"), return_of_capital: d("0.12"), non_cash: d("0") }] };
    fund_tax::save(&conn, "ZZB:XTSE", 2024, &read, "2025-04-02T00:00:00Z").unwrap();
    fund_tax::save(&conn, "ZZB:XTSE", 2025, &Found::None, "2026-02-01T00:00:00Z").unwrap();
    let kept = fund_tax::kept(&conn, "ZZB:XTSE").unwrap();
    assert_eq!(kept.len(), 2);
    assert_eq!(kept[0].found, read);
    assert_eq!(fund_tax::cusip_of(&conn, "ZZB:XTSE").unwrap().as_deref(), Some("000000ZZ1"));
}
