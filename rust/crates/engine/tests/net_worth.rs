//! Net worth by day, worked out by Bagholder (`docs/plans/debts-held-outside-wealthsimple.md`):
//! a point for every day from the first anything is stated to today, each the
//! sum of what each account and each thing held or owed outside was last stated
//! to be on or before it; today's point the tiles' own figures; a day that cannot
//! be stated kept, with why. Every figure here is made up.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::jiff::civil::Date;
use bagholder_core::{Currency, Dec, Money};
use bagholder_engine::gap::Gap;
use bagholder_engine::input::{Debt, Debts};
use bagholder_engine::scope::{Filters, NetWorthDay};
use bagholder_engine::Change;
use common::*;
use serde_json::{json, Value};

fn day(s: &str) -> Date {
    s.parse().unwrap()
}

fn cad(s: &str) -> Money {
    Money::new(Dec::parse(s).unwrap(), Currency::CAD)
}

/// Two open accounts stated on different days, one in US dollars; a closed
/// account; a home stated twice and a line of credit stated once. The margin
/// account's positions read states its cash on each day from its first stated
/// value (brief 79 §2.4): what it owes on each is that read's.
fn case() -> Value {
    let mut c = json!({
        "name": "net worth by day",
        "today": "2026-10-20",
        "now": "2026-10-20T22:00:00Z",
        "accounts": [{"id": "A", "kind": "margin"}, {"id": "B"}, {"id": "Z", "status": "closed"}],
        "instruments": [],
        "rates": {"USD": {"2026-10-06": "1.30", "2026-10-13": "1.40", "2026-10-20": "1.50"}},
        "holidays": ["2026-10-12"],
        "brokers": {
            "A": {"cash": {"CAD": "1350"}, "net_value": {"2026-10-05": "1000", "2026-10-07": "1100", "2026-10-09": "1200", "2026-10-19": "1300"}, "now": "1350", "as_of": "2026-10-20T15:00:00Z", "buying_power": "0"},
            "B": {"currency": "USD", "cash": {"USD": "120"}, "net_value": {"2026-10-06": "100", "2026-10-13": "110"}, "now": "120", "as_of": "2026-10-20T15:00:00Z"},
            "Z": {"net_value": {"2026-10-05": "500", "2026-10-08": "400"}}
        },
        "transactions": []
    });
    let mut d = day("2026-10-05");
    while d < day("2026-10-20") {
        c["brokers"]["A"]["days_held"][d.to_string()] = json!({"cash": {"CAD": "1350"}});
        d = d.tomorrow().unwrap();
    }
    c
}

fn outside() -> Debts {
    let home = Debt { id: "home".into(), kind: "RealEstate".into(), name: "Home".into(), institution: String::new(), balance: cad("310000"), as_of: Some("2026-10-15T18:00:00Z".parse().unwrap()) };
    let loc = Debt { id: "loc".into(), kind: "LineOfCredit".into(), name: "Line of credit".into(), institution: "Bank".into(), balance: cad("5000"), as_of: Some("2026-10-07T18:00:00Z".parse().unwrap()) };
    Debts {
        owed: vec![loc],
        assets: vec![home],
        history: BTreeMap::from([
            ("home".to_string(), BTreeMap::from([(day("2026-10-06"), cad("300000")), (day("2026-10-15"), cad("310000"))])),
            ("loc".to_string(), BTreeMap::from([(day("2026-10-07"), cad("5000"))])),
        ]),
        repayments: BTreeMap::new(),
        asked: vec![],
    }
}

fn engine_of(case: &Value) -> (Built, bagholder_engine::Engine) {
    let mut b = build(case);
    let mut e = engine(&mut b);
    e.apply(Change::Debts(outside()));
    (b, e)
}

fn stated(days: &[NetWorthDay]) -> Vec<(String, String, String)> {
    days.iter().map(|n| (n.day.to_string(), n.assets.clone().unwrap().amount.to_string(), n.debts.clone().unwrap().amount.to_string())).collect()
}

fn rows(xs: &[(&str, &str, &str)]) -> Vec<(String, String, String)> {
    xs.iter().map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string())).collect()
}

