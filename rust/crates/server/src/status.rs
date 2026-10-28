//! What the header shows: the connection, the sync under way, the update on
//! offer, the counts. It rides with the model on the page's stream, so a change
//! here reaches the page as the field that changed.

use serde::Serialize;
use std::sync::Arc;

use bagholder_diff_derive::Diff;
use ts_rs::TS;

use crate::app::{self, App};
use crate::notify::NotifyStatus;
use crate::{login, notify, orders, session, update, versions};

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
    /// The import running now and how far it has come, for the import window.
    pub importing: Option<crate::csv_import::Importing>,
    /// The last import that ended: what it did, or why nothing of it was kept.
    pub imported: Option<crate::csv_import::Imported>,
    /// The language model's file while it downloads, and how far it has come.
    pub model_download: Option<ModelDownload>,
    /// The agents' address: on or off, its tunnel, the sign-ins waiting on the
    /// person, and every agent that may read (`http::agents`).
    pub agents: crate::http::agents::AgentsStatus,
}

/// The language model's file downloading: the bytes written of its pinned size.
#[derive(Clone, Debug, Default, PartialEq, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownload {
    pub received: u64,
    pub size: u64,
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
    /// Absent when the market cache cannot be read, whose failure the error line
    /// says: never made up.
    pub data_version: Option<String>,
    pub core_version: Option<String>,
}

/// The header, as the page's stream and `/api/status` both read it. Read with the
/// figure path not open, it opens it again first: a book or cache mended while the
/// server runs opens at the next reading of the header (#516).
pub fn status(app: &Arc<App>) -> Status {
    if app.figures.get().is_none() && app.state.lock().unwrap().open_tried {
        crate::open_figures(app);
    }
    // what the header reads that fails is said in its error line, never read as
    // nothing, once it fails again at the next reading, as every failure is
    // (`feeds::feed_failed`): each of its own reads by name, answered when it reads
    let read = |key: &str, failed: Option<String>| match failed {
        Some(why) => crate::feeds::feed_failed(app, key, why),
        None => crate::feeds::feed_answered(app, key),
    };
    // with the book not open, the reason is the open's own failure, said once; the
    // notifications add nothing under their own name
    let open = app.figures.get().is_some();
    let notices = crate::notify::book(app).map_err(|e| format!("The book could not be opened: {e}"));
    // the book's counts, as the figures hold them
    let (acts, accounts) = app.figures.get().and_then(|f| f.read(|e| (e.inputs().ledger.transactions.len() as i64, e.inputs().ledger.accounts.len() as i64))).unwrap_or((0, 0));
    let upd = update::update_status(app);
    // the update check is kept in the market cache: with the cache not opening, the
    // cache's own failure is said, once, and the check adds nothing of its own
    if upd.is_ok() || app.cache().is_ok() {
        read("header:update", upd.as_ref().err().cloned());
    }
    // said above: the header reads on without it
    let upd = match upd {
        Ok(u) => u,
        Err(_) => update::UpdateRecord::default(),
    };
    let sess = session::load_session(app);
    read("header:login", sess.as_ref().err().cloned());
    let sess = sess.unwrap_or(None);
    let notify_status = match &notices {
        Ok(b) => {
            let s = notify::status(app, b).map_err(|e| format!("The notifications could not be read: {e}"));
            read("header:notifications", s.as_ref().err().cloned());
            match s {
                Ok(n) => n,
                // the key stays the run's own whatever the read did
                Err(_) => NotifyStatus { push: notify::PushStatus { key: notify::push_key(app), subscriptions: 0 }, ..NotifyStatus::default() },
            }
        }
        Err(e) => {
            if open {
                read("header:notifications", Some(e.clone()));
            }
            NotifyStatus::default()
        }
    };
    let open_orders = orders::open_orders_count(app, None);
    let can_update = update::can_update(app, &upd);
    let off = update::updates_off();
    // a market source failing is asked again at its next turn and goes to the log,
    // as it starts and as it ends, never to the page (brief 79, amended 2026-10-28)
    let market = app.figures.get().map(|f| f.source_failures(app.net.started()).unwrap_or_else(|e| vec![format!("What the market sources answered could not be read: {e}")])).unwrap_or_default();
    app.market_failing.lock().unwrap_or_else(|e| e.into_inner()).pass("the market sources", market.into_iter().map(|s| (s, String::new())));
    let mut sources: Vec<String> = vec![];
    sources.extend(crate::feeds::feed_failures(app));
    sources.extend(orders::order_failures(app));
    // the agents' part reads their tables: never while holding the app's state,
    // which a CSV import takes inside its own write
    let agents = crate::http::agents::status(app);
    let st = app.state.lock().unwrap();
    let connected = st.connected && sess.as_ref().map(|x| !x.access_token.is_empty()).unwrap_or(false);
    let error = failures(&st, &sources);
    let email = if !st.email.is_empty() { st.email.clone() } else { sess.as_ref().map(|x| x.email.clone()).unwrap_or_default() };
    Status {
        ok: true,
        connected,
        email,
        last_sync: synced(&st),
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
        orders_live: orders::orders_live(app),
        open_orders,
        updating: st.updating.clone(),
        update_error: st.update_error.clone(),
        importing: st.importing.clone(),
        imported: st.imported.clone(),
        model_download: bagholder_market::localmodel::downloading().map(|(received, size)| ModelDownload { received, size }),
        notify: notify_status,
        agents,
    }
}

