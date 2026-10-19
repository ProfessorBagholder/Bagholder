//! What the engine is given (`docs/plans/stage-2-engine.md`, "The engine's entry
//! point"): the book's ledger, the facts the figures use, the market's prices and
//! the clock. The engine reads nothing else: no database, no network, no clock.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::account::Account;
use bagholder_core::instrument::{Instrument, Name, OptionTerms};
use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::tz::TimeZone;
use bagholder_core::jiff::Timestamp;
use bagholder_core::journal::{Group, JournalEntry, JournalSubject, Trade};
use bagholder_core::record::Problem;
use bagholder_core::transaction::Transaction;
use bagholder_core::{AccountId, Broker, Currency, Dec, InstrumentId, Money, RecordId, SourceName, TransactionId};

/// An account and the broker it is held at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountInfo {
    pub account: Account,
    pub broker: Broker,
    /// What the person calls the broker it is held at (`Wealthsimple`), for the
    /// screens: the connection's label.
    pub broker_label: String,
    /// The smallest coin order the broker takes, as it states it: a coin amount
    /// worth less is dust (`crate::dust`). None where the broker states none.
    pub coin_minimum: Option<Money>,
}

/// An instrument with what it is called and, for a contract, its terms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstrumentInfo {
    pub instrument: Instrument,
    /// Oldest first; never overlapping.
    pub names: Vec<Name>,
    pub terms: Option<OptionTerms>,
}

impl InstrumentInfo {
    /// What it is called now.
    pub fn current_name(&self) -> Option<&Name> {
        self.names.last()
    }
}

/// What the ledger needs to know of a record besides its transactions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecordInfo {
    /// The source's own id for it: what orders transactions no instant separates,
    /// so storing a record again never reorders the book.
    pub source_key: String,
    pub problems: Vec<Problem>,
    /// The connection it came through (none for an imported file), and when it
    /// first arrived: what a record that gave no transaction is said by.
    pub connection: Option<bagholder_core::ConnectionId>,
    pub first_received_at: Option<Timestamp>,
}

/// Where a transaction stands in the ledger's order: its account, its trade day,
/// the instant it happened (none first), its record and its leg.
pub type Place = (AccountId, Date, Option<Timestamp>, RecordId, bagholder_core::Leg);

fn place(t: &Transaction) -> Place {
    (t.account, t.trade_date, t.occurred_at, t.id.record, t.id.leg.clone())
}

/// The live transactions, held by key: in the ledger's order (`Place`), found
/// by id, and replaced a record at a time, so a change to one record touches
/// that record and nothing else (`docs/plans/stage-3-the-record-by-change.md`, B).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Transactions {
    rows: BTreeMap<Place, Transaction>,
    by_id: BTreeMap<TransactionId, Place>,
    /// Each instrument's transactions, in any account.
    by_instrument: BTreeMap<InstrumentId, BTreeSet<TransactionId>>,
    /// Each account's transactions that name no instrument.
    cash_by_account: BTreeMap<AccountId, BTreeSet<TransactionId>>,
    /// Each account's instruments, by how many of its transactions name each.
    held_in: BTreeMap<AccountId, BTreeMap<InstrumentId, usize>>,
}

impl Transactions {
    /// Every transaction, in the ledger's order.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Transaction> + Clone {
        self.rows.values()
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn get(&self, id: &TransactionId) -> Option<&Transaction> {
        self.by_id.get(id).and_then(|p| self.rows.get(p))
    }

    /// Where a transaction stands in the ledger's order.
    pub fn place_of(&self, id: &TransactionId) -> Option<&Place> {
        self.by_id.get(id)
    }

    /// A record's transactions, by leg.
    pub fn of_record(&self, record: RecordId) -> impl Iterator<Item = &Transaction> {
        self.by_id.range(TransactionId::new(record, bagholder_core::Leg::first())..).take_while(move |(id, _)| id.record == record).filter_map(|(_, p)| self.rows.get(p))
    }

    /// Every transaction of an instrument, in any account, by id.
    pub fn of_instrument(&self, instrument: InstrumentId) -> impl Iterator<Item = &Transaction> {
        self.by_instrument.get(&instrument).into_iter().flatten().filter_map(|id| self.get(id))
    }

