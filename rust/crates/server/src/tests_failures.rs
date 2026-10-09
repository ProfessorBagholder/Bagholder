//! Each failure the stage 5 survey found (`docs/plans/stage-5-interface-and-running.md`,
//! "Failures.") made to happen: it is said in the header's error line, or in the
//! answer to the request that met it, while it lasts, and it is gone on the next
//! good read. Never an empty answer, a default, or `ok: true`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bagholder_core::jiff::Timestamp;

use crate::app::App;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// An app on a home of its own, its store prepared and its figures open.
fn fresh() -> (tempfile::TempDir, Arc<App>) {
    crate::tests_common::home(); // offline, dry orders
    let home = tempfile::tempdir().unwrap();
    let app = App::new(home.path().to_path_buf(), root(), "127.0.0.1".into());
    app.set_figures(crate::figures::Figures::open(home.path(), Timestamp::now()).unwrap());
    (home, app)
}

fn error(app: &Arc<App>) -> String {
    crate::status::status(app).error
}

/// The market cache's file moved aside and a folder put in its place: nothing can open it.
fn break_cache(home: &Path) {
    std::fs::rename(home.join("market.db"), home.join("market.db.kept")).unwrap();
    std::fs::create_dir(home.join("market.db")).unwrap();
}

fn mend_cache(home: &Path) {
    std::fs::remove_dir(home.join("market.db")).unwrap();
    std::fs::rename(home.join("market.db.kept"), home.join("market.db")).unwrap();
}

#[test]
fn test_a_market_cache_that_will_not_open_is_said_by_the_pass_that_met_it_until_it_opens() {
    let (home, app) = fresh();
    assert_eq!(error(&app), "");
    break_cache(home.path());
    assert_eq!(crate::feeds::sweep_shorts(&app), 0);
    assert!(!error(&app).contains("The market cache could not be opened"), "one failure, which the next pass may answer, is not said");
    assert_eq!(crate::feeds::sweep_shorts(&app), 0);
    let said = error(&app);
    assert_eq!(said.matches("The market cache could not be opened").count(), 1, "said once, by the pass and the header alike: {said}");
    // the request that meets it answers it, not an empty or a default answer
    assert!(crate::status::answer(&app).unwrap_err().contains("The market cache could not be opened"));
    assert!(crate::feeds::filings_stored(&app, "QNC").unwrap_err().contains("The market cache could not be opened"));
    mend_cache(home.path());
    assert_eq!(crate::feeds::sweep_shorts(&app), 0);
    assert_eq!(error(&app), "", "the next pass that opens it takes the failure away");
    assert!(crate::status::answer(&app).is_ok());
}

#[test]
fn test_a_read_that_fails_is_the_answer_of_the_request_never_an_empty_list() {
    let (_home, app) = fresh();
    assert!(crate::feeds::filings_stored(&app, "QNC").unwrap().filings.is_empty());
    let c = app.cache().unwrap();
    c.execute("ALTER TABLE filings RENAME TO filings_away", []).unwrap();
    let e = crate::feeds::filings_stored(&app, "QNC").unwrap_err();
    assert!(e.contains("filings"), "{e}");
    c.execute("ALTER TABLE filings_away RENAME TO filings", []).unwrap();
    assert!(crate::feeds::filings_stored(&app, "QNC").is_ok());
}

#[test]
fn test_a_saved_login_that_cannot_be_read_is_said_never_taken_for_signed_out() {
    let (home, app) = fresh();
    let file = home.path().join("session.json");
    std::fs::write(&file, "{ not a login").unwrap();
    // said once the next reading finds it too (`feeds::feed_failed`)
    assert!(!error(&app).contains("The saved Wealthsimple login could not be read"), "one reading is not said");
    let said = error(&app);
    assert!(said.contains("The saved Wealthsimple login could not be read"), "{said}");
    match crate::session::sync_now(&app) {
        crate::session::SyncAnswer { ok: false, error: Some(e), .. } => assert!(e.contains("could not be read"), "{e}"),
        other => panic!("a sync with a login that cannot be read is refused with why: {:?}", other.error),
    }
    std::fs::remove_file(&file).unwrap();
    assert_eq!(error(&app), "", "no login saved is no failure");
    // a sign-in over one that cannot be read says so rather than writing over it unseen
    std::fs::write(&file, "[1, 2]").unwrap();
    let capture = crate::session::Capture { access_token: "a".into(), ..Default::default() };
    let answer = crate::session::capture_tokens(&app, &capture);
    assert!(!answer.ok && answer.error.as_deref().is_some_and(|e| e.contains("could not be read")), "{:?}", answer.error);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "[1, 2]", "left as it was");
}

