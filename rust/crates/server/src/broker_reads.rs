//! Wealthsimple read on the figure path (`docs/plans/stage-3c-switch.md`, §3):
//! the pull, on weekdays after 2 PM Mountain and on Sync now, and the balances
//! (cash, and what a margin account can borrow), when a page opens and every five
//! minutes while one is open. Each through the adapter over the one saved sign-in, each stored in
//! the book and applied to the figures (`Figures::record_changed`,
//! `Figures::broker_changed`).
//!
//! Its own thread beside the public sources' scheduler (`due`): a first pull
//! reads every account in full and takes minutes, and the quotes of what is
//! held must not wait on it. Every wait is a known deadline: the next pull
//! window, the next balances read, a failed read's rest; or a change that can
//! make one due (Sync now, a sign-in, a page opening).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use bagholder_book::Book;
use bagholder_broker::{BrokerAdapter, Failure, Step};
use bagholder_core::jiff::civil::{Time, Weekday};
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::{SignedDuration, Timestamp};
use bagholder_core::{Broker, ConnectionId};
use bagholder_wealthsimple::adapter::Wealthsimple;
use bagholder_wealthsimple::client::Client;
use bagholder_wealthsimple::session::SessionFile;

use crate::app::{log, App};
use crate::figures::Figures;

/// Where Wealthsimple's day is settled: a weekday's rows are final after 2 PM
/// Mountain (`SPEC.md` §2, Wealthsimple).
const PULL_ZONE: &str = "America/Edmonton";
const PULL_AT: Time = Time::constant(14, 0, 0, 0);
/// How often the balances are read while a page is open.
pub const BALANCES_EVERY: SignedDuration = SignedDuration::from_secs(5 * 60);
/// A failed read rests this long first, doubling to `REST_MOST`.
const REST_FIRST: SignedDuration = SignedDuration::from_secs(30);
const REST_MOST: SignedDuration = SignedDuration::from_secs(30 * 60);

/// The saved sign-in the adapter reads with.
pub fn session_file(app: &App) -> SessionFile {
    SessionFile { path: app.home.join("session.json") }
}

fn pull_zone() -> Result<TimeZone, String> {
    TimeZone::get(PULL_ZONE).map_err(|e| format!("{PULL_ZONE}: {e}"))
}

fn weekday(d: bagholder_core::jiff::civil::Date) -> bool {
    !matches!(d.weekday(), Weekday::Saturday | Weekday::Sunday)
}

/// The latest weekday 2 PM Mountain at or before `now`.
pub fn last_window(now: Timestamp, zone: &TimeZone) -> Option<Timestamp> {
    let mut day = now.to_zoned(zone.clone()).date();
    for _ in 0..8 {
        if weekday(day) {
            let at = day.to_datetime(PULL_AT).to_zoned(zone.clone()).ok()?.timestamp();
            if at <= now {
                return Some(at);
            }
        }
        day = day.yesterday().ok()?;
    }
    None
}

/// The earliest weekday 2 PM Mountain after `now`.
pub fn next_window(now: Timestamp, zone: &TimeZone) -> Option<Timestamp> {
    let mut day = now.to_zoned(zone.clone()).date();
    for _ in 0..8 {
        if weekday(day) {
            let at = day.to_datetime(PULL_AT).to_zoned(zone.clone()).ok()?.timestamp();
            if at > now {
                return Some(at);
            }
        }
        day = day.tomorrow().ok()?;
    }
    None
}

/// Whether the pull is due: never pulled, or not since the latest window opened.
pub fn pull_due(last_pull: Option<Timestamp>, now: Timestamp, zone: &TimeZone) -> bool {
    match (last_pull, last_window(now, zone)) {
        (None, _) => true,
        (Some(last), Some(window)) => last < window,
        (Some(_), None) => false,
    }
}

/// The book's Wealthsimple connection, made on the first read.
fn connection(book: &Book, now: Timestamp) -> Result<ConnectionId, String> {
    let ws = Broker::named("wealthsimple");
    match book.connections().map_err(|e| e.to_string())?.into_iter().find(|c| c.broker == ws) {
        Some(c) => Ok(c.id),
        None => book.add_connection(&ws, "Wealthsimple", now).map_err(|e| e.to_string()),
    }
}

