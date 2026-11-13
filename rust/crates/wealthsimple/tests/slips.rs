//! Wealthsimple's tax slips, read from their text as `pdftotext -layout` gives it
//! (brief 82, PR 1). The slips here are made up, every value and every name, in
//! the layouts the person's own slips took for 2023 to 2025 (read 2026-11-12 with
//! every value masked); each `.json` beside one is what a reader makes of it.

use std::path::PathBuf;

use bagholder_core::json::{self, Value};
use bagholder_core::Dec;
use bagholder_sources::reply::Node;
use bagholder_wealthsimple::slips;

fn slip(name: &str) -> (String, Value) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/replies/slips");
    let text = std::fs::read_to_string(dir.join(format!("{name}.txt"))).unwrap();
    let want = json::parse(&std::fs::read_to_string(dir.join(format!("{name}.json"))).unwrap()).unwrap();
    (text, want)
}

/// Each sale's row as the slip writes it, a name that wraps joined, a box 20 left
/// blank read as no cost, and the slip held to its own totals: the three layouts
/// the person's T5008s took.
#[test]
fn a_t5008_is_its_sales_as_its_rows_state_them() {
    for name in ["t5008-made-up-a", "t5008-made-up-b", "t5008-made-up-c", "t5008-made-up-d"] {
        let (t, want) = slip(name);
        let want = Node::root(&want);
        let got = slips::t5008(&t).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(got.year as i64, want.int("year").unwrap(), "{name}");
        let rows = want.list("rows").unwrap();
        assert_eq!(got.sales.len(), rows.len(), "{name}");
        for (s, w) in got.sales.iter().zip(&rows) {
            assert_eq!(s.day, w.day("day").unwrap(), "{name}");
            assert_eq!(s.type_code, w.text("type").unwrap(), "{name}");
            assert_eq!(s.quantity, w.dec_text("quantity").unwrap(), "{name}");
            assert_eq!(s.security, w.text("security").unwrap(), "{name}");
            assert_eq!(s.cost.as_ref().map(|c| c.amount), w.opt_dec_text("cost").unwrap(), "{name}");
            assert_eq!(s.proceeds.amount, w.dec_text("proceeds").unwrap(), "{name}");
            assert_eq!(s.proceeds.currency.to_string(), "CAD", "{name}");
        }
    }
}

/// A slip whose rows do not come to its own totals is refused, never read in part.
#[test]
fn a_t5008_whose_rows_do_not_come_to_its_totals_is_refused() {
    let (t, _) = slip("t5008-made-up-c");
    // one sale's proceeds raised by a thousand dollars: the slip's totals no longer hold
    let at = t.find(" CAD").and_then(|i| t[i + 4..].find('$').map(|j| i + 4 + j)).unwrap();
    let altered = format!("{}$1{}", &t[..at], &t[at + 1..]);
    let e = slips::t5008(&altered).unwrap_err();
    assert!(e.contains("proceeds come to"), "{e}");
    assert!(Dec::parse("1").is_ok());
}

/// A consolidated T3's boxes as its slip states them, and, where it carries its
/// breakdown, each fund's distributions by box with its cash and non-cash parts:
/// the two layouts the person's T3s took (the 2023 slip alone; from 2024 with
/// the breakdown), held to the slip's own arithmetic.
#[test]
fn a_t3_is_its_boxes_and_each_fund_s_distributions() {
    for name in ["t3-made-up-a", "t3-made-up-b", "t3-made-up-c"] {
        let (t, want) = slip(name);
        let want = Node::root(&want);
        let got = slips::t3(&t).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(got.year as i64, want.int("year").unwrap(), "{name}");
        let boxes = want.obj("boxes").unwrap();
        let keys = boxes.keys().unwrap();
        assert_eq!(got.boxes.len(), keys.len(), "{name}: {:?}", got.boxes);
        for k in keys {
            assert_eq!(got.boxes.get(&k.parse::<u16>().unwrap()).copied(), Some(boxes.dec_text(k).unwrap()), "{name}: box {k}");
        }
        let funds = want.list("funds").unwrap();
        assert_eq!(got.funds.len(), funds.len(), "{name}");
        for (f, w) in got.funds.iter().zip(&funds) {
            assert_eq!(f.name, w.text("name").unwrap(), "{name}");
            let rows = w.list("rows").unwrap();
            assert_eq!(f.distributions.len(), rows.len(), "{name}");
            for (d, r) in f.distributions.iter().zip(&rows) {
                assert_eq!((d.record, d.paid), (r.day("record").unwrap(), r.day("paid").unwrap()), "{name}");
                let rb = r.obj("boxes").unwrap();
                for k in rb.keys().unwrap() {
                    assert_eq!(d.boxes.get(&k.parse::<u16>().unwrap()).copied(), Some(rb.dec_text(k).unwrap()), "{name}: {} box {k}", d.record);
                }
                assert_eq!((d.cash, d.non_cash), (r.dec_text("cash").unwrap(), r.dec_text("non_cash").unwrap()), "{name}");
            }
        }
    }
}

/// A T3 whose fund's rows do not come to its totals row, or whose funds do not
/// come to the slip's own boxes, is refused, never read in part.
#[test]
fn a_t3_that_does_not_add_up_is_refused() {
    let (t, _) = slip("t3-made-up-a");
    // a fund's totals row changed in its first amount
    let at = t.find("Fund Totals").unwrap();
    let dollar = at + t[at..].find('$').unwrap();
    let altered = format!("{}$1{}", &t[..dollar], &t[dollar + 1..]);
    let e = slips::t3(&altered).unwrap_err();
    assert!(e.contains("totals row says"), "{e}");
    // the slip's box 21 changed: the funds no longer come to it
    let b21 = t.find("  21\n").map(|i| i + 5).unwrap();
    let next = b21 + t[b21..].find('$').unwrap();
    let altered = format!("{}$9{}", &t[..next], &t[next + 1..]);
    let e = slips::t3(&altered).unwrap_err();
    assert!(e.contains("the slip's box 21"), "{e}");
}

