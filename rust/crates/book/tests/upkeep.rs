//! The earlier app's rows removed from the book, once
//! (`docs/plans/remove-the-earlier-app.md`, Part 2): nothing that counts moves,
//! nothing is removed while one still counts, and what the person made on the
//! book is the same after, field for field.

mod common;

use std::collections::BTreeMap;

use bagholder_book::orders::{BracketPlace, OrderRequest};
use bagholder_book::upkeep::{EarlierAppRows, EARLIER_APP_ROWS_REMOVED};
use bagholder_book::watched::{ListingDraft, ListingName};
use bagholder_book::BookError;
use bagholder_core::bracket::{BracketEvent, StopLeg};
use bagholder_core::instrument::{InstrumentKind, RefScheme, Reference};
use bagholder_core::journal::{Anchor, Grade, JournalEntry, JournalSubject};
use bagholder_core::order::{Asker, OrderEvent, OrderKind, OrderRole, Side, TimeInForce};
use bagholder_core::record::RecordState;
use bagholder_core::{Currency, Dec, RecordId};
use common::*;
use serde_json::{json, Value};

/// The mapping the earlier app's rows are stored under here; the source is
/// renamed to the earlier app's by `as_earlier_app`, as the first start's copy
/// named it.
fn earlier() -> Spelled {
    Spelled { source: "earlier-copy", version: 1 }
}

/// Mark the records stored under `earlier()` as the earlier app's.
fn as_earlier_app(f: &Fixture) {
    f.book.conn_for_tests().execute("UPDATE source_records SET source = 'bagholder-import' WHERE source = 'earlier-copy'", []).unwrap();
}

fn earlier_rows(f: &Fixture) -> i64 {
    f.book.conn_for_tests().query_row("SELECT COUNT(*) FROM source_records WHERE source = 'bagholder-import'", [], |r| r.get(0)).unwrap()
}

/// Units held by (account, instrument), and cash by (account, currency), from
/// the live transactions.
fn counted(f: &Fixture) -> (BTreeMap<String, Dec>, BTreeMap<String, Dec>) {
    let txs = f.book.transactions().unwrap();
    let units = bagholder_book::records::positions(&txs).unwrap().into_iter().map(|((a, i), q)| (format!("{a}/{i}"), q)).collect();
    let mut cash: BTreeMap<String, Dec> = BTreeMap::new();
    for t in &txs {
        if let Some(c) = t.cash {
            let e = cash.entry(format!("{}/{}", t.account, c.currency.as_str())).or_insert(Dec::ZERO);
            *e = e.checked_add(c.amount).unwrap();
        }
    }
    (units, cash)
}

/// The rows of `table` in a fixed order, the columns named only.
fn rows(f: &Fixture, table: &str, cols: &str) -> Vec<String> {
    let c = f.book.conn_for_tests();
    let mut stmt = c.prepare(&format!("SELECT {cols} FROM {table} ORDER BY 1, 2")).unwrap();
    let n = stmt.column_count();
    let mut rows = stmt.query([]).unwrap();
    let mut out = Vec::new();
    while let Some(r) = rows.next().unwrap() {
        let cell = |i| match r.get_ref(i).unwrap() {
            rusqlite::types::ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
            other => format!("{other:?}"),
        };
        out.push((0..n).map(cell).collect::<Vec<_>>().join(" | "));
    }
    out
}

/// What the person made on the book: the trades and their anchors, the journal,
/// the groups, the orders and brackets with their logs, the notifications and
/// what was told, the watchlist and the tiles, and the settings, each field but
/// the earlier app's keys and the step's own record.
fn the_persons(f: &Fixture) -> Vec<(String, Vec<String>)> {
    [
        ("trades", "id, anchor_record, anchor_leg, anchor_instrument, orphaned_reason, created_at"),
        ("journal", "*"),
        ("journal_tags", "*"),
        ("trade_groups", "id, locked, created_at"),
        ("trade_group_members", "*"),
        ("orders", "*"),
        ("order_events", "*"),
        ("brackets", "*"),
        ("bracket_events", "*"),
        ("notifications", "*"),
        ("told", "*"),
        ("watched", "*"),
        ("tiles", "*"),
        ("listings_named", "*"),
        ("settings WHERE key <> 'upkeep.earlier-app-rows-removed'", "*"),
    ]
    .into_iter()
    .map(|(t, c)| (t.to_string(), rows(f, t, c)))
    .collect()
}

