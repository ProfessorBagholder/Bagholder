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
    // what the header reads that fails is said in its error line, never read as nothing
    let mut unread: Vec<String> = vec![];
    let notices = crate::notify::book(app).map_err(|e| format!("The book could not be opened: {e}"));
    // the book's counts, as the figures hold them
    let (acts, accounts) = app.figures.get().and_then(|f| f.read(|e| (e.inputs().ledger.transactions.len() as i64, e.inputs().ledger.accounts.len() as i64))).unwrap_or((0, 0));
    let upd = update::update_status(app).unwrap_or_else(|e| {
        unread.push(e);
        update::UpdateRecord::default()
    });
    let sess = session::load_session(app).unwrap_or_else(|e| {
        unread.push(e);
        None
    });
    let notify_status = match &notices {
        Ok(b) => notify::status(b).unwrap_or_else(|e| {
            unread.push(format!("The notifications could not be read: {e}"));
            NotifyStatus::default()
        }),
        Err(e) => {
            unread.push(e.clone());
            NotifyStatus::default()
        }
    };
    let open_orders = orders::open_orders_count(app, None);
    let can_update = update::can_update(app, &upd);
    let off = update::updates_off();
    let mut sources = unread;
    sources.extend(app.figures.get().map(|f| f.source_failures(app.net.started()).unwrap_or_else(|e| vec![format!("What the market sources answered could not be read: {e}")])).unwrap_or_default());
    sources.extend(app.figures.get().and_then(|f| f.read(broker_failures)).unwrap_or_default());
    sources.extend(crate::feeds::feed_failures(app));
    sources.extend(orders::order_failures(app));
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
        orders_live: orders::orders_live(app),
        open_orders,
        updating: st.updating.clone(),
        update_error: st.update_error.clone(),
        importing: st.importing.clone(),
        notify: notify_status,
    }
}

/// The header's error line: every failure standing now, each from its own
/// state and each gone when its own next success comes, said together as
/// sentences in one line. Wealthsimple's (the pull, the session, the sign-in),
/// the balances', the figures', then each market source failing (SPEC §1: every
/// failure is said in the header until that source succeeds). Empty when
/// nothing is failing; one failure met by two readers is said once.
pub fn failures(st: &app::State, sources: &[String]) -> String {
    let own = [st.error.as_str(), st.statement_error.as_str(), st.portfolio_error.as_str(), st.figures_error.as_str()];
    let mut said: Vec<String> = vec![];
    for s in own.into_iter().chain(sources.iter().map(String::as_str)).map(str::trim).filter(|e| !e.is_empty()).map(sentence) {
        if !said.contains(&s) {
            said.push(s);
        }
    }
    said.join(" ")
}

/// Where the book disagrees with the broker, one sentence each (`SPEC.md` §4,
/// the header's failures): each difference the broker check finds between the
/// book's holdings and cash and the broker's statement, unless it may still be a
/// fill not read into the book yet (`Difference::pending`), and each sale or
/// move that took out more units than the book held. Each is said while the
/// derivation finds it and gone with the next derivation that agrees. Figures
/// are written exactly: a difference below the tables' rounding would read as
/// two equal numbers.
pub fn broker_failures(e: &bagholder_engine::Engine) -> Vec<String> {
    use bagholder_engine::equity::Difference;
    let inputs = e.inputs();
    let figures = e.figures();
    // in the order a reader looks them up, never by the ids' random order: by
    // account name, then cash by currency, then the holdings by what they say
    let mut checks: Vec<_> = figures.checks.iter().map(|c| (crate::wire::build::account_name(inputs, c.account), c)).collect();
    checks.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.account.cmp(&b.1.account)));
    let mut out = vec![];
    for (_, c) in checks {
        let (broker, account) = (broker_label(inputs, c.account), in_account(inputs, c.account));
        let (mut cash, mut held) = (vec![], vec![]);
        for d in c.differences.iter().filter(|d| !d.pending()) {
            let (list, said) = match d {
                Difference::Units { instrument, own, broker: stated, .. } => (&mut held, format!("{} in {account}: {broker} states {}, the book {}.", symbol(inputs, *instrument), units(*stated), own.as_ref().map(|o| format!("holds {}", exact(*o))).unwrap_or_else(|g| format!("cannot count its units ({})", g.words().join(", "))))),
                Difference::Cash { currency, own, broker: stated, .. } => (&mut cash, format!("{currency} cash in {account}: {broker} states {}, the book {}.", money(*stated), own.as_ref().map(|o| format!("holds {}", money(*o))).unwrap_or_else(|g| format!("cannot sum it ({})", g.words().join(", "))))),
            };
            list.push(said);
        }
        held.sort();
        out.extend(cash);
        out.extend(held);
    }
    let beyond = &figures.matched.beyond;
    if !beyond.is_empty() {
        let wanted: std::collections::BTreeSet<_> = beyond.iter().map(|b| &b.transaction).collect();
        let txs: std::collections::BTreeMap<_, _> = inputs.ledger.transactions.iter().filter(|t| wanted.contains(&t.id)).map(|t| (&t.id, t)).collect();
        for b in beyond {
            let t = txs.get(&b.transaction);
            let what = t.map(|t| kind_phrase(t.kind)).unwrap_or("A record");
            let day = t.map(|t| format!(" on {}", t.trade_date)).unwrap_or_default();
            out.push(format!("{what} of {} in {}{day} took {} more than the book held.", symbol(inputs, b.instrument), in_account(inputs, b.account), units(b.qty)));
        }
    }
    out.extend(unread_rows(inputs));
    out
}