/// The rest after a failed read, and when it ends.
#[derive(Default)]
struct Rest {
    length: Option<SignedDuration>,
    until: Option<Timestamp>,
}

impl Rest {
    fn failed(&mut self, now: Timestamp) {
        let next = self.length.map_or(REST_FIRST, |d| (d * 2).min(REST_MOST));
        self.length = Some(next);
        self.until = now.checked_add(next).ok();
    }
    fn cleared(&mut self) {
        *self = Rest::default();
    }
    fn resting(&self, now: Timestamp) -> bool {
        self.until.is_some_and(|u| now < u)
    }
}

/// Run the broker's reads until the app stops.
pub fn run(app: Arc<App>) {
    let Some(f) = app.figures.get() else { return };
    // when Wealthsimple was last pulled, for the header
    let last = f.book().and_then(|b| {
        let ws = Broker::named("wealthsimple");
        let Some(c) = b.connections().map_err(|e| e.to_string())?.into_iter().find(|c| c.broker == ws) else { return Ok(None) };
        b.last_read(c.id, "accounts").map_err(|e| e.to_string())
    });
    match last {
        Ok(t) => app.state.lock().unwrap().last_sync = t.map(|t| t.to_string()).unwrap_or_default(),
        Err(e) => log(&format!("bagholder: when Wealthsimple was last pulled could not be read: {e}")),
    }
    let mut rest = Rest::default();
    // the page openings the balances have been read for
    let mut served = 0;
    while !app.stopping() {
        let now = Timestamp::now();
        let before = f.version();
        let next = match pass(&app, f, now, &mut rest, &mut served) {
            Ok(next) => next,
            Err(e) => {
                app.state.lock().unwrap().error = format!("Wealthsimple could not be read: {e}");
                log(&format!("bagholder: Wealthsimple could not be read: {e}"));
                rest.failed(now);
                rest.until
            }
        };
        if f.version() != before {
            app.events.signal();
        }
        let seen = Seen::now(&app, served, rest.resting(Timestamp::now()));
        // until the next deadline, or until Sync now, a sign-in or a page opening
        let changed = || seen.changed(&app);
        match next {
            Some(n) => {
                let wait = Duration::from_secs(n.duration_since(Timestamp::now()).as_secs().max(1) as u64);
                app.events.park_until_or(&app, wait, changed);
            }
            None => {
                app.events.park_until(&app, changed);
            }
        }
    }
}

/// What the loop parked on: it wakes when any of it moves.
pub(crate) struct Seen {
    signed_in: bool,
    open: bool,
    /// The page openings already served by a read.
    served: u64,
    /// A failed read is resting: an opening waits for the rest's end, which the
    /// loop's deadline already is.
    resting: bool,
}

impl Seen {
    pub(crate) fn now(app: &App, served: u64, resting: bool) -> Seen {
        Seen { signed_in: session_file(app).load().ok().flatten().is_some(), open: app.events.watchers() > 0, served, resting }
    }

    /// Sync now, a sign-in or its end, the last page closing, or a page opening
    /// (a second one beside an open page too) that no read has served.
    pub(crate) fn changed(&self, app: &App) -> bool {
        app.pull_asked.load(Ordering::SeqCst)
            || session_file(app).load().ok().flatten().is_some() != self.signed_in
            || (app.events.watchers() > 0) != self.open
            || (!self.resting && app.events.opened() != self.served)
    }
}

/// Which read a pass makes.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Due {
    Pull,
    Balances,
}

/// The pull when asked or due (a sync reads the balances too); else the
/// balances while a page is open, when one has opened since the last read or
/// the last read is five minutes old (`SPEC.md` §4 Portfolio, Refresh).
pub(crate) fn due(pull: bool, open: bool, page_opened: bool, last_cash: Option<Timestamp>, now: Timestamp) -> Option<Due> {
    if pull {
        Some(Due::Pull)
    } else if open && (page_opened || last_cash.is_none_or(|t| now.duration_since(t) >= BALANCES_EVERY)) {
        Some(Due::Balances)
    } else {
        None
    }
}