    /// Every transaction of an account that names no instrument, by id.
    pub fn cash_of(&self, account: AccountId) -> impl Iterator<Item = &Transaction> {
        self.cash_by_account.get(&account).into_iter().flatten().filter_map(|id| self.get(id))
    }

    /// The instruments an account's transactions name.
    pub fn instruments_in(&self, account: AccountId) -> impl Iterator<Item = InstrumentId> + '_ {
        self.held_in.get(&account).into_iter().flat_map(|m| m.keys().copied())
    }

    /// A record's transactions replaced by `now` (none: the record is gone).
    pub fn put_record(&mut self, record: RecordId, now: Vec<Transaction>) {
        let gone: Vec<TransactionId> = self.of_record(record).map(|t| t.id.clone()).collect();
        for id in gone {
            self.remove(&id);
        }
        for t in now {
            debug_assert_eq!(t.id.record, record, "a transaction of another record");
            self.insert(t);
        }
    }

    fn remove(&mut self, id: &TransactionId) {
        let Some(p) = self.by_id.remove(id) else { return };
        let Some(t) = self.rows.remove(&p) else { return };
        match t.instrument {
            Some(i) => {
                if let Some(s) = self.by_instrument.get_mut(&i) {
                    s.remove(id);
                    if s.is_empty() {
                        self.by_instrument.remove(&i);
                    }
                }
                if let Some(m) = self.held_in.get_mut(&t.account) {
                    if let Some(n) = m.get_mut(&i) {
                        *n -= 1;
                        if *n == 0 {
                            m.remove(&i);
                        }
                    }
                    if m.is_empty() {
                        self.held_in.remove(&t.account);
                    }
                }
            }
            None => {
                if let Some(s) = self.cash_by_account.get_mut(&t.account) {
                    s.remove(id);
                    if s.is_empty() {
                        self.cash_by_account.remove(&t.account);
                    }
                }
            }
        }
    }

    fn insert(&mut self, t: Transaction) {
        self.remove(&t.id);
        self.by_id.insert(t.id.clone(), place(&t));
        match t.instrument {
            Some(i) => {
                self.by_instrument.entry(i).or_default().insert(t.id.clone());
                *self.held_in.entry(t.account).or_default().entry(i).or_default() += 1;
            }
            None => {
                self.cash_by_account.entry(t.account).or_default().insert(t.id.clone());
            }
        }
        self.rows.insert(place(&t), t);
    }
}

impl FromIterator<Transaction> for Transactions {
    fn from_iter<I: IntoIterator<Item = Transaction>>(items: I) -> Transactions {
        let mut out = Transactions::default();
        for t in items {
            out.insert(t);
        }
        out
    }
}

impl<'a> IntoIterator for &'a Transactions {
    type Item = &'a Transaction;
    type IntoIter = std::collections::btree_map::Values<'a, Place, Transaction>;
    fn into_iter(self) -> Self::IntoIter {
        self.rows.values()
    }
}

/// The transfers between the person's accounts the book links: each transfer
/// out and the transfer in that received it, found from either side.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Links {
    outs: BTreeMap<TransactionId, TransactionId>,
    ins: BTreeMap<TransactionId, TransactionId>,
}

impl Links {
    /// Each link, as (out, in), by its out side.
    pub fn iter(&self) -> impl Iterator<Item = (&TransactionId, &TransactionId)> + Clone {
        self.outs.iter()
    }

    pub fn len(&self) -> usize {
        self.outs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.outs.is_empty()
    }

    /// The transfer in that received `out`.
    pub fn received_by(&self, out: &TransactionId) -> Option<&TransactionId> {
        self.outs.get(out)
    }

    /// The transfer out that `received` received.
    pub fn sent_by(&self, received: &TransactionId) -> Option<&TransactionId> {
        self.ins.get(received)
    }

    pub fn insert(&mut self, out: TransactionId, received: TransactionId) {
        if let Some(was) = self.outs.insert(out.clone(), received.clone()) {
            self.ins.remove(&was);
        }
        // a transfer in is received from one transfer out
        if let Some(was) = self.ins.insert(received, out.clone()) {
            if was != out {
                self.outs.remove(&was);
            }
        }
    }

