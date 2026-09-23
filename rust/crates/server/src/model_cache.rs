//! The model, kept in layers and rebuilt only where something moved.
//!
//! The store keeps a generation counter for each thing the model reads
//! (`bagholder_store::gens`). Each input is read again only when its counter has
//! moved, and each derived layer is rebuilt only when an input it reads was read
//! again, or the day turned:
//!
//! | what changed                       | read again    | rebuilt                          |
//! |------------------------------------|---------------|----------------------------------|
//! | a quote                            | quotes        | positions                        |
//! | the journal                        | journal       | trades, positions (their notes)  |
//! | an FX rate                         | fx            | trades, cashflow                 |
//! | a balance, an account, margin      | that table    | positions, accounts              |
//! | news, a universe, the watchlist... | that table    | nothing                          |
//! | an activity, a security, the day   | both          | the match, and all that read it  |
//!
//! So a price tick reads one small table and marks a dozen rows; it no longer
//! reads every activity and runs the FIFO match again, which is what made the
//! original unusable on a modest machine. The cache is one lock held for a
//! build, so concurrent requests after a change wait for one build instead of
//! each starting their own.

use rusqlite::Connection;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use bagholder_model::base::{self, Base, Inputs, Layers};
use bagholder_model::wire::View;
use bagholder_store::{activities, gens, market, rows, snapshot, tables};

type Gens = BTreeMap<String, i64>;

#[derive(Default)]
struct Inner {
    gens: Gens,
    today: String,
    inputs: Option<Inputs>,
    layers: Option<Layers>,
    base: Option<Arc<Base>>,
}

/// A view as the page is sent it, for one base, one set of filters and one open
/// trade. The base is held so the entry can be told from a later base's.
struct Seen {
    base: Arc<Base>,
    key: String,
    view: Arc<View>,
}

/// How many views of the current base are kept: the page's own, another tab's,
/// the filter just cleared.
const VIEWS_KEPT: usize = 8;

#[derive(Default)]
pub struct ModelCache {
    inner: Mutex<Inner>,
    views: Mutex<Vec<Seen>>,
}

/// What a call found it had to do: for the tests, and for a log line.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Work {
    pub read: Vec<&'static str>,
    pub built: Vec<&'static str>,
}

fn moved(old: &Gens, new: &Gens, names: &[&str]) -> bool {
    names.iter().any(|n| old.get(*n) != new.get(*n))
}

impl ModelCache {
    pub fn new() -> ModelCache {
        ModelCache::default()
    }

