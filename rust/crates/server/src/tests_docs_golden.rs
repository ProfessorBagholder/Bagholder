//! Golden ops (stage 5d7c): the exact patch operations pinned for a realistic
//! change to each document the stream carries -- the header status, the
//! orders panel, the bell, a ticket's quote, a listing's disclosures, the
//! fear gauge and the short-interest feed. `docs`, `events`, `status`,
//! `notify` and the ticket's quote are all typed, so each before/after pair
//! is built as the real struct and compared with `patch::typed`
//! (`Diff::diff`), the same call `Feed::step` makes.
//! Every expected op below is unchanged from the untyped differ this test
//! pinned before the conversion: a difference here would have been
//! stop-and-report material (see `d7c-design.md`).

use serde_json::{json, Value};

use bagholder_model::patch::{self, Diff};
use bagholder_store::feeds::{
    FiledDocument, Filing, Gauge, GaugePoint, GaugeReading, Regulator, ShortMarket, Shorts, StoredGauge, StoredShorts,
};

use crate::docs::HistoryPending;
use crate::feeds::{FearDoc, ShortsFeedRow, ShortsFeed};
use crate::notify::{NotifySettings, NotifyStatus};
use crate::orders::{OrderCard, OrdersDoc};
use crate::status::Status;

fn tdiff<T: Diff>(old: &T, new: &T) -> Vec<Value> {
    patch::typed(old, new)
}

// --- status: one field changing is one `set` -----------------------------

/// The header's status, with everything but `syncStep` fixed.
fn status_doc(sync_step: &str) -> Status {
    Status {
        ok: true,
        connected: true,
        email: "person@example.com".into(),
        last_sync: "2026-09-15T14:00:00Z".into(),
        activity_count: 40,
        account_count: 2,
        capturing: false,
        syncing: true,
        listings_filling: false,
        sync_step: sync_step.into(),
        error: String::new(),
        summary_ready: true,
        protocol: "3".into(),
        started_at: "2026-09-15T13:00:00Z".into(),
        version: "1.46.2".into(),
        latest_version: "1.46.2".into(),
        update_available: false,
        update_url: String::new(),
        can_update: true,
        update_by: "app".into(),
        login_view: false,
        orders_live: true,
        open_orders: 0,
        updating: String::new(),
        update_error: String::new(),
        notify: NotifyStatus { settings: NotifySettings { fills: true, ..Default::default() }, native: String::new(), unread: 0 },
    }
}

#[test]
fn test_status_field_change_is_one_set() {
    let (before, after) = (status_doc(""), status_doc("Reading activity"));
    let ops = patch::typed_under(&["status"], &before, &after);
    assert_eq!(ops, vec![json!(["set", ["status", "syncStep"], "Reading activity"])]);
}

// --- orders: a row added, changed, then removed ---------------------------

fn dec(s: &str) -> crate::wire::Dec {
    crate::wire::Dec(bagholder_core::Dec::parse(s).unwrap())
}

fn order(id: &str, state: &str) -> OrderCard {
    OrderCard {
        id: id.into(),
        account: "acct-1".into(),
        exchange: "TSX-V".into(),
        symbol: "QNC".into(),
        side: "buy".into(),
        kind: "limit".into(),
        tif: Some("day".into()),
        quantity: dec("5"),
        limit_price: Some(dec("1.75")),
        stop_price: None,
        state: state.into(),
        filled: dec(if state == "filled" { "5" } else { "0" }),
        average: None,
        why: None,
        value: Some(crate::wire::Fig::Stated(dec("8.75"))),
        approx: false,
        tab: if state == "filled" { "filled" } else { "pending" }.into(),
        at: "2026-09-15T14:00:00Z".into(),
        live: state == "pending",
        editable: state == "pending",
        legs: vec![],
    }
}

fn orders_doc(orders: Vec<OrderCard>) -> OrdersDoc {
    OrdersDoc { ok: true, live: true, refreshed_at: Some("2026-09-15T14:00:00Z".into()), orders, brackets: vec![], error: None }
}

