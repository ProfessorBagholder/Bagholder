//! Every operation the adapter asks Wealthsimple, on recorded replies
//! (`docs/plans/stage-3b-wealthsimple.md`, "Recorded replies"): its real answer,
//! an empty answer, a refusal, a lapsed session, and a wrong-shaped and a
//! wrong-meaning copy (`tests/replies/wealthsimple-ops`, each a real reply with
//! one field edited). Each reply is answered by the network client on a fake
//! network, the rest of a pull by the recorded replies beside it; each test
//! states what is written of it, or that nothing is and what is recorded instead.
//!
//! The refusal and the lapse are Wealthsimple's own (`tests/session.rs`): the
//! same for every operation, each is asked once per operation. A single object
//! (an internal transfer, a card account) has no empty form: Wealthsimple
//! answers one it does not have with the refusal.

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bagholder_book::mapping::Mapped;
use bagholder_book::Book;
use bagholder_broker::pull::{pull, Report};
use bagholder_broker::{Answer, BrokerAdapter, Failure};
use bagholder_core::account::{AccountKind, AccountRef, AccountStatus, AccountType, Registration};
use bagholder_core::instrument::{RefScheme, Reference};
use bagholder_core::json::{self, Value};
use bagholder_core::transaction::{Effect, Kind};
use bagholder_core::{AccountId, Broker, Currency, Dec, Money};
use bagholder_net::{Ask, Limiter, ManualClock, Net, NetError, Transport};
use bagholder_sources::reply::Node;
use bagholder_wealthsimple::adapter::{row_of, Source, Wealthsimple};
use bagholder_wealthsimple::client::Client;
use bagholder_wealthsimple::read;
use bagholder_wealthsimple::replay::Replay;
use bagholder_wealthsimple::session::SessionFile;
use common::{day_of, fixtures, map_payload, Moves};

// -- the fake network --------------------------------------------------------

/// Answers each request with the next reply queued, and fails a test on a
/// request with none left.
#[derive(Default)]
struct Queue {
    replies: Mutex<Vec<(u16, String)>>,
    asked: Mutex<usize>,
}

struct Shared(Arc<Queue>);

impl Transport for Shared {
    fn answer(&self, ask: &Ask) -> Result<bagholder_net::Answer, NetError> {
        *self.0.asked.lock().unwrap() += 1;
        let mut r = self.0.replies.lock().unwrap();
        assert!(!r.is_empty(), "a request nothing answers: {}", ask.url);
        let (status, body) = r.remove(0);
        Ok((status, ask.url.to_string(), vec![], body.into_bytes()))
    }
}

/// A signed-in session on a network that answers with `replies`, in order.
struct Fake {
    queue: Arc<Queue>,
    net: Net,
    _dir: tempfile::TempDir,
    session: PathBuf,
}

impl Fake {
    fn client(&self) -> Client<'_> {
        Client::new(&self.net, SessionFile { path: self.session.clone() })
    }
    fn asked(&self) -> usize {
        *self.queue.asked.lock().unwrap()
    }
}

fn fake(replies: Vec<(u16, String)>) -> Fake {
    let queue = Arc::new(Queue { replies: Mutex::new(replies), asked: Mutex::new(0) });
    let net = Net::answered_by(Arc::new(ManualClock::at("2026-09-24T12:00:00Z".parse().unwrap())), Arc::new(Limiter::new()), Box::new(Shared(queue.clone())));
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("session.json");
    std::fs::write(&session, r#"{"access_token":"a1","refresh_token":"r1","client_id":"the-client","identity_canonical_id":"identity-1","expires_at":"2026-09-24T13:00:00Z"}"#).unwrap();
    Fake { queue, net, _dir: dir, session }
}

/// Each body answered in turn, with 200.
fn answering(bodies: &[String]) -> Fake {
    fake(bodies.iter().map(|b| (200, b.clone())).collect())
}

fn replies(dir: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/replies").join(dir)
}

/// A recorded reply as Wealthsimple sent it: `<folder>/<file>`.
fn body(path: &str) -> String {
    std::fs::read_to_string(replies("").join(path)).unwrap()
}

fn edited(name: &str) -> String {
    body(&format!("wealthsimple-ops/{name}"))
}

fn dec(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

/// The mismatch a failure names, or the test fails.
fn mismatch<T: std::fmt::Debug>(got: Answer<T>) -> String {
    match got {
        Err(Failure::Mismatch(w)) => w,
        other => panic!("expected a mismatch, got {other:?}"),
    }
}

// -- a source answering one operation from the network -------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Accounts,
    Activity,
    Securities,
    Order,
    Entitlements,
    Conversion,
    Transfer,
    Positions,
    Balances,
    History,
}

/// The recorded replies, but for `op`, which the client asks the network.
struct Routed<'n> {
    replay: Replay,
    client: Client<'n>,
    op: Op,
}

impl Source for Routed<'_> {
    fn accounts(&mut self) -> Answer<Vec<Value>> {
        if self.op == Op::Accounts { self.client.accounts() } else { self.replay.accounts() }
    }
    fn activity(&mut self, account: &str, from: Option<jiff::civil::Date>) -> Answer<Vec<Value>> {
        if self.op == Op::Activity { self.client.activity(account, from) } else { self.replay.activity(account, from) }
    }
    fn securities(&mut self, ids: &[String]) -> Answer<Vec<Value>> {
        if self.op == Op::Securities { self.client.securities(ids) } else { self.replay.securities(ids) }
    }
    fn order(&mut self, batch: &str) -> Answer<Option<Value>> {
        if self.op == Op::Order { self.client.order(batch) } else { self.replay.order(batch) }
    }
    fn entitlements(&mut self, activity: &str) -> Answer<Option<Value>> {
        if self.op == Op::Entitlements { self.client.entitlements(activity) } else { self.replay.entitlements(activity) }
    }
    fn conversion(&mut self, id: &str) -> Answer<Option<Value>> {
        if self.op == Op::Conversion { self.client.conversion(id) } else { self.replay.conversion(id) }
    }
    fn transfer(&mut self, id: &str) -> Answer<Option<Value>> {
        if self.op == Op::Transfer { self.client.transfer(id) } else { self.replay.transfer(id) }
    }
    fn card(&mut self, account: &str) -> Answer<Value> {
        self.replay.card(account)
    }
    fn buying_power(&mut self, account: &str) -> Answer<Value> {
        self.replay.buying_power(account)
    }
    fn positions(&mut self, account: &str, day: jiff::civil::Date) -> Answer<Value> {
        if self.op == Op::Positions { self.client.positions(account, day) } else { self.replay.positions(account, day) }
    }
    fn balances(&mut self, accounts: &[String]) -> Answer<Vec<Value>> {
        if self.op == Op::Balances { self.client.balances(accounts) } else { self.replay.balances(accounts) }
    }
    fn history(&mut self, account: &str, from: Option<jiff::civil::Date>) -> Answer<Vec<Value>> {
        if self.op == Op::History { self.client.history(account, from) } else { self.replay.history(account, from) }
    }
    fn requests(&self) -> usize {
        self.replay.requests() + self.client.requests()
    }
}