    /// Every link either side of which is on `record`, taken out.
    pub fn remove_record(&mut self, record: RecordId) {
        let on = |m: &BTreeMap<TransactionId, TransactionId>| -> Vec<TransactionId> {
            m.range(TransactionId::new(record, bagholder_core::Leg::first())..).take_while(|(id, _)| id.record == record).map(|(id, _)| id.clone()).collect()
        };
        for out in on(&self.outs) {
            if let Some(r) = self.outs.remove(&out) {
                self.ins.remove(&r);
            }
        }
        for received in on(&self.ins) {
            if let Some(o) = self.ins.remove(&received) {
                self.outs.remove(&o);
            }
        }
    }
}

impl FromIterator<(TransactionId, TransactionId)> for Links {
    fn from_iter<I: IntoIterator<Item = (TransactionId, TransactionId)>>(items: I) -> Links {
        let mut out = Links::default();
        for (o, i) in items {
            out.insert(o, i);
        }
        out
    }
}

/// What changed in the ledger, by key (`docs/plans/stage-3-the-record-by-change.md`,
/// B): each thing named, as it reads now, or none where it is gone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LedgerDelta {
    pub accounts: BTreeMap<AccountId, Option<AccountInfo>>,
    pub instruments: BTreeMap<InstrumentId, Option<InstrumentInfo>>,
    /// Each record named: what the ledger knows of it and its transactions, or
    /// none where it is no longer live.
    pub records: BTreeMap<RecordId, Option<(RecordInfo, Vec<Transaction>)>>,
    /// Every link either side of which is on a record named, as the book links
    /// them now: they replace every link on those records.
    pub links: Vec<(TransactionId, TransactionId)>,
    pub trades: BTreeMap<bagholder_core::TradeId, Option<(Rank, Trade)>>,
    pub groups: BTreeMap<bagholder_core::GroupId, Option<(Rank, Group)>>,
    pub journal: BTreeMap<JournalSubject, Option<JournalEntry>>,
    /// Each transaction's adjustments: every one of a live record that applies to
    /// it, in the book's order (none: nothing applies any more). They are the
    /// person's records, read and applied with the rest.
    pub adjustments: BTreeMap<TransactionId, Vec<Adjustment>>,
}

impl LedgerDelta {
    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
            && self.instruments.is_empty()
            && self.records.is_empty()
            && self.links.is_empty()
            && self.trades.is_empty()
            && self.groups.is_empty()
            && self.journal.is_empty()
            && self.adjustments.is_empty()
    }

    /// Whether the match can move: the records, the instruments, the accounts or
    /// the adjustments changed (the trades, groups and journal do not match).
    pub fn matches(&self) -> bool {
        !self.records.is_empty() || !self.instruments.is_empty() || !self.accounts.is_empty() || !self.adjustments.is_empty()
    }
}

/// Where a row stands in the book's own order of what the person keeps: when it
/// was made, then the row's own number (trades and groups are read in it, and
/// which of two trades keeps a round trip turns on it).
pub type Rank = (String, i64);

/// What a ranked store holds is found by: its key, and the instrument it is
/// about, where it is about one.
pub trait Keyed {
    type Key: Ord + Clone;
    fn key(&self) -> Self::Key;
    fn instrument(&self) -> Option<InstrumentId> {
        None
    }
    /// The transaction it is anchored on, where it is anchored on one.
    fn anchor(&self) -> Option<bagholder_core::TransactionId> {
        None
    }
}

impl Keyed for Trade {
    type Key = bagholder_core::TradeId;
    fn key(&self) -> bagholder_core::TradeId {
        self.id
    }
    /// The instrument it is anchored on.
    fn instrument(&self) -> Option<InstrumentId> {
        match &self.anchor {
            bagholder_core::journal::Anchor::Opening(o) => Some(o.instrument),
            bagholder_core::journal::Anchor::Orphaned(_) => None,
        }
    }
    fn anchor(&self) -> Option<bagholder_core::TransactionId> {
        match &self.anchor {
            bagholder_core::journal::Anchor::Opening(o) => Some(o.transaction.clone()),
            bagholder_core::journal::Anchor::Orphaned(_) => None,
        }
    }
}

impl Keyed for Group {
    type Key = bagholder_core::GroupId;
    fn key(&self) -> bagholder_core::GroupId {
        self.id
    }
}