/// A generator of made-up figures: the same seed, the same book.
struct Gen(u64);

impl Gen {
    fn next(&mut self, below: u64) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) % below
    }
}

/// A book of `n` of the earlier app's rows over a few accounts, shares and
/// currencies, each superseded by the broker's own row (sometimes split in two)
/// or removed by its source, with rows only the broker states beside them.
fn generated(seed: u64, n: usize) -> Fixture {
    let f = Fixture::new();
    let mut g = Gen(seed);
    let accounts = ["acct-1", "acct-2", "acct-3"];
    for a in accounts {
        f.account(&[a]);
    }
    let shares = [("CA0000000001", "QNC"), ("CA0000000002", "ZQR"), ("US0000000003", "XMPL"), ("CA0000000004", "BHLD")];
    let currencies = ["CAD", "USD"];
    let broker = Spelled::v(1);
    let mut kept = Vec::new();
    for i in 0..n {
        let account = accounts[g.next(3) as usize];
        let (isin, symbol) = shares[g.next(4) as usize];
        let currency = currencies[g.next(2) as usize];
        let qty = 1 + g.next(500);
        let cash = format!("-{}.{:02}", 1 + g.next(9000), g.next(100));
        let when = format!("2025-{:02}-{:02}T15:{:02}:00Z", 1 + g.next(12), 1 + g.next(28), g.next(60));
        let mut leg = buy(account, share(isin, symbol), &qty.to_string(), &cash, &when);
        leg["currency"] = json!(currency);
        let r = f.store(&earlier(), &format!("old-{i}"), &legs(vec![leg.clone()]));
        if g.next(5) == 0 {
            f.book.mark_removed(r.record, t0()).unwrap();
        } else {
            kept.push((i, r.record, leg));
        }
    }
    for (i, record, leg) in kept {
        if g.next(3) == 0 {
            // the broker states it as two fills
            let q: u64 = leg["quantity"].as_str().unwrap().parse().unwrap();
            if q > 1 {
                let mut a = leg.clone();
                let mut b = leg.clone();
                a["quantity"] = json!((q / 2).to_string());
                b["quantity"] = json!((q - q / 2).to_string());
                b["cash"] = json!("0");
                let first = f.store(&broker, &format!("ws-{i}-a"), &legs(vec![a]));
                let second = f.store(&broker, &format!("ws-{i}-b"), &legs(vec![b]));
                f.book.supersede(&[record], &[first.record, second.record], "the broker's own rows", t0()).unwrap();
                continue;
            }
        }
        let text = legs(vec![leg]).to_string();
        f.book.store_superseding(&broker, &f.incoming(&format!("ws-{i}"), &text), &[record], "the broker's own row", t0()).unwrap();
    }
    for i in 0..g.next(6) {
        let mut leg = buy(accounts[g.next(3) as usize], share(shares[0].0, shares[0].1), "3", "-30", "2026-02-02T15:00:00Z");
        leg["currency"] = json!("CAD");
        f.store(&broker, &format!("ws-only-{i}"), &legs(vec![leg]));
    }
    as_earlier_app(&f);
    f
}

#[test]
fn every_account_holds_the_same_units_and_cash_after_the_step() {
    for seed in 0..24 {
        let f = generated(seed, 5 + (seed as usize % 7) * 4);
        let before = counted(&f);
        assert!(earlier_rows(&f) > 0);
        let done = f.book.remove_earlier_app_rows(t0()).unwrap();
        assert!(matches!(done, EarlierAppRows::Removed { records, .. } if records > 0), "seed {seed}: {done:?}");
        assert_eq!(counted(&f), before, "seed {seed}: units and cash of every account");
        assert_eq!(earlier_rows(&f), 0, "seed {seed}");
        for table in ["record_revisions", "record_refs", "record_problems", "link_records", "transactions", "instrument_sightings"] {
            let orphans: i64 = f.book.conn_for_tests().query_row(&format!("SELECT COUNT(*) FROM {table} WHERE record_id NOT IN (SELECT id FROM source_records)"), [], |r| r.get(0)).unwrap();
            assert_eq!(orphans, 0, "seed {seed}: {table} names no record that is gone");
        }
        let one_sided: i64 = f
            .book
            .conn_for_tests()
            .query_row("SELECT COUNT(*) FROM links l WHERE NOT EXISTS (SELECT 1 FROM link_records r WHERE r.link_id = l.id AND r.side = 'from')", [], |r| r.get(0))
            .unwrap();
        assert_eq!(one_sided, 0, "seed {seed}: no link is left superseding nothing");
        f.book.conn_for_tests().execute_batch("PRAGMA foreign_key_check").unwrap();
        let broken: i64 = f.book.conn_for_tests().query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r.get(0)).unwrap();
        assert_eq!(broken, 0, "seed {seed}");
    }
}

