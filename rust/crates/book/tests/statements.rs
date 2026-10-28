//! What a broker states beside its rows (`docs/plans/stage-3b-wealthsimple.md`,
//! "Statements and reads"): kept as stated, a later statement beside an earlier
//! one, the newest read.

mod common;

use std::collections::BTreeMap;

use bagholder_book::statements::{AccountDay, UnitsLine};
use bagholder_core::{Currency, Dec, Money};
use common::*;

fn day(s: &str) -> jiff::civil::Date {
    s.parse().unwrap()
}

#[test]
fn a_day_stated_again_the_same_writes_nothing_and_differently_is_kept_beside_it() {
    let f = Fixture::new();
    let a = f.account(&["tfsa-1"]);
    let r1 = f.book.broker_read(f.connection, "history:tfsa-1", t0()).unwrap();
    let days = [AccountDay { day: day("2026-09-01"), net_value: cad("100"), net_deposits: cad("90") }, AccountDay { day: day("2026-09-02"), net_value: cad("101"), net_deposits: cad("90") }];
    assert!(f.book.store_account_days(a, &days, &r1).unwrap().is_empty());
    let r2 = f.book.broker_read(f.connection, "history:tfsa-1", at("2026-09-24T12:00:00Z")).unwrap();
    // the same again: nothing changed, nothing written
    assert!(f.book.store_account_days(a, &days, &r2).unwrap().is_empty());
    // the broker restates the second day
    let restated = [AccountDay { day: day("2026-09-02"), net_value: cad("102"), net_deposits: cad("90") }];
    assert_eq!(f.book.store_account_days(a, &restated, &r2).unwrap(), vec![day("2026-09-02")]);
    let stated = f.book.stated(a).unwrap();
    assert_eq!(stated.days[&day("2026-09-02")].0, cad("102"));
    assert_eq!(stated.days[&day("2026-09-01")].0, cad("100"));
    assert_eq!(f.book.last_account_day(a).unwrap(), Some(day("2026-09-02")));
}

#[test]
fn only_a_read_of_every_page_is_a_full_read() {
    let f = Fixture::new();
    let a = f.account(&["tfsa-1"]);
    f.book.note_activity_read(a, at("2026-09-23T10:00:00Z"), true).unwrap();
    f.book.note_activity_read(a, at("2026-09-24T10:00:00Z"), false).unwrap();
    assert_eq!(f.book.activity_read_at(a).unwrap(), Some(at("2026-09-23T10:00:00Z")));
}

#[test]
fn the_newest_cash_and_units_are_the_ones_read() {
    let f = Fixture::new();
    let a = f.account(&["tfsa-1"]);
    let r = f.book.broker_read(f.connection, "balances", t0()).unwrap();
    f.book.store_cash(a, at("2026-09-23T10:00:00Z"), &BTreeMap::from([(Currency::CAD, d("5"))]), &r).unwrap();
    f.book.store_cash(a, at("2026-09-24T10:00:00Z"), &BTreeMap::from([(Currency::CAD, d("7")), (Currency::USD, d("-1"))]), &r).unwrap();
    let rec = f.store(&Spelled::v(1), "buy", &legs(vec![buy("tfsa-1", share("CA0000000001", "XYZ"), "3", "-30", "2026-09-01T15:00:00Z")]));
    let i = f.book.transactions_of(rec.record).unwrap()[0].instrument.unwrap();
    f.book.store_units(a, day("2026-09-22"), &[UnitsLine { instrument: i, quantity: d("3"), book_value: Some(cad("31")), value: Some(cad("36.75")) }], &Default::default(), &r, t0()).unwrap();
    let s = f.book.stated(a).unwrap();
    let (when, cash) = s.cash.unwrap();
    assert_eq!(when, at("2026-09-24T10:00:00Z"));
    assert_eq!(cash[&Currency::USD], d("-1"));
    let (as_of, units) = s.units.unwrap();
    assert_eq!((as_of, units[&i]), (day("2026-09-22"), Dec::parse("3").unwrap()));
    assert_eq!(s.unit_values[&i], cad("36.75"), "what the broker states the units are worth");
}