/// (a) Every day from the first anything is stated to today has a point, each
/// the sum of what each thing was last stated to be on or before it: an account
/// at the rate of the day it was stated for, the home held from each statement
/// to the next, the line of credit from its one statement on; (e) the closed
/// account stops after its last stated day; (b) today is the tiles' figures.
#[test]
fn every_day_is_the_sum_of_what_each_thing_was_last_stated_to_be() {
    let (_, e) = engine_of(&case());
    let days = e.scope(&Filters::default()).portfolio.net_worth_history;
    let want = rows(&[
        ("2026-10-05", "1500", "0"),          // A 1000, Z 500
        ("2026-10-06", "301630", "0"),     // and B 100 USD at 1.30, the home 300,000
        ("2026-10-07", "301730", "5000"),  // A 1100; the line of credit stated
        ("2026-10-08", "301630", "5000"),  // Z 400, its last statement
        ("2026-10-09", "301330", "5000"),  // A 1200; Z closed, counted no more
        ("2026-10-10", "301330", "5000"),
        ("2026-10-11", "301330", "5000"),
        ("2026-10-12", "301330", "5000"),
        ("2026-10-13", "301354", "5000"),  // B 110 USD at 1.40
        ("2026-10-14", "301354", "5000"),
        ("2026-10-15", "311354", "5000"),  // the home restated
        ("2026-10-16", "311354", "5000"),
        ("2026-10-17", "311354", "5000"),
        ("2026-10-18", "311354", "5000"),
        ("2026-10-19", "311454", "5000"),  // A 1300
        // today: the tiles' figures, the accounts' cash (1,350 and 120 USD at the
        // live 1.50) and the home, less the line of credit
        ("2026-10-20", "311530", "5000"),
    ]);
    assert_eq!(stated(&days), want);
}

/// (b) The last point is today's, the very figures the tiles show.
#[test]
fn todays_point_is_the_tiles() {
    let (_, e) = engine_of(&case());
    let pf = e.scope(&Filters::default()).portfolio;
    let last = pf.net_worth_history.last().unwrap();
    let now = pf.net_worth_now.clone().unwrap();
    assert_eq!(last, &now);
    assert_eq!(last.day, day("2026-10-20"));
    // the tiles' parts: the cash in the accounts, the home, the line of credit
    let assets = pf.invested_cash.clone().unwrap().add_to_fit(pf.chequing_cash.clone().unwrap()).unwrap().add_to_fit(pf.outside_assets[0].1.clone().unwrap()).unwrap();
    assert_eq!(now.assets.clone().unwrap(), assets);
    assert_eq!(now.debts.clone().unwrap(), pf.owed.clone().unwrap().0.unwrap());
    assert_eq!(now.net().unwrap(), cad("306530"));
}

/// (c) A day whose part cannot be put in CAD (the Bank's rate for the day an
/// account was stated for not stored) keeps its point, waiting on that rate, and
/// so does every day that statement is held; the days around it are stated.
#[test]
fn a_day_waiting_on_a_rate_keeps_its_point_and_says_why() {
    let mut c = case();
    c["rates"]["USD"].as_object_mut().unwrap().remove("2026-10-13");
    let (_, e) = engine_of(&c);
    let days = e.scope(&Filters::default()).portfolio.net_worth_history;
    assert_eq!(days.len(), 16, "no day vanishes");
    let gap = Gap::RateMissing { currency: Currency::USD, day: day("2026-10-13") };
    for n in &days {
        let waiting = (day("2026-10-13")..=day("2026-10-19")).contains(&n.day);
        assert_eq!(n.assets.is_err(), waiting, "{}", n.day);
        if waiting {
            assert!(n.assets.clone().unwrap_err().contains(&gap), "{}: {:?}", n.day, n.assets);
            assert!(n.net().unwrap_err().contains(&gap));
            assert_eq!(n.debts.clone().unwrap(), cad("5000"), "what is owed is still stated");
        }
    }
}

/// (d) Under an account filter: those accounts' own values, no outside thing.
#[test]
fn under_an_account_filter_nothing_outside_counts() {
    let (mut b, e) = engine_of(&case());
    let f = Filters { accounts: BTreeSet::from([b.ids.account("A"), b.ids.account("B")]), ..Filters::default() };
    let pf = e.scope(&f).portfolio;
    let got = stated(&pf.net_worth_history);
    assert_eq!(got.first().unwrap(), &rows(&[("2026-10-05", "1000", "0")])[0]);
    assert_eq!(got.iter().find(|r| r.0 == "2026-10-15").unwrap(), &rows(&[("2026-10-15", "1354", "0")])[0], "no home, no line of credit");
    assert_eq!(got.last().unwrap(), &rows(&[("2026-10-20", "1530", "0")])[0], "today: the two accounts' cash");
    assert_eq!(pf.net_worth_history.last(), pf.net_worth_now.as_ref());
}

