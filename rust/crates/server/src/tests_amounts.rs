//! Every amount as Wealthsimple states it (brief 79,
//! `docs/plans/every-amount-as-wealthsimple-states-it.md` §5): the holdings the
//! page shows are the made-up Wealthsimple's, units and book value exactly,
//! whatever the history says; a holding no trade names is held; the units now
//! are the read asked as of today; a past day is the read of that day, made
//! once; and no sentence of the app says Bagholder's figures differ from
//! Wealthsimple's.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use bagholder_book::Book;
use bagholder_broker::pull::pull;
use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::Timestamp;
use bagholder_core::json::Value;
use bagholder_core::{Broker, Dec};
use bagholder_wealthsimple::adapter::{Source, Wealthsimple};
use bagholder_wealthsimple::generated::{Generated, Shape, Size, FILL_FORMS, SHAPES};

/// Each holding, by Bagholder's account and instrument (an account or a listing
/// Wealthsimple knows by two ids is one): its units (a short's negative), its
/// book value and that value's currency.
type Holdings = BTreeMap<(String, String), (Dec, Dec, String)>;

fn noon(day: Date) -> Timestamp {
    day.to_zoned(bagholder_core::jiff::tz::TimeZone::UTC).unwrap().timestamp().checked_add(bagholder_core::jiff::SignedDuration::from_hours(20)).unwrap()
}

/// What the made-up Wealthsimple states every account holds on `day`, read from
/// its answer as Wealthsimple writes it, each named as the book names it.
fn stated(ws: &mut Generated, day: Date, book: &Book) -> Holdings {
    let ids = ws.account_ids();
    let mut out = Holdings::new();
    for (ws_account, nodes) in ws.positions_of(&ids, day).unwrap() {
        let account = book.account_by_ref_value(&ws_account).unwrap().unwrap_or_else(|| panic!("{ws_account}: no account")).to_string();
        let Value::Array(nodes) = nodes else { panic!("{account}: no list of positions") };
        for n in nodes {
            let text = |path: &[&str]| -> String {
                let mut v = &n;
                for p in path {
                    let Value::Object(m) = v else { panic!("{path:?}") };
                    v = &m[*p];
                }
                match v {
                    Value::String(s) | Value::Number(s) => s.clone(),
                    other => panic!("{path:?}: {other:?}"),
                }
            };
            let security = text(&["security", "id"]);
            if security.starts_with("sec-c-") {
                continue;
            }
            let instrument = book.instrument_by_ref(&bagholder_core::instrument::Reference::new(bagholder_core::instrument::RefScheme::BrokerSecurity(Broker::named("wealthsimple")), security.clone())).unwrap().unwrap_or_else(|| panic!("{security}: no instrument")).to_string();
            let value = Dec::parse(&text(&["bookValue", "amount"])).unwrap().abs();
            let (units, currency) = (Dec::parse(&text(&["quantity"])).unwrap(), text(&["bookValue", "currency"]));
            // the same holding under each id of an account: once
            let e = out.entry((account.clone(), instrument)).or_insert((Dec::ZERO, Dec::ZERO, currency));
            e.0 = e.0.checked_add(units).unwrap();
            e.1 = e.1.checked_add(value).unwrap();
        }
    }
    out
}

/// The cash each account's positions read states on `day`, in CAD at `usd` a
/// US dollar, the positive balances only, as the Portfolio's Cash adds them.
fn stated_cash(ws: &mut Generated, day: Date, usd: Dec) -> Dec {
    let ids = ws.account_ids();
    let mut total = Dec::ZERO;
    for (_, nodes) in ws.positions_of(&ids, day).unwrap() {
        let Value::Array(nodes) = nodes else { panic!("no list of positions") };
        for n in nodes {
            let Value::Object(m) = &n else { panic!("a position not an object") };
            let Some(Value::Object(security)) = m.get("security") else { panic!("no security") };
            let Some(Value::String(id)) = security.get("id") else { panic!("no security id") };
            let rate = match id.as_str() {
                "sec-c-cad" => Dec::ONE,
                "sec-c-usd" => usd,
                _ => continue,
            };
            let Some(Value::String(q)) = m.get("quantity") else { panic!("no quantity") };
            let q = Dec::parse(q).unwrap();
            if q.is_positive() {
                total = total.checked_add(q.checked_mul(rate).unwrap()).unwrap();
            }
        }
    }
    total
}

