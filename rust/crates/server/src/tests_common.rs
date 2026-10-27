//! Shared harness for the ported server tests: one app() on one temp home,
//! and one lock every test touching global state takes.
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::app::App;

static LOCK: Mutex<()> = Mutex::new(());
static APP: OnceLock<Arc<App>> = OnceLock::new();

/// A folder of this run's own, removed as the run exits: for the homes of the
/// apps the tests share, which no one test's end can take. A run stopped by a
/// signal leaves it.
pub fn run_folder(name: &str) -> PathBuf {
    static RUN: OnceLock<PathBuf> = OnceLock::new();
    extern "C" {
        fn atexit(f: extern "C" fn()) -> std::os::raw::c_int;
    }
    extern "C" fn remove() {
        if let Some(d) = RUN.get() {
            let _ = std::fs::remove_dir_all(d);
        }
    }
    let run = RUN.get_or_init(|| {
        let d = tempfile::Builder::new().prefix("bh-server-tests-").tempdir().unwrap().keep();
        // SAFETY: atexit keeps a function to call once, as the process exits
        assert_eq!(unsafe { atexit(remove) }, 0);
        d
    });
    let d = run.join(name);
    std::fs::create_dir_all(&d).unwrap();
    d
}

pub fn home() -> PathBuf {
    static H: OnceLock<PathBuf> = OnceLock::new();
    H.get_or_init(|| {
        let d = run_folder("home");
        std::env::set_var("BAGHOLDER_DRY_ORDERS", "1");
        // nothing this binary does reaches beyond loopback; the Wealthsimple
        // stand-in (`bagholder_ws::standin`) answers on 127.0.0.1, which stays
        // allowed, so it still works
        std::env::set_var("BAGHOLDER_OFFLINE", "1");
        d
    })
    .clone()
}

/// The shared test app, made once, whoever asks first.
fn made() -> &'static Arc<App> {
    APP.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let app = App::new(home(), root, "127.0.0.1".into());
        // the figure path, on a month of one account's recorded replies of its own
        let book = home().join("figures");
        std::fs::create_dir_all(&book).unwrap();
        pulled_book(&book);
        let now = bagholder_core::jiff::Timestamp::now();
        let f = crate::figures::Figures::open(&book, now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        app.set_figures(f);
        app
    })
}

/// Serialize and make sure the shared test app exists.
pub fn guard() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    made();
    g
}

/// The one shared test app.
pub fn app() -> Arc<App> {
    made().clone()
}

/// The same app as a genuine `'static` reference: for a local `db()`/`conn()`
/// test helper that returns a pooled connection borrowed from it (the pool
/// ties a `Pooled<'_>` to the `&App` that opened it, and only a `'static`
/// reference lets that claim `Pooled<'static>` as the old global did).
pub fn app_ref() -> &'static App {
    made()
}

/// A book in `home` holding one account's month, pulled from the recorded
/// Wealthsimple replies (`wealthsimple/tests/replies/wealthsimple-pull`).
pub fn pulled_book(home: &std::path::Path) {
    use bagholder_core::Broker;
    let at: bagholder_core::jiff::Timestamp = "2025-11-19T20:00:00Z".parse().unwrap();
    let (book, _) = bagholder_book::Book::open_in(home, crate::app::APP_VERSION, at).unwrap();
    let connection = book.add_connection(&Broker::named("wealthsimple"), "Wealthsimple", at).unwrap();
    let replies = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../wealthsimple/tests/replies/wealthsimple-pull");
    let mut ws = bagholder_wealthsimple::adapter::Wealthsimple::new(bagholder_wealthsimple::replay::Replay::read(&replies).unwrap().taken_before_statements());
    let r = bagholder_broker::pull::pull(&book, &mut ws, connection, "2025-11-19".parse().unwrap(), at, &mut |_| {}).unwrap();
    assert!(r.failures.is_empty(), "{:?}", r.failures);
}

/// The same accounts in the book the ticket reads, beside the shared test book's
/// own month: each added once, the margin account's buying power stated again.
pub fn order_accounts_in_book() {
    order_accounts_in(&app());
}

