//! Shared harness for the ported server tests: one app() on one temp home,
//! and one lock every test touching global state takes.
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::app::App;

static LOCK: Mutex<()> = Mutex::new(());
static APP: OnceLock<Arc<App>> = OnceLock::new();

pub fn home() -> PathBuf {
    static H: OnceLock<PathBuf> = OnceLock::new();
    H.get_or_init(|| {
        let d = std::env::temp_dir().join(format!("bh-server-tests-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
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
        bagholder_store::schema::init_schema(&app.open().unwrap()).unwrap();
        // the figure path, on a month of one account's recorded replies of its own
        let book = home().join("figures");
        std::fs::create_dir_all(&book).unwrap();
        pulled_book(&book);
        let now = bagholder_core::jiff::Timestamp::now();
        let f = crate::figures::Figures::open(&book, now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        let _ = app.figures.set(f);
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
    let mut ws = bagholder_wealthsimple::adapter::Wealthsimple::new(bagholder_wealthsimple::replay::Replay::read(&replies).unwrap());
    let r = bagholder_broker::pull::pull(&book, &mut ws, connection, "2025-11-19".parse().unwrap(), at).unwrap();
    assert!(r.failures.is_empty(), "{:?}", r.failures);
}

/// The same accounts in the book the ticket reads, beside the shared test book's
/// own month: each added once, the margin account's buying power stated again.
pub fn order_accounts_in_book() {
    use bagholder_core::account::{AccountKind, AccountRef, AccountStatus, AccountType, Registration};
    use bagholder_core::{Broker, Currency, Dec, Money};
    let a = app();
    let f = a.figures.get().unwrap();
    let book = f.book().unwrap();
    let now = bagholder_core::jiff::Timestamp::now();
    let ws = Broker::named("wealthsimple");
    let conn = book.connections().unwrap().into_iter().find(|c| c.broker == ws).unwrap().id;
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
    f.record_changed(now).unwrap();
}
