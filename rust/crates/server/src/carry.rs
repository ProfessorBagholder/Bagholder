//! The earlier store (`bagholder.db`, the Python app's schema) carried into the
//! two stores `docs/architecture.md` §6 names, once, at start
//! (`docs/plans/stage-6-cutover.md`, 6a): its figures, journal and orders into
//! the book (`legacy_import`, `legacy_orders`), its watchlist and tiles and its
//! notices into the book, and what its market readers keep into the market
//! cache. This is the one place the server opens the earlier store; after it,
//! nothing reads or writes it.

use std::path::Path;
use std::sync::Arc;

use rusqlite::Connection;

use bagholder_book::Book;
use bagholder_core::jiff::Timestamp;

use crate::app::{log, App};
use crate::clear::Kind;
use crate::figures::{Figures, CACHE_FILE, OLD_FILE};

/// The earlier readers' tables carried whole into the market cache: those not
/// kept per listing. What the earlier store kept per listing it kept by a bare
/// ticker two listings could share (charts, short interest, news, a share's or a
/// fund's breakdown), so it cannot be told apart and is read again (plan
/// `market-data-keyed-by-listing`, Approach, 4); filings are carried to their
/// filers (`bagholder_sources::cache::carry_filings`).
pub const MARKET_TABLES: [&str; 3] = ["exposures", "gauges", "universes"];

/// The earlier store's `meta` keys the market readers remember by, by what each
/// begins with: each names one listing by the source's own name for it (a
/// Coinbase market, a Yahoo symbol), so it is carried; a key naming a bare
/// ticker is not.
pub const MARKET_META_PREFIXES: [&str; 3] = ["coinbase_product:", "coinbase_prev:", "yahoo_miss:"];

/// The earlier store's `meta` keys carried whole: the disclosures' hold on what
/// cannot be named yet, and the update check.
pub const MARKET_META_KEYS: [&str; 2] = [crate::feeds::FILINGS_HOLD_KEY, crate::update::CHECK_KEY];

/// The market cache's `meta` key that says the earlier store was carried, and what came.
pub const CARRIED: &str = "carried.earlier-store";

/// What the market cache took from the earlier store: rows by table, and keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CarriedMarket {
    pub rows: Vec<(String, i64)>,
    pub keys: i64,
    /// Filings that named no filer, so could not be carried.
    pub dropped_filings: i64,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Whether `key` is one the market readers keep.
pub fn market_key(key: &str) -> bool {
    MARKET_META_KEYS.contains(&key) || MARKET_META_PREFIXES.iter().any(|p| key.starts_with(p))
}

/// Carry the earlier store's market tables and keys at `old` into the market
/// cache at `cache`, once, in one transaction: every row with its values, or
/// nothing. The earlier store is only read. `None` when the cache was carried
/// into before; a cache made where there was no earlier store is marked as
/// carried from none, so a store that appears later is never merged into what
/// the cache holds by then.
pub fn carry_market(old: Option<&Path>, cache: &Path, at: Timestamp) -> Result<Option<CarriedMarket>, String> {
    // brought to this build's schema first, on a connection of its own
    bagholder_sources::cache::MarketCache::open(cache, crate::app::APP_VERSION, at).map_err(err)?;
    let c = bagholder_sqlite::open_db(cache).map_err(err)?;
    let done: Option<String> = rusqlite::OptionalExtension::optional(c.query_row("SELECT value FROM meta WHERE key = ?", [CARRIED], |r| r.get(0))).map_err(err)?;
    if done.is_some() {
        return Ok(None);
    }
    let mark = |c: &Connection, what: &serde_json::Value| -> rusqlite::Result<usize> {
        c.execute("INSERT INTO meta (key, value) VALUES (?1, ?2)", rusqlite::params![CARRIED, serde_json::json!({ "at": at.to_string(), "carried": what }).to_string()])
    };
    let Some(old) = old else {
        bagholder_sqlite::atomically(&c, || mark(&c, &serde_json::Value::Null)).map_err(err)?;
        return Ok(Some(CarriedMarket::default()));
    };
    c.execute("ATTACH DATABASE ?1 AS old", [old.to_string_lossy()]).map_err(err)?;
    let carried = bagholder_sqlite::atomically(&c, || {
        let mut out = CarriedMarket::default();
        for t in MARKET_TABLES {
            let there: i64 = c.query_row("SELECT COUNT(*) FROM old.sqlite_master WHERE type = 'table' AND name = ?", [t], |r| r.get(0))?;
            if there == 0 {
                continue;
            }
            // the cache's columns as the earlier store has them: a column an older
            // file lacks is left as the table's default, one the cache does not
            // keep (the filings' single-source columns, unused since) is not read
            let cols = |schema: &str| -> rusqlite::Result<Vec<String>> {
                c.prepare(&format!("PRAGMA {schema}.table_info(\"{t}\")"))?.query_map([], |r| r.get::<_, String>(1))?.collect()
            };
            let theirs = cols("old")?;
            let both: Vec<String> = cols("main")?.into_iter().filter(|x| theirs.contains(x)).map(|x| format!("\"{x}\"")).collect();
            let list = both.join(", ");
            // a share's or a fund's breakdown was kept by ticker, and is read again
            let only = if t == "exposures" { " WHERE key NOT LIKE 'share:%' AND key NOT LIKE 'fund:%'" } else { "" };
            let n = c.execute(&format!("INSERT INTO main.\"{t}\" ({list}) SELECT {list} FROM old.\"{t}\"{only}"), [])?;
            out.rows.push((t.to_string(), n as i64));
        }
        let there: i64 = c.query_row("SELECT COUNT(*) FROM old.sqlite_master WHERE type = 'table' AND name = 'filings'", [], |r| r.get(0))?;
        if there > 0 {
            let (carried, dropped) = bagholder_sources::cache::carry_filings(&c, "old")?;
            out.rows.push(("filings".into(), carried as i64));
            out.dropped_filings = dropped as i64;
        }
        let keys: Vec<(String, Option<String>)> = c.prepare("SELECT key, value FROM old.meta ORDER BY key")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        for (key, value) in keys.into_iter().filter(|(k, _)| market_key(k)) {
            c.execute("INSERT INTO main.meta (key, value) VALUES (?1, ?2)", rusqlite::params![key, value])?;
            out.keys += 1;
        }
        let rows: serde_json::Map<String, serde_json::Value> = out.rows.iter().map(|(t, n)| (t.clone(), (*n).into())).collect();
        mark(&c, &serde_json::json!({ "rows": rows, "keys": out.keys }))?;
        Ok(out)
    });
    let detached = c.execute("DETACH DATABASE old", []);
    let carried = carried.map_err(|e| format!("the earlier store's market data could not be carried into the market cache: {e}"))?;
    detached.map_err(err)?;
    Ok(Some(carried))
}