/// Every problem the book found with a source's row, said until it is resolved
/// (`docs/decisions.md` 2026-10-04: every problem with a Wealthsimple row is said
/// in the header, not only rows it has no rule for), in the existing sentence:
/// how many, why in the mapping's words, and the first day each reason occurs.
/// - a row the mapping could not place (`Kind::Unclassified`): counted in no figure;
/// - a row placed with a problem: kept, the problem said;
/// - a row that gave no transaction at all (one that could not be read): counted
///   in no figure, said by its source and when it first arrived.
pub fn unread_rows(inputs: &bagholder_engine::input::Inputs) -> Vec<String> {
    use bagholder_core::transaction::Kind;
    use std::collections::{BTreeMap, BTreeSet};
    type Whys = BTreeMap<String, (usize, bagholder_core::jiff::civil::Date)>;
    let note = |m: &mut Whys, why: String, day: bagholder_core::jiff::civil::Date| {
        let e = m.entry(why).or_insert((0, day));
        e.0 += 1;
        e.1 = e.1.min(day);
    };
    let why_of = |r: Option<&bagholder_engine::input::RecordInfo>, default: &str| -> Vec<String> {
        match r.map(|r| &r.problems) {
            Some(ps) if !ps.is_empty() => ps.iter().map(|p| p.detail.trim().trim_end_matches('.').to_string()).collect(),
            _ => vec![default.to_string()],
        }
    };
    // account → why → (rows, first day), for rows not placed and rows placed with a problem
    let mut unplaced: BTreeMap<bagholder_core::AccountId, Whys> = BTreeMap::new();
    let mut flawed: BTreeMap<bagholder_core::AccountId, Whys> = BTreeMap::new();
    // a record is said once, by its first transaction's account and its earliest day
    let mut first: BTreeMap<bagholder_core::RecordId, (bagholder_core::AccountId, bagholder_core::jiff::civil::Date, bool)> = BTreeMap::new();
    for t in &inputs.ledger.transactions {
        let e = first.entry(t.id.record).or_insert((t.account, t.trade_date, false));
        e.1 = e.1.min(t.trade_date);
        e.2 |= t.kind == Kind::Unclassified;
    }
    for (record, (account, day, unclassified)) in &first {
        let info = inputs.ledger.records.get(record);
        if *unclassified {
            for why in why_of(info, "a row its mapping does not place") {
                note(unplaced.entry(*account).or_default(), why, *day);
            }
        } else if info.is_some_and(|r| !r.problems.is_empty()) {
            for why in why_of(info, "") {
                note(flawed.entry(*account).or_default(), why, *day);
            }
        }
    }
    // records that gave no transaction: said by their source, on the day they arrived
    let placed: BTreeSet<_> = first.keys().collect();
    let mut unread: BTreeMap<Option<bagholder_core::ConnectionId>, Whys> = BTreeMap::new();
    for (record, info) in &inputs.ledger.records {
        if placed.contains(record) || info.problems.is_empty() {
            continue;
        }
        let day = info.first_received_at.map(|t| t.to_zoned(inputs.clock.home.clone()).date()).unwrap_or(inputs.clock.today);
        for why in why_of(Some(info), "") {
            note(unread.entry(info.connection).or_default(), why, day);
        }
    }
    let list = |whys: &Whys| whys.iter().map(|(why, (c, first))| if *c == 1 { format!("{why}, on {first}") } else { format!("{why}, {c} rows, the first on {first}") }).collect::<Vec<_>>().join("; ");
    let count = |whys: &Whys| -> usize { whys.values().map(|(c, _)| c).sum() };
    let rows = |n: usize| if n == 1 { "A row".to_string() } else { format!("{n} rows") };
    let mut out: Vec<(String, String)> = Vec::new();
    for (account, whys) in &unplaced {
        let n = count(whys);
        let verb = if n == 1 { "could not be placed and counts" } else { "could not be placed and count" };
        out.push((crate::wire::build::account_name(inputs, *account), format!("{} in {} {verb} in no figure: {}.", rows(n), in_account(inputs, *account), list(whys))));
    }
    for (account, whys) in &flawed {
        let n = count(whys);
        let verb = if n == 1 { "was kept with a problem" } else { "were kept with a problem" };
        out.push((crate::wire::build::account_name(inputs, *account), format!("{} in {} {verb}: {}.", rows(n), in_account(inputs, *account), list(whys))));
    }
    for (connection, whys) in &unread {
        out.push((String::new(), format!("{} from {} gave nothing the book can count: {}.", rows(count(whys)), from(inputs, *connection), list(whys))));
    }
    out.sort();
    out.into_iter().map(|(_, s)| s).collect()
}