// -- a pull of one account's November, one operation from the network ---------

const NOW: &str = "2025-11-19T20:00:00Z";

/// What a pull wrote: its report, and the book.
struct Pulled {
    _home: tempfile::TempDir,
    book: Book,
    report: Report,
    /// Requests the network answered.
    asked: usize,
}

impl Pulled {
    fn account(&self) -> Option<AccountId> {
        self.book.account_by_ref(&AccountRef::new(Broker::named("wealthsimple"), "anon-tfsa-1")).unwrap()
    }
    fn stated(&self) -> bagholder_book::statements::Stated {
        self.book.stated(self.account().expect("the account")).unwrap()
    }
    fn problems(&self) -> Vec<(String, String)> {
        self.book.problems().unwrap().into_iter().map(|(_, p)| (p.code, p.detail)).collect()
    }
    fn failures(&self) -> Vec<(String, String)> {
        self.report.failures.iter().map(|(part, f)| (part.clone(), mismatch::<()>(Err(f.clone())))).collect()
    }
}

/// A pull of `tests/replies/wealthsimple-pull`, `op` answered by the network
/// with `bodies`.
fn pulled(op: Op, bodies: &[String]) -> Pulled {
    let home = tempfile::tempdir().unwrap();
    let now: jiff::Timestamp = NOW.parse().unwrap();
    let (book, _) = Book::open_in(home.path(), "test", now).unwrap();
    let connection = book.add_connection(&Broker::named("wealthsimple"), "Wealthsimple", now).unwrap();
    let f = answering(bodies);
    let mut ws = Wealthsimple::new(Routed { replay: Replay::read(&replies("wealthsimple-pull")).unwrap(), client: f.client(), op });
    let report = pull(&book, &mut ws, connection, "2025-11-19".parse().unwrap(), now).unwrap();
    drop(ws);
    Pulled { asked: f.asked(), _home: home, book, report }
}

fn as_of() -> jiff::civil::Date {
    "2025-11-18".parse().unwrap()
}

// -- a record of one row, one reply beside it from the network -----------------

/// The record of the row `pick` finds among `tests/replies/wealthsimple`'s,
/// `op` answered by the network with `reply`, and how many requests it sent.
fn recorded(op: Op, reply: &str, pick: impl Fn(&Node) -> bool) -> (Answer<Value>, usize) {
    let f = answering(&[reply.to_string()]);
    let replay = Replay::read(&fixtures()).unwrap();
    let rows = replay.rows.clone();
    let mut ws = Wealthsimple::new(Routed { replay, client: f.client(), op });
    // the rows read as a pull reads them, for a transfer's other rows
    let accounts: std::collections::BTreeSet<String> = rows.iter().map(|r| Node::root(r).text("accountId").unwrap().to_string()).collect();
    for a in accounts {
        ws.activity(&a, None).unwrap();
    }
    let row = rows.iter().find(|r| pick(&Node::root(r))).expect("the row").clone();
    let got = ws.record(&row_of(&row, day_of(&row)).unwrap(), &mut Moves::default());
    drop(ws);
    (got, f.asked())
}

fn row_id(id: &'static str) -> impl Fn(&Node) -> bool {
    move |n| n.text("canonicalId").ok() == Some(id)
}

/// The reply's own object at `field` under `data`.
fn node_of(reply: &str, at: &[&str]) -> Value {
    let v = json::parse(reply).unwrap();
    let mut n = Node::root(&v).obj("data").unwrap();
    for k in at {
        n = n.field(k).unwrap();
    }
    n.value().clone()
}

fn codes(m: &Mapped) -> Vec<&str> {
    m.problems.iter().map(|p| p.code.as_str()).collect()
}

fn security(d: &bagholder_book::mapping::Draft) -> &str {
    &d.instrument.as_ref().expect("an instrument").refs.iter().find(|r| matches!(r.scheme, RefScheme::BrokerSecurity(_))).unwrap().value
}

const MULTILEG: &str = "anon-order-6225";
const CONSOLIDATION: &str = "anon-us-1";
const FUNDING: &str = "anon-funding-181";
const INTERNAL: &str = "anon-internal-224";
const TRANSFER: &str = "anon-ares-4";

// -- the refusal and the lapse, per operation ---------------------------------

