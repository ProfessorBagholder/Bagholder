//! The reply reader (`docs/plans/stage-3a-sources.md`, acceptance "The reply
//! reader"): exact, strict, and saying when a reply's shape moved.

use bagholder_core::Dec;
use bagholder_sources::reply::{parse, recorded, shape, shape_change, Node, Presence};

fn dec(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

#[test]
fn a_decimal_is_read_to_its_last_written_digit() {
    let v = parse(r#"{"price": 224.255004882812, "v": "1.36650", "tiny": 1e-9}"#).unwrap();
    let r = Node::root(&v);
    assert_eq!(r.dec("price").unwrap(), dec("224.255004882812"));
    assert_eq!(r.dec("price").unwrap().to_text(), "224.255004882812");
    assert_eq!(r.dec_text("v").unwrap(), dec("1.3665"));
    assert_eq!(r.dec("tiny").unwrap(), dec("0.000000001"));
}

#[test]
fn a_number_with_more_digits_than_a_decimal_holds_is_a_mismatch() {
    // 29 significant digits, beyond the 96 bits a decimal holds
    let v = parse(r#"{"amount": 99999999999999999999999999999}"#).unwrap();
    let e = Node::root(&v).dec("amount").unwrap_err();
    assert_eq!(e.path, "amount");
    assert!(e.why.contains("more digits than a decimal holds"), "{e}");
    let v = parse(r#"{"amount": "0.12345678901234567890123456789"}"#).unwrap();
    assert!(Node::root(&v).dec_text("amount").is_err());
}

#[test]
fn a_required_field_absent_null_or_mistyped_is_a_mismatch_naming_its_path() {
    let v = parse(r#"{"dividends": {"dividends": [{"amount": 1}, {"amount": 2}, {"amount": 3}, {"amount": null}, {"amount": "x"}, {}]}}"#).unwrap();
    let r = Node::root(&v);
    let rows = r.obj("dividends").unwrap().list("dividends").unwrap();
    assert_eq!(rows[0].dec("amount").unwrap(), dec("1"));
    let null = rows[3].dec("amount").unwrap_err();
    assert_eq!(null.to_string(), "dividends.dividends[3].amount: expected a decimal, found null");
    let text = rows[4].dec("amount").unwrap_err();
    assert_eq!(text.to_string(), "dividends.dividends[4].amount: expected a decimal, found a string");
    let absent = rows[5].dec("amount").unwrap_err();
    assert_eq!(absent.to_string(), "dividends.dividends[5].amount: absent");
    assert_eq!(r.list("nope").unwrap_err().to_string(), "nope: absent");
    assert_eq!(r.text("dividends").unwrap_err().to_string(), "dividends: expected text, found an object");
}

#[test]
fn a_day_is_iso_and_real() {
    let v = parse(r#"{"a": "2026-09-23", "b": "2026-02-30", "c": "2026-9-23", "d": "20260923", "e": "2026-09-23T00:00:00"}"#).unwrap();
    let r = Node::root(&v);
    assert_eq!(r.day("a").unwrap().to_string(), "2026-09-23");
    for bad in ["b", "c", "d", "e"] {
        assert!(r.day(bad).is_err(), "{bad}");
    }
}

#[test]
fn an_optional_fields_null_is_none_stated_and_its_absent_key_a_mismatch() {
    let v = parse(r#"{"recordDate": null, "payDate": "2026-10-01"}"#).unwrap();
    let r = Node::root(&v);
    assert_eq!(r.opt_day("recordDate").unwrap(), None);
    assert_eq!(r.opt_day("payDate").unwrap().unwrap().to_string(), "2026-10-01");
    assert_eq!(r.opt_day("exDate").unwrap_err().to_string(), "exDate: absent");
    // a required read of the same null is still a mismatch
    assert!(r.day("recordDate").is_err());
}

#[test]
fn a_field_not_read_is_ignored() {
    let v = parse(r#"{"d": "2026-09-23", "extra": {"deep": [1, 2, {"x": null}]}, "noise": "?"}"#).unwrap();
    assert_eq!(Node::root(&v).day("d").unwrap().to_string(), "2026-09-23");
}

#[test]
fn a_path_gone_and_a_path_new_are_each_a_shape_change() {
    let a = parse(r#"{"observations": [{"d": "2026-09-22", "FXUSDCAD": {"v": "1.38"}}], "seriesDetail": {"FXUSDCAD": {"label": "USD/CAD"}}}"#).unwrap();
    let b = parse(r#"{"observations": [], "seriesDetail": {"FXUSDCAD": {"label": "USD/CAD"}}}"#).unwrap();
    let recorded = recorded([shape(&a), shape(&b)]);
    // indices folded: two replies with different numbers of rows have one shape
    let more = parse(r#"{"observations": [{"d": "2026-09-22", "FXUSDCAD": {"v": "1.38"}}, {"d": "2026-09-23", "FXUSDCAD": {"v": "1.39"}}], "seriesDetail": {"FXUSDCAD": {"label": "USD/CAD"}}}"#).unwrap();
    assert_eq!(shape_change(&recorded, &shape(&more)), None);
    // an empty list does not lose its items' paths
    assert_eq!(shape_change(&recorded, &shape(&b)), None);

    let gone = parse(r#"{"observations": [{"d": "2026-09-22", "FXUSDCAD": {}}], "seriesDetail": {"FXUSDCAD": {"label": "USD/CAD"}}}"#).unwrap();
    let c = shape_change(&recorded, &shape(&gone)).unwrap();
    assert_eq!(c.gone, vec!["observations[].FXUSDCAD.v"]);
    assert!(c.new.is_empty());

    let new = parse(r#"{"observations": [{"d": "2026-09-22", "FXUSDCAD": {"v": "1.38", "status": "final"}}], "seriesDetail": {"FXUSDCAD": {"label": "USD/CAD"}}}"#).unwrap();
    let c = shape_change(&recorded, &shape(&new)).unwrap();
    assert_eq!(c.new, vec!["observations[].FXUSDCAD.status"]);
    assert!(c.gone.is_empty());

    // a whole top-level field gone
    let top = parse(r#"{"observations": []}"#).unwrap();
    assert_eq!(shape_change(&recorded, &shape(&top)).unwrap().gone, vec!["seriesDetail"]);
}

#[test]
fn a_part_only_some_recorded_replies_carry_is_not_gone_when_absent() {
    // Yahoo's chart carries `events` only when the span holds a dividend or split
    let with = parse(r#"{"result": [{"meta": {"currency": "USD"}, "events": {"dividends": {"1": {"amount": 0.1}}}}]}"#).unwrap();
    let without = parse(r#"{"result": [{"meta": {"currency": "USD"}}]}"#).unwrap();
    let r = recorded([shape(&with), shape(&without)]);
    assert_eq!(r["result[].events"], Presence::Sometimes);
    assert_eq!(r["result[].events.dividends"], Presence::Always);
    assert_eq!(r["result[].meta.currency"], Presence::Always);
    assert_eq!(shape_change(&r, &shape(&without)), None);
    // a path every reply carried is still gone when it is absent
    let no_meta = parse(r#"{"result": [{"events": {"dividends": {}}}]}"#).unwrap();
    let c = shape_change(&r, &shape(&no_meta)).unwrap();
    assert_eq!(c.gone, vec!["result[].events.dividends.1", "result[].meta"]);
    // the committed form keeps the difference
    let text = bagholder_sources::ask::shape_text(&r);
    assert!(text.contains("result[].events ?\n") && text.contains("result[].meta\n"), "{text}");
    assert_eq!(bagholder_sources::ask::recorded_shape(&text), r);
}

#[test]
fn a_reply_that_is_not_json_is_a_mismatch_at_its_root() {
    let e = parse("<!DOCTYPE html><html>").unwrap_err();
    assert_eq!(e.path, "");
    assert!(e.to_string().starts_with("the reply: not JSON"), "{e}");
}