#[test]
fn test_notifications_that_cannot_be_recorded_or_read_are_said_until_they_can() {
    let (_home, app) = fresh();
    let c = crate::notify::book(&app).unwrap();
    crate::notify::set_settings(&c, &serde_json::from_value(serde_json::json!({"fills": true})).unwrap()).unwrap();
    c.notices().execute("ALTER TABLE notifications RENAME TO notifications_away", []).unwrap();
    assert!(crate::notify::tell(&app, "fills", "order:1:filled", "Order filled · QNC", "Bought 5", None).is_none());
    // the header's own read of the bell fails with each asking: said from the second
    let first = error(&app);
    assert!(!first.contains("A notification could not be recorded") && !first.contains("The notifications could not be read"), "one failure is not said: {first}");
    assert!(crate::notify::tell(&app, "fills", "order:1:filled", "Order filled · QNC", "Bought 5", None).is_none());
    let said = error(&app);
    assert!(said.contains("A notification could not be recorded"), "{said}");
    assert!(said.contains("The notifications could not be read"), "the header's own read of the bell: {said}");
    c.notices().execute("ALTER TABLE notifications_away RENAME TO notifications", []).unwrap();
    assert!(crate::notify::tell(&app, "fills", "order:2:filled", "Order filled · QNC", "Bought 5", None).is_some());
    assert_eq!(error(&app), "");
}

#[test]
fn test_notification_settings_stored_unreadable_are_an_error_never_every_kind_off() {
    let (_home, app) = fresh();
    let c = crate::notify::book(&app).unwrap();
    c.set_setting(crate::notify::SETTINGS_KEY, Some("{ nope"), Timestamp::now()).unwrap();
    assert!(crate::notify::settings(&c).is_err());
    assert!(crate::notify::release_scopes(&c).is_err());
    // said once the next reading finds it too (`feeds::feed_failed`)
    assert!(!error(&app).contains("The notifications could not be read"), "one reading is not said");
    let said = error(&app);
    assert!(said.contains("The notifications could not be read"), "{said}");
    c.set_setting(crate::notify::SETTINGS_KEY, Some("{}"), Timestamp::now()).unwrap();
    assert_eq!(error(&app), "");
}

#[test]
fn test_an_update_check_that_cannot_be_read_is_said_and_refuses_an_update_until_it_can() {
    let (_home, app) = fresh();
    let c = app.cache().unwrap();
    bagholder_store::tables::set_meta(&c, "update_check", "{ nope").unwrap();
    // said once the next reading finds it too (`feeds::feed_failed`)
    assert!(!error(&app).contains("The update check could not be read"), "one reading is not said");
    let said = error(&app);
    assert!(said.contains("The update check could not be read"), "{said}");
    if !crate::update::updates_off() {
        let answer = crate::update::start_update(&app);
        assert!(!answer.ok && answer.error.as_deref().is_some_and(|e| e.contains("could not be read")), "{:?}", answer.error);
    }
    bagholder_store::tables::set_meta(&c, "update_check", "").unwrap();
    assert_eq!(error(&app), "");
}

/// The update rollback's restore: a kept copy that cannot be put back is said by
/// the server started next, and the kept copies stay for another try.
#[test]
#[cfg(unix)]
fn test_a_rollback_that_cannot_put_the_previous_version_back_is_said_and_keeps_it() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("previous")).unwrap();
    std::fs::write(home.path().join("previous").join("bagholder"), "exit 0\n").unwrap();
    // the new version dies at once, inside the healthy window
    std::fs::write(dir.path().join("bagholder"), "exit 7\n").unwrap();
    crate::update::write_pending(home.path(), &crate::update::Pending { tag: "v99.0.0".into(), git: None, at: None }).unwrap();
    // nothing can be written where the executable lives
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    let exe = dir.path().join("bagholder");
    let code = crate::update::supervise_child(home.path(), dir.path(), crate::update::UPDATE_HEALTHY_SEC, || {
        let mut c = std::process::Command::new("sh");
        c.arg(&exe);
        c
    });
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(code, 7, "no version back to start: the supervisor stops with the new one's code");
    assert!(home.path().join("previous").join("bagholder").exists(), "the kept copy stays for another try");
    let a = App::new(home.path().to_path_buf(), home.path().to_path_buf(), "127.0.0.1".into());
    crate::update::recall_failure(&a);
    let said = a.state.lock().unwrap().update_error.clone();
    assert!(said.contains("v99.0.0") && said.contains("could not be put back"), "{said}");
}