/// Wealthsimple's answer to a read of what does not exist (the owner's
/// browser, 2026-09-24), for the field `field`.
fn not_found(field: &str) -> String {
    format!(r#"{{"data":{{"{field}":null}},"errors":[{{"message":"NOT_FOUND","path":["{field}"],"extensions":{{"code":"NOT_FOUND"}}}}]}}"#)
}

/// Wealthsimple's answer to a read whose session is not valid.
const UNAUTHENTICATED: &str = r#"{"errors":[{"message":"Not Authorized","extensions":{"code":"UNAUTHENTICATED"}}]}"#;

fn is_refused<T: std::fmt::Debug>(field: &str, ask: impl FnOnce(&mut Client<'_>) -> Answer<T>) {
    let f = answering(&[not_found(field)]);
    match ask(&mut f.client()) {
        Err(Failure::Refused(w)) => assert!(w.contains("NOT_FOUND"), "{w}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    // a refusal is not asked again
    assert_eq!(f.asked(), 1);
}

fn lapses<T: std::fmt::Debug>(ask: impl FnOnce(&mut Client<'_>) -> Answer<T>) {
    let token = r#"{"access_token":"a2","refresh_token":"r2","expires_in":1800}"#;
    let f = fake(vec![(401, UNAUTHENTICATED.into()), (200, token.into()), (401, UNAUTHENTICATED.into())]);
    assert!(matches!(ask(&mut f.client()), Err(Failure::Lapsed(_))));
    // the read, one refresh, the read once more: never a third time
    assert_eq!(f.asked(), 3);
}

fn day(s: &str) -> jiff::civil::Date {
    s.parse().unwrap()
}

#[test]
fn accounts_refused_is_a_refusal_naming_not_found() {
    is_refused("identity", |c| c.accounts());
}
#[test]
fn accounts_on_a_lapsed_session_lapse_after_one_refresh() {
    lapses(|c| c.accounts());
}
#[test]
fn activity_refused_is_a_refusal_naming_not_found() {
    is_refused("activityFeedItems", |c| c.activity("anon-tfsa-1", None));
}
#[test]
fn activity_on_a_lapsed_session_lapses_after_one_refresh() {
    lapses(|c| c.activity("anon-tfsa-1", None));
}
#[test]
fn securities_refused_is_a_refusal_naming_not_found() {
    is_refused("securities", |c| c.securities(&["sec-s-0611ed76cd8445138631597d171d986f".into()]));
}
#[test]
fn securities_on_a_lapsed_session_lapse_after_one_refresh() {
    lapses(|c| c.securities(&["sec-s-0611ed76cd8445138631597d171d986f".into()]));
}
#[test]
fn order_refused_is_a_refusal_naming_not_found() {
    is_refused("soOrdersMultilegOrder", |c| c.order(MULTILEG));
}
#[test]
fn order_on_a_lapsed_session_lapses_after_one_refresh() {
    lapses(|c| c.order(MULTILEG));
}
#[test]
fn entitlements_refused_are_a_refusal_naming_not_found() {
    is_refused("corporateActionChildActivities", |c| c.entitlements(CONSOLIDATION));
}
#[test]
fn entitlements_on_a_lapsed_session_lapse_after_one_refresh() {
    lapses(|c| c.entitlements(CONSOLIDATION));
}
#[test]
fn funding_intent_refused_is_a_refusal_naming_not_found() {
    is_refused("searchFundingIntents", |c| c.conversion("anon-funding-182"));
}
#[test]
fn funding_intent_on_a_lapsed_session_lapses_after_one_refresh() {
    lapses(|c| c.conversion("anon-funding-182"));
}
#[test]
fn internal_transfer_refused_is_a_refusal_naming_not_found() {
    is_refused("internalTransfer", |c| c.conversion("anon-internal-225"));
}
#[test]
fn internal_transfer_on_a_lapsed_session_lapses_after_one_refresh() {
    lapses(|c| c.conversion("anon-internal-225"));
}
#[test]
fn institutional_transfer_refused_is_a_refusal_naming_not_found() {
    is_refused("accountTransfer", |c| c.transfer("anon-transfer-99"));
}
#[test]
fn institutional_transfer_on_a_lapsed_session_lapses_after_one_refresh() {
    lapses(|c| c.transfer("anon-transfer-99"));
}
#[test]
fn card_refused_is_a_refusal_naming_not_found() {
    is_refused("creditCardAccount", |c| c.card("anon-ca-4"));
}
#[test]
fn card_on_a_lapsed_session_lapses_after_one_refresh() {
    lapses(|c| c.card("anon-ca-4"));
}
#[test]
fn positions_refused_are_a_refusal_naming_not_found() {
    is_refused("accounts", |c| c.positions("anon-tfsa-1", as_of()));
}
#[test]
fn positions_on_a_lapsed_session_lapse_after_one_refresh() {
    lapses(|c| c.positions("anon-tfsa-1", as_of()));
}
#[test]
fn balances_refused_are_a_refusal_naming_not_found() {
    is_refused("accounts", |c| c.balances(&["anon-tfsa-1".into()]));
}
#[test]
fn balances_on_a_lapsed_session_lapse_after_one_refresh() {
    lapses(|c| c.balances(&["anon-tfsa-1".into()]));
}
#[test]
fn history_refused_is_a_refusal_naming_not_found() {
    is_refused("account", |c| c.history("anon-tfsa-1", None));
}
#[test]
fn history_on_a_lapsed_session_lapses_after_one_refresh() {
    lapses(|c| c.history("anon-tfsa-1", None));
}

// -- accounts (FetchAllAccounts) -----------------------------------------------

#[test]
fn accounts_answered_write_the_account_with_its_type_status_and_name() {
    let p = pulled(Op::Accounts, &[body("wealthsimple-pull/edited-accounts-one.json")]);
    assert!(p.report.failures.is_empty(), "{:?}", p.report.failures);
    assert_eq!((p.report.accounts_added, p.report.accounts_linked, p.asked), (1, 0, 1));
    let [a] = p.book.accounts().unwrap().try_into().unwrap();
    assert_eq!(Some(a.id), p.account());
    assert_eq!(a.account_type, AccountType::Known { kind: AccountKind::Cash, registration: Registration::Tfsa, managed: false, joint: false });
    assert_eq!((a.status, a.nickname.as_deref()), (AccountStatus::Open, Some("anon-personal-5")));
}

#[test]
fn accounts_answered_empty_write_nothing_and_read_nothing_more() {
    let p = pulled(Op::Accounts, &[edited("edited-empty-accounts.json")]);
    assert!(p.report.failures.is_empty(), "{:?}", p.report.failures);
    assert!(p.book.accounts().unwrap().is_empty());
    assert_eq!((p.report.rows_read, p.report.records_new, p.report.days_stored), (0, 0, 0));
}

#[test]
fn accounts_with_a_status_of_another_type_write_nothing_and_fail_the_pull() {
    let p = pulled(Op::Accounts, &[edited("wrong-shape-accounts-status-as-number.json")]);
    let [(part, why)] = p.failures().try_into().unwrap();
    assert_eq!(part, "accounts");
    assert!(why.contains("status"), "{why}");
    assert!(p.book.accounts().unwrap().is_empty());
    assert_eq!(p.report.records_new, 0);
}

#[test]
fn accounts_with_a_status_neither_open_nor_closed_write_nothing_and_fail_the_pull() {
    let p = pulled(Op::Accounts, &[edited("wrong-meaning-accounts-status-neither-open-nor-closed.json")]);
    let [(part, why)] = p.failures().try_into().unwrap();
    assert_eq!(part, "accounts");
    assert!(why.contains("status") && why.contains("frozen"), "{why}");
    assert!(p.book.accounts().unwrap().is_empty());
}

#[test]
fn an_account_whose_margin_boost_is_on_backs_the_margin_account_it_names() {
    // the recorded month holds nothing else of the margin account: its other reads are refused, its accounts' read is not
    let accounts_read = |p: &Pulled| assert!(p.report.failures.iter().all(|(part, _)| part != "accounts"), "{:?}", p.report.failures);
    let p = pulled(Op::Accounts, &[edited("edited-accounts-margin-boost.json")]);
    accounts_read(&p);
    let id = |key: &str| p.book.account_by_ref(&AccountRef::new(Broker::named("wealthsimple"), key)).unwrap().unwrap();
    assert_eq!(p.book.margin_backing().unwrap(), [(id("anon-tfsa-1"), id("anon-margin-9"))].into());
    // off, it backs nothing
    let p = pulled(Op::Accounts, &[edited("edited-accounts-margin-boost-off.json")]);
    accounts_read(&p);
    assert!(p.book.margin_backing().unwrap().is_empty());
}

#[test]
fn a_margin_boost_that_names_no_account_or_states_none_fails_the_pull() {
    for (reply, says) in [("wrong-meaning-accounts-margin-boost-names-no-account.json", "anon-hd-404"), ("wrong-shape-accounts-margin-boost-without-metadata.json", "no metadata")] {
        let p = pulled(Op::Accounts, &[edited(reply)]);
        let (_, why) = p.failures().into_iter().find(|(part, _)| part == "accounts").expect("the accounts' read failed");
        assert!(why.contains(says), "{reply}: {why}");
        assert!(p.book.margin_backing().unwrap().is_empty());
    }
}

// -- activity (FetchActivityFeedItems) -----------------------------------------

#[test]
fn activity_answered_writes_each_row_once_and_the_full_read() {
    let p = pulled(Op::Activity, &[body("wealthsimple-pull/edited-activity-november.json")]);
    assert!(p.report.failures.is_empty(), "{:?}", p.report.failures);
    assert_eq!((p.report.rows_read, p.report.records_new, p.asked), (53, 53, 1));
    assert!(p.problems().is_empty(), "{:?}", p.problems());
    assert_eq!(p.book.transactions().unwrap().len(), 53);
    assert_eq!(p.stated().activity_read_at, Some(NOW.parse().unwrap()));
}

#[test]
fn activity_answered_empty_writes_no_row_and_the_full_read() {
    let p = pulled(Op::Activity, &[edited("edited-empty-activity.json")]);
    assert!(p.report.failures.is_empty(), "{:?}", p.report.failures);
    assert_eq!((p.report.rows_read, p.report.records_new), (0, 0));
    assert!(p.book.transactions().unwrap().is_empty());
    // Wealthsimple stated the account has no rows: that is a whole read
    assert_eq!(p.stated().activity_read_at, Some(NOW.parse().unwrap()));
}

#[test]
fn activity_with_an_amount_of_another_type_keeps_that_row_unreadable_and_the_rest() {
    let p = pulled(Op::Activity, &[edited("wrong-shape-activity-buy-amount-as-number.json")]);
    assert!(p.report.failures.is_empty(), "{:?}", p.report.failures);
    assert_eq!(p.report.records_new, 53);
    let [(code, detail)] = p.problems().try_into().unwrap();
    assert_eq!(code, "unreadable");
    assert!(detail.contains("amount"), "{detail}");
    assert_eq!(p.book.transactions().unwrap().len(), 52);
    assert_eq!(p.stated().activity_read_at, Some(NOW.parse().unwrap()));
}

#[test]
fn activity_with_a_currency_that_is_not_a_code_keeps_that_row_unreadable_and_the_rest() {
    let p = pulled(Op::Activity, &[edited("wrong-meaning-activity-currency-not-a-code.json")]);
    assert!(p.report.failures.is_empty(), "{:?}", p.report.failures);
    assert_eq!(p.report.records_new, 53);
    let [(code, detail)] = p.problems().try_into().unwrap();
    assert_eq!(code, "unreadable");
    assert!(detail.contains("currency") && detail.contains("US$"), "{detail}");
    assert_eq!(p.book.transactions().unwrap().len(), 52);
}

// -- securities (Securities) ---------------------------------------------------

/// The contract the account holds that no row names: its record read for the
/// statement of units.
const CONTRACT: &str = "sec-o-c4c7858bee354465931f9426978d5e89";

fn contract_in(p: &Pulled) -> Option<bagholder_core::InstrumentId> {
    p.book.instrument_by_ref(&Reference::new(RefScheme::BrokerSecurity(Broker::named("wealthsimple")), CONTRACT)).unwrap()
}

#[test]
fn securities_answered_write_each_instrument_and_a_contract_s_terms() {
    let p = pulled(Op::Securities, &[body("wealthsimple-pull/edited-securities.json")]);
    assert!(p.report.failures.is_empty(), "{:?}", p.report.failures);
    // one batch answered every security the pull needed
    assert_eq!(p.asked, 1);
    assert!(p.problems().is_empty(), "{:?}", p.problems());
    let terms = p.book.option_terms(contract_in(&p).expect("the contract")).unwrap().expect("its terms");
    assert_eq!((terms.expiry, terms.strike, terms.multiplier), (day("2028-01-21"), dec("12"), Some(dec("100"))));
    assert_eq!(terms.right, bagholder_core::instrument::OptionRight::Call);
    assert_eq!(p.stated().units.expect("the units stated").1.len(), 19);
}

#[test]
fn securities_answered_empty_leave_every_row_waiting_on_its_security_and_the_units_unstated() {
    let empty = edited("edited-empty-securities.json");
    let p = pulled(Op::Securities, &[empty.clone(), empty]);
    assert_eq!(p.report.records_new, 53);
    let problems = p.problems();
    assert_eq!(problems.len(), 53);
    assert!(problems.iter().all(|(c, _)| c == "security-not-read"), "{problems:?}");
    assert!(p.book.transactions().unwrap().is_empty());
    // each holding no row names is one whose security was not answered
    let failures = p.failures();
    assert_eq!(failures.len(), 19);
    assert!(failures.iter().all(|(part, why)| part == "units:anon-tfsa-1" && why.contains("is not answered")), "{failures:?}");
    assert_eq!(p.stated().units, None);
}

#[test]
fn securities_with_a_strike_of_another_type_leave_the_contract_out_and_the_units_unstated() {
    let p = pulled(Op::Securities, &[edited("wrong-shape-securities-strike-as-number.json")]);
    let [(part, why)] = p.failures().try_into().unwrap();
    assert_eq!(part, "units:anon-tfsa-1");
    assert!(why.contains(CONTRACT) && why.contains("strikePrice"), "{why}");
    assert_eq!(contract_in(&p), None);
    assert_eq!(p.stated().units, None);
    assert_eq!(p.report.records_new, 53);
}

#[test]
fn securities_with_a_contract_on_no_units_leave_the_contract_out_and_the_units_unstated() {
    let p = pulled(Op::Securities, &[edited("wrong-meaning-securities-multiplier-zero.json")]);
    let [(part, why)] = p.failures().try_into().unwrap();
    assert_eq!(part, "units:anon-tfsa-1");
    assert!(why.contains(CONTRACT) && why.contains("multiplier"), "{why}");
    assert_eq!(contract_in(&p), None);
    assert_eq!(p.stated().units, None);
}

// -- order (FetchSoOrdersMultilegOrder) ----------------------------------------

fn multileg(reply: &str) -> (Value, Mapped) {
    let (got, asked) = recorded(Op::Order, reply, row_id(MULTILEG));
    assert_eq!(asked, 1);
    let payload = got.unwrap();
    let m = map_payload(&payload);
    (payload, m)
}

#[test]
fn order_answered_is_kept_in_the_row_s_record_and_writes_each_leg_as_stated() {
    let reply = body("wealthsimple/multileg-48.json");
    let (payload, m) = multileg(&reply);
    assert_eq!(Node::root(&payload).obj("order").unwrap().value(), &node_of(&reply, &["soOrdersMultilegOrder"]));
    assert!(m.problems.is_empty(), "{:?}", m.problems);
    let usd = |a: &str| Some(Money::new(dec(a), Currency::USD));
    let got: Vec<_> = m.legs.iter().map(|d| (d.kind, d.effect, security(d).to_string(), d.quantity, d.price, d.cash)).collect();
    assert_eq!(
        got,
        vec![
            (Kind::Buy, Some(Effect::Close), "sec-o-26327e1d88a344f59572437f96168b6f".into(), Some(dec("15")), usd("1.08"), usd("-1620")),
            (Kind::Sell, Some(Effect::Open), "sec-o-6e84a9314038407299d6e8e4f8618bb2".into(), Some(dec("-15")), usd("2.3"), usd("3450")),
        ]
    );
}

#[test]
fn order_answered_with_no_legs_for_an_executed_row_writes_nothing_and_names_the_disagreement() {
    let (_, m) = multileg(&edited("edited-empty-order.json"));
    assert!(m.legs.is_empty());
    assert_eq!(codes(&m), vec!["legs-disagree"]);
    assert!(m.problems[0].detail.contains("1830"), "{}", m.problems[0].detail);
}

#[test]
fn order_with_a_quantity_of_another_type_writes_nothing_and_is_unreadable() {
    let (_, m) = multileg(&edited("wrong-shape-order-filled-quantity-as-number.json"));
    assert!(m.legs.is_empty());
    assert_eq!(codes(&m), vec!["unreadable"]);
    assert!(m.problems[0].detail.contains("order.legs[0].filledQuantity"), "{}", m.problems[0].detail);
}

#[test]
fn order_with_a_side_neither_buy_nor_sell_writes_nothing_and_is_unreadable() {
    let (_, m) = multileg(&edited("wrong-meaning-order-side-neither-buy-nor-sell.json"));
    assert!(m.legs.is_empty());
    assert_eq!(codes(&m), vec!["unreadable"]);
    assert!(m.problems[0].detail.contains("order.legs[0].side") && m.problems[0].detail.contains("HOLD"), "{}", m.problems[0].detail);
}

// -- entitlements (FetchCorporateActionChildActivities) ------------------------

fn consolidation(reply: &str) -> (Value, Mapped) {
    let (got, asked) = recorded(Op::Entitlements, reply, row_id(CONSOLIDATION));
    assert_eq!(asked, 1);
    let payload = got.unwrap();
    let m = map_payload(&payload);
    (payload, m)
}

#[test]
fn entitlements_answered_are_kept_in_the_row_s_record_and_write_the_units_given_and_received() {
    let reply = body("wealthsimple/corporate-action-1.json");
    let (payload, m) = consolidation(&reply);
    assert_eq!(Node::root(&payload).obj("entitlements").unwrap().value(), &node_of(&reply, &["corporateActionChildActivities"]));
    assert!(m.problems.is_empty(), "{:?}", m.problems);
    let got: Vec<_> = m.legs.iter().map(|d| (d.kind, security(d).to_string(), d.quantity)).collect();
    let old = "sec-s-127fa85dbd4b4ad097d2af5c9d29366c".to_string();
    assert_eq!(got[0], (Kind::CorporateEvent, old.clone(), Some(dec("-177"))));
    assert_eq!((got[1].0, got[1].2), (Kind::CorporateEvent, Some(dec("35.4"))));
    assert_ne!(got[1].1, old);
    assert_eq!(got.len(), 2);
    let [a] = m.adjustments.as_slice() else { panic!("{:?}", m.adjustments) };
    assert_eq!((a.legs[0].units_per_unit, a.legs[0].cost_share), (Some(dec("0.2")), Some(Dec::ONE)));
}

#[test]
fn entitlements_answered_empty_write_nothing_and_name_the_event_unstated() {
    let (_, m) = consolidation(&edited("edited-empty-entitlements.json"));
    assert!(m.legs.is_empty() && m.adjustments.is_empty());
    assert_eq!(codes(&m), vec!["event-unstated"]);
}

#[test]
fn entitlements_with_a_quantity_of_another_type_write_nothing_and_are_unreadable() {
    let (_, m) = consolidation(&edited("wrong-shape-entitlements-quantity-as-number.json"));
    assert!(m.legs.is_empty() && m.adjustments.is_empty());
    assert_eq!(codes(&m), vec!["unreadable"]);
    assert!(m.problems[0].detail.contains("entitlements.nodes[0].quantity"), "{}", m.problems[0].detail);
}

#[test]
fn entitlements_neither_given_up_nor_received_write_nothing_and_are_unclassified() {
    let (_, m) = consolidation(&edited("wrong-meaning-entitlements-type-neither-submit-nor-receive.json"));
    assert!(m.legs.is_empty() && m.adjustments.is_empty());
    assert_eq!(codes(&m), vec!["unclassified"]);
    assert!(m.problems[0].detail.contains("SWAP"), "{}", m.problems[0].detail);
}

// -- conversion (FetchFundingIntent) -------------------------------------------

fn converted(id: &'static str, reply: &str) -> (Value, Mapped) {
    let (got, asked) = recorded(Op::Conversion, reply, row_id(id));
    assert_eq!(asked, 1);
    let payload = got.unwrap();
    let m = map_payload(&payload);
    (payload, m)
}

fn cash_of(m: &Mapped) -> Vec<(String, Option<Money>)> {
    m.legs.iter().map(|d| (d.leg.to_string(), d.cash)).collect()
}

#[test]
fn funding_intent_answered_is_kept_and_writes_the_side_received_naming_the_side_paid_unstated() {
    let reply = body("wealthsimple/FetchFundingIntent-1.json");
    let (payload, m) = converted(FUNDING, &reply);
    let v = json::parse(&reply).unwrap();
    let node = Node::root(&v).obj("data").unwrap().obj("searchFundingIntents").unwrap().list("edges").unwrap()[0].obj("node").unwrap().value().clone();
    assert_eq!(Node::root(&payload).obj("conversion").unwrap().value(), &node);
    assert_eq!(cash_of(&m), vec![("received".into(), Some(Money::new(dec("10268.38"), Currency::USD)))]);
    assert_eq!(codes(&m), vec!["conversion-side-unstated"]);
}

#[test]
fn funding_intent_answered_empty_writes_the_side_received_naming_the_side_paid_unstated() {
    let (payload, m) = converted(FUNDING, &edited("edited-empty-funding-intent.json"));
    assert!(Node::root(&payload).field("conversion").is_err());
    assert_eq!(cash_of(&m), vec![("received".into(), Some(Money::new(dec("10268.38"), Currency::USD)))]);
    assert_eq!(codes(&m), vec!["conversion-side-unstated"]);
}

#[test]
fn funding_intent_with_an_amount_of_another_type_writes_nothing_and_is_unreadable() {
    let (_, m) = converted(FUNDING, &edited("wrong-shape-funding-intent-amount-as-number.json"));
    assert!(m.legs.is_empty());
    assert_eq!(codes(&m), vec!["unreadable"]);
    assert!(m.problems[0].detail.contains("conversion.fundableDetails.fxAdjustedAmount"), "{}", m.problems[0].detail);
}

#[test]
fn funding_intent_of_another_amount_than_its_row_names_the_disagreement() {
    let (_, m) = converted(FUNDING, &edited("wrong-meaning-funding-intent-another-amount.json"));
    assert_eq!(cash_of(&m), vec![("received".into(), Some(Money::new(dec("10268.38"), Currency::USD)))]);
    assert_eq!(codes(&m), vec!["conversion-disagrees", "conversion-side-unstated"]);
    assert!(m.problems[0].detail.contains("10286.38"), "{}", m.problems[0].detail);
}

// -- conversion (FetchInternalTransfer) ----------------------------------------

#[test]
fn internal_transfer_answered_is_kept_and_writes_both_sides_at_its_rate() {
    let reply = body("wealthsimple/FetchInternalTransfer-1.json");
    let (payload, m) = converted(INTERNAL, &reply);
    assert_eq!(Node::root(&payload).obj("conversion").unwrap().value(), &node_of(&reply, &["internalTransfer"]));
    assert!(m.problems.is_empty(), "{:?}", m.problems);
    assert_eq!(cash_of(&m), vec![("received".into(), Some(Money::new(dec("527.36"), Currency::CAD))), ("paid".into(), Some(Money::new(dec("-375.4"), Currency::USD)))]);
    assert_eq!(m.legs[1].fx_rate, Some(dec("1.404789")));
}

#[test]
fn internal_transfer_with_an_amount_of_another_type_writes_nothing_and_is_unreadable() {
    let (_, m) = converted(INTERNAL, &edited("wrong-shape-internal-transfer-amount-as-number.json"));
    assert!(m.legs.is_empty());
    assert_eq!(codes(&m), vec!["unreadable"]);
    assert!(m.problems[0].detail.contains("conversion.amount"), "{}", m.problems[0].detail);
}

#[test]
fn internal_transfer_of_another_amount_than_its_row_names_the_disagreement() {
    let (_, m) = converted(INTERNAL, &edited("wrong-meaning-internal-transfer-another-amount.json"));
    assert_eq!(codes(&m), vec!["conversion-disagrees"]);
}

// -- transfer (FetchInstitutionalTransfer) -------------------------------------

fn transfer_record(reply: &str) -> Answer<Value> {
    let (got, asked) = recorded(Op::Transfer, reply, row_id(TRANSFER));
    // read once: the adapter keeps it for the record
    assert_eq!(asked, 1);
    got
}

#[test]
fn institutional_transfer_answered_is_kept_and_writes_what_arrived_on_the_day_it_completed() {
    let reply = body("wealthsimple/institutional-transfer-1.json");
    let payload = transfer_record(&reply).unwrap();
    assert_eq!(Node::root(&payload).obj("transfer").unwrap().value(), &node_of(&reply, &["accountTransfer"]));
    let m = map_payload(&payload);
    assert!(m.problems.is_empty(), "{:?}", m.problems);
    let [leg] = m.legs.as_slice() else { panic!("{:?}", m.legs) };
    assert_eq!((leg.kind, leg.trade_date, leg.cash), (Kind::TransferIn, day("2023-09-15"), Some(Money::new(dec("80650.3"), Currency::CAD))));
}

#[test]
fn institutional_transfer_answered_with_no_history_moves_nothing_and_names_the_arrival_unstated() {
    let m = map_payload(&transfer_record(&edited("edited-empty-transfer.json")).unwrap());
    assert_eq!(codes(&m), vec!["transfer-arrival-unstated"]);
    assert!(m.legs.iter().all(|l| l.cash.is_none() && l.quantity.is_none()));
}

#[test]
fn institutional_transfer_with_an_instant_of_another_type_writes_no_record() {
    let why = mismatch(transfer_record(&edited("wrong-shape-transfer-completed-at-as-number.json")));
    assert!(why.contains("stateHistories[7].transitionedAt"), "{why}");
}

#[test]
fn institutional_transfer_completed_on_no_day_writes_no_record() {
    let why = mismatch(transfer_record(&edited("wrong-meaning-transfer-completed-on-no-day.json")));
    assert!(why.contains("transitionedAt") && why.contains("not an instant"), "{why}");
}

// -- card (FetchCreditCardAccount) ---------------------------------------------

fn card_balance(reply: &str) -> Result<Dec, bagholder_sources::reply::Mismatch> {
    let f = answering(&[reply.to_string()]);
    let node = f.client().card("anon-ca-4").unwrap();
    read::card_balance(&node, "anon-ca-4")
}

#[test]
fn card_answered_states_what_is_owed_on_it() {
    assert_eq!(card_balance(&body("wealthsimple/credit-card-account-1.json")).unwrap(), dec("5140.28"));
}

#[test]
fn card_with_a_balance_of_another_type_is_a_mismatch() {
    assert_eq!(card_balance(&edited("wrong-shape-card-balance-as-number.json")).unwrap_err().path, "balance.current");
}

#[test]
fn card_answered_for_another_account_is_a_mismatch() {
    let m = card_balance(&edited("wrong-meaning-card-another-account.json")).unwrap_err();
    assert_eq!(m.path, "id");
    assert!(m.why.contains("anon-ca-5"), "{}", m.why);
}

// -- buying power (FetchAccountCurrentMarginBuyingPowerV2) ---------------------
//
// The owner's capture holds no reply to this query yet: these are written from
// the query's own selection (`graphql/FetchAccountCurrentMarginBuyingPowerV2.graphql`)
// with Money's amount as text, as every Money of the captured replies is. The
// first real read is the check (the real run of stage 3c).

fn buying_power_reply(bp: &str) -> String {
    format!(r#"{{"data":{{"account":{{"id":"anon-margin-1","financials":{{"current":{{"id":"c","marginV3":{{"trading":{{"buyingPower":{bp},"__typename":"MarginTrading"}},"__typename":"MarginV3"}},"__typename":"Current"}},"__typename":"Financials"}},"__typename":"Account"}}}}}}"#)
}

fn buying_power(reply: &str) -> Result<Result<Dec, String>, bagholder_sources::reply::Mismatch> {
    let f = answering(&[reply.to_string()]);
    let node = f.client().buying_power("anon-margin-1").unwrap();
    read::buying_power(&node, "anon-margin-1")
}

#[test]
fn buying_power_available_is_its_amount_in_cad() {
    let r = buying_power(&buying_power_reply(r#"{"__typename":"BuyingPowerMetricAvailable","total":{"amount":"12345.67","currency":"CAD","__typename":"Money"}}"#));
    assert_eq!(r.unwrap(), Ok(dec("12345.67")));
}

#[test]
fn buying_power_unavailable_is_the_reason_with_how_many_securities_hold_it_back() {
    let r = buying_power(&buying_power_reply(r#"{"__typename":"BuyingPowerMetricUnavailable","reason":{"__typename":"UnavailableSecurities","securities":[{"securityId":"sec-s-1","status":"x","__typename":"S"},{"securityId":"sec-s-2","status":"x","__typename":"S"}]}}"#));
    assert_eq!(r.unwrap(), Err("UnavailableSecurities (2 securities)".to_string()));
}

#[test]
fn buying_power_of_another_shape_or_meaning_is_a_mismatch() {
    let number = buying_power(&buying_power_reply(r#"{"__typename":"BuyingPowerMetricAvailable","total":{"amount":12345.67,"currency":"CAD"}}"#)).unwrap_err();
    assert!(number.path.ends_with("total.amount"), "{}", number.path);
    let usd = buying_power(&buying_power_reply(r#"{"__typename":"BuyingPowerMetricAvailable","total":{"amount":"1","currency":"USD"}}"#)).unwrap_err();
    assert!(usd.why.contains("USD"), "{}", usd.why);
    let other = buying_power(&buying_power_reply(r#"{"__typename":"BuyingPowerMetricSomethingElse"}"#)).unwrap_err();
    assert!(other.why.contains("neither available nor unavailable"), "{}", other.why);
    let another = buying_power(&buying_power_reply(r#"{"__typename":"BuyingPowerMetricAvailable","total":{"amount":"1","currency":"CAD"}}"#).replace("anon-margin-1", "anon-margin-2")).unwrap_err();
    assert_eq!(another.path, "id");
}

// -- positions (FetchHoldingsExportPositionsAsOfDate) --------------------------

#[test]
fn positions_answered_write_the_units_held_as_of_the_day() {
    let p = pulled(Op::Positions, &[body("wealthsimple-pull/positions@anon-tfsa-1@2025-11-18.json")]);
    assert!(p.report.failures.is_empty(), "{:?}", p.report.failures);
    assert_eq!(p.asked, 1);
    let (on, units) = p.stated().units.expect("the units stated");
    assert_eq!((on, units.len()), (as_of(), 19));
    let of = |id: &str| units[&p.book.instrument_by_ref(&Reference::new(RefScheme::BrokerSecurity(Broker::named("wealthsimple")), id)).unwrap().unwrap()];
    // a long's units, and a short's as a negative quantity
    assert_eq!(of("sec-s-0611ed76cd8445138631597d171d986f"), dec("13000"));
    assert_eq!(of(CONTRACT), dec("-16"));
}

#[test]
fn positions_answered_empty_write_that_nothing_is_held() {
    let p = pulled(Op::Positions, &[edited("edited-empty-positions.json")]);
    assert!(p.report.failures.is_empty(), "{:?}", p.report.failures);
    assert_eq!(p.stated().units, Some((as_of(), BTreeMap::new())));
}

#[test]
fn positions_with_a_quantity_of_another_type_write_no_units() {
    let p = pulled(Op::Positions, &[edited("wrong-shape-positions-quantity-as-number.json")]);
    let [(part, why)] = p.failures().try_into().unwrap();
    assert_eq!(part, "units:anon-tfsa-1");
    assert!(why.contains("quantity"), "{why}");
    assert_eq!(p.stated().units, None);
}

#[test]
fn positions_with_a_long_of_negative_units_write_no_units() {
    let p = pulled(Op::Positions, &[edited("wrong-meaning-positions-long-negative.json")]);
    let [(part, why)] = p.failures().try_into().unwrap();
    assert_eq!(part, "units:anon-tfsa-1");
    assert!(why.contains("quantity") && why.contains("-13000") && why.contains("LONG"), "{why}");
    assert_eq!(p.stated().units, None);
}

// -- balances (FetchAccountsWithBalance) ---------------------------------------

#[test]
fn balances_answered_write_the_cash_per_currency() {
    let p = pulled(Op::Balances, &[body("wealthsimple-pull/edited-balances.json")]);
    assert!(p.report.failures.is_empty(), "{:?}", p.report.failures);
    assert_eq!(p.asked, 1);
    let (at, cash) = p.stated().cash.expect("the cash stated");
    assert_eq!(at, NOW.parse().unwrap());
    assert_eq!(cash, BTreeMap::from([(Currency::CAD, dec("36434.77")), (Currency::USD, dec("15322.57"))]));
}

#[test]
fn balances_answered_empty_write_no_cash() {
    let p = pulled(Op::Balances, &[edited("edited-empty-balances.json")]);
    assert!(p.report.failures.is_empty(), "{:?}", p.report.failures);
    // the account asked for and not answered: its cash is not stated, not nothing
    assert_eq!(p.stated().cash, None);
}

#[test]
fn balances_with_a_quantity_of_another_type_write_no_cash() {
    let p = pulled(Op::Balances, &[edited("wrong-shape-balances-quantity-as-number.json")]);
    let [(part, why)] = p.failures().try_into().unwrap();
    assert_eq!(part, "cash");
    assert!(why.contains("quantity"), "{why}");
    assert_eq!(p.stated().cash, None);
}

#[test]
fn balances_in_a_currency_that_is_not_a_code_write_no_cash() {
    let p = pulled(Op::Balances, &[edited("wrong-meaning-balances-currency-not-a-code.json")]);
    let [(part, why)] = p.failures().try_into().unwrap();
    assert_eq!(part, "cash");
    assert!(why.contains("securityId") && why.contains("C$D"), "{why}");
    assert_eq!(p.stated().cash, None);
}

// -- history (FetchAccountHistoricalFinancials) --------------------------------

#[test]
fn history_answered_writes_each_day_s_value_and_net_deposits_over_every_page() {
    let pages = [body("wealthsimple-pull/history-1.json"), body("wealthsimple-pull/history-2.json"), body("wealthsimple-pull/history-3.json")];
    let p = pulled(Op::History, &pages);
    assert!(p.report.failures.is_empty(), "{:?}", p.report.failures);
    assert_eq!((p.asked, p.report.days_stored), (3, 2102));
    let days = p.stated().days;
    let cad = |a: &str| Money::new(dec(a), Currency::CAD);
    assert_eq!(days.first_key_value().unwrap(), (&day("2020-12-22"), &(cad("0"), cad("0"))));
    // a value stated to more places than a cent is read in its whole cents
    assert_eq!(days[&day("2020-12-29")], (cad("98.87"), cad("100")));
    assert_eq!(*days.last_key_value().unwrap().0, day("2026-09-23"));
}

#[test]
fn history_answered_empty_writes_no_day() {
    let p = pulled(Op::History, &[edited("edited-empty-history.json")]);
    assert!(p.report.failures.is_empty(), "{:?}", p.report.failures);
    assert_eq!(p.report.days_stored, 0);
    assert!(p.stated().days.is_empty());
}

#[test]
fn history_with_cents_of_another_type_writes_no_day() {
    let p = pulled(Op::History, &[edited("wrong-shape-history-cents-as-text.json")]);
    let [(part, why)] = p.failures().try_into().unwrap();
    assert_eq!(part, "history:anon-tfsa-1");
    assert!(why.contains("netLiquidationValueV2.cents"), "{why}");
    assert!(p.stated().days.is_empty());
}

#[test]
fn history_with_a_day_that_is_not_a_day_writes_no_day() {
    let p = pulled(Op::History, &[edited("wrong-meaning-history-day-not-a-day.json")]);
    let [(part, why)] = p.failures().try_into().unwrap();
    assert_eq!(part, "history:anon-tfsa-1");
    assert!(why.contains("date") && why.contains("2026-06-31"), "{why}");
    assert!(p.stated().days.is_empty());
}

#[test]
fn every_status_the_web_app_lists_is_placed_and_any_other_is_a_mismatch() {
    use bagholder_wealthsimple::adapter::settled;
    for s in ["COMPLETED", "CANCELLED", "DECLINED", "EXPIRED", "FAILED", "REJECTED", "REFUNDED", "REVERSED"] {
        assert_eq!(settled(s), Some(true), "{s}");
    }
    for s in ["PENDING", "IN_PROGRESS", "IN_REVIEW", "PARTIALLY_FILLED", "ACTION_REQUIRED", "CANCEL_PENDING", "TRANSFERRING"] {
        assert_eq!(settled(s), Some(false), "{s}");
    }
    assert_eq!(settled("PROCESSING"), None);
}