/// (e) A closed account stops after its last stated day; an open one's last
/// statement is held to today.
#[test]
fn a_closed_account_stops_after_its_last_stated_day() {
    let (mut b, e) = engine_of(&case());
    let mut only = |a: &str| e.scope(&Filters { accounts: BTreeSet::from([b.ids.account(a)]), ..Filters::default() }).portfolio.net_worth_history;
    let z = only("Z");
    assert_eq!(stated(&z[..4]), rows(&[("2026-10-05", "500", "0"), ("2026-10-06", "500", "0"), ("2026-10-07", "500", "0"), ("2026-10-08", "400", "0")]));
    assert!(z[4..z.len() - 1].iter().all(|n| n.assets.clone().unwrap().amount.is_zero()), "nothing after its last day");
    let a = only("A");
    assert_eq!(stated(&a[a.len() - 2..a.len() - 1]), rows(&[("2026-10-19", "1300", "0")]));
}

/// Under a range, the points in it; one ending before today, its last day's
/// figures are the tiles'.
#[test]
fn a_past_range_ends_at_its_last_day() {
    let (_, e) = engine_of(&case());
    let f = Filters { dates: bagholder_engine::scope::Dates::Range { from: Some(day("2026-10-06")), to: Some(day("2026-10-09")) }, ..Filters::default() };
    let pf = e.scope(&f).portfolio;
    assert_eq!(pf.net_worth_history.iter().map(|n| n.day.to_string()).collect::<Vec<_>>(), ["2026-10-06", "2026-10-07", "2026-10-08", "2026-10-09"]);
    let _ = pf;
}

/// A small generator for made-up books: the same seed, the same books.
struct Gen(u64);

impl Gen {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    /// An amount of cents in `-span..span`, as text.
    fn amount(&mut self, span: i64) -> String {
        let cents = (self.next() as i64 % (2 * span)) - span;
        Dec::new(cents, 2).unwrap().to_text()
    }
    fn coin(&mut self) -> bool {
        self.next() % 2 == 0
    }
}

fn sum(xs: &[Dec]) -> Dec {
    xs.iter().fold(Dec::ZERO, |t, x| t.checked_add(*x).unwrap())
}