/// Where a record came from, as the person knows it: its connection's broker, by
/// the label the book keeps for it; a file imported by hand has none.
fn from(inputs: &bagholder_engine::input::Inputs, connection: Option<bagholder_core::ConnectionId>) -> String {
    match connection {
        None => "an imported file".into(),
        Some(c) => inputs.ledger.accounts.values().find(|a| a.account.connection == c).map(|a| a.broker_label.clone()).filter(|l| !l.is_empty()).unwrap_or_else(|| "a connected broker".into()),
    }
}

/// Where the book keeps what the last pull's statements said, for a restart.
pub const STATEMENTS_SAID: &str = "statements.said";

/// Each month whose statement the book does not reconcile with, one sentence
/// each (`docs/plans/statement-gaps.md`): nothing of that month on is booked
/// from the statements until it does.
pub fn unreconciled(e: &bagholder_engine::Engine, list: &[bagholder_broker::statements::Unreconciled]) -> Vec<String> {
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    let inputs = e.inputs();
    list.iter()
        .map(|u| {
            let (broker, account) = (broker_label(inputs, u.account), in_account(inputs, u.account));
            let month = format!("{} {}", MONTHS[(u.month.month() - 1) as usize], u.month.year());
            let what = match (&u.why, u.statement) {
                (Some(why), _) => format!("{broker}'s {month} statement: {why}"),
                (None, Some(s)) => format!("{broker}'s {month} statement closes at {}, the book at {}", money(s), money(u.book)),
                (None, None) => format!("{broker}'s {month} statement states no closing balance for every account behind it"),
            };
            format!("{} cash in {account}: {what}; nothing from that month on is booked from the statements.", u.currency)
        })
        .collect()
}

/// Each account's movements the activity feed states and no statement lists,
/// one sentence per account and currency: how many, what they come to, and the
/// first one's day. They stay in the book as the feed states them.
pub fn feed_only(e: &bagholder_engine::Engine, list: &[bagholder_broker::statements::FeedOnly]) -> Vec<String> {
    let inputs = e.inputs();
    list.iter()
        .map(|f| {
            let total = f.rows.iter().try_fold(bagholder_core::Dec::ZERO, |a, (_, v)| a.checked_add(*v).ok());
            let first = f.rows.iter().map(|(d, _)| *d).min().map(|d| d.to_string()).unwrap_or_default();
            let n = f.rows.len();
            let what = if n == 1 { "1 movement".to_string() } else { format!("{n} movements") };
            let sum = total.map(|t| format!(", {} in all,", money(t))).unwrap_or_default();
            format!("{} cash in {}: the activity feed has {what}{sum} that no {} statement lists, the first on {first}.", f.currency, in_account(inputs, f.account), broker_label(inputs, f.account))
        })
        .collect()
}