/// What the page's data shows: the holdings document's rows and the Cashflow
/// holdings table's, each as `Holdings` (the table names a holding by its id).
fn shown(f: &crate::figures::Figures, book: &Book) -> (Holdings, BTreeMap<String, (Dec, Dec)>) {
    let a = crate::tests_common::app();
    let context = a.market_context().unwrap();
    let door = crate::wire::context::Door { built: &context, app: &a };
    let names = f.read(|e| crate::wire::build::Names::load(book, e.inputs())).unwrap().unwrap();
    let page = f.read(|e| crate::views::every_view(&crate::views::Cx { engine: e, names: &names, tables: &door, following: &context.following }, &Default::default())).unwrap().unwrap();
    let dec = |v: &serde_json::Value, what: &str| Dec::parse(v.as_str().unwrap_or_else(|| panic!("{what} not stated: {v}"))).unwrap();
    let mut holdings = Holdings::new();
    for p in page["positions"]["positions"].as_array().unwrap() {
        let qty = dec(&p["qty"], "units");
        let units = if p["short"] == true { qty.neg() } else { qty };
        let key = (p["accountId"].as_str().unwrap().to_string(), p["instrument"].as_str().unwrap().to_string());
        assert!(holdings.insert(key.clone(), (units, dec(&p["cost"], "book value"), p["bookCurrency"].as_str().unwrap().to_string())).is_none(), "{key:?} twice");
    }
    let table = page["cashflow"]["holdings"].as_array().map(|rows| rows.iter().map(|r| (r["id"].as_str().unwrap().to_string(), (dec(&r["qty"], "units"), dec(&r["cost"], "book value")))).collect()).unwrap_or_default();
    (holdings, table)
}

/// The history spoiled four ways, every balance and holding stated as before: a
/// buy removed, a fill doubled, a conversion's side paid unstated, and a month
/// of rows missing.
fn spoil(rows: &mut Vec<Value>) {
    let field = |r: &Value, k: &str| match r {
        Value::Object(m) => match m.get(k) {
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        },
        _ => String::new(),
    };
    let set = |r: &mut Value, k: &str, v: String| {
        if let Value::Object(m) = r {
            m.insert(k.to_string(), Value::String(v));
        }
    };
    if let Some(i) = rows.iter().position(|r| field(r, "type").ends_with("_BUY")) {
        rows.remove(i);
    }
    if let Some(mut again) = rows.iter().find(|r| field(r, "type").ends_with("_SELL")).cloned() {
        let id = field(&again, "canonicalId");
        set(&mut again, "canonicalId", format!("{id}-again"));
        rows.push(again);
    }
    if let Some(r) = rows.iter_mut().find(|r| field(r, "type") == "FUNDS_CONVERSION") {
        set(r, "externalCanonicalId", "a detail Wealthsimple does not answer".to_string());
    }
    let months: BTreeSet<String> = rows.iter().filter_map(|r| field(r, "occurredAt").get(..7).map(str::to_string)).collect();
    if let Some(month) = months.iter().nth(months.len() / 2).cloned() {
        rows.retain(|r| !field(r, "occurredAt").starts_with(&month));
    }
}