#[test]
fn test_an_order_row_added_changed_and_removed() {
    let empty = orders_doc(vec![]);
    let one_pending = orders_doc(vec![order("o1", "pending")]);
    let added = tdiff(&empty, &one_pending);
    assert_eq!(added.len(), 1);
    assert_eq!(added[0][0], json!("rows"));
    assert_eq!(added[0][1], json!(["orders"]));
    assert_eq!(added[0][2], json!("id"));
    assert_eq!(added[0][3], json!(["o1"]));
    assert_eq!(added[0][4].as_object().unwrap().len(), 1, "the new row, whole, keyed by its id");

    let one_filled = orders_doc(vec![order("o1", "filled")]);
    let changed = tdiff(&one_pending, &one_filled);
    let fields: Vec<Value> = changed.iter().map(|op| op[1][2].clone()).collect();
    assert_eq!(fields, vec![json!("state"), json!("filled"), json!("tab"), json!("live"), json!("editable")], "the fields of one row that moved, not the list again: {changed:?}");
    assert!(changed.iter().all(|op| op[0] == json!("set") && op[1][1] == json!({"k": "id", "v": "o1"})));

    // no row key can be named on an empty array, so the list going empty is a
    // whole-list `set`, not a `rows` op naming no rows -- true of the typed
    // differ exactly as it was of the untyped one
    let none = orders_doc(vec![]);
    let removed = tdiff(&one_filled, &none);
    assert_eq!(removed, vec![json!(["set", ["orders"], []])]);
}

// --- notifications: a row inserted, then marked read ----------------------

use bagholder_store::feeds::Notification;

fn notice_row(id: i64, read_at: &str) -> Notification {
    Notification { id, at: "2026-09-15T14:00:00Z".into(), kind: "fills".into(), key: "order:1:filled".into(), title: "Order filled · QNC".into(), body: "Bought 5 at 1.75".into(), extra: Default::default(), seen_at: String::new(), read_at: read_at.into() }
}

fn bell(rows: Vec<Notification>, unread: i64) -> crate::notify::NotificationsDoc {
    crate::notify::NotificationsDoc { rows, unread }
}

#[test]
fn test_a_notification_inserted_then_marked_read() {
    let inserted = tdiff(&bell(vec![], 0), &bell(vec![notice_row(1, "")], 1));
    assert_eq!(inserted, vec![json!(["rows", ["rows"], "id", ["1"], {"1": serde_json::to_value(notice_row(1, "")).unwrap()}]), json!(["set", ["unread"], 1])]);

    let read = tdiff(&bell(vec![notice_row(1, "")], 1), &bell(vec![notice_row(1, "2026-09-15T14:05:00Z")], 0));
    assert_eq!(read, vec![json!(["set", ["rows", {"k": "id", "v": "1"}, "readAt"], "2026-09-15T14:05:00Z"]), json!(["set", ["unread"], 0])]);
}

// --- a ticket's quote: a tick -----------------------------------------------

fn quote(bid: f64, ask: f64, last: f64) -> crate::orders::TicketQuote {
    crate::orders::TicketQuote::Ok(crate::orders::TicketQuoteOk {
        ok: true,
        quote: crate::orders::TicketQuoteDetail {
            security_id: "sec-1".into(),
            symbol: "QNC".into(),
            name: "Quantum eMotion".into(),
            exchange: "TSX-V".into(),
            currency: "CAD".into(),
            last: Some(last),
            bid: Some(bid),
            ask: Some(ask),
            mid: Some((bid + ask) / 2.0),
            ..Default::default()
        },
        order_types: vec!["MARKET".into(), "LIMIT".into(), "STOP".into(), "STOP_LIMIT".into()],
        margin_rate: None,
        accounts: vec![],
        account: None,
        buying_power: Some(500.0),
        cash: Some(500.0),
        margin_available: None,
        live: true,
    })
}

#[test]
fn test_a_quote_tick_is_the_moved_fields_alone() {
    let ops = tdiff(&quote(1.74, 1.76, 1.75), &quote(1.75, 1.77, 1.76));
    assert_eq!(ops, vec![
        json!(["set", ["quote", "last"], 1.76]),
        json!(["set", ["quote", "bid"], 1.75]),
        json!(["set", ["quote", "ask"], 1.77]),
        json!(["set", ["quote", "mid"], 1.76]),
    ]);
}

// --- a listing's disclosures: a filing row arriving ------------------------