    /// Forget everything: the next call reads and builds it all. Nothing needs
    /// this to stay correct -- the counters are exact -- it is for a caller that
    /// replaced the database file itself, which only a test does.
    #[cfg(test)]
    pub fn clear(&self) {
        *self.inner.lock().unwrap_or_else(|e| e.into_inner()) = Inner::default();
        self.views.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    pub fn base(&self, conn: &Connection, today: &str) -> rusqlite::Result<Arc<Base>> {
        self.base_and_work(conn, today).map(|(b, _)| b)
    }

    /// The view of `base` under `filters`, with `detail`'s legs and fills, as the
    /// page is sent it. Asked again for the same base, filters and trade it is the
    /// same object: a second tab, a reload, a filter set and cleared cost nothing.
    /// Views of an earlier base are dropped the first time a newer one is asked for.
    pub fn view(&self, base: &Arc<Base>, filters: Option<&Value>, detail: Option<&str>) -> Arc<View> {
        let key = format!("{}|{}", bagholder_model::filters::clean_filters(filters).key(), detail.unwrap_or(""));
        {
            let mut views = self.views.lock().unwrap_or_else(|e| e.into_inner());
            views.retain(|s| Arc::ptr_eq(&s.base, base));
            if let Some(at) = views.iter().position(|s| s.key == key) {
                let hit = views.remove(at);
                let view = hit.view.clone();
                views.push(hit); // most recently used last
                return view;
            }
        }
        // built outside the lock: a slow view never holds up a cached one
        let view = Arc::new(bagholder_model::view::view_of(base, filters, bagholder_model::view::Detail::Only(detail)));
        let mut views = self.views.lock().unwrap_or_else(|e| e.into_inner());
        views.retain(|s| Arc::ptr_eq(&s.base, base));
        if views.len() >= VIEWS_KEPT {
            views.remove(0);
        }
        views.push(Seen { base: base.clone(), key, view: view.clone() });
        view
    }

    pub fn base_and_work(&self, conn: &Connection, today: &str) -> rusqlite::Result<(Arc<Base>, Work)> {
        // a build that panicked left nothing half made: every field is replaced whole
        let mut c = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut work = Work::default();

        // one read transaction, so the counters and the rows are of the same moment
        let tx = conn.unchecked_transaction()?;
        let now = gens::all(conn)?;
        if c.base.is_some() && c.gens == now && c.today == today {
            return Ok((c.base.clone().unwrap(), work));
        }
        let first = c.inputs.is_none();
        let old = std::mem::take(&mut c.gens);
        let mut i = match c.inputs.take() {
            Some(i) => i,
            None => Inputs::default(),
        };
        macro_rules! part {
            ($name:literal, [$($gen:literal),+], $load:expr) => {
                if first || moved(&old, &now, &[$($gen),+]) {
                    $load;
                    work.read.push($name);
                }
            };
        }
        part!("activities", ["activities"], i.set_raw_activities(activities::all_raw_activities(conn)?));
        part!("securities", ["securities"], i.securities = Arc::new(rows::securities(conn)?));
        part!("fx", ["fx"], i.fx = Arc::new(rows::fx(conn, tables::FX_PAIR)?));
        part!("benchmark", ["benchmark"], {
            i.benchmark = Arc::new(rows::benchmark(conn, tables::BENCHMARK_SYMBOL)?);
            let mut all = std::collections::HashMap::new();
            for sym in market::BENCHMARK_SYMBOLS.iter() {
                all.insert((*sym).to_string(), rows::benchmark(conn, sym)?);
            }
            i.benchmarks = Arc::new(all);
        });
        part!("distributions", ["distributions"], i.distributions = Arc::new(rows::distributions(conn)?));
        part!("quotes", ["quotes"], i.quotes = Arc::new(rows::quotes(conn)?));
        part!("groups", ["groups"], i.groups = Arc::new(rows::groups(conn)?));
        part!("journal", ["journal"], i.journal = Arc::new(rows::journal(conn)?));
        part!("accounts", ["accounts"], i.accounts = Arc::new(rows::accounts(conn)?));
        part!("balances", ["balances"], i.balances = Arc::new(rows::balances(conn)?));
        part!("margin", ["margin"], i.margin = Arc::new(rows::margin(conn)?));
        part!("nav", ["nav"], {
            let (nav, by_account) = rows::nav(conn)?;
            i.nav = Arc::new(nav);
            i.nav_by_account = Arc::new(by_account);
        });
        part!("exposures", ["exposures"], i.exposures = Arc::new(rows::exposures(conn)?));
        part!("watchlist", ["watchlist"], i.watchlist = Arc::new(rows::watchlist(conn)?));
        part!("news", ["news"], i.news = Arc::new(rows::news(conn)?));
        part!("universes", ["universes"], i.universes = Arc::new(rows::universes(conn)?));
        part!("tiles", ["tiles"], i.tiles = Arc::new(rows::tiles(conn)?));
        part!("synced", ["synced"], i.synced_at = snapshot::synced_at_part(conn)?);
        drop(tx);

        let day_turned = c.today != today;
        let was = |names: &[&str]| work.read.iter().any(|r| names.contains(r));
        let book_moved = first || day_turned || was(&["activities", "securities"]);
        let prev = c.layers.take();
        let book = match (&prev, book_moved) {
            (Some(p), false) => p.book.clone(),
            _ => {
                work.built.push("book");
                Arc::new(base::book_layer(&i, today))
            }
        };
        macro_rules! layer {
            ($name:literal, $field:ident, $moved:expr, $build:expr) => {
                match (&prev, book_moved || $moved) {
                    (Some(p), false) => p.$field.clone(),
                    _ => {
                        work.built.push($name);
                        Arc::new($build)
                    }
                }
            };
        }
        let trades = layer!("trades", trades, was(&["fx", "groups", "journal"]), base::trades_layer(&book, &i));
        let cashflow = layer!("cashflow", cashflow, was(&["fx"]), base::cashflow_layer(&book, &i));
        let positions = layer!("positions", positions, was(&["balances", "accounts", "journal", "quotes"]), base::positions_layer(&book, &i, today));
        let accounts = layer!("accounts", accounts, was(&["accounts"]), base::accounts_layer(&i));
        let (equity, equity_by_account) = match (&prev, first || was(&["nav"])) {
            (Some(p), false) => (p.equity.clone(), p.equity_by_account.clone()),
            _ => {
                work.built.push("equity");
                let (e, by) = base::equity_layer(&i);
                (Arc::new(e), Arc::new(by))
            }
        };
        let layers = Layers { book, trades, cashflow, positions, equity, equity_by_account, accounts };
        let built = Arc::new(Base::assemble(today, &i, &layers));

        c.gens = now;
        c.today = today.to_string();
        c.inputs = Some(i);
        c.layers = Some(layers);
        c.base = Some(built.clone());
        Ok((built, work))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let conn = bagholder_store::open_db(&dir.path().join("bagholder.db")).unwrap();
        bagholder_store::relabel::ensure(&conn).unwrap();
        (dir, conn)
    }

    fn act(id: &str, side: &str, qty: f64, px: f64, day: &str) -> Value {
        let sign = if side == "BUY" { 1.0 } else { -1.0 };
        json!({"id": id, "canonicalId": id, "accountId": "a1", "accountType": "Trading", "symbol": "QNC", "name": "Quantum", "currency": "CAD",
               "commission": 0, "category": "trade", "activityType": side, "activitySubType": "", "rawType": format!("DIY_{}", side),
               "quantity": sign * qty, "unitPrice": px, "netCashAmount": -sign * qty * px, "transactionDate": day, "source": "wealthsimple"})
    }

    fn book_of(conn: &Connection, rows: &[Value]) {
        let n = std::cell::Cell::new(0u64);
        let id = || { n.set(n.get() + 1); format!("00000000-0000-4000-8000-{:012}", n.get()) };
        let rows: Vec<bagholder_store::activities::ActivityRow> = rows.iter().map(|v| serde_json::from_value(v.clone()).unwrap()).collect();
        bagholder_store::merge::apply_wealthsimple_mapped(conn, &rows, &id).unwrap();
    }

    const TODAY: &str = "2026-03-02";

    fn seeded() -> (tempfile::TempDir, Connection, ModelCache) {
        let (dir, conn) = db();
        book_of(&conn, &[act("b1", "BUY", 100.0, 1.50, "2026-01-05"), act("s1", "SELL", 40.0, 1.80, "2026-02-02")]);
        let cache = ModelCache::new();
        let (first, work) = cache.base_and_work(&conn, TODAY).unwrap();
        assert!(work.built.contains(&"book") && work.read.contains(&"activities"), "the first call reads and builds everything");
        assert_eq!((first.trades.len(), first.positions.len()), (1, 1));
        (dir, conn, cache)
    }

    #[test]
    fn test_nothing_changed_is_the_same_base_and_no_work() {
        let (_d, conn, cache) = seeded();
        let (a, _) = cache.base_and_work(&conn, TODAY).unwrap();
        let (b, work) = cache.base_and_work(&conn, TODAY).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(work, Work::default());
    }

    #[test]
    fn test_a_quote_tick_reads_the_quotes_and_marks_the_positions_and_nothing_else() {
        let (_d, conn, cache) = seeded();
        let (before, _) = cache.base_and_work(&conn, TODAY).unwrap();
        market::upsert_quote(&conn, "QNC", &bagholder_store::market::QuoteRecord { price: Some(2.10), ..Default::default() }, "tmx", "2026-03-02T15:00:00Z").unwrap();
        let (after, work) = cache.base_and_work(&conn, TODAY).unwrap();
        assert_eq!(work, Work { read: vec!["quotes"], built: vec!["positions"] });
        // the match, the closed trades, the cashflow and the equity curve are the very same objects
        assert!(Arc::ptr_eq(&before.book, &after.book), "the FIFO match was not run again");
        assert!(Arc::ptr_eq(&before.trades, &after.trades));
        assert!(Arc::ptr_eq(&before.cashflow, &after.cashflow));
        assert!(Arc::ptr_eq(&before.equity, &after.equity));
        // and the position is marked at the new price
        assert_eq!((after.positions[0].last, after.positions[0].price_source), (2.10, bagholder_model::wire::Mark::Quote));
        assert_ne!(before.positions[0].last, after.positions[0].last);
    }

    #[test]
    fn test_the_same_price_read_again_is_no_work_at_all() {
        let (_d, conn, cache) = seeded();
        market::upsert_quote(&conn, "QNC", &bagholder_store::market::QuoteRecord { price: Some(2.10), ..Default::default() }, "tmx", "2026-03-02T15:00:00Z").unwrap();
        let (a, _) = cache.base_and_work(&conn, TODAY).unwrap();
        market::upsert_quote(&conn, "QNC", &bagholder_store::market::QuoteRecord { price: Some(2.10), ..Default::default() }, "tmx", "2026-03-02T15:01:00Z").unwrap();
        let (b, work) = cache.base_and_work(&conn, TODAY).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(work, Work::default());
    }

    #[test]
    fn test_a_view_asked_for_again_is_the_same_view_until_the_base_moves() {
        let (_d, conn, cache) = seeded();
        let base = cache.base(&conn, TODAY).unwrap();
        let all = cache.view(&base, None, None);
        assert!(Arc::ptr_eq(&all, &cache.view(&base, None, None)), "the same base and filters: not built again");
        assert!(Arc::ptr_eq(&all, &cache.view(&base, Some(&json!({})), None)), "no filters, spelled another way");
        let winners = cache.view(&base, Some(&json!({"lists": {"result": ["Winners"]}})), None);
        assert!(!Arc::ptr_eq(&all, &winners));
        assert!(Arc::ptr_eq(&all, &cache.view(&base, None, None)), "and the first is still kept beside it");
        market::upsert_quote(&conn, "QNC", &bagholder_store::market::QuoteRecord { price: Some(2.10), ..Default::default() }, "tmx", "2026-03-02T15:00:00Z").unwrap();
        let ticked = cache.base(&conn, TODAY).unwrap();
        let after = cache.view(&ticked, None, None);
        assert!(!Arc::ptr_eq(&all, &after));
        assert_eq!(after.to_value()["positions"][0]["last"], json!(2.10));
    }

    #[test]
    fn test_news_and_margin_rebuild_no_layer() {
        let (_d, conn, cache) = seeded();
        let (before, _) = cache.base_and_work(&conn, TODAY).unwrap();
        bagholder_store::feeds::replace_news(&conn, "QNC", "TSX-V", &[bagholder_store::feeds::NewsItem {
            id: "n1".into(), headline: "QNC files".into(), source: "Wire".into(), url: "https://example.test/1".into(),
            published_at: "2026-03-01T12:00:00Z".into(), summary: String::new(), kind: bagholder_store::feeds::NewsKind::Story, via: bagholder_store::feeds::Feed::Tmx,
        }], "2026-03-02T15:00:00Z").unwrap();
        tables::replace_margin(&conn, &[bagholder_store::broker::Margin { account_id: "a1".into(), buying_power: Some(500.0), currency: "CAD".into(), unavailable: String::new(), fetched_at: String::new() }], "2026-03-02T15:00:00Z").unwrap();
        let (after, work) = cache.base_and_work(&conn, TODAY).unwrap();
        assert_eq!(work, Work { read: vec!["margin", "news"], built: vec![] });
        assert!(Arc::ptr_eq(&before.book, &after.book) && Arc::ptr_eq(&before.positions, &after.positions));
        assert_eq!((after.news.len(), after.margin.len()), (1, 1), "though the base carries the new rows");
    }

    #[test]
    fn test_a_journal_entry_leaves_the_match_standing() {
        let (_d, conn, cache) = seeded();
        let (before, _) = cache.base_and_work(&conn, TODAY).unwrap();
        let id = before.trades[0].id.clone();
        bagholder_store::admin::save_journal_entry(&conn, &id, Some(&json!({"thesis": "held through the quarter", "tags": ["swing"], "grade": "B"}))).unwrap();
        let (after, work) = cache.base_and_work(&conn, TODAY).unwrap();
        assert_eq!(work.read, vec!["journal"]);
        assert!(!work.built.contains(&"book"));
        assert!(Arc::ptr_eq(&before.book, &after.book) && Arc::ptr_eq(&before.cashflow, &after.cashflow));
        assert_eq!(after.trades[0].grade, "B");
    }

    #[test]
    fn test_a_new_activity_runs_the_match_again() {
        let (_d, conn, cache) = seeded();
        let (before, _) = cache.base_and_work(&conn, TODAY).unwrap();
        book_of(&conn, &[act("s2", "SELL", 60.0, 2.00, "2026-02-20")]);
        let (after, work) = cache.base_and_work(&conn, TODAY).unwrap();
        assert!(work.built.contains(&"book") && work.built.contains(&"trades") && work.built.contains(&"positions"));
        assert!(!Arc::ptr_eq(&before.book, &after.book));
        assert!(Arc::ptr_eq(&before.equity, &after.equity), "the NAV history did not move");
        // the position is closed out: one trade of two legs, nothing left open
        assert_eq!((after.trades.len(), after.positions.len()), (1, 0));
        assert_eq!(after.trades[0].leg_count, 2);
    }

    #[test]
    fn test_the_day_turning_runs_the_match_again_with_nothing_read() {
        let (_d, conn, cache) = seeded();
        let (after, work) = cache.base_and_work(&conn, "2026-03-03").unwrap();
        assert!(work.read.is_empty());
        assert!(work.built.contains(&"book"), "an option can expire overnight: the day is an input of the match");
        assert_eq!(after.today, "2026-03-03");
    }

    #[test]
    fn test_the_layered_base_is_the_base_built_from_scratch() {
        // after a run of changes, what the cache holds is what a whole rebuild gives
        let (_d, conn, cache) = seeded();
        market::upsert_quote(&conn, "QNC", &bagholder_store::market::QuoteRecord { price: Some(2.10), ..Default::default() }, "tmx", "2026-03-02T15:00:00Z").unwrap();
        cache.base_and_work(&conn, TODAY).unwrap();
        book_of(&conn, &[act("b2", "BUY", 25.0, 1.95, "2026-02-25")]);
        cache.base_and_work(&conn, TODAY).unwrap();
        market::upsert_quote(&conn, "QNC", &bagholder_store::market::QuoteRecord { price: Some(2.25), ..Default::default() }, "tmx", "2026-03-02T15:05:00Z").unwrap();
        let (layered, _) = cache.base_and_work(&conn, TODAY).unwrap();

        let snap = snapshot::snapshot(&conn, true).unwrap();
        let mkt = market::market_data(&conn).unwrap();
        let journal = snapshot::journal(&conn).unwrap();
        let scratch = bagholder_model::base::build_base(&snap, &mkt, &journal, Some(TODAY));
        let view = |b: &Base| bagholder_model::view::build_view(b, None).to_value();
        assert_eq!(view(&layered), view(&scratch));
    }
}