/// One book of the made-up Wealthsimple, every shape on, pulled on `today` with
/// its history spoiled or not: what it states and what the page shows.
fn pulled(seed: u64, today: Date, spoiled: bool) -> (Holdings, Holdings, BTreeMap<String, (Dec, Dec)>, usize) {
    let now = noon(today);
    let home = tempfile::tempdir().unwrap();
    let shapes: Vec<Shape> = SHAPES.iter().chain(FILL_FORMS.iter()).copied().collect();
    let mut made = Generated::shaped(Size { accounts: 8, instruments: 16, trades: 60, months: 12, seed, deep: 0 }, today, &shapes);
    if spoiled {
        spoil(made.rows_mut());
    }
    let mut ws = Wealthsimple::new(made);
    {
        let (book, _) = Book::open_in(home.path(), "test", now).unwrap();
        book.state_zone("America/Toronto", now).unwrap();
        let connection = book.add_connection(&Broker::named("wealthsimple"), "Wealthsimple", now).unwrap();
        pull(&book, &mut ws, connection, today, now, &mut |_| {}).unwrap();
    }
    let f = crate::figures::Figures::open(home.path(), now).unwrap();
    let book = f.book().unwrap();
    let (holdings, table) = shown(&f, &book);
    let trips = f.read(|e| e.figures().positions.iter().filter(|p| p.key.is_some()).count()).unwrap();
    (stated(&mut ws.source, today, &book), holdings, table, trips)
}

/// Test 1: over the made-up Wealthsimple, every shape at twelve seeds and on two
/// todays, each holding's units and book value in the page's data are what it
/// states, and the Cashflow table's are the holding's; the same with its history
/// spoiled four ways.
#[test]
fn a_book_whose_history_is_wrong_still_shows_wealthsimple_s_amounts_exactly() {
    let _g = crate::tests_common::guard();
    for today in ["2026-10-25", "2026-10-21"].map(|d| d.parse::<Date>().unwrap()) {
        for seed in 1..=12u64 {
            for spoiled in [false, true] {
                let at = format!("seed {seed}, {today}{}", if spoiled { ", the history spoiled" } else { "" });
                let (want, got, table, trips) = pulled(seed, today, spoiled);
                assert!(!want.is_empty(), "{at}: nothing held");
                assert!(trips > 0, "{at}: no holding links to a trade");
                let differ: Vec<_> = want.keys().chain(got.keys()).collect::<BTreeSet<_>>().into_iter().filter(|k| got.get(*k) != want.get(*k)).map(|k| (k, want.get(k), got.get(k))).collect();
                assert!(differ.is_empty(), "{at}: the holdings shown are not what Wealthsimple states (stated, shown): {differ:?}");
                for (id, (qty, cost)) in &table {
                    assert!(got.values().any(|(u, b, _)| u.abs() == *qty && b == cost), "{at}: the Cashflow table's {id} is no holding stated");
                }
            }
        }
    }
}

/// The recorded month of one account, pulled, and its figures open.
fn month(home: &Path) -> crate::figures::Figures {
    crate::tests_common::pulled_book(home);
    let now: Timestamp = "2025-11-19T21:00:00Z".parse().unwrap();
    let f = crate::figures::Figures::open(home, now).unwrap();
    f.state_zone("America/Toronto", now).unwrap();
    f
}

/// Test 8: a holding Wealthsimple states that no row of the history names is
/// held: its price is quoted, its payers read, and it is on the held lists the
/// news and filings read.
#[test]
fn a_holding_with_no_trade_is_quoted_and_held() {
    let _g = crate::tests_common::guard();
    let home = tempfile::tempdir().unwrap();
    let f = month(home.path());
    let book = f.book().unwrap();
    // the month starts mid-way: some holdings were bought before it, and no row names them
    let untraded: Vec<(bagholder_core::InstrumentId, bagholder_core::AccountId)> = f.read(|e| e.figures().positions.iter().filter(|p| p.key.is_none()).map(|p| (p.instrument, p.account)).collect()).unwrap();
    assert!(!untraded.is_empty(), "the month holds something it never traded");
    let needs = f.read(|e| e.needs()).unwrap();
    let held_by_rows: BTreeSet<_> = book.transactions().unwrap().iter().filter_map(|t| t.instrument).collect();
    for (i, _) in &untraded {
        assert!(!held_by_rows.contains(i) || needs.held.contains(i), "a row names it");
        assert!(needs.held.contains(i), "{i}: held and not quoted");
    }
    let a = crate::app::App::new(home.path().to_path_buf(), std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into());
    a.set_figures(f);
    let base = a.market_base().unwrap();
    let f = a.figures.get().unwrap();
    let names = f.read(|e| crate::wire::build::Names::load(&book, e.inputs())).unwrap().unwrap();
    let symbols: Vec<String> = f.read(|e| untraded.iter().map(|(i, _)| crate::status::symbol(e.inputs(), *i)).collect()).unwrap();
    drop(names);
    let filings: BTreeSet<String> = crate::filings::known_filing_listings(&a, &["held".to_string()]).unwrap().into_iter().map(|l| l.symbol).collect();
    for ((i, a), s) in untraded.iter().zip(&symbols) {
        assert!(base.positions.iter().any(|p| p.account_id == a.to_string() && p.id.contains(&i.to_string())), "{s}: not on the held list the news reads");
        // a contract by what it is written on, a listing by its bare symbol
        let bare = s.split([' ', '.']).next().unwrap_or(s);
        assert!(filings.iter().any(|x| x == s || x == bare), "{s}: not on the held list the filings read: {filings:?}");
    }
}