#[test]
fn the_cash_a_full_read_covers_is_the_newest_stated_at_or_before_it() {
    let f = Fixture::new();
    let a = f.account(&["tfsa-1"]);
    let r = f.book.broker_read(f.connection, "balances", t0()).unwrap();
    // fractions of different lengths: ordered by the instant, not the text
    f.book.store_cash(a, at("2026-09-23T10:00:00.5Z"), &BTreeMap::from([(Currency::CAD, d("5"))]), &r).unwrap();
    f.book.store_cash(a, at("2026-09-23T10:00:00.450401Z"), &BTreeMap::from([(Currency::CAD, d("4"))]), &r).unwrap();
    f.book.store_cash(a, at("2026-09-24T10:00:00Z"), &BTreeMap::from([(Currency::CAD, d("7"))]), &r).unwrap();
    assert!(f.book.stated(a).unwrap().cash_read.is_none(), "no full read: no cash it covers");
    f.book.note_activity_read(a, at("2026-09-23T10:00:00.450401Z"), true).unwrap();
    let s = f.book.stated(a).unwrap();
    assert_eq!(s.cash.unwrap().1[&Currency::CAD], d("7"));
    assert_eq!(s.cash_read.unwrap()[&Currency::CAD], d("4"));
    f.book.note_activity_read(a, at("2026-09-23T11:00:00Z"), true).unwrap();
    assert_eq!(f.book.stated(a).unwrap().cash_read.unwrap()[&Currency::CAD], d("5"));
}

#[test]
fn a_move_s_two_sides_are_linked_as_stated() {
    let f = Fixture::new();
    f.account(&["tfsa-1"]);
    f.account(&["rrsp-1"]);
    let out = f.store(&Spelled::v(1), "out", &legs(vec![sell("tfsa-1", share("CA0000000001", "XYZ"), "3", "0", "2026-09-01T15:00:00Z")]));
    let into = f.store(&Spelled::v(1), "in", &legs(vec![buy("rrsp-1", share("CA0000000001", "XYZ"), "3", "0", "2026-09-01T15:00:00Z")]));
    let o = f.book.transactions_of(out.record).unwrap()[0].id.clone();
    let i = f.book.transactions_of(into.record).unwrap()[0].id.clone();
    f.book.link_transfer(&o, &i).unwrap();
    assert_eq!(f.book.transfer_links().unwrap(), vec![(o, i)]);
}

#[test]
fn what_an_account_can_borrow_is_the_newest_statement_an_amount_or_why_not() {
    let f = Fixture::new();
    let a = f.account(&["margin-1"]);
    assert!(f.book.stated(a).unwrap().buying_power.is_none(), "nothing stated is nothing, not zero");
    let r = f.book.broker_read(f.connection, "balances", t0()).unwrap();
    f.book.store_buying_power(a, at("2026-09-23T10:00:00Z"), &Ok(cad("1500.25")), &r).unwrap();
    assert_eq!(f.book.stated(a).unwrap().buying_power, Some((at("2026-09-23T10:00:00Z"), Ok(cad("1500.25")))));
    f.book.store_buying_power(a, at("2026-09-24T10:00:00Z"), &Err("UnavailableSecurities (2 securities)".into()), &r).unwrap();
    assert_eq!(f.book.stated(a).unwrap().buying_power, Some((at("2026-09-24T10:00:00Z"), Err("UnavailableSecurities (2 securities)".to_string()))));
}

#[test]
fn a_part_s_last_read_is_the_newest_of_its_reads() {
    let f = Fixture::new();
    assert_eq!(f.book.last_read(f.connection, "accounts").unwrap(), None);
    f.book.broker_read(f.connection, "accounts", at("2026-09-23T20:00:00Z")).unwrap();
    f.book.broker_read(f.connection, "accounts", at("2026-09-24T20:00:00Z")).unwrap();
    f.book.broker_read(f.connection, "cash", at("2026-09-25T20:00:00Z")).unwrap();
    assert_eq!(f.book.last_read(f.connection, "accounts").unwrap(), Some(at("2026-09-24T20:00:00Z")));
}