/// The time the header's "Synced" states: the last pull that read Wealthsimple,
/// unless a read whose figures are shown has failed at its turn since, then that
/// read's last answer (brief 79, amended 2026-10-28, point 3): a part that keeps
/// failing ages it without the page naming the part, and the reads made once a
/// day never age it while they answer.
pub fn synced(st: &app::State) -> String {
    let last: Option<bagholder_core::jiff::Timestamp> = st.last_sync.parse().ok();
    let failing = [(st.balances_failing.any(), st.balances_answered), (st.pull_failing.any(), st.pull_answered)];
    let oldest = failing.iter().filter(|(any, _)| *any).map(|(_, at)| at.or(last)).min().flatten();
    match oldest {
        Some(at) if last.is_none_or(|l| at < l) => at.to_string(),
        _ => st.last_sync.clone(),
    }
}

/// The header's error line: every failure standing now, each from its own
/// state and each gone when its own next success comes, said together as
/// sentences in one line. Wealthsimple's (the pull, the session, the sign-in),
/// the balances', the figures', then each market source failing (SPEC §1: every
/// failure is said in the header until that source succeeds). Empty when
/// nothing is failing; one failure met by two readers is said once.
pub fn failures(st: &app::State, sources: &[String]) -> String {
    let own = [st.error.as_str(), st.portfolio_error.as_str(), st.figures_error.as_str(), st.start_error.as_str(), st.open_error.as_str()];
    let mut said: Vec<String> = vec![];
    for s in own.into_iter().chain(sources.iter().map(String::as_str)).map(str::trim).filter(|e| !e.is_empty()).map(sentence) {
        if !said.contains(&s) {
            said.push(s);
        }
    }
    said.join(" ")
}

/// An instrument as the page names it: its bare ticker.
#[cfg(test)]
pub(crate) fn symbol(inputs: &bagholder_engine::input::Inputs, i: bagholder_core::InstrumentId) -> String {
    let s = crate::wire::build::shown(inputs, i).symbol;
    if s.is_empty() { "An instrument with no name on record".into() } else { crate::orders::bare_symbol(&s) }
}