/// The earlier store's watchlist and tile row carried into the book once
/// (`Book::carry_following`), every row or none.
pub fn carry_following(book: &Book, old: &Connection, now: Timestamp) -> Result<(), String> {
    if book.following_carried().map_err(err)? {
        return Ok(());
    }
    let rows = bagholder_store::feeds::list_watchlist(old).map_err(err)?;
    let mut watched = Vec::new();
    for w in &rows {
        let n = crate::following::Named { instrument: None, symbol: w.symbol.clone(), exchange: w.exchange.clone(), name: w.name.clone(), currency: w.currency.clone(), security_id: w.security_id.clone() };
        let d = crate::following::draft(book, &n).map_err(|e| format!("the watched {} could not be carried into the book: {e}", w.symbol))?;
        // a row the earlier store kept without its day counts from now
        let added = match w.added_at.trim() {
            "" => now,
            t => t.parse::<Timestamp>().map_err(|e| format!("the watched {} was added at {t:?}: {e}", w.symbol))?,
        };
        watched.push((d, added));
    }
    let tiles = match bagholder_store::rows::tiles(old).map_err(err)? {
        Some(saved) => Some(
            saved
                .iter()
                .filter(|t| bagholder_core::directory::find(&t.symbol, &t.exchange).is_some())
                .map(|t| crate::following::tile_draft(book, &t.symbol, &t.exchange))
                .collect::<Result<Vec<_>, String>>()?,
        ),
        None => None,
    };
    book.carry_following(&watched, tiles.as_deref(), now).map_err(err)
}

/// The folder an earlier version watched, watched again, once: taken from the
/// earlier store and cleared there, so a folder no longer watched is not taken
/// again at the next start.
pub fn carry_watch_folder(f: &Figures, old: &Connection, now: Timestamp) -> Result<(), String> {
    let folder = bagholder_store::csvimport::watch_folder(old).map_err(err)?;
    crate::csv_import::adopt(f, &folder, now)?;
    if !folder.is_empty() {
        bagholder_store::tables::set_meta(old, bagholder_store::csvimport::WATCH_META, "").map_err(err)?;
    }
    Ok(())
}

/// Every table of the earlier store, and the kind of data Clear data empties it
/// with: the kind whose new place its rows were carried into (or, for the figure
/// and order tables still there when their carry was refused, would be).
pub const OLD_TABLES: &[(&str, Kind)] = &[
    ("activities", Kind::Broker),
    ("accounts", Kind::Broker),
    ("balances", Kind::Broker),
    ("margin", Kind::Broker),
    ("nav_history", Kind::Broker),
    ("securities", Kind::Broker),
    ("grouped_trades", Kind::Journal),
    ("fx_rates", Kind::Market),
    ("benchmark_prices", Kind::Market),
    ("distributions", Kind::Market),
    ("distribution_fetches", Kind::Market),
    ("quotes", Kind::Market),
    ("price_history", Kind::Market),
    ("history_fetches", Kind::Market),
    ("price_bars", Kind::Market),
    ("bar_fetches", Kind::Market),
    ("exposures", Kind::Market),
    ("filings", Kind::Market),
    ("gauges", Kind::Market),
    ("news", Kind::Market),
    ("shorts", Kind::Market),
    ("universes", Kind::Market),
    ("orders", Kind::Orders),
    ("brackets", Kind::Orders),
    ("watchlist", Kind::Settings),
    ("notifications", Kind::Settings),
    ("told", Kind::Settings),
];