#[test]
fn a_statement_of_cash_keeps_what_the_live_records_held_as_it_was_stated() {
    use bagholder_core::hold::HoldKind;
    let f = Fixture::new();
    let a = f.account(&["tfsa-1"]);
    let m = Spelled::v(1);
    let order = f.store(&m, "order", &serde_json::json!({"hold": {"account": "tfsa-1", "kind": "buy", "currency": "USD", "amount": "70.00"}}));
    let r = f.book.broker_read(f.connection, "cash", t0()).unwrap();
    let first = at("2026-10-01T14:47:07Z");
    f.book.note_activity_read(a, first, true).unwrap();
    f.book.store_cash(a, first, &BTreeMap::from([(Currency::USD, d("14881.57"))]), &r).unwrap();
    let s = f.book.stated(a).unwrap();
    assert_eq!(s.cash_read_holds.len(), 1);
    let h = &s.cash_read_holds[0];
    assert_eq!((h.record, h.kind, h.currency, h.amount), (order.record, HoldKind::Buy, Some(Currency::USD), Some(d("70.00"))));

    // the order lapses: its record holds nothing, and the next statement none,
    // while the first keeps what it was stated beside
    let lapsed = f.book.store(&m, &f.incoming("order", r#"{"legs": []}"#), at("2026-10-02T14:09:55Z")).unwrap();
    assert_eq!(lapsed.record, order.record);
    let second = at("2026-10-02T14:09:55Z");
    f.book.note_activity_read(a, second, true).unwrap();
    f.book.store_cash(a, second, &BTreeMap::from([(Currency::USD, d("14951.57"))]), &r).unwrap();
    assert!(f.book.stated(a).unwrap().cash_read_holds.is_empty());
    let n: i64 = f.book.conn_for_tests().query_row("SELECT COUNT(*) FROM statement_holds", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 1);
}

#[test]
fn a_record_no_longer_live_holds_nothing_at_the_next_statement() {
    let f = Fixture::new();
    let a = f.account(&["tfsa-1"]);
    let order = f.store(&Spelled::v(1), "order", &serde_json::json!({"hold": {"account": "tfsa-1", "kind": "withdrawal", "currency": "CAD"}}));
    f.book.mark_removed(order.record, t0()).unwrap();
    let r = f.book.broker_read(f.connection, "cash", t0()).unwrap();
    f.book.note_activity_read(a, t0(), true).unwrap();
    f.book.store_cash(a, t0(), &BTreeMap::from([(Currency::CAD, d("1"))]), &r).unwrap();
    assert!(f.book.stated(a).unwrap().cash_read_holds.is_empty());
}

/// What the statement of cash a test makes holds for the record `r`.
fn held_at(f: &Fixture, a: bagholder_core::AccountId, when: &str) -> Vec<bagholder_core::hold::Hold> {
    let r = f.book.broker_read(f.connection, "cash", at(when)).unwrap();
    f.book.note_activity_read(a, at(when), true).unwrap();
    f.book.store_cash(a, at(when), &BTreeMap::from([(Currency::USD, d("1"))]), &r).unwrap();
    f.book.stated(a).unwrap().cash_read_holds
}

#[test]
fn a_working_buy_with_a_fill_against_its_order_holds_an_amount_its_row_does_not_state() {
    let f = Fixture::new();
    let a = f.account(&["tfsa-1"]);
    let m = Spelled::v(1);
    let pending = serde_json::json!({"hold": {"account": "tfsa-1", "kind": "buy", "currency": "USD", "amount": "500.00"}, "orders": ["order-a", "order-ext-a"]});
    f.store(&m, "order-a", &pending);
    // nothing filled against it: the row's amount is the hold
    assert_eq!(held_at(&f, a, "2026-10-01T15:00:00Z")[0].amount, Some(d("500.00")));

    // the app's own order of that id has a fill booked: the rest is unstated
    f.book
        .conn_for_tests()
        .execute(
            "INSERT INTO orders (id, broker, broker_account, broker_security, symbol, currency, side, order_type, quantity, time_in_force, request, created_at, state, filled, updated_at)
             VALUES ('order-ext-a', 'wealthsimple', 'tfsa-1', 'sec', 'XYZ', 'USD', 'buy', 'limit', '10', 'day', '{}', '2026-10-01T15:00:00Z', 'partly-filled', '4', '2026-10-01T15:00:00Z')",
            [],
        )
        .unwrap();
    assert_eq!(held_at(&f, a, "2026-10-01T16:00:00Z")[0].amount, None);
}

#[test]
fn a_working_buy_whose_order_has_another_record_that_moved_something_holds_an_unstated_amount() {
    let f = Fixture::new();
    let a = f.account(&["tfsa-1"]);
    let m = Spelled::v(1);
    f.store(&m, "order-b", &serde_json::json!({"hold": {"account": "tfsa-1", "kind": "buy", "currency": "USD", "amount": "300.00"}, "orders": ["order-b"]}));
    // a record of another order moves something: the hold is untouched
    let mut other = legs(vec![buy("tfsa-1", share("CA0000000001", "XYZ"), "3", "-90", "2026-10-01T15:00:00Z")]);
    other["orders"] = serde_json::json!(["order-other"]);
    f.store(&m, "fill-other", &other);
    assert_eq!(held_at(&f, a, "2026-10-01T15:30:00Z")[0].amount, Some(d("300.00")));
    // a fill of the same order, as its own record: the rest is unstated
    let mut fill = legs(vec![buy("tfsa-1", share("CA0000000001", "XYZ"), "1", "-30", "2026-10-01T15:00:00Z")]);
    fill["orders"] = serde_json::json!(["order-b"]);
    f.store(&m, "fill-b", &fill);
    assert_eq!(held_at(&f, a, "2026-10-01T16:00:00Z")[0].amount, None);
}

/// Two statements of an account's cash stated at one instant (a balances read,
/// then the activity read it prompted, at the same moment): the newest is the
/// one written last, which holds what the activity read found (here a pending
/// withdrawal, of no stated size), whatever the ids drawn for them. Held over
/// many draws, since an id is random.
#[test]
fn of_two_statements_at_one_instant_the_one_written_last_is_the_newest() {
    use bagholder_core::hold::HoldKind;
    for _ in 0..64 {
        let f = Fixture::new();
        let a = f.account(&["cash-1"]);
        let when = at("2026-10-09T14:13:21.740361Z");
        let r = f.book.broker_read(f.connection, "cash", when).unwrap();
        // the balances: the cash after the withdrawal, which the book has not read yet
        f.book.store_cash(a, when, &BTreeMap::from([(Currency::CAD, d("6896.40"))]), &r).unwrap();
        // the activity read finds the withdrawal, pending; the pull states the cash again
        f.store(&Spelled::v(1), "withdrawal", &serde_json::json!({"hold": {"account": "cash-1", "kind": "withdrawal", "currency": "CAD"}}));
        f.book.note_activity_read(a, when, true).unwrap();
        let r = f.book.broker_read(f.connection, "cash", when).unwrap();
        f.book.store_cash(a, when, &BTreeMap::from([(Currency::CAD, d("6896.40"))]), &r).unwrap();
        let s = f.book.stated(a).unwrap();
        assert_eq!(s.cash_read_holds.iter().map(|h| h.kind).collect::<Vec<_>>(), [HoldKind::Withdrawal], "the statement the activity read made, with the hold it found");
        // what the account is worth and can borrow: the one written last
        let v = f.book.broker_read(f.connection, "net-value", when).unwrap();
        f.book.store_net_value(a, when, Money::new(d("1"), Currency::CAD), &v).unwrap();
        f.book.store_net_value(a, when, Money::new(d("2"), Currency::CAD), &v).unwrap();
        assert_eq!(f.book.stated(a).unwrap().net_value_now.map(|(_, m)| m.amount), Some(d("2")));
        // (each read its own, as each pass makes one)
        let b = f.book.broker_read(f.connection, "buying-power", when).unwrap();
        f.book.store_buying_power(a, when, &Ok(Money::new(d("10"), Currency::CAD)), &b).unwrap();
        let b = f.book.broker_read(f.connection, "buying-power", when).unwrap();
        f.book.store_buying_power(a, when, &Ok(Money::new(d("20"), Currency::CAD)), &b).unwrap();
        assert_eq!(f.book.stated(a).unwrap().buying_power.map(|(_, m)| m.unwrap().amount), Some(d("20")));
    }
}

/// An instant's text does not sort as time (its fraction varies in length):
/// a statement stated later in the same second, whose text sorts first, is
/// still the newest.
#[test]
fn the_newest_statement_is_the_latest_stated_whatever_its_text_sorts_as() {
    let f = Fixture::new();
    let a = f.account(&["cash-1"]);
    // "…21.74Z" sorts after "…21.7403Z" as text, though it is earlier
    let (earlier, later) = (at("2026-10-09T14:13:21.74Z"), at("2026-10-09T14:13:21.7403Z"));
    let v = f.book.broker_read(f.connection, "net-value", earlier).unwrap();
    f.book.store_net_value(a, earlier, Money::new(d("1"), Currency::CAD), &v).unwrap();
    f.book.store_net_value(a, later, Money::new(d("2"), Currency::CAD), &v).unwrap();
    assert_eq!(f.book.stated(a).unwrap().net_value_now.map(|(_, m)| m.amount), Some(d("2")));
    let r = f.book.broker_read(f.connection, "cash", earlier).unwrap();
    f.book.store_cash(a, earlier, &BTreeMap::from([(Currency::CAD, d("1"))]), &r).unwrap();
    f.book.store_cash(a, later, &BTreeMap::from([(Currency::CAD, d("2"))]), &r).unwrap();
    assert_eq!(f.book.stated(a).unwrap().cash.map(|(_, c)| c[&Currency::CAD]), Some(d("2")));
}

/// splitmix64: the same seed, the same sequence.
struct Rng(u64);
impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)) % n
    }
}