/// How long after the last pull a fill still missing its row asks for the next,
/// by how long it has waited: Wealthsimple usually lists a fill within seconds,
/// so soon at first, then less often, and hourly after three hours, for as long
/// as it is missing.
pub(crate) fn fill_pull_step(waited: SignedDuration) -> SignedDuration {
    let s = waited.as_secs();
    SignedDuration::from_secs(match s {
        s if s < 60 => 10,
        s if s < 5 * 60 => 30,
        s if s < 30 * 60 => 2 * 60,
        s if s < 3 * 3600 => 15 * 60,
        _ => 3600,
    })
}

/// When the next pull is due for the fills of Bagholder's own orders whose
/// Wealthsimple row the book does not hold yet, if any. A fill is first seen
/// waiting on this pass; one whose row arrived stops waiting.
pub(crate) fn fill_pull_due(app: &Arc<App>, book: &Book, conn: ConnectionId, last_pull: Option<Timestamp>, now: Timestamp) -> Result<Option<Timestamp>, String> {
    let source = bagholder_wealthsimple::mapping::source();
    let mut missing = Vec::new();
    for id in crate::orders::own_fills_booked(app)? {
        if book.record_by_key(Some(conn), &source, &id).map_err(|e| e.to_string())?.is_none() {
            missing.push(id);
        }
    }
    let mut waits = app.fill_waits.lock().unwrap_or_else(|e| e.into_inner());
    waits.retain(|id, _| missing.contains(id));
    for id in &missing {
        waits.entry(id.clone()).or_insert(now);
    }
    let Some(last) = last_pull else { return Ok(waits.values().next().map(|_| now)) };
    Ok(waits.values().filter_map(|since| last.checked_add(fill_pull_step(now.duration_since(*since))).ok()).min())
}

/// One pass: the pull when it is due or asked for, else the balances when a page
/// is open and they are due. The next instant either can be due.
fn pass(app: &Arc<App>, f: &Figures, now: Timestamp, rest: &mut Rest, served: &mut u64) -> Result<Option<Timestamp>, String> {
    let file = session_file(app);
    // taken before any read, so a page opening during one is served by the next
    let opened = app.events.opened();
    if file.load().map_err(|e| e.to_string())?.is_none() {
        // no sign-in: nothing to read until one, and the sign-in's pull reads the balances
        app.pull_asked.store(false, Ordering::SeqCst);
        *served = opened;
        return Ok(None);
    }
    let asked = app.pull_asked.swap(false, Ordering::SeqCst);
    if rest.resting(now) && !asked {
        return Ok(rest.until);
    }
    let book = f.book()?;
    let conn = connection(&book, now)?;
    let zone = pull_zone()?;
    let open = app.events.watchers() > 0;
    let last_pull = book.last_read(conn, "accounts").map_err(|e| e.to_string())?;
    let last_cash = book.last_read(conn, "cash").map_err(|e| e.to_string())?;
    // a fill of Bagholder's own order is in the book once Wealthsimple's own row
    // for it is: until then it is pulled for again, soon at first
    let fill_due = fill_pull_due(app, &book, conn, last_pull, now)?;
    let pull = asked || pull_due(last_pull, now, &zone) || fill_due.is_some_and(|d| d <= now);
    let read = due(pull, open, opened != *served, last_cash, now);
    // every opening up to here is served by this pass, whatever the read's outcome
    // (a failed one is read again after its rest)
    *served = opened;
    let outcome = match read {
        Some(Due::Pull) => Some(pull_now(app, f, &book, conn, file, now)?),
        Some(Due::Balances) => Some(balances_now(app, f, &book, conn, file, now)?),
        None => None,
    };
    match outcome {
        Some(Read::Lapsed) => {
            // nothing is asked again with a lapsed sign-in: the next one wakes this
            crate::session::note_session_expired(app);
            return Ok(None);
        }
        Some(Read::Failed) => rest.failed(now),
        Some(Read::Done) => rest.cleared(),
        None => {}
    }
    let mut next = next_window(now, &zone);
    let last_pull = book.last_read(conn, "accounts").map_err(|e| e.to_string())?;
    if let Some(d) = fill_pull_due(app, &book, conn, last_pull, now)? {
        next = Some(next.map_or(d, |n| n.min(d)));
    }
    if open {
        let cash = book.last_read(conn, "cash").map_err(|e| e.to_string())?;
        let balances = cash.and_then(|t| t.checked_add(BALANCES_EVERY).ok()).unwrap_or(now);
        next = Some(next.map_or(balances, |n| n.min(balances)));
    }
    if let Some(u) = rest.until.filter(|u| *u > now) {
        next = Some(next.map_or(u, |n| n.max(u)));
    }
    Ok(next)
}