/// What took the units out, as a sentence opens with it.
fn kind_phrase(k: bagholder_core::transaction::Kind) -> &'static str {
    use bagholder_core::transaction::Kind;
    match k {
        Kind::Sell => "A sale",
        Kind::Buy => "A buy",
        Kind::TransferOut => "A transfer out",
        Kind::TransferIn => "A transfer in",
        Kind::OptionExpiry => "An expiry",
        Kind::OptionAssignment => "An assignment",
        Kind::OptionExercise => "An exercise",
        Kind::Resolution => "A resolution",
        Kind::CorporateEvent => "A corporate event",
        Kind::StakingMove => "A staking move",
        _ => "A record",
    }
}

/// The broker an account is held at, as the person calls it.
fn broker_label(inputs: &bagholder_engine::input::Inputs, a: bagholder_core::AccountId) -> String {
    inputs.ledger.accounts.get(&a).map(|x| x.broker_label.clone()).filter(|l| !l.trim().is_empty()).unwrap_or_else(|| "The broker".into())
}

/// `the TFSA` for an account named by what it is, its own name for one the
/// person named.
fn in_account(inputs: &bagholder_engine::input::Inputs, a: bagholder_core::AccountId) -> String {
    let name = crate::wire::build::account_name(inputs, a);
    let named = inputs.ledger.accounts.get(&a).and_then(|x| x.account.nickname.as_deref()).is_some_and(|n| !n.trim().is_empty());
    if named { name } else { format!("the {name}") }
}

/// An instrument as the page names it: its bare ticker.
fn symbol(inputs: &bagholder_engine::input::Inputs, i: bagholder_core::InstrumentId) -> String {
    let s = crate::wire::build::shown(inputs, i).symbol;
    if s.is_empty() { "An instrument with no name on record".into() } else { crate::orders::bare_symbol(&s) }
}

/// `1,000,000 units`, `1 unit`, `none`.
fn units(q: bagholder_core::Dec) -> String {
    if q.is_zero() {
        "none".into()
    } else if q == bagholder_core::Dec::ONE {
        "1 unit".into()
    } else {
        format!("{} units", exact(q))
    }
}

/// `$1,234.50`, `−$0.005`: at least cents, every place the value has.
fn money(v: bagholder_core::Dec) -> String {
    let t = grouped(v.abs().to_text());
    let t = match t.split_once('.') {
        None => format!("{t}.00"),
        Some((_, f)) if f.len() < 2 => format!("{t}0"),
        Some(_) => t,
    };
    format!("{}${t}", if v.is_negative() { "\u{2212}" } else { "" })
}

/// A quantity with thousands separators and every place it has.
fn exact(v: bagholder_core::Dec) -> String {
    format!("{}{}", if v.is_negative() { "\u{2212}" } else { "" }, grouped(v.abs().to_text()))
}

