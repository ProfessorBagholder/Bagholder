//! Clear data (`docs/plans/stage-3c-switch.md` §7; `SPEC.md` §3, the menu): the
//! person ticks the kinds of data to delete from this machine, and each is
//! emptied from every store that holds it, one transaction per store. Every table
//! of the book and the market cache belongs to exactly one kind, in the lists here
//! and in `bagholder_book::clear::TABLES`; a test fails on a table none places.
//! The earlier store's tables are placed in `carry::OLD_TABLES`: what it held was
//! carried into these two at the first start of this build, and a kind cleared
//! is emptied from it too (`carry::clear_old_store`), its carry flags kept so
//! nothing comes back at the next start.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use bagholder_book::clear::Clearing;

use crate::app::App;
use crate::figures::Figures;

/// A kind of data the person can clear.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// Wealthsimple's records: activity, statements, balances, history, and the pull's own state.
    Broker,
    /// What the person entered: Add trade, opening balances, event values, imported files.
    Entries,
    /// Notes, grades, tags and saved groups.
    Journal,
    /// What the market sources answered: rates, prices, bars, distributions, news, filings.
    Market,
    /// Orders placed through Bagholder and their brackets.
    Orders,
    /// The watchlist, the Markets tiles, and the notifications and their settings
    /// (all the book's).
    Settings,
    /// The saved Wealthsimple login (`session.json`).
    Login,
}

/// A table of the market cache, and what it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Of(Kind),
    /// The cache's `meta`: each key placed by `meta_kind`.
    Meta,
    /// Kept: the file's own version, or its record of what changed.
    Kept,
}

/// Every table of the market cache.
pub const CACHE_TABLES: &[(&str, Place)] = &[
    ("quotes", Place::Of(Kind::Market)),
    ("quoted_ex_dividends", Place::Of(Kind::Market)),
    ("daily_closes", Place::Of(Kind::Market)),
    ("benchmark_closes", Place::Of(Kind::Market)),
    ("benchmark_events", Place::Of(Kind::Market)),
    ("option_chains", Place::Of(Kind::Market)),
    ("chains", Place::Of(Kind::Market)),
    ("outcomes", Place::Of(Kind::Market)),
    ("reads", Place::Of(Kind::Market)),
    // the earlier readers' (migration 4)
    ("news", Place::Of(Kind::Market)),
    ("filings", Place::Of(Kind::Market)),
    ("exposures", Place::Of(Kind::Market)),
    ("gauges", Place::Of(Kind::Market)),
    ("shorts", Place::Of(Kind::Market)),
    ("universes", Place::Of(Kind::Market)),
    ("price_history", Place::Of(Kind::Market)),
    ("history_fetches", Place::Of(Kind::Market)),
    ("price_bars", Place::Of(Kind::Market)),
    ("bar_fetches", Place::Of(Kind::Market)),
    ("meta", Place::Meta),
    ("gen", Place::Kept),
    ("schema_migrations", Place::Kept),
];

/// The market cache's `meta` keys the app keeps whatever is cleared: the last
/// update check, and the mark that the earlier store was carried (so what it held
/// is never carried again over what was cleared).
pub const META_KEPT: &[&str] = &[crate::update::CHECK_KEY, crate::carry::CARRIED];

/// A cache `meta` key's kind, or `None` for one the app keeps. Every other key is
/// what a source answered (a symbol's form at an exchange, a miss remembered for
/// the day): market data.
pub fn meta_kind(key: &str) -> Option<Kind> {
    (!META_KEPT.contains(&key)).then_some(Kind::Market)
}

/// Why nothing was cleared.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// A pull is running: what it writes would land in what was just emptied.
    Pulling,
    /// A bracket is live: deleting its record, or the login that watches it,
    /// would leave its stop resting at Wealthsimple with nothing watching it.
    BracketLive(String),
    Failed(String),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::Pulling => write!(f, "Wealthsimple is being read. Clear data once it finishes."),
            Refused::BracketLive(what) => write!(f, "The bracket on {what} is live. Cancel it in the Orders panel first."),
            Refused::Failed(why) => write!(f, "{why}"),
        }
    }
}