#[test]
fn nothing_is_removed_while_one_of_the_rows_still_counts() {
    let f = generated(7, 9);
    // one more of the earlier app's rows, which no broker row replaced
    let mut leg = buy("acct-1", share("CA0000000009", "LVE"), "4", "-40", "2025-06-16T15:00:00Z");
    leg["currency"] = json!("CAD");
    f.store(&earlier(), "old-live", &legs(vec![leg]));
    as_earlier_app(&f);
    let before = everything(f.dir.path());
    let done = f.book.remove_earlier_app_rows(t0()).unwrap();
    assert_eq!(done, EarlierAppRows::StillLive(1));
    assert_eq!(done.said().unwrap(), "1 of the earlier app's rows still counts, so none of them was removed: each is removed once the broker's own row replaces it");
    assert_eq!(everything(f.dir.path()), before, "the book is left as it was");
    assert_eq!(f.book.setting(EARLIER_APP_ROWS_REMOVED).unwrap(), None, "not recorded, so it runs again");
}

fn listing(symbol: &str) -> ListingDraft {
    ListingDraft {
        found: None,
        kind: InstrumentKind::Security,
        currency: Currency::CAD,
        refs: vec![Reference::new(RefScheme::Listing, format!("{symbol}@XTSX"))],
        name: ListingName { symbol: symbol.into(), venue_mic: Some("XTSX".into()), venue_name: None, name: Some(format!("{symbol} Inc")) },
    }
}

fn order(id: &str, bracket: Option<(&str, OrderRole)>) -> OrderRequest {
    OrderRequest {
        venue_mic: None,
        venue_name: None,
        id: id.into(),
        broker: "wealthsimple".into(),
        broker_account: "acct-1".into(),
        broker_security: "sec-1".into(),
        symbol: "QNC".into(),
        currency: Currency::CAD,
        side: Side::Buy,
        kind: OrderKind::Limit,
        quantity: d("10"),
        limit_price: Some(d("1.25")),
        stop_price: None,
        time_in_force: TimeInForce::Day,
        bracket: bracket.map(|(b, r)| (b.to_string(), r)),
        request: json!({"externalId": id}),
    }
}