/// The ticket's accounts in `a`'s book, as `order_accounts_in_book`: an app of a
/// test's own gets its figures built and a Wealthsimple connection first.
pub fn order_accounts_in(a: &Arc<App>) {
    use bagholder_core::account::{AccountKind, AccountRef, AccountStatus, AccountType, Registration};
    use bagholder_core::{Broker, Currency, Dec, Money};
    let f = a.figures.get().unwrap();
    let book = f.book().unwrap();
    let now = bagholder_core::jiff::Timestamp::now();
    if f.read(|_| ()).is_none() {
        f.state_zone("America/Toronto", now).unwrap();
    }
    let ws = Broker::named("wealthsimple");
    let conn = match book.connections().unwrap().into_iter().find(|c| c.broker == ws) {
        Some(c) => c.id,
        None => book.add_connection(&ws, "Wealthsimple", now).unwrap(),
    };
    let accounts = [
        ("acct-margin", "Trading", AccountKind::Margin, Registration::Unregistered, false, AccountStatus::Open),
        ("acct-tfsa", "TFSA", AccountKind::Cash, Registration::Tfsa, false, AccountStatus::Open),
        ("acct-crypto", "Crypto", AccountKind::Crypto, Registration::Unregistered, false, AccountStatus::Open),
        ("acct-old", "Old", AccountKind::Cash, Registration::Rrsp, false, AccountStatus::Closed),
        ("acct-managed", "Managed", AccountKind::Cash, Registration::Tfsa, true, AccountStatus::Open),
    ];
    for (key, nickname, kind, registration, managed, status) in accounts {
        let r = AccountRef::new(ws.clone(), key);
        if book.account_by_ref(&r).unwrap().is_none() {
            book.add_account(conn, &[r], &AccountType::Known { kind, registration, managed, joint: false }, status, Some(nickname), now).unwrap();
        }
    }
    let margin = book.account_by_ref(&AccountRef::new(ws.clone(), "acct-margin")).unwrap().unwrap();
    let read = book.broker_read(conn, "buying-power", now).unwrap();
    book.store_buying_power(margin, now, &Ok(Money::new(Dec::parse("12680.45").unwrap(), Currency::CAD)), &read).unwrap();
    // its yearly margin interest rates, made up
    let rates = book.broker_read(conn, "margin-rates", now).unwrap();
    book.store_margin_rates(margin, now, &[(Currency::CAD, Dec::parse("0.0495").unwrap()), (Currency::USD, Dec::parse("0.0695").unwrap())].into(), &rates).unwrap();
    // the TFSA backs the margin account (its Margin Boost)
    let tfsa = book.account_by_ref(&AccountRef::new(ws.clone(), "acct-tfsa")).unwrap().unwrap();
    let accounts = book.broker_read(conn, "accounts", now).unwrap();
    book.store_margin_backing(conn, &[(tfsa, margin)], &accounts).unwrap();
    f.record_changed(now).unwrap();
    // what the margin account can borrow, as a balances read brings it to the figures
    f.broker_changed(margin).unwrap();
}

/// Wealthsimple's mapping a version back, reading a conversion whose detail
/// states its side paid for the side received alone, as a person's book was
/// read before rule set 16: the statements' walk then books the side paid from
/// the statement, and the version after derives it from the record as well.
pub struct PaidUnread;
impl bagholder_book::mapping::Mapping for PaidUnread {
    fn source(&self) -> bagholder_core::SourceName {
        bagholder_wealthsimple::mapping::WealthsimpleMapping.source()
    }
    fn version(&self) -> u32 {
        bagholder_wealthsimple::mapping::WealthsimpleMapping.version() - 1
    }
    fn map(&self, ctx: &bagholder_book::mapping::MapContext, payload: &str) -> bagholder_book::mapping::Mapped {
        let mut m = bagholder_wealthsimple::mapping::WealthsimpleMapping.map(ctx, payload);
        let paid = bagholder_core::Leg::parse("paid").unwrap();
        m.legs.retain(|d| !(d.kind == bagholder_core::transaction::Kind::CurrencyConversion && d.leg == paid));
        m
    }
}