/// Rows in the book's order (`Rank`), found and replaced by key.
#[derive(Clone, Debug, PartialEq)]
pub struct Ranked<V: Keyed> {
    rows: BTreeMap<Rank, V>,
    rank_of: BTreeMap<V::Key, Rank>,
    /// Each instrument's rows, by rank.
    on: BTreeMap<InstrumentId, BTreeSet<Rank>>,
    /// Each transaction's rows anchored on it, by rank.
    anchored: BTreeMap<bagholder_core::TransactionId, BTreeSet<Rank>>,
}

impl<V: Keyed> Default for Ranked<V> {
    fn default() -> Self {
        Ranked { rows: BTreeMap::new(), rank_of: BTreeMap::new(), on: BTreeMap::new(), anchored: BTreeMap::new() }
    }
}

impl<V: Keyed> Ranked<V> {
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &V> + Clone {
        self.rows.values()
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn get(&self, key: &V::Key) -> Option<&V> {
        self.rank_of.get(key).and_then(|r| self.rows.get(r))
    }

    /// Where a row stands in the book's order.
    pub fn rank(&self, key: &V::Key) -> Option<&Rank> {
        self.rank_of.get(key)
    }

    /// The rows about an instrument, in the book's order.
    pub fn on(&self, instrument: InstrumentId) -> impl Iterator<Item = &V> {
        self.on.get(&instrument).into_iter().flatten().filter_map(|r| self.rows.get(r))
    }

    /// The rows anchored on a transaction, in the book's order.
    pub fn anchored_on(&self, t: &bagholder_core::TransactionId) -> impl Iterator<Item = &V> {
        self.anchored.get(t).into_iter().flatten().filter_map(|r| self.rows.get(r))
    }

    /// `v` put at `rank`, in place of what its key held.
    pub fn put(&mut self, rank: Rank, v: V) {
        self.remove(&v.key());
        self.rank_of.insert(v.key(), rank.clone());
        if let Some(i) = v.instrument() {
            self.on.entry(i).or_default().insert(rank.clone());
        }
        if let Some(t) = v.anchor() {
            self.anchored.entry(t).or_default().insert(rank.clone());
        }
        self.rows.insert(rank, v);
    }

    pub fn remove(&mut self, key: &V::Key) -> Option<V> {
        let rank = self.rank_of.remove(key)?;
        let v = self.rows.remove(&rank)?;
        if let Some(i) = v.instrument() {
            if let Some(s) = self.on.get_mut(&i) {
                s.remove(&rank);
                if s.is_empty() {
                    self.on.remove(&i);
                }
            }
        }
        if let Some(t) = v.anchor() {
            if let Some(s) = self.anchored.get_mut(&t) {
                s.remove(&rank);
                if s.is_empty() {
                    self.anchored.remove(&t);
                }
            }
        }
        Some(v)
    }
}

impl<V: Keyed> FromIterator<(Rank, V)> for Ranked<V> {
    fn from_iter<I: IntoIterator<Item = (Rank, V)>>(items: I) -> Ranked<V> {
        let mut out = Ranked::default();
        for (rank, v) in items {
            out.put(rank, v);
        }
        out
    }
}

impl<'a, V: Keyed> IntoIterator for &'a Ranked<V> {
    type Item = &'a V;
    type IntoIter = std::collections::btree_map::Values<'a, Rank, V>;
    fn into_iter(self) -> Self::IntoIter {
        self.rows.values()
    }
}

/// The book's record: accounts, instruments, the live transactions, and what the
/// person keeps on them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ledger {
    pub accounts: BTreeMap<AccountId, AccountInfo>,
    pub instruments: BTreeMap<InstrumentId, InstrumentInfo>,
    /// Only live records' transactions.
    pub transactions: Transactions,
    /// Every live record a transaction belongs to.
    pub records: BTreeMap<RecordId, RecordInfo>,
    /// A transfer out of one of the person's accounts and the transfer into
    /// another that received it, as the book links them.
    /// Each transfer out, and the transfer in that received it.
    pub transfer_links: Links,
    /// Every trade the book has assigned, with its anchor.
    pub trades: Ranked<Trade>,
    pub groups: Ranked<Group>,
    pub journal: BTreeMap<JournalSubject, JournalEntry>,
}

