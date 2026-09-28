//! The movements Wealthsimple's activity feed leaves out, read from its monthly
//! statements (`docs/plans/statement-gaps.md`): the reply read exactly, the
//! request sent as the Documents page sends it, and the reconciliation that
//! books only what the feed lacks, month by month, as bank reconciliation does.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use bagholder_book::mapping::{Draft, InstrumentDraft, MapContext, Mapped, Mapping};
use bagholder_book::records::Incoming;
use bagholder_book::Book;
use bagholder_broker::statements::{run, Done};
use bagholder_broker::{AccountStated, Activity, Answer, BookMoves, BrokerAdapter, DayValue, Failure, Row, StatementRead, StatementRow, Units};
use bagholder_core::account::{AccountRef, AccountStatus};
use bagholder_core::instrument::Reference;
use bagholder_core::json::{self, Value};
use bagholder_core::record::RecordState;
use bagholder_core::transaction::Kind;
use bagholder_core::{AccountId, Broker, ConnectionId, Currency, Dec, Leg, Money, SourceName};
use bagholder_net::{Ask, Limiter, ManualClock, Net, NetError, Transport};
use bagholder_wealthsimple::statement;

fn dec(s: &str) -> Dec {
    Dec::parse(s).unwrap()
}

fn day(s: &str) -> jiff::civil::Date {
    s.parse().unwrap()
}

fn at(s: &str) -> jiff::Timestamp {
    s.parse().unwrap()
}

fn ws() -> Broker {
    Broker::named("wealthsimple")
}

// -- the reply ---------------------------------------------------------------

fn recorded(name: &str) -> Value {
    let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/replies/wealthsimple-statements").join(name)).unwrap();
    let v = json::parse(&text).unwrap();
    bagholder_sources::reply::Node::root(&v).obj("data").unwrap().obj("monthlyStatement").unwrap().value().clone()
}

#[test]
fn the_recorded_june_statement_reads_every_row_exactly_per_currency() {
    let rows = statement::rows(&recorded("lira-2025-06.json"), Currency::CAD).unwrap();
    let got: Vec<(jiff::civil::Date, Option<jiff::civil::Date>, &str, Currency, Dec, Dec)> = rows.iter().map(|r| (r.day, r.executed, r.code.as_str(), r.currency, r.cash, r.balance)).collect();
    assert_eq!(
        got,
        vec![
            (day("2025-06-26"), Some(day("2025-06-25")), "SELL", Currency::CAD, dec("50952.65"), dec("50952.67")),
            (day("2025-06-26"), Some(day("2025-06-26")), "WHTFED", Currency::CAD, dec("-15278.57"), dec("35674.1")),
            (day("2025-06-26"), Some(day("2025-06-26")), "WD", Currency::CAD, dec("-35650.0"), dec("24.1")),
        ],
        "the CAD list read, the USD list empty, the duplicate top-level list not read twice"
    );
}

#[test]
fn a_row_s_unit_names_what_it_moved_and_its_cash_is_in_its_list_s_currency() {
    // the owner's August 2026 LIRA: a fill's unit is the listing's symbol (`CH`), a coin's its ticker
    let text = recorded("lira-2025-06.json").canonical().replacen("\"$CAD\"", "\"CH\"", 1);
    let rows = statement::rows(&json::parse(&text).unwrap(), Currency::CAD).unwrap();
    assert_eq!((rows[0].currency, rows[0].cash), (Currency::CAD, dec("50952.65")));
}