/// What the header says a pull is doing now (`SPEC.md` §4: the header shows the sync step).
fn step_text(s: &Step) -> String {
    match s {
        Step::Accounts => "Fetching accounts…".into(),
        Step::Activity { account, n, of } => format!("Fetching activity for {account} ({n} of {of})…"),
        // in whole percents, so a long pull changes the header a hundred times at most
        Step::Recording { done, of } => format!("Saving transactions… {}%", if *of == 0 { 100 } else { done * 100 / of }),
        Step::Balances => "Fetching balances…".into(),
        Step::Holdings { account, n, of } => format!("Fetching holdings for {account} ({n} of {of})…"),
        Step::History { account, n, of } => format!("Fetching equity history for {account} ({n} of {of})…"),
    }
}

/// The header's sync step, sent to the pages when it changes.
fn set_step(app: &App, text: &str) {
    {
        let mut st = app.state.lock().unwrap();
        if st.sync_step == text {
            return;
        }
        st.sync_step = text.to_string();
    }
    app.events.signal();
}

/// The pull is over: no step, not syncing.
fn end_steps(app: &App) {
    {
        let mut st = app.state.lock().unwrap();
        st.syncing = false;
        st.sync_step.clear();
    }
    app.events.signal();
}

enum Read {
    Done,
    /// A part failed: said in the header, read again after a rest.
    Failed,
    /// The sign-in lapsed.
    Lapsed,
}

/// What failed, as the header says it; whether the sign-in lapsed.
fn failures(parts: &[(String, Failure)]) -> (String, bool) {
    let lapsed = parts.iter().any(|(_, f)| matches!(f, Failure::Lapsed(_)));
    (parts.iter().map(|(p, f)| format!("{p}: {f}")).collect::<Vec<_>>().join("; "), lapsed)
}

fn pull_now(app: &Arc<App>, f: &Figures, book: &Book, conn: ConnectionId, file: SessionFile, now: Timestamp) -> Result<Read, String> {
    {
        let mut st = app.state.lock().unwrap();
        st.syncing = true;
        st.error.clear();
    }
    set_step(app, "Checking session…");
    let mut adapter = Wealthsimple::new(Client::new(&app.net, file));
    let today = adapter.day(now);
    let pulled = bagholder_broker::pull::pull(book, &mut adapter, conn, today, now, &mut |s| set_step(app, &step_text(&s)));
    let report = match pulled {
        Ok(r) => r,
        Err(e) => {
            end_steps(app);
            return Err(e.to_string());
        }
    };
    // what was stored is applied whatever else failed
    set_step(app, "Saving…");
    let applied = f.record_changed(now);
    end_steps(app);
    applied?;
    // a file's row the broker's own row now reports is linked to it
    let linking = crate::csv_import::link(f, now)?;
    if !linking.linked.is_empty() || !linking.ambiguous.is_empty() {
        log(&format!("bagholder: rows imported from files: {} linked to the broker's own, {} with more than one they could be", linking.linked.len(), linking.ambiguous.len()));
    }
    log(&format!(
        "bagholder: pulled Wealthsimple: {} rows read, {} new, {} revised, {} removed, {} imported replaced",
        report.rows_read,
        report.records_new,
        report.records_revised,
        report.removed.len(),
        report.superseded
    ));
    for (account, why) in &report.suspect {
        log(&format!("bagholder: a suspect read of {account}, nothing removed: {why}"));
    }
    let (failed, lapsed) = failures(&report.failures);
    let suspect = report.suspect.iter().map(|(a, _)| format!("a suspect read of {a}, nothing removed")).collect::<Vec<_>>();
    let said = [failed.clone()].into_iter().chain(suspect).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("; ");
    if lapsed {
        return Ok(Read::Lapsed);
    }
    {
        let mut st = app.state.lock().unwrap();
        st.connected = true;
        st.last_sync = now.to_string();
        st.error = if said.is_empty() { String::new() } else { format!("Sync failed: {said}") };
    }
    if !said.is_empty() {
        log(&format!("bagholder: sync failed: {said}"));
    }
    if failed.is_empty() {
        sync_went(app, None);
        Ok(Read::Done)
    } else {
        sync_went(app, Some(&failed));
        Ok(Read::Failed)
    }
}