#[test]
fn test_the_folder_an_earlier_version_watched_is_taken_once_and_not_again_once_let_go() {
    let (home, app) = fresh();
    let folder = home.path().join("drop");
    std::fs::create_dir(&folder).unwrap();
    let old = bagholder_store::connect(home.path()).unwrap();
    bagholder_store::schema::init_schema(&old).unwrap();
    bagholder_store::csvimport::set_watch_folder(&old, &folder.to_string_lossy()).unwrap();
    let f = app.figures.get().unwrap();
    crate::carry::carry_watch_folder(f, &old, Timestamp::now()).unwrap();
    assert!(crate::csv_import::watching(f), "the folder watched before is watched again");
    crate::csv_import::unwatch(f, Timestamp::now()).unwrap();
    // the next start
    crate::carry::carry_watch_folder(f, &old, Timestamp::now()).unwrap();
    assert!(!crate::csv_import::watching(f), "a folder let go stays let go");
    assert_eq!(error(&app), "");
}

#[test]
fn test_a_folder_watched_before_that_cannot_be_read_is_a_failure_until_it_can_be() {
    let (home, app) = fresh();
    let old = bagholder_store::connect(home.path()).unwrap();
    bagholder_store::schema::init_schema(&old).unwrap();
    old.execute("ALTER TABLE meta RENAME TO meta_away", []).unwrap();
    let f = app.figures.get().unwrap();
    assert!(crate::carry::carry_watch_folder(f, &old, Timestamp::now()).is_err());
    old.execute("ALTER TABLE meta_away RENAME TO meta", []).unwrap();
    crate::carry::carry_watch_folder(f, &old, Timestamp::now()).unwrap();
}

#[test]
fn test_a_distribution_notice_whose_record_cannot_be_read_says_why_until_one_can() {
    let (_home, app) = fresh();
    *app.feeds.record_fails.lock().unwrap() = Some("TMX Money could not be reached".into());
    use bagholder_store::feeds::{Feed, NewsItem, NewsKind};
    let item = |id: &str| NewsItem { id: id.into(), headline: "Announces Monthly Distribution".into(), source: String::new(), url: String::new(), published_at: "2026-09-15T13:00:00Z".into(), summary: String::new(), kind: NewsKind::Release, via: Feed::Tmx };
    crate::feeds::release_notice(&app, "QNC", &[item("tmx:1")]);
    assert!(!error(&app).contains("QNC's declared distributions"), "one failure is not said");
    crate::feeds::release_notice(&app, "QNC", &[item("tmx:1")]);
    assert!(error(&app).contains("QNC's declared distributions could not be read again: TMX Money could not be reached"), "{}", error(&app));
    *app.feeds.record_fails.lock().unwrap() = None;
    crate::feeds::release_notice(&app, "QNC", &[item("tmx:2")]);
    assert_eq!(error(&app), "");
}

#[test]
fn test_one_source_down_is_one_sentence_for_every_listing_it_stopped() {
    let (_home, app) = fresh();
    // each failed twice in a row, so each is said
    for _ in 0..2 {
        for sym in ["ASTS", "LUNR", "MU"] {
            crate::feeds::feed_failed(&app, &format!("shorts:{sym}"), "FINRA could not be reached".into());
        }
        crate::feeds::feed_failed(&app, "shorts:VEQT", "CIRO could not be reached".into());
    }
    assert_eq!(
        error(&app),
        "The short interest of ASTS, LUNR, MU could not be read: FINRA could not be reached. The short interest of VEQT could not be read: CIRO could not be reached."
    );
    crate::feeds::feed_answered(&app, "shorts:LUNR");
    assert!(error(&app).starts_with("The short interest of ASTS, MU could not be read"), "{}", error(&app));
}

/// A single failed request to an outside source is not told to the person: the
/// failure is said once the next asking fails too, and an answer between starts
/// the count again.
#[test]
fn test_a_single_failed_request_to_a_source_is_not_said() {
    let (_home, app) = fresh();
    let feed = "shorts:ZZQQ";
    crate::feeds::feed_failed(&app, feed, "CIRO: HTTP 403".into());
    assert_eq!(error(&app), "", "one failure");
    crate::feeds::feed_answered(&app, feed);
    crate::feeds::feed_failed(&app, feed, "CIRO: HTTP 403".into());
    assert_eq!(error(&app), "", "one failure after an answer");
    crate::feeds::feed_failed(&app, feed, "CIRO: HTTP 403".into());
    assert_eq!(error(&app), "The short interest of ZZQQ could not be read: CIRO: HTTP 403.");
    crate::feeds::feed_answered(&app, feed);
    assert_eq!(error(&app), "");
}