/// `s` ending as a sentence does.
pub(crate) fn sentence(s: &str) -> String {
    // started with a capital, as a sentence is, whoever wrote the words; a first
    // word with capitals of its own is a name written that way (iShares), kept
    let first_word = s.split_whitespace().next().unwrap_or("");
    let s = if first_word.chars().any(char::is_uppercase) {
        s.to_string()
    } else {
        let mut c = s.chars();
        c.next().map(|first| first.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
    };
    if s.is_empty() || s.ends_with(['.', '!', '?', '…']) {
        s
    } else {
        format!("{s}.")
    }
}

/// `GET /api/status`'s own answer: `status` plus the two version strings a
/// polling page reloads on. The header is answered whatever the market cache
/// does: a version that cannot be read is absent, and why is in the error line.
pub fn answer(app: &Arc<App>) -> StatusAnswer {
    let versions = crate::feeds::conn(app).ok().and_then(|conn| {
        let read = versions::data_version(&conn).and_then(|d| versions::core_version(&conn).map(|c| (d, c)));
        crate::feeds::went(app, "header:versions", read.map_err(|e| format!("The market cache's version could not be read: {e}")))
    });
    let today = bagholder_model::clock::today_local();
    StatusAnswer {
        status: status(app),
        data_version: versions.as_ref().map(|(d, _)| format!("{d}|{today}")),
        // everything the model reads except the quotes: when this is unchanged but the
        // data version moved, only prices ticked (read by the legacy page, which polls)
        core_version: versions.as_ref().map(|(_, c)| format!("{c}|{today}")),
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
        app.set_figures(crate::figures::Figures::open(home.path(), Timestamp::now()).unwrap());
        (home, app)
    }

    /// Record one outcome of `source` in the app's cache, as a reader does.
    /// A source's outcome; a failure as often in a row as it takes to be said
    /// (`health::SAID_AFTER`), so the source is failing, not one request.
    fn record(app: &App, source: &'static str, outcome: OutcomeKind) -> OutcomeRow {
        let row = OutcomeRow { source: SourceName::named(source), host: "h".into(), kind: DataKind::Quote, instrument: None, outcome, detail: String::new(), shape_change: None, at: Timestamp::now() };
        let times = if outcome.is_failure() || outcome == OutcomeKind::Refused { bagholder_sources::health::SAID_AFTER } else { 1 };
        for _ in 0..times {
            app.figures.get().unwrap().cache().unwrap().record(&row).unwrap();
        }
        row
    }

    fn error(app: &Arc<App>) -> String {
        super::status(app).error
    }

    /// Each failure is a sentence: a capital first, a stop last, and a name written
    /// with its own capitals left as it is.
    #[test]
    fn test_each_failure_is_said_as_a_sentence() {
        for (said, sentence) in [
            ("the book could not be opened: file is not a database", "The book could not be opened: file is not a database."),
            ("The market cache could not be opened.", "The market cache could not be opened."),
            ("iShares Canada refused the request.", "iShares Canada refused the request."),
            ("éclair could not be read", "Éclair could not be read."),
            ("why?", "Why?"),
            ("", ""),
        ] {
            assert_eq!(super::sentence(said), sentence);
        }
    }

    const FAILURES: [OutcomeKind; 4] = [OutcomeKind::Refused, OutcomeKind::Unreachable, OutcomeKind::Mismatch, OutcomeKind::Meaning];

    /// A market source failing is asked again at its next turn and goes to the
    /// log, never to the page (brief 79, amended 2026-10-28): whatever fails, and
    /// however many fail together, the header says nothing of it.
    #[test]
    fn test_a_failing_market_source_is_never_said_on_the_page() {
        let (_home, app) = fresh();
        assert_eq!(error(&app), "");
        for (source, _) in LABELS {
            for kind in FAILURES {
                assert!(health::failure(&record(&app, source, kind)).is_some(), "{source} failed");
                record(&app, source, kind);
                assert_eq!(error(&app), "", "{source}: nothing on the page");
            }
        }
        for (source, _) in LABELS {
            record(&app, source, OutcomeKind::Answered);
        }
        assert_eq!(error(&app), "");
    }

    /// Wealthsimple's one sentence and a failed figures pass each stand until
    /// their own next success; a market source failing beside them adds nothing.
    #[test]
    fn test_wealthsimple_and_the_figures_each_keep_their_own_failure() {
        let (_home, app) = fresh();
        record(&app, LABELS[0].0, OutcomeKind::Unreachable);
        record(&app, LABELS[0].0, OutcomeKind::Unreachable);
        app.state.lock().unwrap().error = crate::reading::UNREACHABLE.into();
        crate::due::settle(&app, Err("the book is locked".into()));
        let all = error(&app);
        assert_eq!(all, format!("{} The figures could not be brought up to date: the book is locked.", crate::reading::UNREACHABLE));
        // Wealthsimple's next good read clears its own, and only its own
        app.state.lock().unwrap().error.clear();
        assert_eq!(error(&app), "The figures could not be brought up to date: the book is locked.");
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
}