/// How many syncs in a row fail before the person is told (`SPEC.md` §3,
/// Notifications: `Sync failing`, once per run of failures).
const SYNC_FAILS_TOLD: i64 = 3;

/// A sync ended: a failure counts toward telling, a success starts the count over.
pub(crate) fn sync_went(app: &Arc<App>, failed: Option<&str>) {
    let Some(reason) = failed else {
        let mut st = app.state.lock().unwrap();
        st.sync_fails = 0;
        st.sync_first_fail.clear();
        return;
    };
    let (fails, first) = {
        let mut st = app.state.lock().unwrap();
        st.sync_fails += 1;
        if st.sync_fails == 1 {
            st.sync_first_fail = crate::app::now_iso();
        }
        (st.sync_fails, st.sync_first_fail.clone())
    };
    if fails == SYNC_FAILS_TOLD {
        match app.open() {
            Ok(conn) => {
                crate::notify::emit(app, &conn, "connection", &format!("sync:{first}"), "Sync failing", reason, None);
            }
            Err(e) => log(&format!("bagholder: Sync failing could not be told: {e}")),
        }
    }
}

fn balances_now(app: &App, f: &Figures, book: &Book, conn: ConnectionId, file: SessionFile, now: Timestamp) -> Result<Read, String> {
    let mut adapter = Wealthsimple::new(Client::new(&app.net, file));
    let read = bagholder_broker::pull::balances(book, &mut adapter, conn, now).map_err(|e| e.to_string())?;
    for a in &read.accounts {
        f.broker_changed(*a)?;
    }
    let (failed, lapsed) = failures(&read.failures);
    if lapsed {
        return Ok(Read::Lapsed);
    }
    let mut st = app.state.lock().unwrap();
    if failed.is_empty() {
        st.portfolio_error.clear();
        Ok(Read::Done)
    } else {
        st.portfolio_error = format!("Balances could not be read: {failed}");
        log(&format!("bagholder: balances could not be read: {failed}"));
        Ok(Read::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn the_header_names_each_step_and_a_long_recording_changes_it_a_hundred_times_at_most() {
        assert_eq!(step_text(&Step::Accounts), "Fetching accounts…");
        assert_eq!(step_text(&Step::Activity { account: "Personal".into(), n: 3, of: 29 }), "Fetching activity for Personal (3 of 29)…");
        assert_eq!(step_text(&Step::History { account: "TFSA".into(), n: 1, of: 2 }), "Fetching equity history for TFSA (1 of 2)…");
        for of in [0usize, 1, 7, 7863, 250_000] {
            let texts: std::collections::BTreeSet<String> = (0..of.max(1)).map(|done| step_text(&Step::Recording { done, of })).collect();
            assert!(texts.len() <= 101, "{of} rows gave {} header texts", texts.len());
            assert!(texts.iter().all(|t| t.starts_with("Saving transactions… ") && t.ends_with('%')));
        }
    }

    #[test]
    fn the_pull_is_due_once_after_each_weekday_2pm_mountain() {
        let z = pull_zone().unwrap();
        // Thursday 2026-09-24, 14:00 MDT is 20:00Z
        assert_eq!(last_window(t("2026-09-24T20:00:00Z"), &z), Some(t("2026-09-24T20:00:00Z")));
        assert_eq!(last_window(t("2026-09-24T19:59:59Z"), &z), Some(t("2026-09-23T20:00:00Z")));
        // the weekend reaches back to Friday, and forward to Monday
        assert_eq!(last_window(t("2026-09-27T12:00:00Z"), &z), Some(t("2026-09-25T20:00:00Z")));
        assert_eq!(next_window(t("2026-09-25T20:00:00Z"), &z), Some(t("2026-09-28T20:00:00Z")));
        // winter 2025: 14:00 MST is 21:00Z (the zone's own rules, whatever they become)
        assert_eq!(next_window(t("2025-12-01T00:00:00Z"), &z), Some(t("2025-12-01T21:00:00Z")));
        assert!(pull_due(None, t("2026-09-27T12:00:00Z"), &z), "never pulled");
        assert!(pull_due(Some(t("2026-09-24T19:00:00Z")), t("2026-09-24T20:30:00Z"), &z), "the window opened since");
        assert!(!pull_due(Some(t("2026-09-24T20:30:00Z")), t("2026-09-25T10:00:00Z"), &z), "pulled since the window");
    }

    #[test]
    fn what_failed_is_said_part_by_part_and_a_lapse_is_told_apart() {
        let parts = vec![("activity:tfsa-1".to_string(), Failure::Unreachable("timed out".into())), ("cash".to_string(), Failure::Mismatch("quantity: not text".into()))];
        let (said, lapsed) = failures(&parts);
        assert_eq!(said, "activity:tfsa-1: unreachable: timed out; cash: a reply of another shape: quantity: not text");
        assert!(!lapsed);
        assert!(failures(&[("accounts".into(), Failure::Lapsed("401".into()))]).1);
    }

    #[test]
    fn a_failed_read_rests_longer_each_time_and_a_good_one_clears_it() {
        let mut r = Rest::default();
        let now = t("2026-09-24T20:00:00Z");
        r.failed(now);
        assert_eq!(r.until, Some(t("2026-09-24T20:00:30Z")));
        r.failed(now);
        assert_eq!(r.until, Some(t("2026-09-24T20:01:00Z")));
        for _ in 0..10 {
            r.failed(now);
        }
        assert_eq!(r.until, Some(t("2026-09-24T20:30:00Z")), "at most half an hour");
        assert!(r.resting(now));
        r.cleared();
        assert!(!r.resting(now));
    }

    /// A page opening reads the balances whatever the last read's age; with no
    /// opening they wait for five minutes; with no page open nothing is read.
    #[test]
    fn test_a_page_opening_reads_the_balances_however_fresh_they_are() {
        let now: Timestamp = "2026-03-02T15:00:00Z".parse().unwrap();
        let ages = [0, 1, 60, BALANCES_EVERY.as_secs() - 1, BALANCES_EVERY.as_secs(), 86_400];
        for age in ages {
            let last = Some(now.checked_sub(SignedDuration::from_secs(age)).unwrap());
            assert_eq!(due(false, true, true, last, now), Some(Due::Balances), "a page opened, last read {age} s ago");
            let aged = age >= BALANCES_EVERY.as_secs();
            assert_eq!(due(false, true, false, last, now), aged.then_some(Due::Balances), "no page opened, last read {age} s ago");
            assert_eq!(due(false, false, true, last, now), None, "no page open, last read {age} s ago");
            assert_eq!(due(true, true, true, last, now), Some(Due::Pull), "a sync reads the balances with it");
        }
        assert_eq!(due(false, true, false, None, now), Some(Due::Balances), "never read");
    }

    /// The loop wakes for every page opening, a second page beside an open one
    /// included, and not while a failed read rests.
    #[test]
    fn test_the_loop_wakes_when_a_page_opens() {
        use crate::events::Watching;
        let _g = crate::tests_common::guard();
        let app = crate::tests_common::app();
        let first = Watching::new(&app.events);
        let seen = Seen::now(&app, app.events.opened(), false);
        assert!(!seen.changed(&app), "nothing moved");
        let second = Watching::new(&app.events);
        assert!(seen.changed(&app), "a second page opening beside an open one wakes the loop");
        let resting = Seen::now(&app, app.events.opened() - 1, true);
        assert!(!resting.changed(&app), "a resting read waits for its rest, which is the loop's deadline");
        drop(second);
        drop(first);
    }
}