/// Test 6: the units now are the positions read asked as of today: an order
/// filled today, the next balances read shows the holding as that read states
/// it, before the feed lists the fill. The stand-in answers as of today in the
/// recorded form; whether Wealthsimple's own read states a fill made the same
/// day is not recorded (plan §2.1), so this proves Bagholder's side only.
#[test]
fn the_units_now_are_the_read_asked_as_of_today() {
    let _g = crate::tests_common::guard();
    let today: Date = "2026-10-21".parse().unwrap();
    let now = noon(today);
    let home = tempfile::tempdir().unwrap();
    let mut ws = Wealthsimple::new(Generated::shaped(Size { accounts: 4, instruments: 8, trades: 20, months: 3, seed: 3, deep: 0 }, today, &[]));
    let book = {
        let (book, _) = Book::open_in(home.path(), "test", now).unwrap();
        book.state_zone("America/Toronto", now).unwrap();
        let connection = book.add_connection(&Broker::named("wealthsimple"), "Wealthsimple", now).unwrap();
        pull(&book, &mut ws, connection, today, now, &mut |_| {}).unwrap();
        book
    };
    let connection = book.connections().unwrap()[0].id;
    let f = crate::figures::Figures::open(home.path(), now).unwrap();
    let (before, _) = shown(&f, &book);
    assert_eq!(before, stated(&mut ws.source, today, &book));
    let account = ws.source.account_ids().into_iter().find(|a| ws.source.cash_of(a).get("CAD").is_some_and(|c| c.is_positive())).expect("an account with cash");
    ws.source.filled_today(&account, 7);
    let read = bagholder_broker::pull::balances(&book, &mut ws, connection, now.checked_add(bagholder_core::jiff::SignedDuration::from_mins(5)).unwrap()).unwrap();
    assert!(read.failures.is_empty(), "{:?}", read.failures);
    for a in &read.accounts {
        f.broker_changed(*a).unwrap();
    }
    let (after, _) = shown(&f, &book);
    assert_ne!(after, before, "the fill moved nothing");
    assert_eq!(after, stated(&mut ws.source, today, &book), "the holdings are the read as of today");
}