/// A statement kept as a period (migration 026) answers what a row per read
/// answered: for any sequence of statements, their instants out of order and
/// tied, the newest (by instant, then the one written last) and the newest at or
/// before each full read of the activity are the same as a model of the rows
/// gives them, and so are buying power and the value now.
#[test]
fn statements_kept_as_periods_answer_what_a_row_per_read_answered_for_any_order_of_instants() {
    for seed in 0..60u64 {
        let f = Fixture::new();
        let a = f.account(&["tfsa-1"]);
        let mut rng = Rng(seed);
        let base = t0();
        let n = 3 + rng.below(25) as usize;
        // (instant offset in seconds, the order written, a value): instants in a
        // narrow range so ties and steps back are common, values from three
        let mut rows: Vec<(i64, usize, u64)> = Vec::new();
        for k in 0..n {
            let offset = rng.below(12) as i64;
            let value = rng.below(3);
            let when = base.checked_add(jiff::SignedDuration::from_secs(offset)).unwrap();
            let read = f.book.broker_read(f.connection, "cash", when).unwrap();
            let cash = BTreeMap::from([(Currency::CAD, Dec::from_int(value as i64))]);
            f.book.store_cash(a, when, &cash, &read).unwrap();
            let power = if value == 2 { Err("not stated".to_string()) } else { Ok(cad(&value.to_string())) };
            let bp_read = f.book.broker_read(f.connection, "buying-power", when).unwrap();
            f.book.store_buying_power(a, when, &power, &bp_read).unwrap();
            let nv_read = f.book.broker_read(f.connection, "net-value", when).unwrap();
            f.book.store_net_value(a, when, cad(&(value * 10).to_string()), &nv_read).unwrap();
            rows.push((offset, k, value));
            // after each statement: the newest at or before every instant, as the rows give it
            let mut sorted = rows.clone();
            sorted.sort();
            for r in -1..13i64 {
                let want = sorted.iter().rev().find(|(t, _, _)| *t <= r).map(|(_, _, v)| *v as i64);
                let got: Option<i64> = f
                    .book
                    .conn_for_tests()
                    .query_row(
                        "SELECT CAST(c.amount AS INTEGER) FROM statements s JOIN statement_cash c ON c.statement_id = s.id WHERE s.kind = 'cash' AND s.first_ns <= ?1 ORDER BY s.first_ns DESC, s.first_seq DESC LIMIT 1",
                        rusqlite::params![base.as_nanosecond() as i64 + r * 1_000_000_000],
                        |x| x.get(0),
                    )
                    .ok();
                if got != want {
                    let periods: Vec<(i64, i64, String)> = f
                        .book
                        .conn_for_tests()
                        .prepare("SELECT (s.first_ns - ?1) / 1000000000, (s.last_ns - ?1) / 1000000000, c.amount FROM statements s JOIN statement_cash c ON c.statement_id = s.id WHERE s.kind = 'cash' ORDER BY s.first_ns, s.first_seq")
                        .unwrap()
                        .query_map(rusqlite::params![base.as_nanosecond() as i64], |x| Ok((x.get(0)?, x.get(1)?, x.get(2)?)))
                        .unwrap()
                        .map(Result::unwrap)
                        .collect();
                    panic!("seed {seed}, after statement {k} ({offset}s, {value}): at +{r}s the periods give {got:?}, the rows {want:?}\nstatements: {rows:?}\nperiods (first, last, cash): {periods:?}");
                }
            }
        }
        let mut sorted = rows.clone();
        sorted.sort();
        let (t_newest, _, v_newest) = *sorted.last().unwrap();
        let s = f.book.stated_latest(a).unwrap();
        let instant = |o: i64| base.checked_add(jiff::SignedDuration::from_secs(o)).unwrap();
        assert_eq!(s.cash, Some((instant(t_newest), BTreeMap::from([(Currency::CAD, Dec::from_int(v_newest as i64))]))), "seed {seed}: newest cash, {rows:?}");
        let power = if v_newest == 2 { Err("not stated".to_string()) } else { Ok(cad(&v_newest.to_string())) };
        assert_eq!(s.buying_power, Some((instant(t_newest), power)), "seed {seed}: buying power, {rows:?}");
        assert_eq!(s.net_value_now, Some((instant(t_newest), cad(&(v_newest * 10).to_string()))), "seed {seed}: value now, {rows:?}");
        // each full read, in order of its instant: the newest statement at or before it
        for r in -1..13i64 {
            f.book.note_activity_read(a, instant(r), true).unwrap();
            let want = sorted.iter().rev().find(|(t, _, _)| *t <= r).map(|(_, _, v)| BTreeMap::from([(Currency::CAD, Dec::from_int(*v as i64))]));
            assert_eq!(f.book.stated_latest(a).unwrap().cash_read, want, "seed {seed}: the cash at a read at +{r}s, {rows:?}");
        }
    }
}