fn grouped(plain: String) -> String {
    let (whole, frac) = plain.split_once('.').map(|(w, f)| (w.to_string(), Some(f.to_string()))).unwrap_or((plain, None));
    let mut g = String::new();
    for (n, ch) in whole.chars().enumerate() {
        if n > 0 && (whole.len() - n) % 3 == 0 {
            g.push(',');
        }
        g.push(ch);
    }
    match frac {
        Some(f) => format!("{g}.{f}"),
        None => g,
    }
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
/// polling page reloads on. A version that cannot be read fails the request: a
/// page told nothing moved would reload nothing.
pub fn answer(app: &Arc<App>) -> Result<StatusAnswer, String> {
    let conn = app.cache().map_err(|e| format!("The market cache could not be opened: {e}"))?;
    let data_version = versions::data_version(&conn).map_err(|e| format!("The market cache's version could not be read: {e}"))?;
    let core_version = versions::core_version(&conn).map_err(|e| format!("The market cache's version could not be read: {e}"))?;
    drop(conn);
    let today = bagholder_model::clock::today_local();
    Ok(StatusAnswer {
        status: status(app),
        data_version: format!("{}|{}", data_version, today),
        // everything the model reads except the quotes: when this is unchanged but the
        // data version moved, only prices ticked (read by the legacy page, which polls)
        core_version: format!("{}|{}", core_version, today),
    })
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
        let now = Timestamp::now();
        let f = crate::figures::Figures::open(home.path(), now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        app.set_figures(f);
        let mut feed = crate::events::Feed::open(app.clone());
        assert!(app.events.watch(&app, feed.id(), [("status".to_string(), crate::events::Want { params: serde_json::json!({}), have: None })].into_iter().collect()));
        let first = serde_json::to_string(&feed.step(&super::status).into_iter().map(|(_, m)| m).collect::<Vec<_>>()).unwrap();
        // the recorded month is not the account's whole history: the broker check disagrees
        let base = error(&app);
        assert!(!base.is_empty() && first.contains(&format!("\"error\":{}", serde_json::to_string(&base).unwrap())), "only the broker check fails yet: {first}");
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

    #[test]
    fn a_month_that_does_not_reconcile_is_one_sentence_naming_the_account_month_and_both_balances_and_stands_in_the_header() {
        let (_h, app) = pulled();
        let f = app.figures.get().unwrap();
        let account = f.read(|e| *e.inputs().ledger.accounts.keys().next().unwrap()).unwrap();
        let u = |statement: Option<&str>, why: Option<&str>| bagholder_broker::statements::Unreconciled {
            account,
            month: "2025-06-01".parse().unwrap(),
            currency: bagholder_core::Currency::CAD,
            statement: statement.map(|s| bagholder_core::Dec::parse(s).unwrap()),
            book: bagholder_core::Dec::parse("-35630.9").unwrap(),
            why: why.map(str::to_string),
        };
        let said = f.read(|e| super::unreconciled(e, &[u(Some("19.1"), None), u(None, None), u(Some("1"), Some("a sell only the statement states, which is not booked from it"))])).unwrap();
        let name = f.read(|e| super::in_account(e.inputs(), account)).unwrap();
        assert_eq!(said[0], format!("CAD cash in {name}: Wealthsimple's June 2025 statement closes at $19.10, the book at \u{2212}$35,630.90; nothing from that month on is booked from the statements."));
        assert!(said[1].contains("states no closing balance for every account behind it"), "{}", said[1]);
        assert!(said[2].contains("a sell only the statement states"), "{}", said[2]);
        // what the feed states that no statement lists: one sentence, a count and a total
        let f_only = bagholder_broker::statements::FeedOnly { account, currency: bagholder_core::Currency::parse("USD").unwrap(), rows: vec![("2025-01-15".parse().unwrap(), bagholder_core::Dec::parse("2.06").unwrap()), ("2026-01-07".parse().unwrap(), bagholder_core::Dec::parse("12.87").unwrap())] };
        let said_f = f.read(|e| super::feed_only(e, &[f_only])).unwrap();
        assert_eq!(said_f[0], format!("USD cash in {name}: the activity feed has 2 movements, $14.93 in all, that no Wealthsimple statement lists, the first on 2025-01-15."));
        // it stands in the header's line with every other failure
        app.state.lock().unwrap().statement_error = said[0].clone();
        assert!(error(&app).contains(&said[0]), "{}", error(&app));
    }

    /// An app on the recorded month's book, its figures built.
    fn pulled() -> (tempfile::TempDir, Arc<App>) {
        let home = tempfile::tempdir().unwrap();
        crate::tests_common::pulled_book(home.path());
        let app = app_on(home.path());
        let now = Timestamp::now();
        let f = crate::figures::Figures::open(home.path(), now).unwrap();
        f.state_zone("America/Toronto", now).unwrap();
        app.set_figures(f);
        (home, app)
    }

    #[test]
    fn test_where_the_book_and_the_broker_disagree_on_units_it_is_said_until_they_agree() {
        use bagholder_book::statements::UnitsLine;
        let (_home, app) = pulled();
        let f = app.figures.get().unwrap();
        let before = error(&app);
        // an account the broker states units for, and one of them
        let (account, as_of, held, instrument, symbol, name) = f
            .read(|e| {
                let i = e.inputs();
                let (a, b) = i.market.brokers.iter().find(|(_, b)| !b.held.is_empty()).expect("the month states units");
                // one the book has a name for, so its sentence is its own
                let (inst, _) = b.held.iter().find(|(i, _)| !crate::wire::build::shown(e.inputs(), **i).symbol.is_empty()).unwrap();
                (*a, b.held_as_of.unwrap(), b.held.clone(), *inst, super::symbol(i, *inst), super::in_account(i, *a))
            })
            .unwrap();
        let book = f.book().unwrap();
        let connection = book.accounts().unwrap().into_iter().find(|a| a.id == account).unwrap().connection;
        let state = |units: &std::collections::BTreeMap<bagholder_core::InstrumentId, bagholder_core::Dec>| {
            let now = Timestamp::now();
            let read = book.broker_read(connection, "positions", now).unwrap();
            let lines: Vec<UnitsLine> = units.iter().map(|(i, q)| UnitsLine { instrument: *i, quantity: *q, book_value: None, value: Some(bagholder_core::Money::new(*q, bagholder_core::Currency::CAD)) }).collect();
            book.store_units(account, as_of, &lines, &read, now).unwrap();
            book.note_activity_read(account, now, true).unwrap();
            // as the reader of balances does: the figures moved, so the pages are told
            let before = f.version();
            f.broker_changed(account).unwrap();
            if f.version() != before {
                app.events.signal();
            }
        };
        let mut more = held.clone();
        let own = f.read(|e| e.figures().matched.units_on(account, instrument, as_of).unwrap()).unwrap();
        let stated = own.checked_add(bagholder_core::Dec::parse("1000000").unwrap()).unwrap();
        // a page open on the header
        let mut feed = crate::events::Feed::open(app.clone());
        assert!(app.events.watch(&app, feed.id(), [("status".to_string(), crate::events::Want { params: serde_json::json!({}), have: None })].into_iter().collect()));
        feed.step(&super::status);
        let mut signals = app.events.subscribe();
        signals.mark_unchanged();
        more.insert(instrument, stated);
        state(&more);
        let said = format!("{symbol} in {name}: Wealthsimple states {} units, the book holds {}.", super::exact(stated), super::exact(own));
        let line = error(&app);
        assert!(line.contains(&said), "{said:?} in {line:?}");
        // the open page is sent the header's error
        assert!(signals.has_changed().unwrap());
        let sent = serde_json::to_string(&feed.step(&super::status).into_iter().map(|(_, m)| m).collect::<Vec<_>>()).unwrap();
        assert!(sent.contains(&said), "{sent}");
        // the broker's next statement agrees: the sentence goes
        more.insert(instrument, own);
        state(&more);
        let line = error(&app);
        let this = format!("{symbol} in {name}: ");
        assert!(!line.contains(&this), "{line:?}");
        // and nothing else changed
        let rest = |l: &str| l.split(". ").map(|s| s.trim_end_matches('.').to_string()).filter(|s| !s.starts_with(&this)).collect::<Vec<_>>();
        assert_eq!(rest(&line), rest(&before));
    }

    #[test]
    fn test_a_sale_of_more_than_the_book_held_is_said_until_the_book_holds_it() {
        let (_home, app) = pulled();
        let f = app.figures.get().unwrap();
        let before = error(&app);
        let trade = |day: &str, side: &str, quantity: &str| {
            let req = crate::entries::EntryRequest::Trade { account: String::new(), instrument: None, symbol: "ZZQQ".into(), currency: "USD".into(), day: day.into(), side: side.into(), quantity: quantity.into(), price: "2".into(), fee: String::new() };
            crate::entries::enter(f, &req, Timestamp::now()).unwrap();
        };
        trade("2025-11-10", "buy", "1");
        trade("2025-11-12", "sell", "3");
        let said = "A sale of ZZQQ in Manual on 2025-11-12 took 2 units more than the book held.";
        let line = error(&app);
        assert!(line.contains(said), "{line:?}");
        // the book is told of the units it sold: the sentence goes
        trade("2025-11-11", "buy", "2");
        assert_eq!(error(&app), before);
    }

    #[test]
    fn test_figures_are_written_exactly() {
        use bagholder_core::Dec;
        let d = |s: &str| Dec::parse(s).unwrap();
        assert_eq!(super::exact(d("1100000")), "1,100,000");
        assert_eq!(super::exact(d("0.00000123")), "0.00000123");
        assert_eq!(super::exact(d("-1234.5")), "\u{2212}1,234.5");
        assert_eq!(super::units(d("0")), "none");
        assert_eq!(super::units(d("1")), "1 unit");
        assert_eq!(super::units(d("100000")), "100,000 units");
        assert_eq!(super::money(d("36434.77")), "$36,434.77");
        assert_eq!(super::money(d("-12.5")), "\u{2212}$12.50");
        assert_eq!(super::money(d("0.005")), "$0.005");
        assert_eq!(super::money(d("100")), "$100.00");
    }
}