impl Ledger {
    /// What `delta` names, as it reads now, in place of what was held; its
    /// adjustments are the facts', applied beside it (`crate::engine`).
    pub fn apply(&mut self, delta: LedgerDelta) {
        let LedgerDelta { accounts, instruments, records, links, trades, groups, journal, adjustments: _ } = delta;
        for (id, a) in accounts {
            match a {
                Some(a) => self.accounts.insert(id, a),
                None => self.accounts.remove(&id),
            };
        }
        for (id, i) in instruments {
            match i {
                Some(i) => self.instruments.insert(id, i),
                None => self.instruments.remove(&id),
            };
        }
        for (id, r) in records {
            self.transfer_links.remove_record(id);
            match r {
                Some((info, txs)) => {
                    self.records.insert(id, info);
                    self.transactions.put_record(id, txs);
                }
                None => {
                    self.records.remove(&id);
                    self.transactions.put_record(id, vec![]);
                }
            }
        }
        for (o, i) in links {
            self.transfer_links.insert(o, i);
        }
        for (id, t) in trades {
            match t {
                Some((rank, t)) => self.trades.put(rank, t),
                None => {
                    self.trades.remove(&id);
                }
            }
        }
        for (id, g) in groups {
            match g {
                Some((rank, g)) => self.groups.put(rank, g),
                None => {
                    self.groups.remove(&id);
                }
            }
        }
        for (subject, j) in journal {
            match j {
                Some(j) => self.journal.insert(subject, j),
                None => self.journal.remove(&subject),
            };
        }
    }
}

/// A fact with where it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sourced<T> {
    pub value: T,
    pub source: SourceName,
}

/// The Bank of Canada's rates and what is known of its calendar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rates {
    /// CAD per unit of the currency, per business day the Bank published.
    pub by_currency: BTreeMap<Currency, BTreeMap<Date, Dec>>,
    /// Per currency, each series held of the Bank's rates (its daily average,
    /// its noon rate, the archive of it), oldest first: a currency with none is
    /// one the Bank publishes no rate for, and a day in none of them is one no
    /// source holds a published rate for.
    pub series: BTreeMap<Currency, Vec<Series>>,
    /// The days the Bank's own schedule says it does not publish (its holidays).
    pub holidays: BTreeSet<Date>,
    /// Per currency, the spans of days completed reads of the Bank's series
    /// covered, and when each read was received: a weekday inside one without a
    /// rate was not published, if that day's 16:30 Eastern had passed by then.
    pub covered: BTreeMap<Currency, Vec<Read>>,
}

/// One series of the Bank's rates for a currency: the days it holds, and whether
/// its source states it has ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Series {
    pub first: Date,
    /// The last day read of it; a series still published goes on past it.
    pub last: Date,
    pub ended: bool,
}

impl Series {
    /// Whether the series holds `d`: on or after its first day, and, once it has
    /// ended, on or before its last.
    pub fn holds(&self, d: Date) -> bool {
        self.first <= d && (!self.ended || d <= self.last)
    }
}

/// One completed read of a series.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Read {
    pub first: Date,
    pub last: Date,
    pub at: Timestamp,
}

/// A distribution the payer declared, as it stated it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declared {
    pub ex_date: Date,
    pub record_date: Option<Date>,
    pub pay_date: Option<Date>,
    /// The cash paid per unit, in the currency it is paid in.
    pub amount: Money,
    /// The part reinvested per unit, where the payer states one: it pays no cash.
    pub reinvested: Option<Dec>,
    /// Whether the source states how it is paid; an unstated row's `amount` is
    /// the whole per unit, its form found from the record (`cashflow`).
    pub form: bagholder_core::distribution::Form,
}

/// One read of a fund's declared record, whole: a distribution the fund
/// withdrew is absent from a later read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeclaredRead {
    pub read_at: Timestamp,
    pub source: SourceName,
    pub items: Vec<Declared>,
    /// The market's record beside a company's, as last read: what it lists
    /// after the company's latest distribution is the next one still to be paid
    /// while the company's publication has not listed it (`SPEC.md` §5, Ex-Div).
    pub market: Vec<Declared>,
}