/// A book where the person's notes, groups, orders, brackets, notifications and
/// watchlist sit on trades opened on the earlier app's rows, each row since
/// superseded by the broker's own. One trade is still anchored on its earlier
/// row, as an earlier build could leave one. The earlier rows of `extra` are
/// superseded by these broker rows instead of their own.
fn a_persons_book(extra: impl Fn(&Fixture, RecordId) -> Value) -> (Fixture, Vec<bagholder_core::TradeId>) {
    let f = Fixture::new();
    f.account(&["acct-1"]);
    let broker = Spelled::v(1);
    let mut trades = Vec::new();
    let rows = [("CA0000000001", "QNC", "100", "-150"), ("CA0000000002", "ZQR", "7", "-70.50"), ("CA0000000003", "XMPL", "12", "-24")];
    for (i, (isin, symbol, qty, cash)) in rows.iter().enumerate() {
        let leg = buy("acct-1", share(isin, symbol), qty, cash, &format!("2025-03-0{}T15:00:00Z", i + 1));
        let mut old = legs(vec![leg.clone()]);
        old["orders"] = json!([format!("old-order-{i}")]);
        let (text, key) = (old.to_string(), format!("old-{i}"));
        let mut incoming = f.incoming(&key, &text);
        incoming.refs = vec![("broker-record:wealthsimple".into(), format!("ws-{i}"))];
        let r = f.book.store(&earlier(), &incoming, t0()).unwrap();
        let opening = f.opens(r.record);
        let trade = f.book.open_trade(&opening, t0()).unwrap();
        f.book.set_journal(JournalSubject::Trade(trade), &JournalEntry { thesis: format!("thesis {i}"), grade: Some(Grade::B), tags: vec!["breakout".into(), format!("tag-{i}")] }, t0()).unwrap();
        let theirs = extra(&f, r.record);
        let ws = if theirs.is_null() { legs(vec![leg]) } else { theirs };
        let text = ws.to_string();
        f.book.store_superseding(&broker, &f.incoming(&format!("ws-{i}"), &text), &[r.record], "the broker's own row", t0()).unwrap();
        if i == 2 {
            // left on the earlier row by an earlier build
            f.book
                .conn_for_tests()
                .execute(
                    "UPDATE trades SET anchor_record = ?, anchor_leg = ?, anchor_instrument = ?, orphaned_reason = NULL WHERE id = ?",
                    [r.record.to_string(), opening.transaction.leg.as_str().to_string(), opening.instrument.to_string(), trade.to_string()],
                )
                .unwrap();
        }
        trades.push(trade);
    }
    let group = f.book.add_group(&trades[..2], true, t0()).unwrap();
    f.book.set_journal(JournalSubject::Group(group), &JournalEntry { thesis: "the pair".into(), grade: Some(Grade::A), tags: vec![] }, t0()).unwrap();
    // the keys the earlier app knew them by, as the first start's copy kept them
    f.book.conn_for_tests().execute("UPDATE trades SET legacy_key = 'rt:' || id", []).unwrap();
    f.book.conn_for_tests().execute("UPDATE trade_groups SET legacy_key = 'grp:1'", []).unwrap();
    let place = BracketPlace { venue_mic: None, venue_name: None, id: "bracket-1".into(), broker: "wealthsimple".into(), broker_account: "acct-1".into(), broker_security: "sec-1".into(), symbol: "QNC".into(), currency: Currency::CAD };
    f.book.write_bracket(&place, &BracketEvent::Created { quantity: d("10"), stop: Some(StopLeg { level: d("1.10"), trail: None, high: None }), target: Some(d("2")) }, &Asker::Person, t0()).unwrap();
    f.book.write_order(&order("old-order-0", Some(("bracket-1", OrderRole::Entry))), false, &Asker::Person, t0()).unwrap();
    f.book.order_event("old-order-0", &Asker::Person, t0(), &OrderEvent::Accepted { broker_id: "o-1".into() }).unwrap().unwrap();
    f.book.write_order(&order("old-order-1", None), true, &Asker::Person, t0()).unwrap();
    let c = f.book.conn_for_tests();
    c.execute("INSERT INTO notifications (at, kind, key, title, body, extra, seen_at, read_at) VALUES ('2025-03-01T15:00:05Z', 'fills', 'order:old-order-0:filled', 'Order filled · QNC', 'Bought 100', NULL, '2025-03-01T15:00:06Z', NULL)", []).unwrap();
    c.execute("INSERT INTO told (scope, event, at) VALUES ('news:QNC@XTSX', 'e-1', '2025-03-01T15:00:00Z')", []).unwrap();
    f.book.set_setting("notify.seen.news:QNC@XTSX", Some("2025-03-01T15:00:00Z|e-1"), t0()).unwrap();
    f.book.watch(&listing("QNC"), t0()).unwrap();
    f.book.set_tiles(&[listing("ZQR"), listing("QNC")], t0()).unwrap();
    as_earlier_app(&f);
    (f, trades)
}