/// A read stating again what the last one did keeps one row whose last moment
/// moves on; the engine's moment for it is that last moment (brief 22, 1): 100 at
/// t1, the activity read in full at t2, 100 again at t3: the cash is stated at t3,
/// after the read, and covers it.
#[test]
fn a_cash_stated_again_unchanged_extends_its_period_and_is_stated_at_its_last_moment() {
    let f = Fixture::new();
    let a = f.account(&["tfsa-1"]);
    let (t1, t2, t3) = (at("2026-10-01T10:00:00Z"), at("2026-10-01T10:05:00Z"), at("2026-10-01T10:10:00Z"));
    let hundred = BTreeMap::from([(Currency::CAD, d("100"))]);
    for t in [t1, t3] {
        let r = f.book.broker_read(f.connection, "cash", t).unwrap();
        f.book.store_cash(a, t, &hundred, &r).unwrap();
    }
    f.book.note_activity_read(a, t2, true).unwrap();
    let rows: i64 = f.book.conn_for_tests().query_row("SELECT count(*) FROM statements WHERE kind = 'cash'", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, 1, "one row for one value");
    let s = f.book.stated_latest(a).unwrap();
    assert_eq!(s.cash, Some((t3, hundred.clone())), "stated at its last moment, after the read");
    assert_eq!(s.cash_read, Some(hundred), "and the cash the read covers");
}