pub use bagholder_core::adjustment::{Adjustment, AdjustmentLeg};

/// The facts the figures use, each kept in the book once used.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pub rates: Rates,
    /// The newest read of each fund's declared record.
    pub declared: BTreeMap<InstrumentId, DeclaredRead>,
    /// Payments per year as a source states it.
    pub frequencies: BTreeMap<InstrumentId, Sourced<u32>>,
    /// By the transaction each explains.
    pub adjustments: Adjustments,
    /// The debts held outside the broker, and the withdrawals that repaid them
    /// (`docs/plans/debts-held-outside-wealthsimple.md`).
    pub debts: Debts,
}

/// A debt held outside the broker, as the broker's net worth feature states it:
/// what is owed, in its currency, as of when it was last stated.
#[derive(Clone, Debug, PartialEq)]
pub struct Debt {
    pub id: String,
    pub name: String,
    pub institution: String,
    pub balance: Money,
    pub as_of: Option<Timestamp>,
}

/// The debts held outside the broker, and which withdrawals repaid one: each
/// such withdrawal by the debt it repaid, as the matching settled it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Debts {
    pub owed: Vec<Debt>,
    pub repayments: BTreeMap<TransactionId, String>,
    /// The payees whose payments may repay a debt and nothing says which: the
    /// person is asked, once a payee.
    pub asked: Vec<Asked>,
}

/// A payee the person is asked about: who was paid, how many payments, and the
/// debts it may be.
#[derive(Clone, Debug, PartialEq)]
pub struct Asked {
    /// What the answer is kept under.
    pub key: String,
    /// The payee as the payments name it.
    pub payee: String,
    pub payments: usize,
    /// The debts it may repay, by id.
    pub candidates: Vec<String>,
}

/// The adjustment that explains each transaction, chosen from every one the
/// book holds: a sourced one supersedes the person's own; two that explain one
/// transaction differently, with neither superseding the other, explain it not
/// at all and are named as a conflict.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Adjustments {
    chosen: BTreeMap<TransactionId, Adjustment>,
    conflicts: BTreeSet<TransactionId>,
}

impl Adjustments {
    pub fn choose(all: impl IntoIterator<Item = Adjustment>) -> Adjustments {
        let mut by_tx: BTreeMap<TransactionId, Vec<Adjustment>> = BTreeMap::new();
        for a in all {
            by_tx.entry(a.applies_to.clone()).or_default().push(a);
        }
        let mut out = Adjustments::default();
        for (tx, list) in by_tx {
            out.replace(tx, list);
        }
        out
    }

    /// What applies to `tx` chosen again from `list`, every adjustment of a live
    /// record that applies to it, in the book's order (none: nothing applies).
    pub fn replace(&mut self, tx: TransactionId, list: Vec<Adjustment>) {
        self.chosen.remove(&tx);
        self.conflicts.remove(&tx);
        if list.is_empty() {
            return;
        }
        let person = SourceName::person();
        let sourced: Vec<&Adjustment> = list.iter().filter(|a| a.source != person).collect();
        let standing: Vec<&Adjustment> = if sourced.is_empty() { list.iter().collect() } else { sourced };
        if standing.iter().all(|a| a.legs == standing[0].legs) {
            self.chosen.insert(tx, standing[0].clone());
        } else {
            self.conflicts.insert(tx);
        }
    }

    pub fn get(&self, transaction: &TransactionId) -> Option<&Adjustment> {
        self.chosen.get(transaction)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&TransactionId, &Adjustment)> {
        self.chosen.iter()
    }

    /// Whether adjustments that disagree leave the transaction unexplained.
    pub fn in_conflict(&self, transaction: &TransactionId) -> bool {
        self.conflicts.contains(transaction)
    }
}

/// Where a quote came from, as far as it decides what the quote may price
/// (`SPEC.md` §2 Position, Price).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuoteSource {
    /// A listing's exchange feed (TMX, Cboe Canada, Yahoo for a US listing).
    Listing,
    /// A coin's spot price.
    Crypto,
    /// An option contract's chain.
    OptionChain,
}

