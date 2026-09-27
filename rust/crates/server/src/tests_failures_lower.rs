//! The lower crates' failures reaching the header (`docs/plans/stage-5-interface-and-running.md`,
//! B): a bar read the store refuses, an archive pass, the exposures and a
//! listing's short interest. Each is made to fail, seen in the header's error
//! line, and seen to leave it on the next good read. The process is offline
//! (`tests_common::home`), so no source is asked anything.
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::App;
use crate::feeds::HistoryQuery;
use crate::tests_common::{app, guard};

fn error(app: &Arc<App>) -> String {
    crate::status::status(app).error
}

fn until(what: &str, ok: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !ok() {
        assert!(Instant::now() < deadline, "{what} never happened");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// An app of its own, on a home of its own.
fn own_app() -> (tempfile::TempDir, Arc<App>) {
    let home = tempfile::tempdir().unwrap();
    let app = App::new(home.path().to_path_buf(), std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into());
    bagholder_store::schema::init_schema(&app.open().unwrap()).unwrap();
    (home, app)
}

/// SQL run on `app`'s store now, and its undoing run when dropped, whatever the
/// test did in between: the shared app's store is left as it was found.
struct Broken<'a> {
    app: &'a Arc<App>,
    undo: &'static str,
}

impl<'a> Broken<'a> {
    fn new(app: &'a Arc<App>, sql: &str, undo: &'static str) -> Self {
        app.open().unwrap().execute_batch(sql).unwrap();
        Broken { app, undo }
    }
}

impl Drop for Broken<'_> {
    fn drop(&mut self) {
        self.app.open().unwrap().execute_batch(self.undo).unwrap();
    }
}

const REFUSE_MISSES: &str = "CREATE TRIGGER refuse_misses BEFORE INSERT ON meta WHEN NEW.key LIKE 'bars_miss:%' BEGIN SELECT RAISE(ABORT, 'refused'); END;";
const ALLOW_MISSES: &str = "DROP TRIGGER refuse_misses;";

#[test]
fn a_chart_s_daily_read_the_store_refuses_is_said_in_the_header_until_one_goes_through() {
    let _g = guard();
    let (_home, app) = own_app();
    let today = bagholder_market::clock_now().0;
    let q = HistoryQuery { symbol: "QMET".into(), exchange: "TSXV".into(), currency: "CAD".into(), kind: "Shares".into(), from: today.clone(), to: today, tf: "1d".into() };
    {
        let _broken = Broken::new(&app, REFUSE_MISSES, ALLOW_MISSES);
        crate::feeds::history_payload(&app, &q);
        until("the refused read said in the header", || error(&app).contains("daily bars of QMET could not be stored"));
    }
    // the refused read left no miss behind, so the chart is due its read again
    crate::feeds::history_payload(&app, &q);
    until("the failure gone once a read went through", || !error(&app).contains("could not be stored"));
}

#[test]
fn an_archive_pass_the_store_refuses_is_said_in_the_header_until_one_goes_through() {
    let _g = guard();
    let app = app();
    {
        let _broken = Broken::new(&app, REFUSE_MISSES, ALLOW_MISSES);
        assert_eq!(crate::feeds::archive_intraday_bars(&app, Some(1)), Vec::<String>::new());
        assert!(error(&app).contains("price bars could not be archived"), "{}", error(&app));
    }
    assert!(!crate::feeds::archive_intraday_bars(&app, Some(1)).is_empty(), "the book has listings to archive");
    assert!(!error(&app).contains("could not be archived"), "{}", error(&app));
}

#[test]
fn exposures_whose_store_cannot_be_read_are_said_in_the_header_until_a_pass_reads_it() {
    let _g = guard();
    let app = app();
    {
        let _broken = Broken::new(&app, "ALTER TABLE exposures RENAME TO exposures_away;", "ALTER TABLE exposures_away RENAME TO exposures;");
        crate::feeds::refresh_exposures(&app);
        assert!(error(&app).contains("exposures could not be refreshed"), "{}", error(&app));
    }
    crate::feeds::refresh_exposures(&app);
    assert!(!error(&app).contains("exposures could not be refreshed"), "{}", error(&app));
}

#[test]
fn short_interest_that_could_not_be_read_is_said_in_the_header_until_the_listing_reads() {
    use bagholder_market::shorts::{self, CaPositionRow, CaVolumeRow};
    use std::collections::HashMap;
    let _g = guard();
    let (_home, app) = own_app();
    let today = bagholder_market::clock_now().0;
    // the regulator's files as already read, so the read asks no one
    let mut position = HashMap::new();
    position.insert("QNC".to_string(), CaPositionRow { venue: "TSXV".into(), shares: 2667164.0, change: Some(64077.0), name: "QUANTUM EMOTION CORP.".into() });
    shorts::clear_files();
    shorts::ca_position_with("QNC", "TSX-V", &today, || Ok(Some(("2026-09-15".to_string(), position)))).unwrap();
    let mut volume = HashMap::new();
    volume.insert("QNC".to_string(), CaVolumeRow { venue: "TSXV".into(), short_volume: 1_000_000.0, volume_pct: Some(20.0), total_volume: Some(5_000_000.0) });
    shorts::warm_ca_volume("ca_volume", "2026-09-01/2026-09-15", volume);
    {
        // the market's trading days, which days to cover is counted over, cannot be read
        let _broken = Broken::new(&app, "ALTER TABLE benchmark_prices RENAME TO benchmark_prices_away;", "ALTER TABLE benchmark_prices_away RENAME TO benchmark_prices;");
        assert!(crate::feeds::read_shorts(&app, "QNC", "TSX-V", "CAD", false, "").unwrap().is_none());
        assert!(error(&app).contains("short interest of QNC could not be read"), "{}", error(&app));
    }
    assert!(crate::feeds::read_shorts(&app, "QNC", "TSX-V", "CAD", false, "").unwrap().is_some());
    assert!(!error(&app).contains("short interest"), "{}", error(&app));
    shorts::clear_files();
}