/// A T5 statement's rows, each security's name put together from the margin
/// lines around its row and its amounts by column, return of capital included:
/// the three layouts the person's T5s took, held to their totals rows.
#[test]
fn a_t5_is_its_rows_by_security() {
    for name in ["t5-made-up-a", "t5-made-up-b", "t5-made-up-c"] {
        let (t, want) = slip(name);
        let want = Node::root(&want);
        let got = slips::t5(&t).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(got.year as i64, want.int("year").unwrap(), "{name}");
        let rows = want.list("rows").unwrap();
        assert_eq!(got.rows.len(), rows.len(), "{name}");
        for (r, w) in got.rows.iter().zip(&rows) {
            assert_eq!(r.security, w.text("security").unwrap(), "{name}");
            let a = w.obj("amounts").unwrap();
            let keys = a.keys().unwrap();
            assert_eq!(r.amounts.len(), keys.len(), "{name}");
            for k in keys {
                let c = if k == "roc" { slips::T5Column::ReturnOfCapital } else { slips::T5Column::Box(k.parse().unwrap()) };
                assert_eq!(r.amounts.get(&c).copied(), Some(a.dec_text(k).unwrap()), "{name}: {k}");
            }
        }
    }
}

/// A T5 whose rows do not come to its totals row is refused.
#[test]
fn a_t5_whose_rows_do_not_come_to_its_totals_is_refused() {
    let (t, _) = slip("t5-made-up-a");
    let at = t.find("Totals").unwrap();
    let dollar = at + t[at..].find('$').unwrap();
    let altered = format!("{}$1{}", &t[..dollar], &t[dollar + 1..]);
    let e = slips::t5(&altered).unwrap_err();
    assert!(e.contains("totals row says"), "{e}");
}

/// The made-up recipient each made-up slip is issued to, printed on it wherever a
/// real slip prints the recipient: under each label and atop a statement's pages
/// (a joint account's slip names a second person after the first).
const RECIPIENT: [&str; 3] = ["Ally Samplesworth", "1234 56 ST", "Sampletown AB T0X0X0"];
const SECOND: &str = "Robin Casey Samplesworth";

/// Every word of nine digits on a slip: the made-up recipient identification numbers.
fn nine_digit_words(t: &str) -> Vec<String> {
    t.split_whitespace().filter(|w| w.len() == 9 && w.chars().all(|c| c.is_ascii_digit())).map(str::to_string).collect()
}

const ALL: [&str; 10] = ["t3-made-up-a", "t3-made-up-b", "t3-made-up-c", "t5-made-up-a", "t5-made-up-b", "t5-made-up-c", "t5008-made-up-a", "t5008-made-up-b", "t5008-made-up-c", "t5008-made-up-d"];

fn kind(name: &str) -> bagholder_wealthsimple::documents::SlipKind {
    bagholder_wealthsimple::documents::SlipKind::of(name.split('-').next().unwrap()).unwrap()
}

/// What a slip keeps holds no social insurance number, in any of the three ways
/// one is written, and no name or address of the recipient's, under its labels or
/// atop a statement's page; every line keeps its width, and each reader reads it
/// exactly as it read the slip whole.
#[test]
fn a_slip_kept_holds_no_sin_name_or_address_and_reads_as_before() {
    for name in ALL {
        let (t, _) = slip(name);
        let sins = nine_digit_words(&t);
        assert!(!sins.is_empty(), "{name}: the made-up slip carries its made-up numbers");
        // the first written with spaces, the second with hyphens, the rest as they are
        let mut written = t.clone();
        for (i, s) in sins.iter().take(2).enumerate() {
            let sep = if i == 0 { " " } else { "-" };
            written = written.replacen(s.as_str(), &format!("{}{sep}{}{sep}{}", &s[..3], &s[3..6], &s[6..]), 1);
        }
        assert!(RECIPIENT.iter().all(|r| written.contains(r)), "{name}: the made-up recipient is printed on it");
        let kept = slips::redact(&written).unwrap_or_else(|e| panic!("{name}: {e}"));
        for s in &sins {
            for form in [s.clone(), format!("{} {} {}", &s[..3], &s[3..6], &s[6..]), format!("{}-{}-{}", &s[..3], &s[3..6], &s[6..])] {
                assert!(!kept.contains(&form), "{name}: {form} kept");
            }
        }
        for r in RECIPIENT.into_iter().chain([SECOND]) {
            assert!(!kept.contains(r), "{name}: {r} kept");
        }
        assert_eq!(kept.lines().map(|l| l.chars().count()).collect::<Vec<_>>(), written.lines().map(|l| l.chars().count()).collect::<Vec<_>>(), "{name}: every line keeps its width");
        assert_eq!(slips::read(kind(name), &kept), slips::read(kind(name), &t), "{name}");
    }
}

/// A slip with no label heading its recipient, or a label with no name and
/// address under it, is refused: it is never kept with them in it.
#[test]
fn a_slip_whose_recipient_cannot_be_found_is_refused() {
    let (t, _) = slip("t5008-made-up-b");
    let unlabelled = t.replace("Name and Address of", "Name and Place of");
    assert!(slips::redact(&unlabelled).unwrap_err().contains("no recipient's name and address found"));
    let alone = "Recipient's name and address\n\n\n                                                                Year\n";
    assert!(slips::redact(alone).unwrap_err().contains("no name and address under the label"));
}