/// Two statements are the same only when their holds are too, a missing amount
/// equal only to a missing one: a hold whose amount becomes unknown starts a
/// period of its own, and the same holds again extend it (brief 22, 3).
#[test]
fn a_statement_whose_only_change_is_a_hold_becoming_unknown_is_a_period_of_its_own() {
    let f = Fixture::new();
    let a = f.account(&["tfsa-1"]);
    let rec = f.store(&Spelled::v(1), "pending-buy", &legs(vec![buy("tfsa-1", share("CA0000000001", "XYZ"), "3", "0", "2026-10-01T15:00:00Z")]));
    let c = f.book.conn_for_tests();
    c.execute("INSERT INTO record_holds (record_id, account_id, kind, currency, amount) VALUES (?1, ?2, 'buy', 'CAD', '30')", rusqlite::params![rec.record.to_string(), a.to_string()]).unwrap();
    let cash = BTreeMap::from([(Currency::CAD, d("100"))]);
    let store = |t: &str| {
        let r = f.book.broker_read(f.connection, "cash", at(t)).unwrap();
        f.book.store_cash(a, at(t), &cash, &r).unwrap();
    };
    let periods = || -> i64 { f.book.conn_for_tests().query_row("SELECT count(*) FROM statements WHERE kind = 'cash'", [], |r| r.get(0)).unwrap() };
    store("2026-10-01T10:00:00Z");
    store("2026-10-01T10:05:00Z");
    assert_eq!(periods(), 1, "the same cash and the same hold");
    c.execute("UPDATE record_holds SET amount = NULL WHERE record_id = ?1", rusqlite::params![rec.record.to_string()]).unwrap();
    store("2026-10-01T10:10:00Z");
    assert_eq!(periods(), 2, "the hold's amount became unknown");
    store("2026-10-01T10:15:00Z");
    assert_eq!(periods(), 2, "an unknown amount is the same as an unknown amount");
    f.book.note_activity_read(a, at("2026-10-01T10:07:00Z"), true).unwrap();
    assert_eq!(f.book.stated_latest(a).unwrap().cash_read_holds[0].amount, Some(d("30")), "the holds as they stood when the read's cash was stated");
}
