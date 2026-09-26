//! What the header shows: the connection, the sync under way, the update on
//! offer, the counts. It rides with the model on the page's stream, so a change
//! here reaches the page as the field that changed.

use serde::Serialize;
use std::sync::Arc;

use bagholder_diff_derive::Diff;
use ts_rs::TS;

use crate::app::{self, App};
use crate::notify::NotifyStatus;
use crate::{feeds, login, notify, orders, session, update, versions};

/// The header's own data: the connection, the sync under way, the update on
/// offer, the counts. In the order `payload` has always written it.
#[derive(Clone, Debug, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    #[ts(type = "true")]
    pub ok: bool,
    pub connected: bool,
    pub email: String,
    pub last_sync: String,
    pub activity_count: i64,
    pub account_count: i64,
    pub capturing: bool,
    pub syncing: bool,
    pub listings_filling: bool,
    pub sync_step: String,
    pub error: String,
    pub summary_ready: bool,
    pub protocol: String,
    pub started_at: String,
    pub version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub update_url: String,
    pub can_update: bool,
    pub update_by: String,
    /// `true` while `BAGHOLDER_LOGIN_VIEW` asks for the sign-in window shown
    /// as a page rather than streamed frames.
    pub login_view: bool,
    pub orders_live: bool,
    pub open_orders: i64,
    pub updating: String,
    pub update_error: String,
    pub notify: NotifyStatus,
    pub news_reading: Vec<String>,
}

/// `GET /api/status`: `Status` plus the two version strings the legacy
/// polling page reads, which the stream never sends (a page told what moved
/// has no use for them).
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StatusAnswer {
    #[serde(flatten)]
    #[ts(flatten)]
    pub status: Status,
    pub data_version: String,
    pub core_version: String,
}

pub fn status(app: &Arc<App>) -> Status {
    let conn = app.open().ok();
    // the book's counts, as the figures hold them
    let (acts, accounts) = app.figures.get().and_then(|f| f.read(|e| (e.inputs().ledger.transactions.len() as i64, e.inputs().ledger.accounts.len() as i64))).unwrap_or((0, 0));
    let upd = update::update_status(app);
    let sess = session::load_session(app);
    let notify_status = conn.as_ref().and_then(|c| notify::status(c).ok()).unwrap_or_default();
    let open_orders = orders::open_orders_count(app);
    let can_update = update::can_update(app, None);
    let off = update::updates_off();
    let sources = app.figures.get().map(|f| f.source_failures().unwrap_or_else(|e| vec![format!("What the market sources answered could not be read: {e}")])).unwrap_or_default();
    let st = app.state.lock().unwrap();
    let connected = st.connected && sess.as_ref().map(|x| !x.access_token.is_empty()).unwrap_or(false);
    let error = failures(&st, &sources);
    let email = if !st.email.is_empty() { st.email.clone() } else { sess.as_ref().map(|x| x.email.clone()).unwrap_or_default() };
    Status {
        ok: true,
        connected,
        email,
        last_sync: st.last_sync.clone(),
        activity_count: acts,
        account_count: accounts,
        capturing: st.capturing,
        syncing: st.syncing,
        listings_filling: st.listings_filling,
        sync_step: st.sync_step.clone(),
        error,
        summary_ready: bagholder_market::enrich::summary_status() == "ready",
        protocol: app::PROTOCOL.to_string(),
        started_at: app.started_at.clone(),
        version: app::APP_VERSION.to_string(),
        latest_version: upd.latest.clone(),
        update_available: upd.update_available,
        update_url: if off { update::image_page() } else if upd.url.is_empty() { update::repo_url() } else { upd.url.clone() },
        can_update,
        update_by: if off { "image" } else { "app" }.to_string(),
        login_view: login::login_view(),
        orders_live: orders::orders_live(),
        open_orders,
        updating: st.updating.clone(),
        update_error: st.update_error.clone(),
        notify: notify_status,
        news_reading: feeds::news_reading(app),
    }
}

