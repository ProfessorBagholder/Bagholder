//! The market universes (SPEC §4 Markets, the heatmap): read while a page shows
//! one, when it has no rows or they are stale, and not otherwise. The sources are
//! stood in for (`feeds::fetch_universes` under test), which counts each read.
use std::sync::Arc;
use std::time::{Duration, Instant};

use bagholder_market::universes::Source;
use bagholder_model::input::UniverseRow;
use serde_json::{json, Value};

use crate::app::{now_unix, stamp_of, App};
use crate::events::Feed;
use crate::feeds::{universe_due_in, UNIVERSE_STALE_SEC};

/// An app of its own, on a home of its own, with no page open.
fn app() -> (tempfile::TempDir, Arc<App>) {
    let home = tempfile::tempdir().unwrap();
    let app = App::new(home.path().to_path_buf(), std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into());
    bagholder_store::schema::init_schema(&app.open().unwrap()).unwrap();
    (home, app)
}

/// Every universe a source carries, with rows read `age` seconds ago.
fn stored(app: &Arc<App>, source: Source, age: f64) {
    let at = stamp_of((now_unix() - age) as i64);
    let c = app.open().unwrap();
    for key in source.keys() {
        let row = UniverseRow { symbol: format!("{}0", key.to_uppercase()), name: "Held".into(), value: 1.0, percent_change: None, sector: "Energy".into(), country: String::new() };
        bagholder_store::feeds::replace_universe(&c, key, &[row], &at).unwrap();
    }
}

/// A page that shows the documents `keys`.
fn page_showing(app: &Arc<App>, keys: &[&str]) -> Feed {
    let feed = Feed::open(app.clone(), None);
    let docs = keys.iter().map(|k| (k.to_string(), json!({}))).collect();
    assert!(app.events.watch(app, feed.id(), docs));
    feed
}

fn reads(app: &Arc<App>) -> Vec<Source> {
    app.feeds.universe_reads.lock().unwrap().clone()
}

fn until(what: &str, ok: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ok() {
        assert!(Instant::now() < deadline, "{what} never happened");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Long enough for a read that was going to start to have started.
fn settle() {
    std::thread::sleep(Duration::from_millis(300));
}

fn doc(app: &Arc<App>, key: &str) -> Value {
    serde_json::to_value(crate::docs::read(app, key).expect("a market universe is a document")).unwrap()
}

#[test]
fn test_a_shown_universe_with_no_rows_is_read_from_its_own_source_only() {
    for (key, source, other) in [("ca", Source::Tmx, Source::Screener), ("us", Source::Screener, Source::Tmx), ("intl", Source::Screener, Source::Tmx)] {
        let (_home, app) = app();
        let _page = page_showing(&app, &[&format!("universe:{key}")]);
        until("the read", || reads(&app) == vec![source]);
        let c = app.open().unwrap();
        for k in source.keys() {
            assert!(bagholder_store::feeds::universe_read_at(&c, k).unwrap().is_some(), "{k}: a read stores every universe its source carries");
        }
        settle();
        assert_eq!(reads(&app), vec![source], "{key}: read once, and a universe nobody shows ({other:?}) is not read");
        assert_eq!(doc(&app, &format!("universe:{key}")), json!({"failed": null}));
    }
}

#[test]
fn test_a_shown_universe_whose_rows_are_stale_is_read_again() {
    for source in [Source::Tmx, Source::Screener] {
        let (_home, app) = app();
        stored(&app, source, UNIVERSE_STALE_SEC + 60.0);
        let _page = page_showing(&app, &[&format!("universe:{}", source.keys()[0])]);
        until("the read", || reads(&app) == vec![source]);
        let c = app.open().unwrap();
        let at = bagholder_store::feeds::universe_read_at(&c, source.keys()[0]).unwrap().unwrap();
        assert!(now_unix() - crate::app::parse_instant(&at).unwrap() < 60.0, "the rows are the new read's");
    }
}

#[test]
fn test_a_shown_universe_whose_rows_are_fresh_is_not_read() {
    for source in [Source::Tmx, Source::Screener] {
        let (_home, app) = app();
        stored(&app, source, UNIVERSE_STALE_SEC - 120.0);
        let keys: Vec<String> = source.keys().iter().map(|k| format!("universe:{k}")).collect();
        let _page = page_showing(&app, &keys.iter().map(|k| k.as_str()).collect::<Vec<_>>());
        settle();
        assert!(reads(&app).is_empty(), "{source:?}: fresh rows are not read again");
    }
}

#[test]
fn test_no_universe_is_read_while_no_page_shows_one() {
    let (_home, app) = app();
    // a page open on something else
    let _page = page_showing(&app, &["orders"]);
    settle();
    assert!(reads(&app).is_empty());
    // a key that is not a market universe is no document and reads nothing
    assert!(crate::docs::read(&app, "universe:holdings").is_none());
    let _other = page_showing(&app, &["universe:holdings"]);
    settle();
    assert!(reads(&app).is_empty());
}

#[test]
fn test_a_failed_read_is_said_in_the_document_and_not_asked_again_within_the_half_hour() {
    let (_home, app) = app();
    *app.feeds.universe_fails.lock().unwrap() = Some("Nasdaq's screener could not be reached.".into());
    let page = page_showing(&app, &["universe:us"]);
    until("the failure", || doc(&app, "universe:us")["failed"] == json!("Nasdaq's screener could not be reached."));
    // the other universe of the same source says the same, and a second page asks nothing more
    assert_eq!(doc(&app, "universe:intl")["failed"], json!("Nasdaq's screener could not be reached."));
    drop(page);
    let _again = page_showing(&app, &["universe:us", "universe:intl"]);
    settle();
    assert_eq!(reads(&app), vec![Source::Screener], "one attempt per half hour");
    assert!(app.open().map(|c| bagholder_store::feeds::universe_read_at(&c, "us").unwrap().is_none()).unwrap());
}

#[test]
fn test_a_source_comes_due_when_a_universe_it_carries_is_missing_or_its_oldest_rows_are_stale() {
    let now = 1_000_000.0;
    let half = UNIVERSE_STALE_SEC;
    // no rows: due now, whatever the others hold
    assert_eq!(universe_due_in(now, &[None], None), 0.0);
    assert_eq!(universe_due_in(now, &[Some(now), None], None), 0.0);
    // rows: due when the oldest comes to the half hour
    assert_eq!(universe_due_in(now, &[Some(now - 60.0)], None), half - 60.0);
    assert_eq!(universe_due_in(now, &[Some(now - 10.0), Some(now - 100.0)], None), half - 100.0);
    assert_eq!(universe_due_in(now, &[Some(now - half - 1.0)], None), 0.0);
    // asked within the half hour, answered or not: not before the half hour is out
    assert_eq!(universe_due_in(now, &[None], Some(now - 60.0)), half - 60.0);
    assert_eq!(universe_due_in(now, &[Some(now - half - 5.0)], Some(now - 5.0)), half - 5.0);
    assert_eq!(universe_due_in(now, &[None], Some(now - half)), 0.0);
}