/// The book's part of the kinds ticked.
pub fn book_clearing(kinds: &[Kind]) -> Clearing {
    Clearing { broker: kinds.contains(&Kind::Broker), entries: kinds.contains(&Kind::Entries), journal: kinds.contains(&Kind::Journal), market: kinds.contains(&Kind::Market), orders: kinds.contains(&Kind::Orders), following: kinds.contains(&Kind::Settings) }
}

/// Empty the market cache's tables and `meta` keys of `kinds`, in one transaction.
pub fn clear_cache(cache: &bagholder_sources::cache::MarketCache, kinds: &[Kind]) -> Result<(), String> {
    let conn = cache.connection();
    let e = |e: rusqlite::Error| e.to_string();
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate).map_err(e)?;
    for (table, place) in CACHE_TABLES {
        if let Place::Of(k) = place {
            if kinds.contains(k) {
                conn.execute(&format!("DELETE FROM {table}"), []).map_err(e)?;
            }
        }
    }
    let keys: Vec<String> = conn.prepare("SELECT key FROM meta").map_err(e)?.query_map([], |r| r.get(0)).map_err(e)?.collect::<rusqlite::Result<_>>().map_err(e)?;
    for key in keys {
        if meta_kind(&key).is_some_and(|k| kinds.contains(&k)) {
            conn.execute("DELETE FROM meta WHERE key = ?", [&key]).map_err(e)?;
        }
    }
    tx.commit().map_err(e)
}

/// The live bracket's symbol, if one is.
fn live_bracket(book: &bagholder_book::Book) -> Result<Option<String>, String> {
    let live = book.live_brackets().map_err(|e| e.to_string())?;
    Ok(live.first().map(|b| b.place.symbol.clone()))
}