#[test]
fn what_the_person_made_is_the_same_after_field_for_field() {
    let (f, trades) = a_persons_book(|_, _| Value::Null);
    let before = the_persons(&f);
    let units = counted(&f);
    let done = f.book.remove_earlier_app_rows(t0()).unwrap();
    assert_eq!(done, EarlierAppRows::Removed { records: 3, links: 3, trades_moved: 1 });
    let after = the_persons(&f);
    for ((table, was), (_, is)) in before.iter().zip(&after) {
        if table == "trades" {
            continue;
        }
        assert_eq!(is, was, "{table}");
    }
    // the trade left on its earlier row now stands on the row that replaced it,
    // as the other two already did; the rest of each trade is as it was
    for (n, t) in trades.iter().enumerate() {
        let Anchor::Opening(o) = f.book.trade(*t).unwrap().anchor else { panic!("trade {n} is anchored") };
        assert_eq!(f.book.record(o.transaction.record).unwrap().state, RecordState::Live, "trade {n}");
        assert_eq!(f.book.journal(JournalSubject::Trade(*t)).unwrap().unwrap().thesis, format!("thesis {n}"));
    }
    let others = |rows: &[String]| -> Vec<String> { rows.iter().filter(|r| !r.starts_with(&trades[2].to_string())).cloned().collect() };
    assert_eq!(others(&after[0].1), others(&before[0].1), "the trades that stood on the broker's rows are untouched");
    assert_eq!(counted(&f), units);
    let keys: i64 = f.book.conn_for_tests().query_row("SELECT (SELECT COUNT(*) FROM trades WHERE legacy_key IS NOT NULL) + (SELECT COUNT(*) FROM trade_groups WHERE legacy_key IS NOT NULL)", [], |r| r.get(0)).unwrap();
    assert_eq!(keys, 0, "no key of the earlier app's is kept");
    assert_eq!(rows(&f, "record_orders", "*").len(), 0, "the orders the earlier rows named are no longer named by any row");
    assert_eq!(f.book.order("old-order-0").unwrap().unwrap().fold.broker_id.as_deref(), Some("o-1"), "the order itself is kept");
}

#[test]
fn a_trade_with_no_single_row_to_move_to_refuses_the_step_and_leaves_the_book_as_it_was() {
    // the broker's row for the earlier row of trade 2 moves another share, so
    // the trade still on that earlier row has nowhere to go
    let (f, trades) = a_persons_book(|f, r| {
        let n = f.book.record(r).unwrap().source_key;
        if n == "old-2" {
            legs(vec![buy("acct-1", share("CA0000000099", "OTHR"), "12", "-24", "2025-03-03T15:00:00Z")])
        } else {
            Value::Null
        }
    });
    let before = everything(f.dir.path());
    let err = f.book.remove_earlier_app_rows(t0()).unwrap_err();
    let BookError::Refused(why) = err else { panic!("refused: {err}") };
    assert!(why.contains(&trades[2].to_string()) && why.contains("no row that replaced it opens its instrument"), "{why}");
    assert_eq!(everything(f.dir.path()), before, "nothing changed");
    assert_eq!(f.book.setting(EARLIER_APP_ROWS_REMOVED).unwrap(), None);
}

#[test]
fn once_done_it_is_recorded_and_never_runs_again() {
    let f = generated(3, 6);
    assert!(matches!(f.book.remove_earlier_app_rows(t0()).unwrap(), EarlierAppRows::Removed { .. }));
    assert!(f.book.setting(EARLIER_APP_ROWS_REMOVED).unwrap().is_some());
    // a row of the earlier app's that somehow appears later is not looked at again
    let r = f.store(&earlier(), "late", &legs(vec![buy("acct-1", share("CA0000000001", "QNC"), "1", "-1", "2025-01-02T15:00:00Z")]));
    f.book.mark_removed(r.record, t0()).unwrap();
    as_earlier_app(&f);
    let before = everything(f.dir.path());
    assert_eq!(f.book.remove_earlier_app_rows(t0()).unwrap(), EarlierAppRows::RemovedBefore);
    assert_eq!(everything(f.dir.path()), before);
}

#[test]
fn a_book_that_never_held_one_records_the_step_done() {
    let f = Fixture::new();
    f.account(&["acct-1"]);
    f.store(&Spelled::v(1), "ws-1", &legs(vec![buy("acct-1", share("CA0000000001", "QNC"), "1", "-1", "2025-01-02T15:00:00Z")]));
    let done = f.book.remove_earlier_app_rows(t0()).unwrap();
    assert_eq!(done, EarlierAppRows::Removed { records: 0, links: 0, trades_moved: 0 });
    assert_eq!(done.said(), None, "nothing to say");
    assert_eq!(f.book.remove_earlier_app_rows(t0()).unwrap(), EarlierAppRows::RemovedBefore);
}