/// A live price for one instrument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Quote {
    pub price: Money,
    /// The day's change per unit, where the source states it.
    pub change: Option<Dec>,
    /// The day's change in percent, where the source states it.
    pub change_pct: Option<Dec>,
    pub at: Option<Timestamp>,
    pub source: QuoteSource,
}

/// What a broker states about one account, the check against Bagholder's own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrokerAccount {
    /// The account's net value per day, as the broker states it, in the
    /// currency it states it in (an account held in US dollars states its value
    /// in US dollars).
    pub net_value: BTreeMap<Date, Money>,
    /// The money put in less the money taken out to date, per day, as the
    /// broker states it beside its net value.
    pub net_deposits: BTreeMap<Date, Money>,
    /// Its net value now, as the broker states it.
    pub net_value_now: Option<Money>,
    /// When the broker stated its balances and holdings now.
    pub as_of: Option<Timestamp>,
    /// When the account's activity was last read in full: every fill the
    /// broker had posted by then is on the record.
    pub activity_read_at: Option<Timestamp>,
    /// Cash per currency now.
    pub cash: BTreeMap<Currency, Dec>,
    /// Cash per currency as stated at or before `activity_read_at`: every fill
    /// it reflects is on the record, so the broker check compares with it.
    pub cash_read: Option<BTreeMap<Currency, Dec>>,
    /// What the broker held against the account when it stated `cash_read`
    /// (`bagholder_core::hold`): its stated cash is net of them.
    pub cash_read_holds: Vec<bagholder_core::hold::Hold>,
    /// Units held per instrument, as of `held_as_of`.
    pub held: BTreeMap<InstrumentId, Dec>,
    /// What the broker states those units are worth, where it states it.
    pub held_value: BTreeMap<InstrumentId, Money>,
    /// The day the broker states `held` as of: the last day whose activity is
    /// read in full (`docs/plans/stage-3b-wealthsimple.md`, question 4); today
    /// where none is stated.
    pub held_as_of: Option<Date>,
    /// What it can borrow, as the broker states it, or why it cannot say.
    pub buying_power: Option<Result<Money, String>>,
}

/// The ETF that tracks a benchmark, as its source states it: each session's
/// close as traded, each dividend by its ex-date as declared, and each split on
/// the day it took effect (`numerator` new units for every `denominator` held).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BenchmarkSeries {
    pub currency: Currency,
    pub closes: BTreeMap<Date, Dec>,
    pub dividends: BTreeMap<Date, Dec>,
    pub splits: BTreeMap<Date, (Dec, Dec)>,
}

/// The market around the book.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Market {
    pub quotes: BTreeMap<InstrumentId, Quote>,
    /// Daily closes per instrument, each in the currency its source states it
    /// in: the instrument's own, or, for a coin whose pair has no market of its
    /// own, its USD market's, converted at the day's rate where it is used
    /// (`SPEC.md` §2, Coinbase).
    pub closes: BTreeMap<InstrumentId, BTreeMap<Date, Money>>,
    /// Each benchmark's tracker, per benchmark key (`SP500`, `TSX`, `TX60`).
    pub benchmarks: BTreeMap<String, BenchmarkSeries>,
    pub brokers: BTreeMap<AccountId, BrokerAccount>,
    /// The ex-dividend date each listing's quote states (TMX's), where it states
    /// one: the Ex-Div of a payer no declared record serves (`SPEC.md` §5,
    /// Cashflow Positions).
    pub ex_dividends: BTreeMap<InstrumentId, Date>,
}

/// When the engine is asked, and where the person is.
#[derive(Clone, Debug)]
pub struct Clock {
    /// Today, in the person's home zone.
    pub today: Date,
    pub now: Timestamp,
    /// The person's home zone.
    pub home: TimeZone,
    /// The Bank of Canada's zone (Eastern), whose 16:30 is when a day's rate is
    /// published.
    pub bank: TimeZone,
}

impl PartialEq for Clock {
    fn eq(&self, other: &Clock) -> bool {
        self.today == other.today && self.now == other.now && self.home.iana_name() == other.home.iana_name() && self.bank.iana_name() == other.bank.iana_name()
    }
}

/// Everything the engine computes from.
#[derive(Clone, Debug, PartialEq)]
pub struct Inputs {
    pub ledger: Ledger,
    pub facts: Facts,
    pub market: Market,
    pub clock: Clock,
}