/// The earlier store's `meta` keys kept whatever is cleared: its version, the
/// one-time repairs it has made, the update check the market cache keeps too, and
/// the flags that say its figures and orders were carried, so a cleared kind is
/// never carried back at the next start.
pub const OLD_META_KEPT: &[&str] = &[
    "schema_version",
    "update_check",
    "history_sources_migrated",
    "close_only_history_dropped",
    bagholder_store::relabel::OPTION_RELABEL_META,
    bagholder_store::relabel::OPTION_UNIT_PRICE_SCALE_META,
    bagholder_store::schema::FIGURES_MOVED_META,
    bagholder_store::schema::ORDERS_MOVED_META,
];

/// An earlier store's `meta` key's kind, or `None` for one kept. A key no rule
/// names is what a source answered: market data.
pub fn old_meta_kind(key: &str) -> Option<Kind> {
    if OLD_META_KEPT.contains(&key) {
        return None;
    }
    let is = |k: &str| key == k;
    Some(if bagholder_store::admin::SYNC_META_KEYS.contains(&key) || is("balances_read_at") {
        Kind::Broker
    } else if is(bagholder_store::tables::JOURNAL_META) || is("trade_groups") || is("trade_notes") {
        Kind::Journal
    } else if is(bagholder_store::csvimport::WATCH_META) || is(bagholder_store::csvimport::WATCH_FILES_META) || is(bagholder_store::csvimport::WATCH_LAST_META) {
        Kind::Entries
    } else if is(bagholder_store::rows::TILES_META) || is(OLD_SETTINGS) || key.starts_with(OLD_SEEN) {
        Kind::Settings
    } else {
        Kind::Market
    })
}

/// The earlier store's keys for the notification settings and marks.
const OLD_SETTINGS: &str = "notify_settings";
const OLD_SEEN: &str = "notify_seen:";

/// Clear data's part in the earlier store (`docs/decisions.md`, 2026-09-25: Clear
/// data clears everything): what it still holds of each kind cleared is emptied,
/// in one transaction, so no copy of a cleared kind stays on this machine. Its
/// carry flags and marks stay (here, in the book and in the market cache), so
/// nothing is carried again at the next start. No earlier store, nothing to do.
/// The one write to the earlier store besides the carry.
pub fn clear_old_store(home: &Path, kinds: &[Kind]) -> Result<(), String> {
    bagholder_store::guard_home(home)?;
    let path = home.join(OLD_FILE);
    if !path.exists() {
        return Ok(());
    }
    let c = bagholder_store::open_db(&path).map_err(|e| format!("the earlier store could not be opened to clear it: {e}"))?;
    let e = |e: rusqlite::Error| format!("the earlier store could not be cleared: {e}");
    bagholder_sqlite::atomically(&c, || {
        let present: Vec<String> = c.prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        for (table, kind) in OLD_TABLES {
            // a table carried and dropped is no longer in the file
            if kinds.contains(kind) && present.iter().any(|p| p == table) {
                c.execute(&format!("DELETE FROM \"{table}\""), [])?;
            }
        }
        if present.iter().any(|p| p == "meta") {
            let keys: Vec<String> = c.prepare("SELECT key FROM meta")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            for key in keys.into_iter().filter(|k| old_meta_kind(k).is_some_and(|k| kinds.contains(&k))) {
                c.execute("DELETE FROM meta WHERE key = ?", [&key])?;
            }
        }
        Ok(())
    })
    .map_err(e)
}