/// A cash statement in the document's shape, of these rows.
fn cash_statement(rows: &[(&str, &str, &str, &str, &str)]) -> Value {
    let list: Vec<String> = rows.iter().map(|(d, code, desc, cash, bal)| format!(r#"{{"transactionDate":"{d}","transactionType":"{code}","description":"{desc}","cashMovement":"{cash}","balance":"{bal}","__typename":"CashMonthlyStatementTransactions"}}"#)).collect();
    json::parse(&format!(r#"{{"id":"s","statementType":"cash_monthly_statement","data":{{"__typename":"CashMonthlyStatementObject","custodianAccountId":"c","currentTransactions":[{}]}},"__typename":"Statement"}}"#, list.join(","))).unwrap()
}

/// A brokerage statement in the document's shape, of these CAD rows.
fn brokerage_statement(rows: &[(&str, &str, &str, &str, &str)]) -> Value {
    let list: Vec<String> = rows.iter().map(|(d, code, desc, cash, bal)| format!(r#"{{"transactionDate":"{d}","transactionType":"{code}","description":"{desc}","cashMovement":"{cash}","balance":"{bal}","unit":"$CAD","__typename":"BrokerageMonthlyStatementTransactions"}}"#)).collect();
    let l = list.join(",");
    json::parse(&format!(
        r#"{{"id":"s","statementType":"brokerage_monthly_statement","data":{{"__typename":"BrokerageMonthlyStatementObject","custodianAccountId":"b","isMultiCurrency":true,"currentTransactions":[{l}],"activitiesPerCurrency":[{{"currency":"CAD","currentTransactions":[{l}],"__typename":"x"}},{{"currency":"USD","currentTransactions":[],"__typename":"x"}}]}},"__typename":"Statement"}}"#
    ))
    .unwrap()
}

#[test]
fn a_cash_statement_s_rows_are_in_the_account_s_own_currency() {
    let v = cash_statement(&[("2025-06-26", "TRFIN", "Transfer in", "35650.0", "35821.69")]);
    let rows = statement::rows(&v, Currency::parse("USD").unwrap()).unwrap();
    assert_eq!((rows[0].currency.as_str(), rows[0].executed, rows[0].book_day()), ("USD", None, day("2025-06-26")));
}

#[test]
fn a_booked_row_is_its_code_s_kind_on_the_day_the_statement_states_never_through_a_zone() {
    let row = &statement::rows(&recorded("lira-2025-06.json"), Currency::CAD).unwrap()[2];
    let payload = statement::payload("lira-1", day("2025-06-01"), 2, row).canonical();
    let m = statement::StatementMapping.map(&ctx(), &payload);
    assert!(m.problems.is_empty(), "{:?}", m.problems);
    assert_eq!((m.legs[0].kind, m.legs[0].trade_date, m.legs[0].cash), (Kind::Withdrawal, day("2025-06-26"), Some(Money::new(dec("-35650.0"), Currency::CAD))));
    // a code the table does not place, and a trade, are problems and never booked as a movement
    for code in ["XYZ", "SELL"] {
        let r = StatementRow { code: code.into(), ..row.clone() };
        let m = statement::StatementMapping.map(&ctx(), &statement::payload("lira-1", day("2025-06-01"), 2, &r).canonical());
        assert!(m.problems.iter().any(|p| p.detail.contains(code)), "{code}: {:?}", m.problems);
        assert_eq!(m.legs[0].kind, Kind::Unclassified);
    }
}

fn ctx() -> MapContext<'static> {
    static ZONES: std::sync::OnceLock<bagholder_book::zones::Zones> = std::sync::OnceLock::new();
    MapContext { connection: None, record: bagholder_core::RecordId::parse("01923e6a-7b1c-7def-8123-456789abcdef").unwrap(), zones: ZONES.get_or_init(bagholder_book::zones::Zones::new) }
}

// -- the request ----------------------------------------------------------------

#[derive(Default)]
struct Seen {
    asks: Mutex<Vec<Vec<(String, String)>>>,
}

struct Recording(Arc<Seen>, String);

impl Transport for Recording {
    fn answer(&self, ask: &Ask) -> Result<bagholder_net::Answer, NetError> {
        self.0.asks.lock().unwrap().push(ask.headers.iter().map(|(k, v)| (k.to_lowercase(), v.to_string())).collect());
        Ok((200, ask.url.to_string(), vec![], self.1.clone().into_bytes()))
    }
}

fn client_with(session: &str) -> (Arc<Seen>, Net, tempfile::TempDir) {
    let seen = Arc::new(Seen::default());
    let body = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/replies/wealthsimple-statements/lira-2025-06.json")).unwrap();
    let net = Net::answered_by(Arc::new(ManualClock::at(at("2026-09-24T12:00:00Z"))), Arc::new(Limiter::new()), Box::new(Recording(seen.clone(), body)));
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("session.json"), session).unwrap();
    (seen, net, dir)
}

#[test]
fn the_statement_is_asked_as_the_documents_page_asks_it() {
    use bagholder_wealthsimple::adapter::Source;
    let (seen, net, dir) = client_with(r#"{"access_token":"a1","refresh_token":"r1","client_id":"c","identity_canonical_id":"identity-1","expires_at":"2026-09-24T13:00:00Z","wssdi":"device-1"}"#);
    let mut client = bagholder_wealthsimple::client::Client::new(&net, bagholder_wealthsimple::session::SessionFile { path: dir.path().join("session.json") });
    let got = client.statement("lira-1", day("2025-06-01"), statement::BROKERAGE).unwrap().expect("issued");
    assert_eq!(statement::rows(&got, Currency::CAD).unwrap().len(), 3);
    let asks = seen.asks.lock().unwrap();
    let h: BTreeMap<&str, &str> = asks[0].iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    for (k, v) in [
        ("x-ws-operation-name", "FetchMonthlyStatementWithTransactions"),
        ("x-ws-operation-hash", "d5950e5b8d4b7b49a8fe02d68d3d5f7d33d3b7bd555a95db3089fff3dd4d917a"),
        ("x-ws-device-id", "device-1"),
        ("x-ws-page", "page-docs"),
        ("x-web-version", "0.3.671473"),
        ("x-ws-identity-id", "identity-1"),
    ] {
        assert_eq!(h.get(k).copied(), Some(v), "{k}");
    }
}

#[test]
fn a_sign_in_that_kept_no_device_id_is_refused_naming_the_request_and_asks_nothing() {
    use bagholder_wealthsimple::adapter::Source;
    let (seen, net, dir) = client_with(r#"{"access_token":"a1","refresh_token":"r1","client_id":"c","identity_canonical_id":"identity-1","expires_at":"2026-09-24T13:00:00Z"}"#);
    let mut client = bagholder_wealthsimple::client::Client::new(&net, bagholder_wealthsimple::session::SessionFile { path: dir.path().join("session.json") });
    match client.statement("lira-1", day("2025-06-01"), statement::BROKERAGE) {
        Err(Failure::Refused(w)) => assert!(w.contains("FetchMonthlyStatementWithTransactions") && w.contains("device id"), "{w}"),
        other => panic!("{other:?}"),
    }
    assert!(seen.asks.lock().unwrap().is_empty());
}

// -- reconciliation ----------------------------------------------------------------

/// A feed row: one cash movement, as the activity feed states it.
struct Feed;

impl Mapping for Feed {
    fn source(&self) -> SourceName {
        SourceName::named("wealthsimple")
    }
    fn version(&self) -> u32 {
        1
    }
    fn map(&self, _ctx: &MapContext, payload: &str) -> Mapped {
        cash_leg(payload)
    }
}

/// A row of a statement file the person imported.
struct File;

impl Mapping for File {
    fn source(&self) -> SourceName {
        bagholder_broker::csv::source()
    }
    fn version(&self) -> u32 {
        1
    }
    fn map(&self, _ctx: &MapContext, payload: &str) -> Mapped {
        cash_leg(payload)
    }
}

fn cash_leg(payload: &str) -> Mapped {
    let v = json::parse(payload).unwrap();
    let n = bagholder_sources::reply::Node::root(&v);
    Mapped {
        legs: vec![Draft {
            leg: Leg::named("cash"),
            account: AccountRef::new(ws(), n.text("account").unwrap().to_string()),
            occurred_at: None,
            trade_date: n.day("day").unwrap(),
            settle_date: None,
            kind: Kind::parse(n.text("kind").unwrap()).unwrap(),
            effect: None,
            instrument: None,
            quantity: None,
            price: None,
            cash: Some(Money::new(dec(n.text("cash").unwrap()), n.field("currency").map(|c| Currency::parse(c.as_text().unwrap()).unwrap()).unwrap_or(Currency::CAD))),
            fee: None,
            fx_rate: None,
            paid_on: None,
            value: None,
        }],
        problems: vec![],
        adjustments: vec![],
    }
}

/// The broker, for the statements alone: each account's months, and what was asked.
struct Statements {
    months: BTreeMap<(String, jiff::civil::Date), Option<Value>>,
    refuse: BTreeSet<(String, jiff::civil::Date)>,
    asked: Vec<String>,
}

impl BrokerAdapter for Statements {
    fn broker(&self) -> Broker {
        ws()
    }
    fn mapping(&self) -> &dyn Mapping {
        &Feed
    }
    fn accounts(&mut self) -> Answer<Vec<AccountStated>> {
        unreachable!()
    }
    fn activity(&mut self, _: &str, _: Option<jiff::civil::Date>) -> Answer<Activity> {
        unreachable!()
    }
    fn record(&mut self, _: &Row, _: &mut dyn BookMoves) -> Answer<Value> {
        unreachable!()
    }
    fn holds(&self, _: &Value, _: &Row) -> bool {
        unreachable!()
    }
    fn reads_positions(&self, _: &Value) -> bool {
        unreachable!()
    }
    fn unsettled(&self, _: &Value) -> Option<(String, Option<jiff::civil::Date>)> {
        unreachable!()
    }
    fn placed(&self, _: &Value) -> Option<(String, jiff::civil::Date)> {
        unreachable!()
    }
    fn day(&self, _: jiff::Timestamp) -> jiff::civil::Date {
        unreachable!()
    }
    fn cash(&mut self, _: &[String]) -> Answer<BTreeMap<String, BTreeMap<Currency, Dec>>> {
        unreachable!()
    }
    fn buying_power(&mut self, _: &[String]) -> Answer<BTreeMap<String, Result<Dec, String>>> {
        unreachable!()
    }
    fn units(&mut self, _: &str, _: jiff::civil::Date) -> Answer<Vec<Units>> {
        unreachable!()
    }
    fn instruments(&mut self, _: &[Reference], _: jiff::civil::Date) -> Vec<(Reference, Answer<InstrumentDraft>)> {
        unreachable!()
    }
    fn history(&mut self, _: &str, _: Option<jiff::civil::Date>) -> Answer<Vec<DayValue>> {
        unreachable!()
    }
    fn statement(&mut self, account: &str, month: jiff::civil::Date) -> Answer<StatementRead> {
        self.asked.push(format!("{account} {month}"));
        if self.refuse.contains(&(account.to_string(), month)) {
            return Err(Failure::Refused("UNPROCESSABLE_ENTITY".into()));
        }
        match self.months.get(&(account.to_string(), month)) {
            Some(Some(v)) => Ok(StatementRead::Issued { payload: v.clone(), rows: self.statement_rows(account, v)? }),
            Some(None) | None => Ok(StatementRead::NotIssued),
        }
    }
    fn statement_rows(&self, _account: &str, payload: &Value) -> Answer<Vec<StatementRow>> {
        statement::rows(payload, Currency::CAD).map_err(|m| Failure::Mismatch(m.to_string()))
    }
    fn statement_mapping(&self) -> Option<&dyn Mapping> {
        Some(&statement::StatementMapping)
    }
    fn statement_record(&self, account: &str, month: jiff::civil::Date, position: usize, row: &StatementRow) -> Option<(String, Value)> {
        Some((statement::key(account, month, position), statement::payload(account, month, position, row)))
    }
    fn statement_gap_record(&self, gap: &bagholder_broker::statements::Gap) -> Option<(String, Value)> {
        Some((statement::gap_key(gap), statement::gap_payload(gap)))
    }
    fn conversion_paid_record(&self, paid: &bagholder_broker::statements::Paid) -> Option<(String, Value)> {
        Some((statement::paid_key(paid), statement::paid_payload(paid)))
    }
    fn fill_record(&self, fill: &bagholder_broker::statements::Fill) -> Option<(String, Value)> {
        Some((statement::fill_key(fill), statement::fill_payload(fill)))
    }
}

/// A fill in the activity feed: a coin's units and the cash they cost.
struct FeedFill;

impl Mapping for FeedFill {
    fn source(&self) -> SourceName {
        SourceName::named("wealthsimple")
    }
    fn version(&self) -> u32 {
        1
    }
    fn map(&self, _ctx: &MapContext, payload: &str) -> Mapped {
        let v = json::parse(payload).unwrap();
        let n = bagholder_sources::reply::Node::root(&v);
        let symbol = n.text("symbol").unwrap().to_string();
        let day = n.day("day").unwrap();
        Mapped {
            legs: vec![Draft {
                leg: Leg::named("trade"),
                account: AccountRef::new(ws(), n.text("account").unwrap().to_string()),
                occurred_at: Some(at(n.text("at").unwrap())),
                trade_date: day,
                settle_date: None,
                kind: Kind::parse(n.text("kind").unwrap()).unwrap(),
                effect: None,
                instrument: Some(InstrumentDraft {
                    refs: vec![Reference::new(bagholder_core::instrument::RefScheme::BrokerSecurity(ws()), format!("sec-z-{}", symbol.to_lowercase()))],
                    kind: bagholder_core::instrument::InstrumentKind::Crypto,
                    currency: Currency::CAD,
                    name: Some(bagholder_book::mapping::NameDraft { symbol, venue_mic: None, venue_name: None, name: None, seen: day }),
                    option: None,
                    standing: None,
                }),
                quantity: Some(dec(n.text("qty").unwrap())),
                price: None,
                cash: Some(Money::new(dec(n.text("cash").unwrap()), Currency::CAD)),
                fee: None,
                fx_rate: None,
                paid_on: None,
                value: None,
            }],
            problems: vec![],
            adjustments: vec![],
        }
    }
}

/// A book of two accounts, a LIRA and a chequing account, and the broker's statements.
struct World {
    _dir: tempfile::TempDir,
    book: Book,
    conn: ConnectionId,
    lira: AccountId,
    cash: AccountId,
    broker: Statements,
    n: usize,
    /// The broker's accounts behind the book's LIRA.
    lira_keys: Vec<String>,
    /// The broker's accounts it states closed.
    closed: BTreeSet<String>,
}

const NOW: &str = "2025-07-15T16:00:00Z";

impl World {
    fn new() -> World {
        let dir = tempfile::tempdir().unwrap();
        let (book, _) = Book::open_in(dir.path(), "test", at(NOW)).unwrap();
        let conn = book.add_connection(&ws(), "Wealthsimple", at(NOW)).unwrap();
        let lira = book.add_account(conn, &[AccountRef::new(ws(), "lira-1")], &bagholder_book::import::wealthsimple_account_type("SELF_DIRECTED_LIRA"), AccountStatus::Open, None, at(NOW)).unwrap();
        let cash = book.add_account(conn, &[AccountRef::new(ws(), "cash-1")], &bagholder_book::import::wealthsimple_account_type("CASH"), AccountStatus::Open, None, at(NOW)).unwrap();
        World { _dir: dir, book, conn, lira, cash, broker: Statements { months: BTreeMap::new(), refuse: BTreeSet::new(), asked: vec![] }, n: 0, lira_keys: vec!["lira-1".into()], closed: BTreeSet::new() }
    }

    fn row(&mut self, mapping: &dyn Mapping, account: &str, d: &str, kind: &str, cash: &str) -> bagholder_core::RecordId {
        self.n += 1;
        let payload = format!(r#"{{"account":"{account}","cash":"{cash}","day":"{d}","kind":"{kind}"}}"#);
        self.book.store(mapping, &Incoming { connection: Some(self.conn), source_key: &format!("row-{}", self.n), payload: &payload, refs: vec![] }, at(NOW)).unwrap().record
    }

    fn feed(&mut self, account: &str, d: &str, kind: &str, cash: &str) -> bagholder_core::RecordId {
        self.row(&Feed, account, d, kind, cash)
    }

    /// A feed row in another currency.
    fn feed_in(&mut self, account: &str, d: &str, kind: &str, cash: &str, currency: &str) -> bagholder_core::RecordId {
        self.n += 1;
        let payload = format!(r#"{{"account":"{account}","cash":"{cash}","currency":"{currency}","day":"{d}","kind":"{kind}"}}"#);
        self.book.store(&Feed, &Incoming { connection: Some(self.conn), source_key: &format!("row-{}", self.n), payload: &payload, refs: vec![] }, at(NOW)).unwrap().record
    }

    /// The broker states the account's cash in each currency.
    fn states_in(&mut self, account: AccountId, cash: &[(&str, &str)]) {
        self.n += 1;
        let when = at(NOW).checked_sub(jiff::Span::new().seconds(1000 - self.n as i64)).unwrap();
        let read = self.book.broker_read(self.conn, "cash", when).unwrap();
        self.book.store_cash(account, when, &cash.iter().map(|(c, v)| (Currency::parse(c).unwrap(), dec(v))).collect(), &read).unwrap();
    }

    fn book_cash_in(&self, account: AccountId, currency: &str) -> Dec {
        let c = Currency::parse(currency).unwrap();
        self.book.transactions().unwrap().iter().filter(|t| t.account == account).filter_map(|t| t.cash).filter(|m| m.currency == c).fold(Dec::ZERO, |a, m| a.checked_add(m.amount).unwrap())
    }

    /// The broker states the account's cash, each statement a second after the one before.
    fn states(&mut self, account: AccountId, cash: &str) {
        self.n += 1;
        let when = at(NOW).checked_sub(jiff::Span::new().seconds(1000 - self.n as i64)).unwrap();
        let read = self.book.broker_read(self.conn, "cash", when).unwrap();
        self.book.store_cash(account, when, &[(Currency::CAD, dec(cash))].into_iter().collect(), &read).unwrap();
    }

    fn month(&mut self, account: &str, m: &str, v: Option<Value>) {
        self.broker.months.insert((account.to_string(), day(m)), v);
    }

    fn run(&mut self, failures: &mut Vec<(String, Failure)>) -> Done {
        self.broker.asked.clear();
        let keys: BTreeMap<AccountId, Vec<String>> = [(self.lira, self.lira_keys.clone()), (self.cash, vec!["cash-1".to_string()])].into_iter().collect();
        run(&self.book, &mut self.broker, self.conn, &keys, &self.closed, day("2025-07-15"), at(NOW), &mut |_| {}, failures).unwrap()
    }

    /// A coin's fill in the feed.
    fn fill(&mut self, account: &str, when: &str, kind: &str, symbol: &str, qty: &str, cash: &str) -> bagholder_core::RecordId {
        self.n += 1;
        let payload = format!(r#"{{"account":"{account}","at":"{when}","cash":"{cash}","day":"{}","kind":"{kind}","qty":"{qty}","symbol":"{symbol}"}}"#, &when[..10]);
        self.book.store(&FeedFill, &Incoming { connection: Some(self.conn), source_key: &format!("fill-{}", self.n), payload: &payload, refs: vec![] }, at(NOW)).unwrap().record
    }

    /// Each kept statement's fills whose units the statement states otherwise, taking the feed's place.
    fn fills(&mut self, failures: &mut Vec<(String, Failure)>) -> usize {
        let keys: BTreeMap<AccountId, Vec<String>> = [(self.lira, self.lira_keys.clone()), (self.cash, vec!["cash-1".to_string()])].into_iter().collect();
        bagholder_broker::statements::fills(&self.book, &mut self.broker, self.conn, &keys, at(NOW), failures).unwrap()
    }

    /// Units of a coin the account's live transactions hold.
    fn units_of(&self, account: AccountId, symbol: &str) -> Dec {
        let i = self.book.instrument_by_ref(&Reference::new(bagholder_core::instrument::RefScheme::BrokerSecurity(ws()), format!("sec-z-{}", symbol.to_lowercase()))).unwrap().unwrap();
        self.book.transactions().unwrap().iter().filter(|t| t.account == account && t.instrument == Some(i)).filter_map(|t| t.quantity).fold(Dec::ZERO, |a, q| a.checked_add(q).unwrap())
    }

    fn book_cash(&self, account: AccountId) -> Dec {
        self.book.transactions().unwrap().iter().filter(|t| t.account == account).filter_map(|t| t.cash).fold(Dec::ZERO, |a, c| a.checked_add(c.amount).unwrap())
    }
}

/// The owner's June 2025: the feed has the LIRA's sale and its withholding tax
/// but not the withdrawal, and nothing of its arrival in the chequing account.
fn owners_june() -> World {
    let mut w = World::new();
    // May: each account's opening, in the feed and on May's statements
    w.feed("lira-1", "2025-05-20", "deposit", "0.02");
    w.feed("cash-1", "2025-05-20", "deposit", "171.69");
    w.month("lira-1", "2025-05-01", Some(brokerage_statement(&[("2025-05-20", "CONT", "Contribution", "0.02", "0.02")])));
    w.month("cash-1", "2025-05-01", Some(cash_statement(&[("2025-05-20", "AFT_IN", "Direct deposit", "171.69", "171.69")])));
    // June: the sale executed on the 25th and settled on the 26th, the tax, and the
    // withdrawal and its arrival, which only the statements state
    w.feed("lira-1", "2025-06-25", "sell", "50952.65");
    w.feed("lira-1", "2025-06-26", "withholding-tax", "-15278.57");
    w.month("lira-1", "2025-06-01", Some(recorded("lira-2025-06.json")));
    w.month("cash-1", "2025-06-01", Some(cash_statement(&[("2025-06-26", "TRFIN", "Transfer in", "35650.0", "35821.69")])));
    let (lira, cash) = (w.lira, w.cash);
    w.states(lira, "24.10");
    w.states(cash, "35821.69");
    w
}

#[test]
fn the_withdrawal_only_the_statements_state_is_booked_once_on_both_sides_and_joined() {
    let mut w = owners_june();
    let mut failures = vec![];
    let done = w.run(&mut failures);
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!((done.booked, done.joined, done.unreconciled.len()), (2, 1, 0), "{done:?}");
    // the sale settled on the 26th matched the feed's row executed on the 25th: not booked again
    assert_eq!(w.book_cash(w.lira), dec("24.10"));
    assert_eq!(w.book_cash(w.cash), dec("35821.69"));
    // each account back to its base month, May, and no further
    let asked: BTreeSet<&str> = w.broker.asked.iter().map(String::as_str).collect();
    assert_eq!(asked, ["lira-1 2025-06-01", "cash-1 2025-06-01", "lira-1 2025-05-01", "cash-1 2025-05-01"].into_iter().collect());
    assert_eq!(w.broker.asked.len(), 4);
    let links = w.book.transfer_links().unwrap();
    assert_eq!(links.len(), 1);
    // a month read is kept: the next pull, with the cash now agreeing, asks nothing
    let done = w.run(&mut failures);
    assert_eq!((done.read, done.booked), (0, 0));
}

#[test]
fn a_persistent_difference_is_read_once_and_the_next_pull_asks_nothing() {
    let mut w = owners_june();
    // a movement in July, not in any statement yet: the cash still disagrees
    w.feed("cash-1", "2025-07-02", "deposit", "5");
    let a = w.cash;
    w.states(a, "35821.69");
    let mut failures = vec![];
    let first = w.run(&mut failures);
    assert!(first.read > 0);
    let second = w.run(&mut failures);
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(w.broker.asked, Vec::<String>::new(), "every month read is kept");
    assert_eq!(second.booked, 0);
}

#[test]
fn an_account_that_agrees_reads_no_statement() {
    let mut w = World::new();
    w.feed("lira-1", "2025-05-20", "deposit", "10");
    let a = w.lira;
    w.states(a, "10");
    let a = w.cash;
    w.states(a, "0");
    let mut failures = vec![];
    let done = w.run(&mut failures);
    assert_eq!((done.read, done.booked), (0, 0));
    assert!(w.broker.asked.is_empty());
}

#[test]
fn a_sale_executed_on_a_month_s_last_session_and_settled_in_the_next_is_outstanding_not_a_difference() {
    let mut w = World::new();
    w.feed("lira-1", "2025-04-10", "deposit", "100");
    w.month("lira-1", "2025-04-01", Some(brokerage_statement(&[("2025-04-10", "CONT", "Contribution", "100", "100")])));
    // sold on Friday 30 May, settled Monday 2 June: May's statement lacks it
    w.feed("lira-1", "2025-05-30", "sell", "50");
    w.month("lira-1", "2025-05-01", Some(brokerage_statement(&[])));
    w.month("lira-1", "2025-06-01", Some(brokerage_statement(&[("2025-06-02", "SELL", "X: Sold 1 share (executed at 2025-05-30)", "50", "150"), ("2025-06-20", "WD", "Withdrawal (executed at 2025-06-20)", "-150", "0")])));
    let a = w.lira;
    w.states(a, "0");
    let a = w.cash;
    w.states(a, "0");
    let mut failures = vec![];
    let done = w.run(&mut failures);
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!((done.booked, done.unreconciled.len()), (1, 0), "{done:?}");
    assert_eq!(w.book_cash(w.lira), dec("0"));
}

/// June as the owner's, with a fee only the statement states besides: the
/// LIRA's cash disagrees whatever the feed says of the withdrawal.
fn with_a_fee(w: &mut World) {
    w.month(
        "lira-1",
        "2025-06-01",
        Some(brokerage_statement(&[
            ("2025-06-26", "SELL", "VFV - Vanguard S&P 500 Index ETF: Sold 343.0000 shares (executed at 2025-06-25)", "50952.65", "50952.67"),
            ("2025-06-26", "WHTFED", "Federal withholding tax (executed at 2025-06-26)", "-15278.57", "35674.1"),
            ("2025-06-26", "WD", "Withdrawal (executed at 2025-06-26)", "-35650.0", "24.1"),
            ("2025-06-30", "FEE", "Account fee", "-5", "19.1"),
        ])),
    );
    let a = w.lira;
    w.states(a, "19.10");
}

#[test]
fn a_movement_the_feed_states_a_day_after_the_statement_is_one_movement_never_booked_twice() {
    let mut w = owners_june();
    with_a_fee(&mut w);
    // the feed's withdrawal, dated a day after the statement's
    w.feed("lira-1", "2025-06-27", "withdrawal", "-35650.0");
    let mut failures = vec![];
    let done = w.run(&mut failures);
    assert!(failures.is_empty(), "{failures:?}");
    assert!(done.unreconciled.is_empty() && done.feed_only.is_empty(), "{done:?}");
    assert_eq!(w.book_cash(w.lira), dec("19.10"), "the withdrawal counted once, the fee booked");
    assert_eq!(w.book_cash(w.cash), dec("35821.69"));
}

#[test]
fn a_move_the_statement_codes_as_a_withdrawal_and_the_feed_as_a_transfer_matches() {
    let mut w = owners_june();
    w.feed("lira-1", "2025-06-26", "transfer-out", "-35650.0");
    w.feed("cash-1", "2025-06-26", "transfer-in", "35650.0");
    let mut failures = vec![];
    let done = w.run(&mut failures);
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!((done.booked, done.unreconciled.len()), (0, 0), "{done:?}");
}

#[test]
fn a_movement_in_the_statement_and_an_imported_file_is_booked_once_the_file_s_row_giving_way() {
    let mut w = owners_june();
    with_a_fee(&mut w);
    let file_row = w.row(&File, "lira-1", "2025-06-26", "withdrawal", "-35650.0");
    let mut failures = vec![];
    let done = w.run(&mut failures);
    assert!(failures.is_empty(), "{failures:?}");
    // the fee and the chequing account's arrival booked; the withdrawal booked in the file row's place
    assert_eq!((done.booked, done.joined, done.unreconciled.len()), (3, 1, 0), "{done:?}");
    assert_eq!(w.book.record(file_row).unwrap().state, RecordState::Superseded);
    assert_eq!(w.book_cash(w.lira), dec("19.10"));
}

#[test]
fn a_month_not_issued_yet_is_its_own_state_and_the_month_before_is_the_newest() {
    let mut w = owners_june();
    // today is in August: July has ended, and its statement is not issued yet
    w.month("lira-1", "2025-07-01", None);
    w.month("cash-1", "2025-07-01", None);
    let keys: BTreeMap<AccountId, Vec<String>> = [(w.lira, vec!["lira-1".to_string()]), (w.cash, vec!["cash-1".to_string()])].into_iter().collect();
    let mut failures = vec![];
    let done = run(&w.book, &mut w.broker, w.conn, &keys, &BTreeSet::new(), day("2025-08-03"), at("2025-08-03T16:00:00Z"), &mut |_| {}, &mut failures).unwrap();
    assert!(failures.is_empty(), "not issued is not a failure: {failures:?}");
    assert_eq!(done.booked, 2);
    // July is asked again next time, since it may be issued by then; it is not kept as none
    assert!(w.book.monthly_statements(w.conn, "lira-1").unwrap().iter().all(|(m, _)| *m != day("2025-07-01")));
}

#[test]
fn a_refused_statement_is_a_failure_naming_the_account_and_month_and_books_nothing() {
    let mut w = owners_june();
    w.broker.refuse.insert(("lira-1".into(), day("2025-06-01")));
    let mut failures = vec![];
    let done = w.run(&mut failures);
    assert!(failures.iter().any(|(part, f)| part == "statement:lira-1:2025-06-01" && f.to_string().contains("UNPROCESSABLE_ENTITY")), "{failures:?}");
    assert_eq!(w.book_cash(w.lira), dec("35674.10"), "nothing booked for the LIRA");
    assert!(done.unreconciled.iter().all(|u| u.account != w.lira));
}

#[test]
fn a_trade_only_the_statement_states_is_not_booked_and_its_month_is_named() {
    let mut w = owners_june();
    // the feed lacks the sale too
    let mut w2 = World::new();
    std::mem::swap(&mut w.broker, &mut w2.broker);
    w2.feed("lira-1", "2025-05-20", "deposit", "0.02");
    w2.feed("cash-1", "2025-05-20", "deposit", "171.69");
    w2.feed("lira-1", "2025-06-26", "withholding-tax", "-15278.57");
    let a = w2.lira;
    w2.states(a, "24.10");
    let a = w2.cash;
    w2.states(a, "35821.69");
    let mut failures = vec![];
    let done = w2.run(&mut failures);
    let u = done.unreconciled.iter().find(|u| u.account == w2.lira).expect("named");
    assert!(u.why.as_deref().is_some_and(|w| w.contains("sell")), "{u:?}");
    assert_eq!(w2.book_cash(w2.lira), dec("-15278.55"), "nothing of June booked in the LIRA");
}

#[test]
fn a_second_broker_account_behind_the_book_s_with_no_row_holds_nothing_where_closed_and_is_not_guessed_where_open() {
    // the LIRA is two of the broker's accounts, joined: one merged into the other long ago, with no statement
    let mut w = owners_june();
    w.book.add_account_ref(w.lira, &AccountRef::new(ws(), "lira-old")).unwrap();
    w.lira_keys.push("lira-old".into());
    w.closed.insert("lira-old".into());
    let mut failures = vec![];
    let done = w.run(&mut failures);
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!((done.booked, done.unreconciled.len()), (2, 0), "{done:?}");
    assert_eq!(w.book_cash(w.lira), dec("24.10"));
    // an open one with no row: its balance then is not what it holds now, and is not guessed
    let mut w = owners_june();
    w.book.add_account_ref(w.lira, &AccountRef::new(ws(), "lira-old")).unwrap();
    w.lira_keys.push("lira-old".into());
    let done = w.run(&mut failures);
    assert_eq!(done.booked, 1, "only the chequing account's arrival: {done:?}");
    assert!(done.unreconciled.iter().any(|u| u.account == w.lira && u.statement.is_none()), "{done:?}");
}

#[test]
fn a_month_s_closing_is_where_its_running_balance_ends_whatever_order_its_rows_are_listed_in() {
    // the owner's July 2024 shape: the day's rows listed out of their running order,
    // the last one listed not the month's close
    let mut w = World::new();
    w.feed("lira-1", "2025-05-20", "deposit", "100");
    w.month("lira-1", "2025-05-01", Some(brokerage_statement(&[("2025-05-20", "CONT", "Deposit", "100", "100")])));
    w.feed("lira-1", "2025-06-10", "deposit", "50");
    w.feed("lira-1", "2025-06-10", "withdrawal", "-30");
    w.month("lira-1", "2025-06-01", Some(brokerage_statement(&[("2025-06-10", "EFTOUT", "Withdrawal", "-30", "120"), ("2025-06-10", "EFT", "Deposit", "50", "150"), ("2025-06-20", "FEE", "Fee", "-5", "115")].iter().rev().cloned().collect::<Vec<_>>())));
    let a = w.lira;
    w.states(a, "115");
    let a = w.cash;
    w.states(a, "0");
    let mut failures = vec![];
    let done = w.run(&mut failures);
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!((done.booked, done.unreconciled.len()), (1, 0), "the fee booked, June reconciled: {done:?}");
    assert_eq!(w.book_cash(w.lira), dec("115"));
}

#[test]
fn a_statement_dates_a_movement_on_the_day_it_posted_and_one_posted_late_is_still_the_same_movement() {
    for posted in ["2025-06-10", "2025-06-11", "2025-06-17", "2025-06-18", "2025-06-30"] {
        let mut w = World::new();
        w.feed("lira-1", "2025-05-20", "deposit", "100");
        w.month("lira-1", "2025-05-01", Some(brokerage_statement(&[("2025-05-20", "CONT", "Deposit", "100", "100")])));
        // made on the 10th in the feed, a fee only the statement states besides
        w.feed("lira-1", "2025-06-10", "withdrawal", "-40");
        w.month("lira-1", "2025-06-01", Some(brokerage_statement(&[(posted, "OBP_OUT", "Online bill payment", "-40", "60"), ("2025-06-30", "FEE", "Fee", "-5", "55")])));
        let a = w.lira;
        w.states(a, "55");
        let a = w.cash;
        w.states(a, "0");
        let mut failures = vec![];
        let done = w.run(&mut failures);
        assert_eq!((done.booked, done.unreconciled.len(), done.feed_only.len()), (1, 0, 0), "posted {posted}: {done:?}");
        assert_eq!(w.book_cash(w.lira), dec("55"), "posted {posted}");
    }
}

#[test]
fn a_closed_account_merged_into_the_book_s_carries_its_history_and_holds_no_cash_now() {
    // the book's account is two of the broker's: the old one's statements carry the
    // rows the book holds from it; the current one has its own
    let mut w = owners_june();
    let june = w.broker.months.remove(&("lira-1".into(), day("2025-06-01"))).unwrap();
    let may = w.broker.months.remove(&("lira-1".into(), day("2025-05-01"))).unwrap();
    w.broker.months.insert(("lira-old".into(), day("2025-06-01")), june);
    w.broker.months.insert(("lira-old".into(), day("2025-05-01")), may);
    w.feed("lira-1", "2025-05-21", "deposit", "10");
    w.broker.months.insert(("lira-1".into(), day("2025-05-01")), Some(brokerage_statement(&[("2025-05-21", "CONT", "Deposit", "10", "10")])));
    w.broker.months.insert(("lira-1".into(), day("2025-06-01")), Some(brokerage_statement(&[])));
    w.book.add_account_ref(w.lira, &AccountRef::new(ws(), "lira-old")).unwrap();
    w.lira_keys.push("lira-old".into());
    w.closed.insert("lira-old".into());
    let a = w.lira;
    w.states(a, "34.10");
    let mut failures = vec![];
    let done = w.run(&mut failures);
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!((done.booked, done.unreconciled.len()), (2, 0), "{done:?}");
    assert_eq!(w.book_cash(w.lira), dec("34.10"));
}

/// A book of one May and one June on the LIRA, the chequing account agreeing.
fn lira_months(w: &mut World, may: &[(&str, &str, &str, &str, &str)], june: &[(&str, &str, &str, &str, &str)], stated: &str) {
    w.month("lira-1", "2025-05-01", Some(brokerage_statement(may)));
    w.month("lira-1", "2025-06-01", Some(brokerage_statement(june)));
    let a = w.lira;
    w.states(a, stated);
    let a = w.cash;
    w.states(a, "0");
}

#[test]
fn a_movement_the_feed_states_and_no_statement_lists_is_set_aside_and_said_and_the_months_after_are_still_proven() {
    let mut w = World::new();
    w.feed("lira-1", "2025-05-20", "deposit", "100");
    // interest the feed states on 1 June that no statement lists, and a fee only the statement states
    w.feed("lira-1", "2025-06-01", "interest", "2.06");
    lira_months(&mut w, &[("2025-05-20", "CONT", "Deposit", "100", "100")], &[("2025-06-25", "FEE", "Fee", "-5", "95")], "97.06");
    let mut failures = vec![];
    let done = w.run(&mut failures);
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!((done.booked, done.unreconciled.len()), (1, 0), "the fee booked: {done:?}");
    assert_eq!(done.feed_only.len(), 1);
    assert_eq!(done.feed_only[0].rows, vec![(day("2025-06-01"), dec("2.06"))]);
    assert_eq!(w.book_cash(w.lira), dec("97.06"), "the feed's interest stays as it states it");
}

#[test]
fn a_movement_posted_later_than_a_week_is_still_one_movement_within_a_month() {
    let mut w = World::new();
    w.feed("lira-1", "2025-05-20", "deposit", "100");
    w.feed("lira-1", "2025-06-02", "withdrawal", "-40");
    lira_months(&mut w, &[("2025-05-20", "CONT", "Deposit", "100", "100")], &[("2025-06-13", "EFTOUT", "Withdrawal", "-40", "60"), ("2025-06-25", "FEE", "Fee", "-5", "55")], "55");
    let mut failures = vec![];
    let done = w.run(&mut failures);
    assert_eq!((done.booked, done.unreconciled.len(), done.feed_only.len()), (1, 0, 0), "{done:?}");
    assert_eq!(w.book_cash(w.lira), dec("55"));
}

#[test]
fn a_coin_s_row_dated_a_day_before_the_execution_it_states_matches_the_feed_s_day() {
    let mut w = World::new();
    w.feed("lira-1", "2025-05-20", "deposit", "100");
    w.feed("lira-1", "2025-06-29", "sell", "25");
    lira_months(&mut w, &[("2025-05-20", "CONT", "Deposit", "100", "100")], &[("2025-06-29", "SELL", "Sale of 1 BTC (executed at 2025-06-30)", "25", "125"), ("2025-06-30", "FEE", "Fee", "-5", "120")], "120");
    let mut failures = vec![];
    let done = w.run(&mut failures);
    assert_eq!((done.booked, done.unreconciled.len(), done.feed_only.len()), (1, 0, 0), "{done:?}");
}

#[test]
fn a_statement_that_opens_a_month_off_where_the_one_before_closed_moved_the_cash_and_the_difference_is_booked() {
    // the owner's crypto account, December 2023: November closes at nothing and
    // December opens a cent up, with no row for it; the broker's cash holds the cent
    for (june, stated) in [
        // opening off it with no row at all
        (vec![("2025-06-10", "EFTOUT", "Withdrawal", "-40", "60.01"), ("2025-06-20", "FEE", "Fee", "-5", "55.01")], "55.01"),
        // a zero-cash row stating the balance a cent up
        (vec![("2025-06-01", "TRFINTF", "Amalgamation transfer", "0.0", "100.01"), ("2025-06-10", "EFTOUT", "Withdrawal", "-40", "60.01"), ("2025-06-20", "FEE", "Fee", "-5", "55.01")], "55.01"),
        // a cent down
        (vec![("2025-06-10", "EFTOUT", "Withdrawal", "-40", "59.99"), ("2025-06-20", "FEE", "Fee", "-5", "54.99")], "54.99"),
    ] {
        let mut w = World::new();
        w.feed("lira-1", "2025-05-20", "deposit", "100");
        w.feed("lira-1", "2025-06-10", "withdrawal", "-40");
        lira_months(&mut w, &[("2025-05-20", "CONT", "Deposit", "100", "100")], &june, stated);
        let mut failures = vec![];
        let done = w.run(&mut failures);
        assert!(failures.is_empty(), "{failures:?}");
        assert_eq!((done.booked, done.unreconciled.len(), done.feed_only.len()), (2, 0, 0), "the fee and the gap booked: {done:?}");
        assert_eq!(w.book_cash(w.lira), dec(stated));
        let gap = w.book.transactions().unwrap().into_iter().find(|t| t.id.leg == bagholder_broker::statements::gap_leg()).expect("the gap");
        assert_eq!((gap.trade_date, gap.kind), (day("2025-06-01"), Kind::Fee), "on the month's first day, as the broker's own cash correction");
        // the next pull, with the cash agreeing, books nothing more
        let done = w.run(&mut failures);
        assert_eq!((done.read, done.booked), (0, 0), "{done:?}");
    }
}

#[test]
fn a_gap_booked_is_matched_to_no_row_and_kept_while_the_account_still_differs() {
    let mut w = World::new();
    w.feed("lira-1", "2025-05-20", "deposit", "100");
    w.feed("lira-1", "2025-06-10", "withdrawal", "-40");
    lira_months(&mut w, &[("2025-05-20", "CONT", "Deposit", "100", "100")], &[("2025-06-10", "EFTOUT", "Withdrawal", "-40", "60.01")], "60.01");
    let mut failures = vec![];
    assert_eq!(w.run(&mut failures).booked, 1);
    // a later movement the feed states and no statement covers yet: the account differs again
    w.feed("lira-1", "2025-07-02", "deposit", "5");
    let a = w.lira;
    w.states(a, "60.01");
    let done = w.run(&mut failures);
    assert_eq!((done.booked, done.withdrawn), (0, 0), "{done:?}");
    assert_eq!(w.book_cash(w.lira), dec("65.01"));
}

#[test]
fn a_month_whose_statement_corrects_its_own_balance_by_the_next_opening_reconciles_as_the_feed_states_it() {
    // the owner's crypto account, April 2025: each purchase stated as a buy and a
    // fee a cent short of the feed's, the month's closing off by as much, and the
    // next month opening where the feed is
    let mut w = World::new();
    w.feed("lira-1", "2025-04-20", "deposit", "100");
    w.feed("lira-1", "2025-05-31", "buy", "-25");
    // the next purchase, made on 2 June, filed with June's first rows
    w.feed("lira-1", "2025-06-02", "buy", "-25");
    w.month("lira-1", "2025-04-01", Some(brokerage_statement(&[("2025-04-20", "CONT", "Deposit", "100", "100")])));
    w.month("lira-1", "2025-05-01", Some(brokerage_statement(&[("2025-05-31", "BUY", "Purchase of 1 BTC (executed at 2025-05-31)", "-24.75", "75.25"), ("2025-05-31", "FEE", "Fee for purchase of 1 BTC (executed at 2025-05-31)", "-0.24", "75.01")])));
    w.month(
        "lira-1",
        "2025-06-01",
        Some(brokerage_statement(&[
            ("2025-06-01", "BUY", "Purchase of 1 BTC (executed at 2025-06-02)", "-24.76", "50.24"),
            ("2025-06-01", "FEE", "Fee for purchase of 1 BTC (executed at 2025-06-02)", "-0.24", "50.0"),
            ("2025-06-20", "FEE", "Fee", "-5", "45.0"),
        ])),
    );
    let a = w.lira;
    w.states(a, "45");
    let a = w.cash;
    w.states(a, "0");
    let mut failures = vec![];
    let done = w.run(&mut failures);
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!((done.booked, done.unreconciled.len(), done.feed_only.len()), (1, 0, 0), "June's fee alone booked, May as the feed states it: {done:?}");
    assert_eq!(w.book_cash(w.lira), dec("45"));
}

#[test]
fn a_month_that_neither_reconciles_nor_is_corrected_by_the_next_opening_still_stops() {
    let mut w = World::new();
    w.feed("lira-1", "2025-04-20", "deposit", "100");
    w.feed("lira-1", "2025-05-31", "buy", "-25");
    w.month("lira-1", "2025-04-01", Some(brokerage_statement(&[("2025-04-20", "CONT", "Deposit", "100", "100")])));
    w.month("lira-1", "2025-05-01", Some(brokerage_statement(&[("2025-05-31", "BUY", "Purchase of 1 BTC (executed at 2025-05-31)", "-24.75", "75.25"), ("2025-05-31", "FEE", "Fee", "-0.24", "75.01")])));
    // June opens where May closed: the statement stands by its rows, not the feed's
    w.month("lira-1", "2025-06-01", Some(brokerage_statement(&[("2025-06-20", "FEE", "Fee", "-5", "70.01")])));
    let a = w.lira;
    w.states(a, "70.01");
    let a = w.cash;
    w.states(a, "0");
    let mut failures = vec![];
    let done = w.run(&mut failures);
    let u = done.unreconciled.iter().find(|u| u.account == w.lira).expect("May named");
    assert_eq!(u.month, day("2025-05-01"));
    assert!(u.why.as_deref().is_some_and(|w| w.contains("buy")), "{u:?}");
    assert_eq!(done.booked, 0, "nothing of May on booked: {done:?}");
}

#[test]
fn a_conversion_whose_side_paid_is_unstated_is_read_from_the_stated_cash_when_it_is_all_that_is_unstated() {
    let conversion = Kind::CurrencyConversion.to_string();
    let mut w = World::new();
    // after the newest statement: money in, converted, the side paid stated nowhere
    w.feed("lira-1", "2025-07-02", "deposit", "100");
    w.feed_in("lira-1", "2025-07-03", &conversion, "70.75", "USD");
    let a = w.lira;
    w.states_in(a, &[("CAD", "0"), ("USD", "70.75")]);
    let a = w.cash;
    w.states(a, "0");
    let mut failures = vec![];
    let done = w.run(&mut failures);
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(done.booked, 1, "{done:?}");
    assert_eq!((w.book_cash_in(w.lira, "CAD"), w.book_cash_in(w.lira, "USD")), (dec("0"), dec("70.75")));
    let paid = w.book.transactions().unwrap().into_iter().find(|t| t.id.leg == bagholder_broker::statements::paid_leg()).expect("the side paid");
    assert_eq!((paid.kind, paid.trade_date, paid.cash), (Kind::CurrencyConversion, day("2025-07-03"), Some(Money::new(dec("-100"), Currency::CAD))));
    // read once: the next pull, agreeing, books nothing
    let done = w.run(&mut failures);
    assert_eq!(done.booked, 0);
}

#[test]
fn a_side_paid_is_not_read_where_it_is_not_all_that_is_unstated() {
    let conversion = Kind::CurrencyConversion.to_string();
    // two conversions with a side paid unstated: which paid what is not stated
    let mut w = World::new();
    w.feed("lira-1", "2025-07-02", "deposit", "200");
    w.feed_in("lira-1", "2025-07-03", &conversion, "70.75", "USD");
    w.feed_in("lira-1", "2025-07-04", &conversion, "70.80", "USD");
    let a = w.lira;
    w.states_in(a, &[("CAD", "0"), ("USD", "141.55")]);
    let mut failures = vec![];
    assert_eq!(w.run(&mut failures).booked, 0);
    assert_eq!(w.book_cash_in(w.lira, "CAD"), dec("200"));
    // the side received disagrees too: something else is unstated
    let mut w = World::new();
    w.feed("lira-1", "2025-07-02", "deposit", "100");
    w.feed_in("lira-1", "2025-07-03", &conversion, "70.75", "USD");
    let a = w.lira;
    w.states_in(a, &[("CAD", "0"), ("USD", "60")]);
    assert_eq!(w.run(&mut failures).booked, 0);
    // the cash disagrees by money received, not paid
    let mut w = World::new();
    w.feed("lira-1", "2025-07-02", "deposit", "100");
    w.feed_in("lira-1", "2025-07-03", &conversion, "70.75", "USD");
    let a = w.lira;
    w.states_in(a, &[("CAD", "150"), ("USD", "70.75")]);
    assert_eq!(w.run(&mut failures).booked, 0);
    assert_eq!(w.book_cash_in(w.lira, "CAD"), dec("100"));
}

#[test]
fn an_account_whose_activity_begins_after_the_newest_statement_reads_nothing() {
    let mut w = World::new();
    w.feed("lira-1", "2025-07-02", "transfer-in", "100");
    let a = w.lira;
    w.states(a, "0");
    let a = w.cash;
    w.states(a, "0");
    let mut failures = vec![];
    let done = w.run(&mut failures);
    assert_eq!((done.read, done.booked, done.unreconciled.len()), (0, 0, 0), "{done:?}");
}

/// Keeps a month's statement for the LIRA as read, without a read.
fn kept(w: &mut World, m: &str, rows: &[(&str, &str, &str, &str, &str)]) {
    let read = w.book.broker_read(w.conn, "statement:lira-1", at(NOW)).unwrap();
    w.book.keep_monthly_statement(w.conn, "lira-1", day(m), &brokerage_statement(rows).canonical(), &read).unwrap();
}

#[test]
fn a_fill_the_statement_states_other_units_of_takes_the_statement_s_units_once() {
    // the feed's market purchase states an estimate of its units; the statement the units executed
    let mut w = World::new();
    let feed = w.fill("lira-1", "2024-11-11T15:16:35Z", "buy", "DOGE", "2442.098654", "-1039.16");
    w.fill("lira-1", "2024-11-11T15:17:39Z", "sell", "DOGE", "-2442.502592", "1091.47");
    kept(&mut w, "2024-11-01", &[
        ("2024-11-11", "BUY", "Purchase of 2442.5025927700 DOGE (executed at 2024-11-11), FX Rate: 1.3990", "-1039.16", "0.0"),
        ("2024-11-11", "FEE", "Fee for purchase of 2442.5025927700 DOGE (executed at 2024-11-11)", "0.0", "0.0"),
        ("2024-11-11", "SELL", "Sale of 2442.5025920000 DOGE (executed at 2024-11-11), FX Rate: 1.3866", "1091.47", "1091.47"),
    ]);
    let mut failures = vec![];
    assert_eq!(w.fills(&mut failures), 1, "the purchase's units, and the sale's that agree to the feed's places: {failures:?}");
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(w.book.record(feed).unwrap().state, RecordState::Superseded);
    assert_eq!(w.units_of(w.lira, "DOGE"), dec("0.0000007700"), "the statement's units in, the feed's out");
    assert_eq!(w.book_cash(w.lira), dec("52.31"), "cash as the feed states it");
    // read again, nothing changes
    assert_eq!(w.fills(&mut failures), 0);
}

#[test]
fn same_day_fills_of_one_amount_are_paired_by_their_nearest_units_and_another_coin_s_never() {
    let mut w = World::new();
    w.fill("lira-1", "2024-03-05T15:00:00Z", "buy", "BTC", "0.000223", "-25");
    w.fill("lira-1", "2024-03-05T16:00:00Z", "buy", "BTC", "0.000231", "-25");
    w.fill("lira-1", "2024-03-05T17:00:00Z", "buy", "ETH", "0.010000", "-25");
    kept(&mut w, "2024-03-01", &[
        ("2024-03-05", "BUY", "Purchase of 0.0002227000 BTC (executed at 2024-03-05)", "-25", "-25"),
        ("2024-03-05", "BUY", "Purchase of 0.0002305900 BTC (executed at 2024-03-05)", "-25", "-50"),
        ("2024-03-05", "BUY", "Purchase of 0.0099000000 SOL (executed at 2024-03-05)", "-25", "-75"),
    ]);
    let mut failures = vec![];
    assert_eq!(w.fills(&mut failures), 2);
    assert_eq!(w.units_of(w.lira, "BTC"), dec("0.0004532900"));
    assert_eq!(w.units_of(w.lira, "ETH"), dec("0.010000"), "a row of another coin is no fill of it");
    // a fill taken from its row keeps it: the next read pairs nothing again
    assert_eq!(w.fills(&mut failures), 0);
    assert_eq!(w.units_of(w.lira, "BTC"), dec("0.0004532900"));
}