/// Yesterday meets today: on a book where nothing moves between yesterday and
/// today and no price or rate changes, yesterday's point and today's (the tiles)
/// are the same, assets and debts both. Each generated book has a margin account
/// holding a stock, with cash in two currencies; a cash account holding it too; a
/// chequing account, a line of credit and a credit card, each stating a value or
/// not, the card's balance owed or in credit; any account's cash may be below
/// zero; every balance carried from before the first transaction. Today's net is
/// also each account's holdings and cash, worked out here on its own.
#[test]
fn yesterday_meets_today_when_nothing_moves() {
    let mut g = Gen(7);
    for n in 0..64 {
        let price = format!("{}.{:02}", 1 + g.next() % 300, g.next() % 100);
        let (qm, qc) = (1 + g.next() % 500, 1 + g.next() % 500);
        let ids = ["M", "C", "S", "L", "K"];
        // each account's cash now per currency, and whether its value is stated
        let mut cash: BTreeMap<&str, Vec<(&str, String)>> = BTreeMap::new();
        cash.insert("M", vec![("CAD", g.amount(5_000_000)), ("USD", g.amount(500_000))]);
        for a in &ids[1..] {
            cash.insert(a, vec![("CAD", g.amount(500_000))]);
        }
        let stated: BTreeMap<&str, bool> = ids.iter().map(|a| (*a, matches!(*a, "M" | "C") || g.coin())).collect();
        let usd = Dec::parse("1.40").unwrap();
        let p = Dec::parse(&price).unwrap();
        let held = |a: &str| -> Dec {
            match a {
                "M" => p.checked_mul(Dec::from_int(qm as i64)).unwrap(),
                "C" => p.checked_mul(Dec::from_int(qc as i64)).unwrap(),
                _ => Dec::ZERO,
            }
        };
        let in_cad = |c: &str, v: &str| -> Dec {
            let v = Dec::parse(v).unwrap();
            if c == "USD" { v.checked_mul(usd).unwrap() } else { v }
        };
        let value = |a: &str| sum(&[held(a), sum(&cash[a].iter().map(|(c, v)| in_cad(c, v)).collect::<Vec<_>>())]);
        let mut brokers = serde_json::Map::new();
        for a in ids {
            let mut b = json!({"cash": cash[a].iter().map(|(c, v)| (c.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(), "as_of": "2026-10-20T15:00:00Z"});
            if stated[a] {
                b["net_value"] = json!({"2026-10-19": value(a).to_text()});
                b["now"] = json!(value(a).to_text());
            }
            if a == "M" {
                b["buying_power"] = json!("0");
            }
            // yesterday's positions read of every account but the card and the line
            // of credit, whose balances are worked back: nothing moves since
            if !matches!(a, "L" | "K") {
                b["days_held"] = json!({"2026-10-19": {"cash": cash[a].iter().map(|(c, v)| (c.to_string(), json!(v))).collect::<serde_json::Map<_, _>>()}});
            }
            brokers.insert(a.to_string(), b);
        }
        // the moves that brought each account there, all before yesterday
        let mut transactions = vec![
            json!({"id": "b1", "account": "M", "day": "2026-09-02", "at": "2026-09-02T15:00:00Z", "kind": "buy", "instrument": "G", "qty": qm.to_string(), "cash": format!("-{}", held("M").to_text())}),
            json!({"id": "b2", "account": "C", "day": "2026-09-03", "at": "2026-09-03T15:00:00Z", "kind": "buy", "instrument": "G", "qty": qc.to_string(), "cash": format!("-{}", held("C").to_text())}),
        ];
        for (i, a) in ids.iter().enumerate() {
            for k in 0..(g.next() % 4) {
                let day = format!("2026-10-{:02}", 1 + g.next() % 17);
                let kind = if g.coin() { "deposit" } else { "withdrawal" };
                transactions.push(json!({"id": format!("t{i}-{k}"), "account": a, "day": day, "at": format!("{day}T15:00:00Z"), "kind": kind, "cash": g.amount(200_000)}));
            }
        }
        let c = json!({
            "name": "yesterday meets today",
            "today": "2026-10-20",
            "now": "2026-10-20T22:00:00Z",
            "accounts": [{"id": "M", "kind": "margin"}, {"id": "C"}, {"id": "S", "kind": "spending"}, {"id": "L", "kind": "line-of-credit"}, {"id": "K", "kind": "credit-card"}],
            "instruments": [{"id": "G"}],
            "quotes": {"G": {"price": price}},
            "rates": {"USD": {"2026-10-19": "1.40", "2026-10-20": "1.40"}},
            "brokers": brokers,
            "transactions": transactions,
        });
        let mut b = build(&c);
        let e = engine(&mut b);
        let pf = e.scope(&Filters::default()).portfolio;
        let days = &pf.net_worth_history;
        let (yesterday, today) = (&days[days.len() - 2], &days[days.len() - 1]);
        assert_eq!(yesterday.day, day("2026-10-19"), "book {n}");
        assert_eq!(yesterday.assets, today.assets, "book {n}: assets");
        assert_eq!(yesterday.debts, today.debts, "book {n}: debts");
        let net = sum(&ids.iter().map(|a| value(a)).collect::<Vec<_>>());
        assert_eq!(today.net().unwrap().amount, net, "book {n}: each account's holdings and cash");
    }
}

/// A card's balance owed by day is its statements', from a balance carried in
/// from before the first transaction read: worked back from what the broker
/// states now, never summed up from nothing. Each generated card opens owing a
/// balance no transaction holds, then is charged and paid on some days; another
/// account, stated from the first day, makes every day a point.
#[test]
fn a_cards_balance_owed_by_day_is_its_statements() {
    let mut g = Gen(11);
    for n in 0..32 {
        let opening = Dec::parse(&g.amount(1_000_000)).unwrap();
        // the card's statements: its balance at each day's close
        let mut balance = opening;
        let mut statements: BTreeMap<Date, Dec> = BTreeMap::new();
        let mut transactions = Vec::new();
        let mut d = day("2026-10-01");
        while d < day("2026-10-20") {
            if g.next() % 3 == 0 {
                let m = g.amount(100_000);
                balance = balance.checked_add(Dec::parse(&m).unwrap()).unwrap();
                let kind = if m.starts_with('-') { "withdrawal" } else { "deposit" };
                transactions.push(json!({"id": format!("k{d}"), "account": "K", "day": d.to_string(), "at": format!("{d}T15:00:00Z"), "kind": kind, "cash": m}));
            }
            statements.insert(d, balance);
            d = d.tomorrow().unwrap();
        }
        let c = json!({
            "name": "a card's balance by day",
            "today": "2026-10-20",
            "now": "2026-10-20T22:00:00Z",
            "accounts": [{"id": "A"}, {"id": "K", "kind": "credit-card"}],
            "instruments": [],
            "brokers": {
                "A": {"cash": {"CAD": "100"}, "net_value": {"2026-10-01": "100"}, "now": "100", "as_of": "2026-10-20T15:00:00Z"},
                "K": {"cash": {"CAD": balance.to_text()}, "as_of": "2026-10-20T15:00:00Z"}
            },
            "transactions": transactions,
        });
        let mut b = build(&c);
        let e = engine(&mut b);
        let days = e.scope(&Filters::default()).portfolio.net_worth_history;
        for nw in &days[..days.len() - 1] {
            assert_eq!(nw.debts.clone().unwrap().amount, statements[&nw.day].neg(), "book {n}, {}", nw.day);
            assert_eq!(nw.assets.clone().unwrap().amount, Dec::from_int(100), "book {n}, {}: the card holds nothing", nw.day);
        }
        assert_eq!(days.last().unwrap().debts.clone().unwrap().amount, balance.neg(), "book {n}: today");
    }
}

/// Net worth by day before today is kept from one change to the next
/// (`Engine::net_worth_past`), and what is kept is what a fresh working-out
/// gives, under every account set, after each kind of change that moves it: a
/// transaction added, a new rate, the debts held outside, what the broker
/// states, the day.
#[test]
fn the_days_kept_are_the_days_worked_out_afresh_after_every_change() {
    let (mut b, mut e) = engine_of(&case());
    let (a, bb) = (b.ids.account("A"), b.ids.account("B"));
    let sets = [BTreeSet::new(), BTreeSet::from([a]), BTreeSet::from([bb]), BTreeSet::from([a, bb])];
    let same = |e: &bagholder_engine::Engine, what: &str| {
        for s in &sets {
            assert_eq!(*e.net_worth_past(s), bagholder_engine::scope::net_worth_past(s, e.inputs()), "after {what}, over {s:?}");
        }
    };
    same(&e, "the first build");
    // the same book with each part changed, its ids the same as the labels' order
    // is; a deposit larger than the cash stated now, so the account owes before it
    let mut other = case();
    other["transactions"] = json!([{"id": "d1", "account": "A", "day": "2026-10-08", "at": "2026-10-08T15:00:00Z", "kind": "deposit", "cash": "2000"}]);
    other["rates"]["USD"]["2026-10-13"] = json!("1.45");
    other["brokers"]["B"]["net_value"]["2026-10-16"] = json!("115");
    other["today"] = json!("2026-10-21");
    other["now"] = json!("2026-10-21T22:00:00Z");
    let o = build(&other);
    let record = o.tx["d1"].record;
    let mut delta = bagholder_engine::input::LedgerDelta::default();
    delta.records.insert(record, Some((o.inputs.ledger.records[&record].clone(), o.inputs.ledger.transactions.of_record(record).cloned().collect())));
    e.apply(Change::Ledger(delta));
    same(&e, "a transaction added");
    e.apply(Change::Rates(o.inputs.facts.rates.clone()));
    same(&e, "a new rate");
    let mut debts = outside();
    debts.history.get_mut("home").unwrap().insert(day("2026-10-10"), cad("305000"));
    e.apply(Change::Debts(debts));
    same(&e, "the debts held outside");
    e.apply(Change::Broker(bb, o.inputs.market.brokers.get(&bb).cloned()));
    same(&e, "what the broker states");
    e.apply(Change::Clock(o.inputs.clock.clone()));
    same(&e, "the day");
}

/// Each of net worth's rows of the broker's own accounts names where it is
/// held: a card's, a line of credit's and any account's cash below zero by its
/// own account's broker, as the person calls it; the holdings, the cash waiting
/// to be invested, the chequing cash and the margin by the brokers of the
/// accounts behind each. Every kind of account, each at a broker of its own,
/// with cash above zero and below; no name is fixed in the code.
#[test]
fn each_row_of_the_broker_s_accounts_names_the_broker_it_is_held_at() {
    use bagholder_core::account::AccountKind;
    use bagholder_engine::input::BrokerAccount;
    let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/book.json")).unwrap();
    let mut case: Value = serde_json::from_str(&text).unwrap();
    let mut made = Vec::new();
    for kind in AccountKind::ALL {
        for sign in ["", "-"] {
            let label = format!("{}{}", kind.as_str(), if sign.is_empty() { "-up" } else { "-down" });
            case["accounts"].as_array_mut().unwrap().push(json!({ "id": label, "kind": kind.as_str() }));
            made.push((label, *kind, sign));
        }
    }
    let mut b = build(&case);
    // every account at a broker of its own, by a name no code knows
    let label_of = |l: &str| format!("Made-up Broker {l}");
    let ids: Vec<_> = b.inputs.ledger.accounts.keys().copied().collect();
    for id in ids {
        let l = b.ids.account_name(id);
        b.inputs.ledger.accounts.get_mut(&id).unwrap().broker_label = label_of(&l);
    }
    let mut e = engine(&mut b);
    for (l, _, sign) in &made {
        let id = b.ids.account(l);
        e.apply(Change::Broker(id, Some(BrokerAccount { cash: BTreeMap::from([(Currency::CAD, Dec::parse(&format!("{sign}100")).unwrap())]), ..BrokerAccount::default() })));
    }
    let pf = e.scope(&Filters::default()).portfolio;
    let labels = |ls: Vec<&str>| -> BTreeSet<String> { ls.into_iter().map(label_of).collect() };
    let of = |want: &dyn Fn(AccountKind, &str) -> bool| -> BTreeSet<String> { labels(made.iter().filter(|(_, k, s)| want(*k, s)).map(|(l, _, _)| l.as_str()).collect()) };
    let set = |v: &[String]| -> BTreeSet<String> { v.iter().cloned().collect() };
    let (_, owed) = pf.owed.clone().expect("owed");
    // each account's debt by its own account's broker
    let mut named = BTreeSet::new();
    for (d, _) in &owed {
        let id = b.inputs.ledger.accounts.keys().find(|id| d.id.starts_with(&id.to_string())).copied().expect("an account's debt");
        assert_eq!(d.institution, label_of(&b.ids.account_name(id)), "{}", d.name);
        named.insert(d.institution.clone());
    }
    let named: BTreeSet<String> = named.intersection(&of(&|_, _| true)).cloned().collect();
    // a card's and a line of credit's balance whichever its sign (in credit
    // above zero); any other account's cash below zero, a margin account's
    // being the margin
    let cards = |k: AccountKind| matches!(k, AccountKind::CreditCard | AccountKind::LineOfCredit);
    assert_eq!(named, of(&|k, s| cards(k) || (s == "-" && k != AccountKind::Margin)));
    assert_eq!(set(&pf.held_at.margin), of(&|k, s| s == "-" && k == AccountKind::Margin));
    assert_eq!(set(&pf.held_at.chequing_cash), of(&|k, s| s.is_empty() && k == AccountKind::Spending));
    assert_eq!(set(&pf.held_at.invested_cash).intersection(&of(&|_, _| true)).cloned().collect::<BTreeSet<_>>(), of(&|k, s| s.is_empty() && !matches!(k, AccountKind::Spending | AccountKind::CreditCard | AccountKind::LineOfCredit)));
    // the holdings by the brokers of the accounts holding them
    let holding: BTreeSet<String> = pf.positions.iter().map(|i| b.inputs.ledger.accounts[&e.figures().positions[*i].account].broker_label.clone()).collect();
    assert!(!holding.is_empty());
    assert_eq!(set(&pf.held_at.holdings), holding);
    for list in [&pf.held_at.holdings, &pf.held_at.invested_cash, &pf.held_at.chequing_cash, &pf.held_at.margin] {
        assert!(list.iter().all(|l| !l.is_empty()), "{list:?}");
    }
}