/// Everything the earlier store in `app`'s data folder still holds that the
/// app keeps, carried where it belongs: its figure tables retired once the book
/// holds their rows, its orders and brackets, its watchlist and tiles, the
/// watched folder, its notices, and its market readers' tables and keys. A
/// failure of what the server cannot start without (the orders it follows, the
/// notices and the market data it would otherwise lose) stops the start, naming
/// what; the rest is said in the header and tried again at the next start.
pub fn from_old_store(app: &Arc<App>, f: &Figures, now: Timestamp) -> Result<(), String> {
    bagholder_store::guard_home(&app.home)?;
    let path = app.home.join(OLD_FILE);
    let cache = app.home.join(CACHE_FILE);
    if !path.exists() {
        carry_market(None, &cache, now)?;
        return Ok(());
    }
    let book = f.book()?;
    {
        // the earlier store brought to its own schema and repairs, as its pool did,
        // for the carries that read it through its own code
        let old = bagholder_store::open_db(&path).map_err(|e| format!("the earlier store could not be opened: {e}"))?;
        bagholder_store::relabel::ensure(&old).map_err(|e| format!("the earlier store could not be prepared: {e}"))?;
        match crate::legacy_import::retire_old_figures(&app.home, &old, &book, now) {
            Ok(Some(snapshot)) => log(&format!("bagholder: the earlier store's figure tables are the book's now; the file as it was is kept at {}", snapshot.display())),
            Ok(None) => {}
            Err(e) => log(&format!("bagholder: {e}")),
        }
        // the earlier orders and brackets, once: a live one is followed from the first check
        let c = crate::legacy_orders::carry_orders(&app.home, &old, &book, now)?;
        if let Some(snapshot) = &c.snapshot {
            log(&format!(
                "bagholder: {} orders and {} brackets carried into the book ({} placed in Wealthsimple's own app left to its feed); the file as it was is kept at {}",
                c.orders,
                c.brackets,
                c.left_to_the_feed,
                snapshot.display()
            ));
        }
        crate::feeds::went(app, "following", carry_following(&book, &old, now).map_err(|e| format!("The watchlist could not be carried into the book: {e}")));
        crate::feeds::went(app, "watch-folder", carry_watch_folder(f, &old, now).map_err(|e| format!("The folder watched before could not be watched again: {e}")));
    }
    if let Some(n) = book.carry_notices(&path, now).map_err(|e| format!("the earlier store's notifications could not be carried into the book: {e}"))? {
        log(&format!("bagholder: {} notifications, {} marks of what was told and {} notification settings carried into the book", n.notifications, n.told, n.settings));
    }
    if let Some(m) = carry_market(Some(&path), &cache, now)? {
        let rows: Vec<String> = m.rows.iter().map(|(t, n)| format!("{n} {t}")).collect();
        log(&format!("bagholder: the earlier store's market data carried into the market cache: {} rows ({}), {} keys", m.rows.iter().map(|(_, n)| n).sum::<i64>(), rows.join(", "), m.keys));
    }
    Ok(())
}

/// The book's setting that says its told marks were carried to the scopes a
/// listing and a filer are kept under (plan `market-data-keyed-by-listing`,
/// Approach, 3), and how many.
pub const TOLD_BY_LISTING: &str = "carried.told-by-listing";

/// What a stream's mark says it has shown up to: the moment before its `|`.
fn mark_moment(value: &str) -> &str {
    value.split_once('|').map_or(value, |(m, _)| m)
}

/// The scopes an earlier build kept what was told under, by a ticker, and those
/// the same marks belong under now: `news:<SYM>@<venue words>` is its listing's
/// `news:<ListingKey>`; `filings:<SYM>:<source>` is each filer the market cache
/// carried that ticker's filings to (`filings_carried`), `filings:<source>:<filer>`.
/// A scope already in the new form, or one whose listing names no venue a key is
/// made from, has none: it is left to lapse, and the stream it was is a new scope
/// whose first sweep sets its mark without telling (`notify::fresh_in`).
fn told_scopes(cache: &Connection, scope: &str) -> Result<Vec<String>, String> {
    use bagholder_store::feeds::Regulator;
    if let Some(rest) = scope.strip_prefix("news:") {
        if rest.parse::<bagholder_core::ListingKey>().is_ok() {
            return Ok(vec![]);
        }
        let Some((sym, ex)) = rest.rsplit_once('@') else { return Ok(vec![]) };
        // a scope whose venue words name no venue a key is made from is left to lapse
        return Ok(bagholder_core::ListingKey::on(sym, ex).map_or_else(|_| vec![], |k| vec![format!("news:{k}")]));
    }
    if let Some(rest) = scope.strip_prefix("filings:") {
        let parts: Vec<&str> = rest.split(':').collect();
        let is_source = |s: &str| Regulator::parse(s).is_some();
        let [sym, source] = parts.as_slice() else { return Ok(vec![]) };
        if is_source(sym) || !is_source(source) {
            return Ok(vec![]);
        }
        let mut stmt = cache.prepare("SELECT filer FROM filings_carried WHERE symbol = ? AND source = ? ORDER BY filer").map_err(err)?;
        let filers = stmt.query_map([sym, source], |r| r.get::<_, String>(0)).map_err(err)?.collect::<rusqlite::Result<Vec<_>>>().map_err(err)?;
        return Ok(filers.into_iter().map(|f| format!("filings:{source}:{f}")).collect());
    }
    Ok(vec![])
}

/// Whether a told scope is of an earlier build's form: a news scope by a ticker
/// and venue words, or a filings scope by a ticker and a source.
fn earlier_form(scope: &str) -> bool {
    use bagholder_store::feeds::Regulator;
    if let Some(rest) = scope.strip_prefix("news:") {
        return rest.parse::<bagholder_core::ListingKey>().is_err();
    }
    match scope.strip_prefix("filings:").map(|r| r.split(':').collect::<Vec<_>>()) {
        Some(parts) => parts.len() == 2 && Regulator::parse(parts[0]).is_none() && Regulator::parse(parts[1]).is_some(),
        None => false,
    }
}