/// Test 5: a past Portfolio is the positions read as of its day: its units,
/// book value and cash are what that read states, and the day is read once,
/// however often it is opened.
#[test]
fn a_past_day_is_wealthsimple_s_and_is_read_once() {
    let _g = crate::tests_common::guard();
    let today: Date = "2026-10-21".parse().unwrap();
    let day: Date = "2026-09-15".parse().unwrap();
    let now = noon(today);
    let home = tempfile::tempdir().unwrap();
    let shapes: Vec<Shape> = SHAPES.iter().copied().collect();
    let mut ws = Wealthsimple::new(Generated::shaped(Size { accounts: 4, instruments: 8, trades: 30, months: 6, seed: 5, deep: 0 }, today, &shapes));
    let book = {
        let (book, _) = Book::open_in(home.path(), "test", now).unwrap();
        book.state_zone("America/Toronto", now).unwrap();
        let connection = book.add_connection(&Broker::named("wealthsimple"), "Wealthsimple", now).unwrap();
        pull(&book, &mut ws, connection, today, now, &mut |_| {}).unwrap();
        // the Bank's rate that day, so the US dollars count
        book.store_rates(bagholder_core::Currency::USD, &[(day, Dec::parse("1.35").unwrap())], (day, day), &bagholder_core::SourceName::named("bank-of-canada"), now).unwrap();
        book
    };
    let f = crate::figures::Figures::open(home.path(), now).unwrap();
    let connection = book.connections().unwrap()[0].id;
    let on_the_day = bagholder_engine::scope::Filters { dates: bagholder_engine::scope::Dates::Range { from: None, to: Some(day) }, ..Default::default() };
    let past = f.read(|e| e.past(day)).unwrap();
    assert!(past.positions.is_empty(), "a day not read yet holds nothing yet");
    assert!(f.read(|e| e.needs().holdings_on.contains(&day)).unwrap(), "a past day a screen holds is asked for");
    let asked = ws.source.asked();
    let (accounts, failures) = bagholder_broker::pull::holdings_as_of(&book, &mut ws, connection, day, now).unwrap();
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(ws.source.asked() - asked, 1, "one request for every account");
    for a in accounts {
        f.holdings_read(a).unwrap();
    }
    assert!(!f.read(|e| e.needs().holdings_on.contains(&day)).unwrap(), "a day read is never asked for again");
    drop(past);
    let past = f.read(|e| e.past(day)).unwrap();
    let got: Holdings = past
        .positions
        .iter()
        .map(|p| {
            let units = if p.direction == bagholder_engine::ledger::Direction::Short { p.qty.as_ref().unwrap().neg() } else { *p.qty.as_ref().unwrap() };
            let book_value = p.book.as_ref().unwrap();
            ((p.account.to_string(), p.instrument.to_string()), (units, book_value.amount, book_value.currency.to_string()))
        })
        .collect();
    assert!(!got.is_empty(), "the day held something");
    assert_eq!(got, stated(&mut ws.source, day, &book));
    // the Portfolio's Cash that day: what the read states, not worked back from today's
    let cash = f.read(|e| e.holdings(&on_the_day).cash).unwrap().unwrap();
    assert_eq!(cash.amount, stated_cash(&mut ws.source, day, Dec::parse("1.35").unwrap()), "the cash that day is not what Wealthsimple states");
    // opened again, from a new opening of the book: nothing is asked
    drop(past);
    drop(f);
    let f = crate::figures::Figures::open(home.path(), now).unwrap();
    let _past = f.read(|e| e.past(day)).unwrap();
    assert!(!f.read(|e| e.needs().holdings_on.contains(&day)).unwrap(), "a day kept is never asked for again");
}

/// Test 3: no sentence of the app says Bagholder's figures differ from
/// Wealthsimple's: the server's and the page's code holds none of the words the
/// comparing lines used.
#[test]
fn no_sentence_says_the_figures_differ() {
    // the comparing lines' own words: the units line, the month that did not
    // reconcile, the movements no statement listed
    const SAID: [&str; 4] = ["states {}, the book", "cannot count its units", "statement the book does not reconcile", "statement lists"];
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let mut found = Vec::new();
    let mut stack = vec![root.join("rust/crates/server/src"), root.join("web/src")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().map(Result::unwrap) {
            let path = entry.path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            // this test, and the page's tests, which may hold the words to say they are gone
            if name == "tests_amounts.rs" || name.contains(".test.") || !["rs", "ts", "svelte"].iter().any(|e| name.ends_with(e)) {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for (n, line) in text.lines().enumerate() {
                // code only: a comment may say what went
                let code = line.split("//").next().unwrap_or_default();
                if code.trim_start().starts_with('*') || code.trim_start().starts_with("/*") {
                    continue;
                }
                for s in SAID {
                    if code.contains(s) {
                        found.push(format!("{}:{} {s}", path.strip_prefix(&root).unwrap().display(), n + 1));
                    }
                }
            }
        }
    }
    assert_eq!(found, Vec::<String>::new(), "a sentence says Bagholder's figures differ from Wealthsimple's");
}