/// Wealthsimple as the made-up broker states it, its records derived by
/// `PaidUnread`: everything else as the adapter does it.
pub struct ReadBefore(pub bagholder_wealthsimple::adapter::Wealthsimple<bagholder_wealthsimple::generated::Generated>);
impl bagholder_broker::BrokerAdapter for ReadBefore {
    fn broker(&self) -> bagholder_core::Broker {
        self.0.broker()
    }
    fn mapping(&self) -> &dyn bagholder_book::mapping::Mapping {
        &PaidUnread
    }
    fn accounts(&mut self) -> bagholder_broker::Answer<Vec<bagholder_broker::AccountStated>> {
        self.0.accounts()
    }
    fn activity(&mut self, account: &str, from: Option<bagholder_core::jiff::civil::Date>) -> bagholder_broker::Answer<bagholder_broker::Activity> {
        self.0.activity(account, from)
    }
    fn prepare(&mut self, rows: &[&bagholder_broker::Row]) {
        self.0.prepare(rows)
    }
    fn read_beside(&mut self) -> Vec<bagholder_broker::Row> {
        self.0.read_beside()
    }
    fn record(&mut self, row: &bagholder_broker::Row, book: &mut dyn bagholder_broker::BookMoves) -> bagholder_broker::Answer<bagholder_core::json::Value> {
        self.0.record(row, book)
    }
    fn holds(&self, payload: &bagholder_core::json::Value, row: &bagholder_broker::Row) -> bool {
        self.0.holds(payload, row)
    }
    fn reads_positions(&self, payload: &bagholder_core::json::Value) -> bool {
        self.0.reads_positions(payload)
    }
    fn unsettled(&self, payload: &bagholder_core::json::Value) -> Option<(String, Option<bagholder_core::jiff::civil::Date>)> {
        self.0.unsettled(payload)
    }
    fn placed(&self, payload: &bagholder_core::json::Value) -> Option<(String, bagholder_core::jiff::civil::Date)> {
        self.0.placed(payload)
    }
    fn refreshes(&self, payload: &bagholder_core::json::Value) -> bool {
        self.0.refreshes(payload)
    }
    fn refresh(&mut self, payload: &bagholder_core::json::Value, book: &mut dyn bagholder_broker::BookMoves) -> Option<bagholder_broker::Answer<bagholder_core::json::Value>> {
        self.0.refresh(payload, book)
    }
    fn watches(&self, payload: &bagholder_core::json::Value) -> Option<String> {
        self.0.watches(payload)
    }
    fn amend(&self, payload: &bagholder_core::json::Value, row: &bagholder_broker::Row) -> Option<bagholder_core::json::Value> {
        self.0.amend(payload, row)
    }
    fn day(&self, at: bagholder_core::jiff::Timestamp) -> bagholder_core::jiff::civil::Date {
        self.0.day(at)
    }
    fn cash(&mut self, accounts: &[String]) -> bagholder_broker::Answer<std::collections::BTreeMap<String, std::collections::BTreeMap<bagholder_core::Currency, bagholder_core::Dec>>> {
        self.0.cash(accounts)
    }
    fn buying_power(&mut self, accounts: &[String]) -> bagholder_broker::Answer<std::collections::BTreeMap<String, Result<bagholder_core::Dec, String>>> {
        self.0.buying_power(accounts)
    }
    fn margin_rates(&mut self, account: &str) -> bagholder_broker::Answer<Option<std::collections::BTreeMap<bagholder_core::Currency, bagholder_core::Dec>>> {
        self.0.margin_rates(account)
    }
    fn units(&mut self, account: &str, day: bagholder_core::jiff::civil::Date) -> bagholder_broker::Answer<Vec<bagholder_broker::Units>> {
        self.0.units(account, day)
    }
    fn cash_on(&mut self, account: &str, day: bagholder_core::jiff::civil::Date) -> bagholder_broker::Answer<Option<std::collections::BTreeMap<bagholder_core::Currency, bagholder_core::Dec>>> {
        self.0.cash_on(account, day)
    }
    fn outside_accounts(&mut self) -> bagholder_broker::Answer<Option<Vec<bagholder_book::debts::OutsideDebt>>> {
        self.0.outside_accounts()
    }
    fn outside_days(&mut self, entity: &bagholder_book::debts::OutsideDebt, start: bagholder_core::jiff::civil::Date, end: bagholder_core::jiff::civil::Date) -> bagholder_broker::Answer<Option<Vec<bagholder_book::debts::OutsideDay>>> {
        self.0.outside_days(entity, start, end)
    }
    fn instruments(&mut self, refs: &[bagholder_core::instrument::Reference], day: bagholder_core::jiff::civil::Date) -> Vec<(bagholder_core::instrument::Reference, bagholder_broker::Answer<bagholder_book::mapping::InstrumentDraft>)> {
        self.0.instruments(refs, day)
    }
    fn history(&mut self, account: &str, from: Option<bagholder_core::jiff::civil::Date>) -> bagholder_broker::Answer<Vec<bagholder_broker::DayValue>> {
        self.0.history(account, from)
    }
    fn statement(&mut self, account: &str, month: bagholder_core::jiff::civil::Date) -> bagholder_broker::Answer<bagholder_broker::StatementRead> {
        self.0.statement(account, month)
    }
    fn statement_rows(&self, account: &str, payload: &bagholder_core::json::Value) -> bagholder_broker::Answer<Vec<bagholder_broker::StatementRow>> {
        self.0.statement_rows(account, payload)
    }
    fn statement_mapping(&self) -> Option<&dyn bagholder_book::mapping::Mapping> {
        self.0.statement_mapping()
    }
    fn statement_record(&self, account: &str, month: bagholder_core::jiff::civil::Date, position: usize, row: &bagholder_broker::StatementRow) -> Option<(String, bagholder_core::json::Value)> {
        self.0.statement_record(account, month, position, row)
    }
    fn statement_gap_record(&self, gap: &bagholder_broker::statements::Gap) -> Option<(String, bagholder_core::json::Value)> {
        self.0.statement_gap_record(gap)
    }
    fn conversion_paid_record(&self, paid: &bagholder_broker::statements::Paid) -> Option<(String, bagholder_core::json::Value)> {
        self.0.conversion_paid_record(paid)
    }
    fn paid_of(&self, payload: &bagholder_core::json::Value) -> Option<bagholder_broker::statements::Paid> {
        self.0.paid_of(payload)
    }
    fn fill_record(&self, fill: &bagholder_broker::statements::Fill) -> Option<(String, bagholder_core::json::Value)> {
        self.0.fill_record(fill)
    }
}