/// The header's error line: every failure standing now, each from its own
/// state and each gone when its own next success comes, said together as
/// sentences in one line. Wealthsimple's (the pull, the session, the sign-in),
/// the balances', the figures', then each market source failing (SPEC §1: every
/// failure is said in the header until that source succeeds). Empty when
/// nothing is failing.
pub fn failures(st: &app::State, sources: &[String]) -> String {
    let own = [st.error.as_str(), st.portfolio_error.as_str(), st.figures_error.as_str()];
    own.into_iter().chain(sources.iter().map(String::as_str)).map(str::trim).filter(|e| !e.is_empty()).map(sentence).collect::<Vec<_>>().join(" ")
}

/// `s` ending as a sentence does.
fn sentence(s: &str) -> String {
    if s.ends_with(['.', '!', '?', '…']) {
        s.to_string()
    } else {
        format!("{s}.")
    }
}

/// `GET /api/status`'s own answer: `status` plus the two version strings a
/// polling page reloads on.
pub fn answer(app: &Arc<App>) -> StatusAnswer {
    let conn = app.open().ok();
    let data_version = conn.as_ref().and_then(|c| versions::data_version(c).ok()).unwrap_or_default();
    let core_version = conn.as_ref().and_then(|c| versions::core_version(c).ok()).unwrap_or_default();
    let today = bagholder_model::clock::today_local();
    StatusAnswer {
        status: status(app),
        data_version: format!("{}|{}", data_version, today),
        // everything the model reads except the quotes: when this is unchanged but the
        // data version moved, only prices ticked (read by the legacy page, which polls)
        core_version: format!("{}|{}", core_version, today),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bagholder_core::jiff::Timestamp;
    use bagholder_core::SourceName;
    use bagholder_sources::cache::OutcomeRow;
    use bagholder_sources::contract::DataKind;
    use bagholder_sources::health::{self, LABELS};
    use bagholder_sources::outcome::OutcomeKind;

    use crate::app::App;

    fn app_on(home: &std::path::Path) -> Arc<App> {
        crate::tests_common::home(); // offline, dry orders
        App::new(home.to_path_buf(), std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."), "127.0.0.1".into())
    }

    /// An app on a home of its own, its figures open and heard on its bus.
    fn fresh() -> (tempfile::TempDir, Arc<App>) {
        let home = tempfile::tempdir().unwrap();
        let app = app_on(home.path());
        bagholder_store::schema::init_schema(&app.open().unwrap()).unwrap();
        app.set_figures(crate::figures::Figures::open(home.path(), Timestamp::now()).unwrap());
        (home, app)
    }

    /// Record one outcome of `source` in the app's cache, as a reader does.
    fn record(app: &App, source: &'static str, outcome: OutcomeKind) -> OutcomeRow {
        let row = OutcomeRow { source: SourceName::named(source), host: "h".into(), kind: DataKind::Quote, instrument: None, outcome, detail: String::new(), shape_change: None, at: Timestamp::now() };
        app.figures.get().unwrap().cache().unwrap().record(&row).unwrap();
        row
    }

    fn error(app: &Arc<App>) -> String {
        super::status(app).error
    }

    const FAILURES: [OutcomeKind; 4] = [OutcomeKind::Refused, OutcomeKind::Unreachable, OutcomeKind::Mismatch, OutcomeKind::Meaning];

    #[test]
    fn test_a_failing_source_is_said_in_the_header_until_that_source_next_answers() {
        let (_home, app) = fresh();
        assert_eq!(error(&app), "");
        for (source, _) in LABELS {
            for kind in FAILURES {
                let said = health::failure(&record(&app, source, kind)).unwrap();
                assert_eq!(error(&app), said);
                // saying it does not carry something says nothing of the source
                record(&app, source, OutcomeKind::NotCarried);
                assert_eq!(error(&app), said);
                record(&app, source, OutcomeKind::Answered);
                assert_eq!(error(&app), "", "{source} answered");
            }
        }
    }

    #[test]
    fn test_several_failing_sources_are_said_together_in_one_line() {
        let (_home, app) = fresh();
        let mut said = vec![];
        for (source, _) in LABELS {
            said.push(health::failure(&record(&app, source, OutcomeKind::Unreachable)).unwrap());
            let line = error(&app);
            assert!(!line.contains('\n'));
            for s in &said {
                assert!(line.contains(s.as_str()), "{s:?} in {line:?}");
            }
        }
        // one answering takes only its own away
        record(&app, LABELS[0].0, OutcomeKind::Answered);
        let line = error(&app);
        assert!(!line.contains(said[0].as_str()));
        assert!(said[1..].iter().all(|s| line.contains(s.as_str())));
    }

    #[test]
    fn test_wealthsimple_and_the_figures_and_the_sources_each_keep_their_own_failure() {
        let (_home, app) = fresh();
        let source = health::failure(&record(&app, LABELS[0].0, OutcomeKind::Unreachable)).unwrap();
        app.state.lock().unwrap().error = "Sync failed: the pull was refused".into();
        app.state.lock().unwrap().portfolio_error = "Balances could not be read: refused".into();
        crate::due::settle(&app, Err("the book is locked".into()));
        let all = error(&app);
        for part in ["Sync failed: the pull was refused.", "Balances could not be read: refused.", "The figures could not be brought up to date: the book is locked.", source.as_str()] {
            assert!(all.contains(part), "{part:?} in {all:?}");
        }
        // Wealthsimple's next good pull clears its own, and only its own
        app.state.lock().unwrap().error.clear();
        let after = error(&app);
        assert!(!after.contains("Sync failed") && after.contains("The figures could not") && after.contains(source.as_str()), "{after:?}");
        // the source answering clears its own, and only its own
        record(&app, LABELS[0].0, OutcomeKind::Answered);
        let after = error(&app);
        assert!(!after.contains(source.as_str()) && after.contains("The figures could not") && after.contains("Balances"), "{after:?}");
    }

    #[test]
    fn test_a_failed_figures_pass_is_said_until_a_pass_succeeds() {
        let (_home, app) = fresh();
        let signals = app.events.subscribe();
        assert_eq!(crate::due::settle(&app, Err("the cache is locked".into())), None);
        assert_eq!(error(&app), "The figures could not be brought up to date: the cache is locked.");
        let next = Some(Timestamp::now());
        assert_eq!(crate::due::settle(&app, Ok(next)), next);
        assert_eq!(error(&app), "");
        // a good pass with nothing to clear writes nothing, so tells no page
        let before = *signals.borrow();
        crate::due::settle(&app, Ok(None));
        assert_eq!(*signals.borrow(), before);
    }

    #[test]
    fn test_a_source_failing_or_answering_is_sent_to_the_open_page() {
        let home = tempfile::tempdir().unwrap();
        crate::tests_common::pulled_book(home.path());
        let app = app_on(home.path());
        bagholder_store::relabel::ensure(&app.open().unwrap()).unwrap();
        let now = Timestamp::now();
        let f = crate::figures::Figures::open(home.path(), now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        app.set_figures(f);
        let mut feed = crate::events::Feed::open(app.clone(), None);
        let first = serde_json::to_string(&feed.step(&super::status).into_iter().map(|(_, m)| m).collect::<Vec<_>>()).unwrap();
        assert!(first.contains("\"error\":\"\""), "nothing is failing yet: {first}");
        let mut signals = app.events.subscribe();
        signals.mark_unchanged();
        let said = health::failure(&record(&app, LABELS[0].0, OutcomeKind::Unreachable)).unwrap();
        // the commit itself tells the bus: nothing polls for it
        assert!(signals.has_changed().unwrap());
        let sent = serde_json::to_string(&feed.step(&super::status).into_iter().map(|(_, m)| m).collect::<Vec<_>>()).unwrap();
        assert!(sent.contains(&said), "{sent}");
        signals.mark_unchanged();
        record(&app, LABELS[0].0, OutcomeKind::Answered);
        assert!(signals.has_changed().unwrap());
        let sent = serde_json::to_string(&feed.step(&super::status).into_iter().map(|(_, m)| m).collect::<Vec<_>>()).unwrap();
        assert!(sent.contains("\"error\"") && !sent.contains(&said), "{sent}");
    }
}