/// Clear what `kinds` ticks, and build the figures again.
pub fn clear(app: &Arc<App>, f: &Figures, kinds: &[Kind], now: bagholder_core::jiff::Timestamp) -> Result<(), Refused> {
    if app.state.lock().unwrap().syncing {
        return Err(Refused::Pulling);
    }
    let book = f.book().map_err(Refused::Failed)?;
    if kinds.contains(&Kind::Orders) || kinds.contains(&Kind::Login) {
        if let Some(symbol) = live_bracket(&book).map_err(Refused::Failed)? {
            return Err(Refused::BracketLive(symbol));
        }
    }
    book.clear(&book_clearing(kinds)).map_err(|e| Refused::Failed(e.to_string()))?;
    // what a page kept of the book as it was is not shown again
    book.renew_id(now).map_err(|e| Refused::Failed(e.to_string()))?;
    clear_cache(&f.cache().map_err(Refused::Failed)?, kinds).map_err(Refused::Failed)?;
    // and what the earlier store still holds of them: a cleared kind leaves no copy on this machine
    crate::carry::clear_old_store(&app.home, kinds).map_err(Refused::Failed)?;
    // and the copies a migration or a carry kept of the files as they were, which hold
    // every kind: restoring one would bring back what was just cleared
    // (`snapshots/` beside the stores, `bagholder_sqlite::migrate::snapshot_dir`)
    match std::fs::remove_dir_all(app.home.join("snapshots")) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(Refused::Failed(format!("the snapshots of the data as it was could not be removed: {e}"))),
    }
    if kinds.contains(&Kind::Login) {
        crate::session::delete_session(app).map_err(Refused::Failed)?;
    }
    if kinds.contains(&Kind::Broker) {
        let mut st = app.state.lock().unwrap();
        st.last_sync.clear();
        st.error.clear();
    }
    f.rebuild(now).map_err(Refused::Failed)?;
    if kinds.contains(&Kind::Settings) {
        // the tile row is the default six again, until the person changes it
        crate::following::open(app);
    }
    app.events.signal();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bagholder_book::clear::{Holds, TABLES};
    use std::collections::BTreeMap;

    const ALL: [Kind; 7] = [Kind::Broker, Kind::Entries, Kind::Journal, Kind::Market, Kind::Orders, Kind::Settings, Kind::Login];

    fn now() -> bagholder_core::jiff::Timestamp {
        "2025-11-19T21:30:00Z".parse().unwrap()
    }

    /// An app of its own on a home of its own: the earlier store, the book holding a
    /// pulled month, and the figures built.
    fn app() -> (tempfile::TempDir, Arc<App>) {
        let home = tempfile::tempdir().unwrap();
        crate::tests_common::home();
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let app = App::new(home.path().to_path_buf(), root, "127.0.0.1".into());
        crate::tests_common::pulled_book(home.path());
        let at: bagholder_core::jiff::Timestamp = "2025-11-19T21:00:00Z".parse().unwrap();
        let f = Figures::open(home.path(), at).unwrap();
        f.state_zone("America/Toronto", at).unwrap();
        app.set_figures(f);
        (home, app)
    }

    fn tables(conn: &rusqlite::Connection) -> Vec<String> {
        conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap().query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
    }

    fn counts(conn: &rusqlite::Connection) -> BTreeMap<String, i64> {
        tables(conn).into_iter().map(|t| (t.clone(), conn.query_row(&format!("SELECT COUNT(*) FROM \"{t}\""), [], |r| r.get(0)).unwrap())).collect()
    }

    /// One row in every table that has none, each column given a value of its type.
    fn fill(conn: &rusqlite::Connection) {
        // a stand-in row: what it says is not read, only that it is there
        conn.execute_batch("PRAGMA ignore_check_constraints = ON; PRAGMA foreign_keys = OFF;").unwrap();
        for t in tables(conn) {
            let n: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM \"{t}\""), [], |r| r.get(0)).unwrap();
            if n > 0 {
                continue;
            }
            let cols: Vec<(String, String)> = conn.prepare(&format!("PRAGMA table_info(\"{t}\")")).unwrap().query_map([], |r| Ok((r.get(1)?, r.get::<_, String>(2)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
            let names = cols.iter().map(|(c, _)| format!("\"{c}\"")).collect::<Vec<_>>().join(", ");
            let values = cols.iter().map(|(_, ty)| if ty.to_uppercase().contains("INT") || ty.to_uppercase().contains("REAL") { "1" } else { "'x'" }).collect::<Vec<_>>().join(", ");
            conn.execute(&format!("INSERT INTO \"{t}\" ({names}) VALUES ({values})"), []).unwrap_or_else(|e| panic!("{t}: {e}"));
        }
        conn.execute_batch("PRAGMA ignore_check_constraints = OFF;").unwrap();
    }

    fn cache_place(t: &str) -> Place {
        CACHE_TABLES.iter().find(|(n, _)| *n == t).unwrap_or_else(|| panic!("the market cache's table {t} is in no kind")).1
    }

    /// The earlier store in `app`'s data folder, read only.
    fn old_store(app: &App) -> rusqlite::Connection {
        rusqlite::Connection::open_with_flags(app.home.join(crate::figures::OLD_FILE), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap()
    }

    /// What the book and the market cache hold, table by table: a start after a
    /// clear must carry nothing back into them.
    fn held(f: &Figures) -> (BTreeMap<String, i64>, BTreeMap<String, i64>, Vec<String>) {
        let cache = f.cache().unwrap();
        (counts(&f.book().unwrap().conn_for_tests()), counts(cache.connection()), cache_keys(cache.connection()))
    }

    fn cache_keys(conn: &rusqlite::Connection) -> Vec<String> {
        conn.prepare("SELECT key FROM meta").unwrap().query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
    }

    #[test]
    fn every_table_of_every_store_belongs_to_a_kind() {
        let (_h, app) = app();
        let f = app.figures.get().unwrap();
        let listed = |l: Vec<&str>| -> Vec<String> {
            let mut v: Vec<String> = l.into_iter().map(str::to_string).collect();
            v.sort();
            v
        };
        assert_eq!(f.book().unwrap().tables().unwrap(), listed(TABLES.iter().map(|(t, _)| *t).collect()), "the book's tables and bagholder_book::clear::TABLES");
        assert_eq!(tables(f.cache().unwrap().connection()), listed(CACHE_TABLES.iter().map(|(t, _)| *t).collect()), "the market cache's tables and CACHE_TABLES");
        // the earlier store's, as its own schema makes them
        let other = tempfile::tempdir().unwrap();
        let old = bagholder_store::connect(other.path()).unwrap();
        bagholder_store::schema::init_schema(&old).unwrap();
        assert_eq!(tables(&old), listed(crate::carry::OLD_TABLES.iter().map(|(t, _)| *t).chain(["meta", "gen"]).collect()), "the earlier store's tables and carry::OLD_TABLES");
    }

    fn px_of(s: &str) -> bagholder_core::Dec {
        bagholder_core::Dec::parse(s).unwrap()
    }

    /// Everything the app keeps, in both stores: a pulled month, an entry, an
    /// imported file, a note, a rate, the notices, and a row in every table of the
    /// market cache.
    fn filled(app: &Arc<App>) {
        let f = app.figures.get().unwrap();
        let t = now();
        crate::entries::enter(f, &crate::entries::EntryRequest::Trade { account: String::new(), instrument: None, symbol: "ZZQQ".into(), currency: "USD".into(), day: "2025-11-19".into(), side: "buy".into(), quantity: "1".into(), price: "2".into(), fee: String::new() }, t).unwrap();
        crate::csv_import::import(f, "a.csv", "Date,Action,Symbol,Quantity,Price,Amount,Currency\n2025-11-18,Buy,ZZQR,1,1,1,USD\n11/18/2025,Buy,ZZQR,1,1,1,USD\n", None, t).unwrap();
        let trade = f.read(|e| e.figures().trades.iter().find_map(|x| x.trade)).unwrap().expect("a trade");
        f.write_journal(&trade.to_string(), &bagholder_core::journal::JournalEntry { thesis: "why".into(), grade: None, tags: vec!["t".into()] }, t).unwrap();
        let book = f.book().unwrap();
        let usd = bagholder_core::Currency::parse("USD").unwrap();
        let day: bagholder_core::jiff::civil::Date = "2025-11-18".parse().unwrap();
        book.store_rates(usd, &[(day, bagholder_core::Dec::parse("1.4").unwrap())], (day, day), &bagholder_core::SourceName::named("bank-of-canada"), t).unwrap();
        book.set_setting("watch.folder", Some("/somewhere"), t).unwrap();
        // what the person follows: the default tiles, and a listing watched under the name picked for it
        crate::following::ensure(app).unwrap();
        let shop = crate::following::Named { symbol: "SHOP".into(), exchange: "TSX".into(), name: "Shopify Inc.".into(), ..Default::default() };
        book.watch(&crate::following::draft(&book, &shop).unwrap(), t).unwrap();
        let ws_account = book.accounts().unwrap().into_iter().find(|a| a.nickname.as_deref() != Some("Manual")).unwrap();
        let read = book.broker_read(ws_account.connection, "balances", t).unwrap();
        book.store_buying_power(ws_account.id, t, &Ok(bagholder_core::Money::new(bagholder_core::Dec::parse("100").unwrap(), bagholder_core::Currency::parse("CAD").unwrap())), &read).unwrap();
        use bagholder_core::account::{AccountKind, AccountRef, AccountStatus, AccountType, Registration};
        let margin_kind = AccountType::Known { kind: AccountKind::Margin, registration: Registration::Unregistered, managed: false, joint: false };
        let margin = book.add_account(ws_account.connection, &[AccountRef::new(bagholder_core::Broker::named("wealthsimple"), "margin-x")], &margin_kind, AccountStatus::Open, Some("Margin"), t).unwrap();
        book.store_margin_backing(ws_account.connection, &[(ws_account.id, margin)], &read).unwrap();
        // an order and a bracket with their logs
        use bagholder_core::order::{Asker, OrderKind, OrderRole, Side, TimeInForce};
        let place = bagholder_book::orders::BracketPlace { id: "bracket-x".into(), broker: "wealthsimple".into(), broker_account: "acct".into(), broker_security: "sec".into(), symbol: "ZZQQ".into(), currency: usd };
        book.write_bracket(&place, &bagholder_core::bracket::BracketEvent::Created { quantity: bagholder_core::Dec::ONE, stop: None, target: Some(px_of("3")) }, &Asker::Person, t).unwrap();
        let order = bagholder_book::orders::OrderRequest {
            id: "order-x".into(), broker: "wealthsimple".into(), broker_account: "acct".into(), broker_security: "sec".into(), symbol: "ZZQQ".into(), currency: usd,
            side: Side::Buy, kind: OrderKind::Market, quantity: bagholder_core::Dec::ONE, limit_price: None, stop_price: None, time_in_force: TimeInForce::Day,
            bracket: Some(("bracket-x".into(), OrderRole::Entry)), request: serde_json::json!({}),
        };
        book.write_order(&order, true, &Asker::Person, t).unwrap();
        // ended: a live bracket would refuse the clear
        book.bracket_event("bracket-x", &Asker::Engine, t, &bagholder_core::bracket::BracketEvent::EntryEnded { why: "entry dry".into() }).unwrap().unwrap();
        // the cache's figure tables by its own writes (the engine reads them), the rest stood in for
        let cache = f.cache().unwrap();
        let src = bagholder_core::SourceName::named("yahoo");
        let held = f.read(|e| *e.inputs().ledger.instruments.keys().next().unwrap()).unwrap();
        let cur = f.read(|e| e.inputs().ledger.instruments[&held].instrument.currency).unwrap();
        let px = bagholder_core::Dec::parse("10").unwrap();
        cache.store_quote(&bagholder_sources::cache::StoredQuote { instrument: held, source: src.clone(), price: bagholder_core::Money::new(px, cur), change: None, change_pct: None, quoted_at: t, allowance: Default::default(), received_at: t }).unwrap();
        cache.store_ex_dividend(held, &bagholder_core::SourceName::named("tmx"), Some(day), t).unwrap();
        cache.store_closes(held, &[(day, px)], cur, &src, t).unwrap();
        let spx = bagholder_sources::contract::Benchmark::Sp500;
        cache.store_benchmark_closes(spx, &[(day, px)], &src, t).unwrap();
        cache.store_benchmark_events(spx, &[(day, bagholder_sources::cache::TrackerEvent::Dividend(px))], &src, t).unwrap();
        // the earlier store as a first start leaves it: its figures and orders carried
        // and dropped, the rest carried and still held, a key of every kind
        let old = bagholder_store::connect(&app.home).unwrap();
        bagholder_store::relabel::ensure(&old).unwrap();
        bagholder_store::schema::drop_figure_tables(&old, "2025-11-19T21:00:00Z").unwrap();
        bagholder_store::schema::drop_order_tables(&old, "2025-11-19T21:00:00Z").unwrap();
        fill(&old);
        old.execute("DELETE FROM watchlist", []).unwrap();
        bagholder_store::feeds::add_watch(&old, "SHOP", "TSX", "Shopify Inc.", "CAD", "", "2025-11-18T14:00:00Z").unwrap();
        for (key, value) in [
            ("synced_at", "2025-11-18T00:00:00Z"), ("journal_v2", "{}"), (bagholder_store::csvimport::WATCH_FILES_META, "{}"),
            ("market_tiles", "[{\"symbol\": \"VIX\", \"exchange\": \"Index\"}]"), ("notify_seen:x", "m|a"), ("bars_source:OLD", "tmx|OLD"), ("update_check", "{}"),
        ] {
            old.execute("INSERT OR REPLACE INTO meta(key, value) VALUES (?, ?)", [key, value]).unwrap();
        }
        drop(old);
        crate::carry::from_old_store(app, f, t).unwrap();
        fill(cache.connection());
        for key in ["bars_source:ABC", "tmx_form:ABC", crate::update::CHECK_KEY, crate::carry::CARRIED] {
            cache.connection().execute("INSERT OR REPLACE INTO meta(key, value) VALUES (?, 'x')", [key]).unwrap();
        }
        // the notices: a notification, a stream's memory, the settings and a mark
        let notices = crate::notify::book(app).unwrap();
        bagholder_store::feeds::add_notification(notices.notices(), "fills", "order:x", "Order filled", "", None, false, "2025-11-19T21:30:00Z").unwrap().unwrap();
        bagholder_store::feeds::mark_told(notices.notices(), "news:ZZQQ@TSX", &["a release".to_string()], "2025-11-19T21:30:00Z").unwrap();
        crate::notify::set_settings(&notices, &serde_json::from_value(serde_json::json!({"fills": true})).unwrap()).unwrap();
        crate::notify::keep_mark(&notices, &Some((format!("{}x", crate::notify::WATERMARK), "m".into()))).unwrap();
        std::fs::write(app.ws_home().session_path(), "{}").unwrap();
    }

    #[test]
    fn a_clear_removes_the_copies_kept_of_the_files_as_they_were() {
        let (_h, app) = app();
        filled(&app);
        let dir = app.home.join("snapshots");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bagholder-before-the-book-1.db"), "every kind").unwrap();
        clear(&app, app.figures.get().unwrap(), &[Kind::Journal], now()).unwrap();
        assert!(!dir.exists(), "a snapshot holds every kind, the cleared one among them");
        // with none kept, a clear is still a clear
        clear(&app, app.figures.get().unwrap(), &[Kind::Journal], now()).unwrap();
    }

    #[test]
    fn clear_all_leaves_nothing_but_the_files_own_versions() {
        let (_h, app) = app();
        filled(&app);
        let f = app.figures.get().unwrap();
        let before = counts(&f.book().unwrap().conn_for_tests());
        for (t, holds) in TABLES {
            if !matches!(holds, Holds::Schema) && !["adjustments", "adjustment_legs", "links", "link_records", "transfer_links", "account_links", "issuers", "instrument_routes", "declared_reads", "declared_distributions", "stated_frequencies", "bank_holidays", "fx_series", "trade_groups", "trade_group_members"].contains(t) {
                assert!(before[*t] > 0, "the book's {t} is empty before the clear, so the test proves nothing of it");
            }
        }
        clear(&app, f, &ALL, now()).unwrap();
        for (t, n) in counts(&f.book().unwrap().conn_for_tests()) {
            match t.as_str() {
                "schema_migrations" => {}
                "settings" => assert_eq!(f.book().unwrap().setting("zone").unwrap().as_deref(), Some("America/Toronto"), "the zone stays"),
                // the tile row is the default six again, and nothing else names an instrument
                "tiles" => assert_eq!(n as usize, crate::following::DEFAULT_TILES.len(), "the default tiles after Clear all"),
                "instruments" | "instrument_refs" | "instrument_routes" | "listings_named" => {
                    let b = f.book().unwrap();
                    let conn = b.conn_for_tests();
                    let column = if t == "instruments" { "id" } else { "instrument_id" };
                    let strays: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {t} WHERE {column} NOT IN (SELECT instrument_id FROM tiles)"), [], |r| r.get(0)).unwrap();
                    assert_eq!(strays, 0, "the book's {t} after Clear all holds only the default tiles'");
                }
                _ => assert_eq!(n, 0, "the book's {t} after Clear all"),
            }
        }
        assert_eq!(f.book().unwrap().setting("watch.folder").unwrap(), None);
        let cache = f.cache().unwrap();
        for (t, n) in counts(cache.connection()) {
            match cache_place(&t) {
                Place::Kept => {}
                Place::Meta => {
                    let keys = cache_keys(cache.connection());
                    assert!(!keys.is_empty() && keys.iter().all(|k| META_KEPT.contains(&k.as_str())), "meta keeps only the app's own: {keys:?}");
                }
                Place::Of(_) => assert_eq!(n, 0, "the market cache's {t} after Clear all"),
            }
        }
        let b = f.book().unwrap();
        assert!(b.setting(crate::notify::SETTINGS_KEY).unwrap().is_none() && b.setting(&format!("{}x", crate::notify::WATERMARK)).unwrap().is_none(), "the notification settings and marks are gone");
        assert!(!app.ws_home().session_path().exists(), "the login is gone");
        assert_eq!(f.read(|e| e.inputs().ledger.transactions.len()), Some(0), "the figures were built again from nothing");
        // the earlier store holds nothing that was cleared, and keeps what stops the carry
        let old = old_store(&app);
        for (t, n) in counts(&old) {
            if !["meta", "gen"].contains(&t.as_str()) {
                assert_eq!(n, 0, "the earlier store's {t} after Clear all");
            }
        }
        let keys = cache_keys(&old);
        assert!(keys.iter().all(|k| crate::carry::OLD_META_KEPT.contains(&k.as_str())), "the earlier store keeps only its own keys: {keys:?}");
        assert!(keys.iter().any(|k| k == bagholder_store::schema::FIGURES_MOVED_META), "and the flag that its figures were carried");
        // a start after it carries nothing back
        let before = held(f);
        crate::carry::from_old_store(&app, f, now()).unwrap();
        assert_eq!(held(f), before, "nothing carried back at the next start");
    }

    #[test]
    fn each_kind_alone_clears_its_own_and_nothing_else() {
        for kind in ALL {
            let (_h, app) = app();
            filled(&app);
            let f = app.figures.get().unwrap();
            let sources = |f: &Figures| -> BTreeMap<String, i64> {
                let b = f.book().unwrap();
                let c = b.conn_for_tests();
                let mut stmt = c.prepare("SELECT source, COUNT(*) FROM source_records GROUP BY source").unwrap();
                let out: BTreeMap<String, i64> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
                out
            };
            let book_before = counts(&f.book().unwrap().conn_for_tests());
            let src_before = sources(f);
            let cache_before = counts(f.cache().unwrap().connection());
            let old_before = counts(&old_store(&app));
            let old_keys_before = cache_keys(&old_store(&app));
            let settings_before = f.book().unwrap().setting(crate::notify::SETTINGS_KEY).unwrap();
            clear(&app, f, &[kind], now()).unwrap();
            let book_after = counts(&f.book().unwrap().conn_for_tests());
            let src_after = sources(f);
            // the book: records by source, the rest by what each table holds
            for (source, n) in &src_before {
                let person = source == "person" || source == "csv";
                let gone = (kind == Kind::Broker && !person) || (kind == Kind::Entries && person);
                assert_eq!(src_after.get(source).copied().unwrap_or(0), if gone { 0 } else { *n }, "{kind:?}: {source}'s records");
            }
            for (t, holds) in TABLES {
                let own = match holds {
                    Holds::Broker => kind == Kind::Broker,
                    Holds::Journal => kind == Kind::Journal,
                    Holds::Market => kind == Kind::Market,
                    _ => continue,
                };
                if own && *t == "trades" {
                    // the round trips still recorded are given their trades again
                    assert!(book_after[*t] > 0, "{kind:?}: trades given again");
                } else if own {
                    assert_eq!(book_after[*t], 0, "{kind:?} empties the book's {t}");
                } else if !(kind == Kind::Broker && *holds == Holds::Journal && *t == "trades") {
                    assert_eq!(book_after[*t], book_before[*t], "{kind:?} leaves the book's {t}");
                }
            }
            // the notices go with what the person follows, and nothing else
            for t in ["notifications", "told"] {
                assert!(book_before[t] > 0, "{t} is empty before the clear, so the test proves nothing of it");
                assert_eq!(book_after[t], if kind == Kind::Settings { 0 } else { book_before[t] }, "{kind:?}: the book's {t}");
            }
            let settings_after = f.book().unwrap().setting(crate::notify::SETTINGS_KEY).unwrap();
            assert_eq!(settings_after, if kind == Kind::Settings { None } else { settings_before }, "{kind:?}: the notification settings");
            let cache = f.cache().unwrap();
            for (t, n) in counts(cache.connection()) {
                match cache_place(&t) {
                    Place::Of(k) => assert_eq!(n, if k == kind { 0 } else { cache_before[&t] }, "{kind:?}: the market cache's {t}"),
                    Place::Kept => assert_eq!(n, cache_before[&t], "{kind:?}: the market cache's {t}"),
                    Place::Meta => {
                        let keys = cache_keys(cache.connection());
                        assert!(keys.iter().all(|k| meta_kind(k) != Some(kind)), "{kind:?}: meta {keys:?}");
                        assert!(keys.iter().any(|k| META_KEPT.contains(&k.as_str())), "{kind:?}: the app's own keys stay");
                        if kind != Kind::Market {
                            assert_eq!(n, cache_before[&t], "{kind:?}: the market cache's keys");
                        }
                    }
                }
            }
            assert_eq!(app.ws_home().session_path().exists(), kind != Kind::Login, "{kind:?}: the login");
            // the earlier store: the kind's rows and keys gone, every other kind's kept
            let old = old_store(&app);
            for (t, n) in counts(&old) {
                match crate::carry::OLD_TABLES.iter().find(|(x, _)| *x == t) {
                    Some((_, k)) => {
                        assert!(old_before[&t] > 0, "the earlier store's {t} is empty before the clear, so the test proves nothing of it");
                        assert_eq!(n, if *k == kind { 0 } else { old_before[&t] }, "{kind:?}: the earlier store's {t}");
                    }
                    None => assert!(["meta", "gen"].contains(&t.as_str()), "the earlier store's {t} is in no kind"),
                }
            }
            let old_keys = cache_keys(&old);
            let want: Vec<String> = old_keys_before.iter().filter(|k| crate::carry::old_meta_kind(k) != Some(kind)).cloned().collect();
            assert_eq!(old_keys, want, "{kind:?}: the earlier store's keys");
            // a start after it carries nothing back
            let before = held(f);
            crate::carry::from_old_store(&app, f, now()).unwrap();
            assert_eq!(held(f), before, "{kind:?}: nothing carried back at the next start");
        }
    }

    #[test]
    fn clearing_the_broker_s_records_keeps_the_journal_orphaned_and_the_next_pull_reads_in_full() {
        let (h, app) = app();
        let f = app.figures.get().unwrap();
        let trade = f.read(|e| e.figures().trades.iter().find_map(|x| x.trade)).unwrap().expect("a trade");
        f.write_journal(&trade.to_string(), &bagholder_core::journal::JournalEntry { thesis: "kept".into(), grade: None, tags: vec![] }, now()).unwrap();
        clear(&app, f, &[Kind::Broker], now()).unwrap();
        let book = f.book().unwrap();
        let c = book.conn_for_tests();
        let reason: Option<String> = c.query_row("SELECT orphaned_reason FROM trades WHERE id = ?", [trade.to_string()], |r| r.get(0)).unwrap();
        assert_eq!(reason.as_deref(), Some("the record it opened on was cleared"));
        let thesis: String = c.query_row("SELECT thesis FROM journal WHERE trade_id = ?", [trade.to_string()], |r| r.get(0)).unwrap();
        assert_eq!(thesis, "kept");
        // the pull starts from nothing: every row read again and stored new
        let first = {
            let other = tempfile::tempdir().unwrap();
            crate::tests_common::pulled_book(other.path());
            let (b, _) = bagholder_book::Book::open_in(other.path(), crate::app::APP_VERSION, now()).unwrap();
            b.live_records(&bagholder_core::SourceName::named("wealthsimple")).unwrap().len()
        };
        crate::tests_common::pulled_book(h.path());
        assert_eq!(book.live_records(&bagholder_core::SourceName::named("wealthsimple")).unwrap().len(), first);
    }

    #[test]
    fn a_live_bracket_refuses_orders_and_the_login_and_names_what_it_is_on() {
        let (_h, app) = app();
        let f = app.figures.get().unwrap();
        let usd = bagholder_core::Currency::parse("USD").unwrap();
        let place = bagholder_book::orders::BracketPlace { id: "b1".into(), broker: "wealthsimple".into(), broker_account: "acct".into(), broker_security: "sec".into(), symbol: "ZZQQ".into(), currency: usd };
        let created = bagholder_core::bracket::BracketEvent::Created { quantity: bagholder_core::Dec::ONE, stop: None, target: Some(bagholder_core::Dec::ONE) };
        f.book().unwrap().write_bracket(&place, &created, &bagholder_core::order::Asker::Person, now()).unwrap();
        for kinds in [vec![Kind::Orders], vec![Kind::Login], ALL.to_vec()] {
            assert_eq!(clear(&app, f, &kinds, now()), Err(Refused::BracketLive("ZZQQ".into())), "{kinds:?}");
        }
        assert!(f.read(|e| !e.inputs().ledger.transactions.is_empty()).unwrap(), "nothing was cleared");
        // what does not reach the bracket is cleared
        clear(&app, f, &[Kind::Journal], now()).unwrap();
        app.state.lock().unwrap().syncing = true;
        assert_eq!(clear(&app, f, &[Kind::Journal], now()), Err(Refused::Pulling));
    }
}