fn filing(id: &str, form: &str, date: &str) -> Filing {
    Filing {
        doc: FiledDocument { id: id.into(), source: Regulator::Sec, category: String::new(), profile_no: String::new(), issuer: String::new(), form: form.into(), title: String::new(), date: date.into(), date_text: String::new(), size: String::new(), url: String::new() },
        subject: "Insider report".into(),
        summary: String::new(),
        enriched_at: String::new(),
        enrich_version: None,
        enrich_final: false,
        enrich_reads: 0,
        fetched_at: String::new(),
    }
}

fn filings_doc(filings: Vec<Filing>) -> crate::feeds::FilingsDoc {
    crate::feeds::FilingsDoc {
        ok: true,
        symbol: "NBIS".into(),
        available: true,
        sources: Default::default(),
        categories: vec!["insider".into()],
        fetched_at: "2026-09-15T14:00:00Z".into(),
        ever_read: true,
        summary_status: "ready".into(),
        reading: vec![],
        filings,
    }
}

#[test]
fn test_a_filing_row_arriving_is_one_row_inserted() {
    let ops = tdiff(&filings_doc(vec![filing("sec:1", "4", "2026-09-14")]), &filings_doc(vec![filing("sec:2", "4", "2026-09-15"), filing("sec:1", "4", "2026-09-14")]));
    assert_eq!(ops.len(), 1);
    assert_eq!((ops[0][0].clone(), ops[0][1].clone(), ops[0][2].clone(), ops[0][3].clone()), (json!("rows"), json!(["filings"]), json!("id"), json!(["sec:2", "sec:1"])));
    assert_eq!(ops[0][4].as_object().unwrap().len(), 1, "only the new row is carried; the one already shown is not");
}

// --- the fear gauge: a reading change --------------------------------------

fn fear_doc(score: f64) -> FearDoc {
    FearDoc {
        ok: true,
        gauge: Some(StoredGauge {
            gauge: Gauge { index: "fear-greed".into(), source: "cnn".into(), score, rating: "greed".into(), as_of: "2026-09-15".into(), previous: vec![GaugeReading { label: "yesterday".into(), score: 70.0, rating: "greed".into() }], parts: vec![], series: vec![GaugePoint { date: "2026-09-14".into(), score: 70.0 }] },
            fetched_at: "2026-09-15T14:00:00Z".into(),
            read_version: 1,
        }),
        reading: false,
    }
}

#[test]
fn test_a_fear_reading_change_is_the_score_and_the_as_of() {
    let ops = tdiff(&fear_doc(71.0), &fear_doc(74.0));
    assert_eq!(ops, vec![json!(["set", ["gauge", "score"], 74.0])]);
}

// --- short selling: a listing's figure changing ----------------------------

fn shorts_row(shares: f64) -> ShortsFeedRow {
    ShortsFeedRow {
        shorts: StoredShorts {
            shorts: Shorts { symbol: "QNC".into(), exchange: "TSX-V".into(), market: ShortMarket::Ca, name: "Quantum eMotion".into(), as_of: "2026-09-15".into(), shares: Some(shares), previous: Some(shares - 1000.0), previous_of: "2026-08-31".into(), change: Some(1000.0), float: None, of_float: None, average_volume: None, days_to_cover: None, volume_of: String::new(), volume_span: None, short_volume: None, total_volume: None, volume_pct: None, series: None },
            fetched_at: "2026-09-15T14:00:00Z".into(),
            read_version: 1,
        },
        position_id: Some("p1".into()),
        held: true,
        watched: false,
    }
}

#[test]
fn test_a_shorts_figure_change_is_the_row_that_moved() {
    let (a, b) = (ShortsFeed { ok: true, rows: vec![shorts_row(120_000.0)], reading: false }, ShortsFeed { ok: true, rows: vec![shorts_row(135_000.0)], reading: false });
    let ops = tdiff(&a, &b);
    assert_eq!(ops, vec![
        json!(["set", ["rows", {"k": "symbol", "v": "QNC"}, "shares"], 135000.0]),
        json!(["set", ["rows", {"k": "symbol", "v": "QNC"}, "previous"], 134000.0]),
    ]);
}

// --- a chart's history: pending flips -------------------------------------

#[test]
fn test_history_pending_flip_is_one_set() {
    let ops = tdiff(&HistoryPending { pending: true }, &HistoryPending { pending: false });
    assert_eq!(ops, vec![json!(["set", ["pending"], false])]);
}