/// Carry the book's told marks to the scopes a listing and a filer are kept
/// under, once, after the market cache's migration 6 has named each ticker's
/// filers: each event told under an old scope is told under each new one, and
/// each stream's mark is the later of the old and the new, so nothing is told
/// twice and a filing that arrives after the old mark is still told. The old
/// scopes are removed. What came, as (events, marks); `None` when carried before.
pub fn carry_told_by_listing(book: &Book, cache: &Connection, now: Timestamp) -> Result<Option<(usize, usize)>, String> {
    if book.setting(TOLD_BY_LISTING).map_err(err)?.is_some() {
        return Ok(None);
    }
    let c = book.notices();
    let seen = bagholder_book::notices::SEEN;
    let carried = bagholder_sqlite::atomically(c, || -> rusqlite::Result<(usize, usize)> {
        let as_sql = |e: String| rusqlite::Error::ToSqlConversionFailure(e.into());
        let mut scopes: Vec<String> = c.prepare("SELECT DISTINCT scope FROM told WHERE scope LIKE 'news:%' OR scope LIKE 'filings:%'")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        let marked: Vec<(String, String)> = c
            .prepare("SELECT key, value FROM settings WHERE key LIKE ? || 'news:%' OR key LIKE ? || 'filings:%'")?
            .query_map([seen, seen], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        scopes.extend(marked.iter().map(|(k, _)| k[seen.len()..].to_string()));
        scopes.sort();
        scopes.dedup();
        let (mut events, mut marks) = (0usize, 0usize);
        for old in scopes {
            let new = told_scopes(cache, &old).map_err(as_sql)?;
            for n in &new {
                events += c.execute("INSERT OR IGNORE INTO told (scope, event, at) SELECT ?1, event, at FROM told WHERE scope = ?2", [n, &old])?;
                if let Some((_, v)) = marked.iter().find(|(k, _)| k[seen.len()..] == old) {
                    let key = format!("{seen}{n}");
                    let held: Option<String> = c.query_row("SELECT value FROM settings WHERE key = ?", [&key], |r| r.get(0)).ok();
                    if held.as_deref().is_none_or(|h| mark_moment(h) < mark_moment(v)) {
                        book.set_setting(&key, Some(v), now).map_err(crate::notify::sql)?;
                        marks += 1;
                    }
                }
            }
            // a scope of the earlier form is gone once carried, or when it names nothing a scope is kept under now
            if earlier_form(&old) {
                c.execute("DELETE FROM told WHERE scope = ?", [&old])?;
                book.set_setting(&format!("{seen}{old}"), None, now).map_err(crate::notify::sql)?;
            }
        }
        book.set_setting(TOLD_BY_LISTING, Some(&format!("{events} events, {marks} marks")), now).map_err(crate::notify::sql)?;
        Ok((events, marks))
    })
    .map_err(err)?;
    Ok(Some(carried))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn now() -> Timestamp {
        "2025-11-19T21:00:00Z".parse().unwrap()
    }

    type Rows = Vec<Vec<String>>;

    /// Every row of `table` in `conn`, each value spelled with its type, in one order.
    fn rows(conn: &Connection, table: &str, cols: &[String]) -> Rows {
        let list = cols.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(", ");
        let mut stmt = conn.prepare(&format!("SELECT {list} FROM \"{table}\"")).unwrap();
        let mut out: Rows = stmt.query_map([], |r| (0..cols.len()).map(|i| r.get_ref(i).map(|v| format!("{v:?}"))).collect()).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        out.sort();
        out
    }

    fn columns(conn: &Connection, table: &str) -> Vec<String> {
        conn.prepare(&format!("PRAGMA table_info(\"{table}\")")).unwrap().query_map([], |r| r.get(1)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
    }

    /// Two rows in `table`, each column given a value of its declared type (the
    /// second row's nullable columns left null), so a carry that loses a column, a
    /// type or a null is seen.
    fn fill(conn: &Connection, table: &str) {
        let info: Vec<(String, String, bool)> = conn
            .prepare(&format!("PRAGMA table_info(\"{table}\")"))
            .unwrap()
            .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)? == 1 || r.get::<_, i64>(5)? > 0)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        for i in 1..=2i64 {
            let names = info.iter().map(|(c, _, _)| format!("\"{c}\"")).collect::<Vec<_>>().join(", ");
            let values: Vec<rusqlite::types::Value> = info
                .iter()
                .map(|(c, ty, required)| match ty.to_uppercase().as_str() {
                    _ if i == 2 && !required => rusqlite::types::Value::Null,
                    t if t.contains("INT") => rusqlite::types::Value::Integer(i * 7),
                    t if t.contains("REAL") => rusqlite::types::Value::Real(i as f64 + 0.25),
                    _ => rusqlite::types::Value::Text(format!("{table}.{c}.{i}")),
                })
                .collect();
            let holes = vec!["?"; values.len()].join(", ");
            conn.execute(&format!("INSERT INTO \"{table}\" ({names}) VALUES ({holes})"), rusqlite::params_from_iter(values)).unwrap_or_else(|e| panic!("{table}: {e}"));
        }
    }

    /// The earlier store's keys: the market readers', the notices', and the rest.
    const KEYS: [(&str, &str); 19] = [
        ("tmx_form:QNC", "@QNC:CNX"), ("bars_source:QNC", "tmx|QNC"), ("bars_miss:QNC|1h", "2026-09-25T10:00:00Z"),
        ("coinbase_product:BTC-CAD", "@BTC-CAD"), ("coinbase_prev:BTC-CAD", "2026-09-25@90000.5"), ("yahoo_miss:QNC", "2026-09-25"),
        ("news_fetched:QNC@TSX", "2026-09-25T10:00:00Z"), ("news_source_fetched:gnews:QNC@TSX", "2026-09-25T10:00:00Z"),
        ("filings_fetched:QNC", "2026-09-25T10:00:00Z"), ("filings_sources:QNC", "{\"sedar\":\"ok\"}"), ("sedar_profile:QNC", "000012345"),
        ("filings:held-since", "2026-09-25T09:00:00Z"), ("update_check", "{\"ok\":true,\"latest\":\"v1.47.0\"}"),
        ("notify_settings", "{\"fills\":true,\"releasesAll\":true}"), ("notify_seen:news:QNC@TSX", "2026-09-25T10:00:00Z|a"),
        ("synced_at", "2026-09-01T00:00:00Z"), ("journal_v2", "{}"), ("coingecko_id:BTC", "bitcoin"), ("market_attempt_at", "2026-09-01T00:00:00Z"),
    ];

    /// A data folder holding a book of the recorded month and an earlier store with
    /// rows in every table the carry moves and keys of every kind; the app and its
    /// figures open on it.
    fn home_with_an_earlier_store() -> (tempfile::TempDir, Arc<App>, Figures) {
        crate::tests_common::home(); // offline, dry orders
        let home = tempfile::tempdir().unwrap();
        crate::tests_common::pulled_book(home.path());
        let old = bagholder_store::connect(home.path()).unwrap();
        bagholder_store::relabel::ensure(&old).unwrap();
        for t in MARKET_TABLES.iter().chain(&["notifications", "told", "filings", "price_history", "news", "shorts"]) {
            fill(&old, t);
        }
        // the filled filings name a filer: a SEDAR+ profile, or an SEC address
        old.execute("UPDATE filings SET source = 'SEDAR+', profile_no = '000012345' WHERE rowid = 1", []).unwrap();
        old.execute("UPDATE filings SET source = 'SEC', url = 'https://www.sec.gov/Archives/edgar/data/0000320193/1/a.htm' WHERE rowid = 2", []).unwrap();
        for (k, v) in KEYS {
            bagholder_store::tables::set_meta(&old, k, v).unwrap();
        }
        drop(old);
        let app = App::new(home.path().to_path_buf(), std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into());
        let f = Figures::open(home.path(), now()).unwrap();
        f.state_zone("America/Toronto", now()).unwrap();
        (home, app, f)
    }

    fn open(path: &Path) -> Connection {
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap()
    }

    #[test]
    fn every_row_and_key_of_the_earlier_store_is_carried_once_with_its_values() {
        let (home, app, f) = home_with_an_earlier_store();
        from_old_store(&app, &f, now()).unwrap();
        let old = open(&home.path().join(OLD_FILE));
        let cache = open(&home.path().join(CACHE_FILE));
        let book = f.book().unwrap();
        for t in MARKET_TABLES {
            let cols = columns(&cache, t);
            let (was, is) = (rows(&old, t, &cols), rows(&cache, t, &cols));
            assert_eq!(was.len(), 2, "{t} held rows to carry");
            assert_eq!(is, was, "the market cache's {t}");
        }
        // filings carried to their filers, with what they held
        let filed: Vec<(String, String, String)> = cache.prepare("SELECT source, id, filer FROM filings ORDER BY source").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(filed, vec![("SEC".into(), "filings.id.2".into(), "320193".into()), ("SEDAR+".into(), "filings.id.1".into(), "000012345".into())]);
        // what was kept by a bare ticker is read again, never carried
        for t in ["price_history", "news", "shorts"] {
            let n: i64 = cache.query_row(&format!("SELECT COUNT(*) FROM \"{t}\""), [], |r| r.get(0)).unwrap();
            assert_eq!(n, 0, "{t} is read again");
        }
        for t in ["notifications", "told"] {
            let cols = columns(book.notices(), t);
            assert_eq!(rows(book.notices(), t, &cols), rows(&old, t, &cols), "the book's {t}");
        }
        let kept: BTreeMap<String, String> = cache.prepare("SELECT key, value FROM meta WHERE key != ?").unwrap().query_map([CARRIED], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        let want: BTreeMap<String, String> = KEYS.iter().filter(|(k, _)| market_key(k)).map(|(k, v)| (k.to_string(), v.to_string())).collect();
        assert_eq!(want.len(), 5, "every reader's key that names a listing by a source's own name, the update check among them");
        assert_eq!(kept, want, "the readers' keys and nothing else");
        assert_eq!(book.setting(crate::notify::SETTINGS_KEY).unwrap().as_deref(), Some("{\"fills\":true,\"releasesAll\":true}"));
        assert_eq!(book.setting(&format!("{}news:QNC@TSX", crate::notify::WATERMARK)).unwrap().as_deref(), Some("2026-09-25T10:00:00Z|a"));
        // what the app reads is what was carried
        assert_eq!(crate::update::update_status(&app).unwrap().latest, "v1.47.0");
        assert!(crate::notify::settings(&book).unwrap().fills);

        // a second start carries nothing
        let before: Vec<Rows> = MARKET_TABLES.iter().map(|t| rows(&cache, t, &columns(&cache, t))).collect();
        let notices_before = rows(book.notices(), "notifications", &columns(book.notices(), "notifications"));
        from_old_store(&app, &f, now()).unwrap();
        assert_eq!(carry_market(Some(&home.path().join(OLD_FILE)), &home.path().join(CACHE_FILE), now()).unwrap(), None);
        assert_eq!(book.carry_notices(&home.path().join(OLD_FILE), now()).unwrap(), None);
        assert_eq!(MARKET_TABLES.iter().map(|t| rows(&cache, t, &columns(&cache, t))).collect::<Vec<_>>(), before);
        assert_eq!(rows(book.notices(), "notifications", &columns(book.notices(), "notifications")), notices_before);
    }

    /// After the carry nothing writes the earlier store: with its file made
    /// read-only, a news pass, a disclosures pass, a notification and a chart read
    /// all go through, and the file is as it was.
    #[test]
    #[cfg(unix)]
    fn after_the_carry_nothing_writes_the_earlier_store() {
        use std::os::unix::fs::PermissionsExt;
        let (home, app, f) = home_with_an_earlier_store();
        from_old_store(&app, &f, now()).unwrap();
        app.set_figures(f);
        let files: Vec<std::path::PathBuf> = ["", "-wal", "-shm"].iter().map(|x| home.path().join(format!("{OLD_FILE}{x}"))).filter(|p| p.exists()).collect();
        let before: Vec<Vec<u8>> = files.iter().map(|p| std::fs::read(p).unwrap()).collect();
        for p in &files {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o444)).unwrap();
        }
        let book = crate::notify::book(&app).unwrap();
        crate::notify::set_settings(&book, &serde_json::from_value(serde_json::json!({"fills": true, "releasesAll": true, "disclosuresAll": true})).unwrap()).unwrap();
        drop(book);
        crate::feeds::refresh_news(&app);
        crate::feeds::sweep_filings(&app);
        assert!(crate::notify::tell(&app, "fills", "order:ro:filled", "Order filled · QNC", "Bought 5 at 1.75", None).is_some(), "the notice is recorded");
        let q = crate::feeds::HistoryQuery::parse("symbol=QNC&exchange=TSX-V&currency=CAD&kind=Shares&from=2025-10-01&to=2025-11-19&tf=1d");
        assert!(matches!(crate::feeds::history_payload(&app, &q), crate::feeds::HistoryAnswer::Ok(_)), "the chart is read");
        let said = crate::status::status(&app).error;
        assert!(!said.to_lowercase().contains("readonly") && !said.contains("could not be opened"), "{said}");
        for p in &files {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        assert_eq!(files.iter().map(|p| std::fs::read(p).unwrap()).collect::<Vec<_>>(), before, "the earlier store is as it was");
    }

    #[test]
    fn a_cache_made_where_there_was_no_earlier_store_never_takes_one_in_later() {
        crate::tests_common::home();
        let home = tempfile::tempdir().unwrap();
        let cache = home.path().join(CACHE_FILE);
        assert_eq!(carry_market(None, &cache, now()).unwrap(), Some(CarriedMarket::default()));
        let old = bagholder_store::connect(home.path()).unwrap();
        bagholder_store::schema::init_schema(&old).unwrap();
        fill(&old, "news");
        assert_eq!(carry_market(Some(&home.path().join(OLD_FILE)), &cache, now()).unwrap(), None);
        assert_eq!(open(&cache).query_row("SELECT COUNT(*) FROM news", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    /// A table whose rows cannot all be carried stops the carry, naming it, and
    /// leaves the cache as it was: carried at the next start, never in part.
    #[test]
    fn a_carry_that_fails_part_way_carries_nothing_and_says_what() {
        let (home, _app, _f) = home_with_an_earlier_store();
        let old = bagholder_store::connect(home.path()).unwrap();
        // a universe row with no symbol, which the market cache refuses
        old.execute_batch("DROP TABLE universes; CREATE TABLE universes (key TEXT, symbol TEXT, name TEXT, value REAL, percent_change REAL, sector TEXT, country TEXT, fetched_at TEXT); INSERT INTO universes (key) VALUES ('tsx60');").unwrap();
        drop(old);
        let cache = home.path().join(CACHE_FILE);
        let e = carry_market(Some(&home.path().join(OLD_FILE)), &cache, now()).unwrap_err();
        assert!(e.contains("could not be carried") && e.contains("universes.symbol"), "{e}");
        let c = open(&cache);
        for t in MARKET_TABLES {
            assert_eq!(c.query_row(&format!("SELECT COUNT(*) FROM {t}"), [], |r| r.get::<_, i64>(0)).unwrap(), 0, "{t}");
        }
        assert_eq!(c.query_row("SELECT COUNT(*) FROM meta", [], |r| r.get::<_, i64>(0)).unwrap(), 0, "not marked: tried again at the next start");
    }

    /// The told marks of an earlier build go to the scopes each listing and filer
    /// is kept under: every event told under an old scope is told under each new
    /// one, each stream's mark is the later of the two, the old scopes go, and a
    /// second start carries nothing. Over every venue a news scope may name.
    #[test]
    fn told_marks_are_carried_to_each_listing_and_filer_once() {
        let home = tempfile::tempdir().unwrap();
        crate::tests_common::pulled_book(home.path());
        let at: Timestamp = "2026-10-01T12:00:00Z".parse().unwrap();
        let f = Figures::open(home.path(), at).unwrap();
        let book = f.book().unwrap();
        let cache = Connection::open_in_memory().unwrap();
        for m in &bagholder_sources::cache::MIGRATIONS {
            cache.execute_batch(m.sql).unwrap();
        }
        // a ticker whose filings were carried to two filers, and one carried to none
        cache.execute_batch("INSERT INTO filings_carried VALUES ('ZQ', 'SEDAR+', '000000001'), ('ZQ', 'SEC', '111'), ('ZQ', 'SEC', '222');").unwrap();
        let seen = bagholder_book::notices::SEEN;
        let tell = |scope: &str, event: &str| {
            book.notices().execute("INSERT INTO told (scope, event, at) VALUES (?, ?, '2026-09-01')", [scope, event]).unwrap();
        };
        let words: Vec<&str> = bagholder_core::listing::VENUE_WORDS.iter().map(|(_, w)| w[0]).collect();
        for w in &words {
            tell(&format!("news:ZQ@{w}"), "e-news");
            book.set_setting(&format!("{seen}news:ZQ@{w}"), Some("2026-09-02T00:00:00Z|a"), at).unwrap();
        }
        tell("filings:ZQ:SEC", "e-sec");
        book.set_setting(&format!("{seen}filings:ZQ:SEC"), Some("2026-09-03T00:00:00Z|x"), at).unwrap();
        tell("filings:NONE:SEDAR+", "e-none");
        // a new scope already further on keeps its own mark
        book.set_setting(&format!("{seen}filings:SEC:222"), Some("2026-09-09T00:00:00Z|y"), at).unwrap();

        let (events, _) = carry_told_by_listing(&book, &cache, at).unwrap().expect("carried");
        assert!(events > 0);
        let told = |scope: &str| -> Vec<String> {
            book.notices().prepare("SELECT event FROM told WHERE scope = ? ORDER BY event").unwrap().query_map([scope], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
        };
        for w in &words {
            let key = bagholder_core::ListingKey::on("ZQ", w).unwrap();
            assert_eq!(told(&format!("news:{key}")), vec!["e-news".to_string()], "{w}");
            assert_eq!(book.setting(&format!("{seen}news:{key}")).unwrap().as_deref(), Some("2026-09-02T00:00:00Z|a"), "{w}");
            assert!(told(&format!("news:ZQ@{w}")).is_empty() || bagholder_core::ListingKey::on("ZQ", w).unwrap().to_string() == format!("ZQ@{w}"), "the old scope goes: {w}");
        }
        assert_eq!(told("filings:SEC:111"), vec!["e-sec".to_string()]);
        assert_eq!(told("filings:SEC:222"), vec!["e-sec".to_string()]);
        assert!(told("filings:SEDAR+:000000001").is_empty(), "nothing was told of that source under the ticker");
        assert_eq!(book.setting(&format!("{seen}filings:SEC:111")).unwrap().as_deref(), Some("2026-09-03T00:00:00Z|x"));
        assert_eq!(book.setting(&format!("{seen}filings:SEC:222")).unwrap().as_deref(), Some("2026-09-09T00:00:00Z|y"), "the later mark stands");
        assert!(told("filings:ZQ:SEC").is_empty() && told("filings:NONE:SEDAR+").is_empty(), "the old scopes go");
        assert_eq!(book.setting(&format!("{seen}filings:ZQ:SEC")).unwrap(), None);
        assert_eq!(carry_told_by_listing(&book, &cache, at).unwrap(), None, "once");
    }
}
