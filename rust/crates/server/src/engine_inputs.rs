//! The engine's inputs read from the book (`docs/plans/stage-2-engine.md`, "The
//! entry point"): its ledger and the facts it keeps. The market's part comes from
//! the market cache.

use std::collections::BTreeMap;

use bagholder_book::Book;
use bagholder_core::Currency;
use bagholder_book::changes::Changes;
use bagholder_core::journal::JournalSubject;
use bagholder_engine::input::{AccountInfo, Adjustments, Declared, DeclaredRead, Facts, InstrumentInfo, Ledger, LedgerDelta, Rates, Read, RecordInfo, Series, Sourced};

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The smallest coin order each broker takes, as it states it; none for a
/// broker that states none.
fn coin_minimum(broker: &bagholder_core::Broker) -> Option<bagholder_core::Money> {
    (*broker == bagholder_wealthsimple::mapping::broker()).then(bagholder_wealthsimple::mapping::coin_minimum)
}

/// The broker's connections, by id: what an account is held at.
type Brokers = BTreeMap<bagholder_core::ConnectionId, (bagholder_core::Broker, String)>;

fn brokers_of(book: &Book) -> Result<Brokers, String> {
    Ok(book.connections().map_err(err)?.into_iter().map(|c| (c.id, (c.broker, c.label))).collect())
}

fn account_info(brokers: &Brokers, a: bagholder_core::account::Account) -> Result<AccountInfo, String> {
    let (broker, broker_label) = brokers.get(&a.connection).cloned().ok_or_else(|| format!("account {} has no connection", a.id))?;
    let coin_minimum = coin_minimum(&broker);
    Ok(AccountInfo { account: a, broker, broker_label, coin_minimum })
}

fn instrument_info(book: &Book, i: bagholder_core::instrument::Instrument) -> Result<InstrumentInfo, String> {
    let names = book.names(i.id).map_err(err)?;
    let terms = book.option_terms(i.id).map_err(err)?;
    Ok(InstrumentInfo { instrument: i, names, terms })
}

/// The ledger: accounts, instruments with their names and terms, the live
/// transactions and their records, the trades, groups and journal.
pub fn ledger(book: &Book) -> Result<Ledger, String> {
    let brokers = brokers_of(book)?;
    let mut accounts = BTreeMap::new();
    for a in book.accounts().map_err(err)? {
        accounts.insert(a.id, account_info(&brokers, a)?);
    }
    let mut instruments = BTreeMap::new();
    for i in book.instruments().map_err(err)? {
        instruments.insert(i.id, instrument_info(book, i)?);
    }
    let mut records: BTreeMap<_, RecordInfo> = book
        .live_record_facts()
        .map_err(err)?
        .into_iter()
        .map(|(id, key, connection, at)| (id, RecordInfo { source_key: key, problems: vec![], connection, first_received_at: Some(at) }))
        .collect();
    for (id, p) in book.problems().map_err(err)? {
        if let Some(r) = records.get_mut(&id) {
            r.problems.push(p);
        }
    }
    Ok(Ledger {
        accounts,
        instruments,
        transactions: book.transactions().map_err(err)?.into_iter().collect(),
        records,
        // the two sides of each move of holdings, as the broker's rows state them
        transfer_links: book.transfer_links().map_err(err)?.into_iter().collect(),
        trades: book.trades_ranked().map_err(err)?.into_iter().collect(),
        groups: book.groups_ranked().map_err(err)?.into_iter().collect(),
        journal: book.journal_entries().map_err(err)?.into_iter().collect(),
    })
}

/// What the record's change log names, read by key as the whole read
/// (`ledger`, `facts().adjustments`) reads it (`docs/plans/stage-3-the-record-by-change.md`,
/// A): each thing named, as it reads now, or none where it is gone. Read in the
/// same snapshot as the log (`Book::snapshot`).
pub fn delta(book: &Book, changes: &Changes) -> Result<LedgerDelta, String> {
    let mut d = LedgerDelta::default();
    if !changes.accounts.is_empty() {
        let brokers = brokers_of(book)?;
        for id in &changes.accounts {
            let a = book.account_held(*id).map_err(err)?.map(|a| account_info(&brokers, a)).transpose()?;
            d.accounts.insert(*id, a);
        }
    }
    for id in &changes.instruments {
        let i = book.instrument_as_read(*id).map_err(err)?.map(|i| instrument_info(book, i)).transpose()?;
        d.instruments.insert(*id, i);
    }
    let mut links = std::collections::BTreeSet::new();
    for id in &changes.records {
        let now = match book.live_record_fact(*id).map_err(err)? {
            Some((source_key, connection, at)) => {
                let problems = book.problems_of(*id).map_err(err)?;
                let info = RecordInfo { source_key, problems, connection, first_received_at: Some(at) };
                Some((info, book.transactions_of(*id).map_err(err)?))
            }
            None => None,
        };
        d.records.insert(*id, now);
        links.extend(book.transfer_links_of(*id).map_err(err)?);
    }
    d.links = links.into_iter().collect();
    for id in &changes.trades {
        d.trades.insert(*id, book.trade_ranked(*id).map_err(err)?);
        let subject = JournalSubject::Trade(*id);
        d.journal.insert(subject, book.journal(subject).map_err(err)?);
    }
    for id in &changes.groups {
        d.groups.insert(*id, book.group_ranked(*id).map_err(err)?);
        let subject = JournalSubject::Group(*id);
        d.journal.insert(subject, book.journal(subject).map_err(err)?);
    }
    for tx in &changes.adjustments {
        d.adjustments.insert(tx.clone(), book.adjustments_of(tx).map_err(err)?);
    }
    Ok(d)
}

/// Per currency, each series of the Bank's rates the book holds.
fn series(book: &Book) -> Result<BTreeMap<Currency, Vec<Series>>, String> {
    let mut out: BTreeMap<Currency, Vec<Series>> = BTreeMap::new();
    for s in book.rate_series().map_err(err)? {
        out.entry(s.currency).or_default().push(Series { first: s.first_day, last: s.last_day, ended: s.ended });
    }
    Ok(out)
}

/// The facts the book keeps.
pub fn facts(book: &Book) -> Result<Facts, String> {
    let rates = Rates { by_currency: book.rates().map_err(err)?, series: series(book)?, holidays: book.bank_holidays().map_err(err)?, covered: book.rate_reads().map_err(err)?.into_iter().map(|(c, reads)| (c, reads.into_iter().map(|(first, last, at)| Read { first, last, at }).collect())).collect() };
    let declared = book
        .declared()
        .map_err(err)?
        .into_iter()
        .map(|(i, r)| {
            let rows = |items: Vec<bagholder_book::facts::DeclaredRow>| -> Vec<Declared> {
                items.into_iter().map(|d| Declared { ex_date: d.ex_date, record_date: d.record_date, pay_date: d.pay_date, amount: d.amount, reinvested: d.reinvested, form: d.form }).collect()
            };
            let market = r.market.map(|m| rows(m.items)).unwrap_or_default();
            (i, DeclaredRead { read_at: r.read_at, source: r.source, items: rows(r.items), market })
        })
        .collect();
    let frequencies = book.frequencies().map_err(err)?.into_iter().map(|(i, f)| (i, Sourced { value: f.per_year, source: f.source })).collect();
    let adjustments = Adjustments::choose(book.adjustments().map_err(err)?);
    Ok(Facts { rates, declared, frequencies, adjustments, debts: debts(book)? })
}

/// The debts held outside the broker and the withdrawals that repaid them
/// (`docs/plans/debts-held-outside-wealthsimple.md`): each outside debt as the
/// newest read states it, and each Wealthsimple withdrawal matched to the debt
/// it paid, where a rule or the person's answer names exactly one.
pub fn debts(book: &Book) -> Result<bagholder_engine::input::Debts, String> {
    use bagholder_wealthsimple::repayments::{match_payment, payment_of, Liability, Matched, LIABILITY_TYPES};
    let (owed, held): (Vec<_>, Vec<_>) = book.outside_debts().map_err(err)?.into_iter().partition(|d| LIABILITY_TYPES.contains(&d.kind.as_str()));
    let liabilities: Vec<Liability> = owed.iter().map(|d| Liability { id: d.entity_id.clone(), kind: d.kind.clone(), institution: d.institution.clone(), last4: d.last4.clone() }).collect();
    let answers = book.repayment_answers().map_err(err)?.into_iter().collect();
    let mut repayments = BTreeMap::new();
    let mut asked: BTreeMap<String, bagholder_engine::input::Asked> = BTreeMap::new();
    for w in book.withdrawals_of("wealthsimple").map_err(err)? {
        // a payload the book holds is the reply as received: one that is not JSON names no payee
        let Ok(record) = bagholder_core::json::parse(&w.payload) else { continue };
        // the record holds the row under `activity`, beside the replies read for it
        let bagholder_core::json::Value::Object(parts) = &record else { continue };
        let Some(row) = parts.get("activity") else { continue };
        let Some(payment) = payment_of(row) else { continue };
        match match_payment(&payment, &liabilities, &answers) {
            Matched::Debt { liability, .. } => {
                repayments.insert(w.transaction, liability);
            }
            Matched::Unsettled { candidates } => {
                let payee = match &payment {
                    bagholder_wealthsimple::repayments::Payment::BillPay { company, .. } => company.clone(),
                    bagholder_wealthsimple::repayments::Payment::Debit { originator, .. } => originator.clone(),
                };
                let a = asked.entry(payment.payee_key()).or_insert_with(|| bagholder_engine::input::Asked { key: payment.payee_key(), payee, payments: 0, candidates });
                a.payments += 1;
            }
            Matched::NotDebt | Matched::NotOne => {}
        }
    }
    let stated = |d: bagholder_book::debts::OutsideDebt| bagholder_engine::input::Debt { id: d.entity_id, kind: d.kind, name: d.name, institution: d.institution, balance: bagholder_core::Money::new(d.balance, d.currency), as_of: d.as_of };
    let owed = owed.into_iter().map(stated).collect();
    let assets = held.into_iter().map(stated).collect();
    // each outside thing's balance by day, as stated of it alone
    let history = book.outside_days().map_err(err)?.into_iter().map(|(id, days)| (id, days.into_iter().map(|d| (d.day, bagholder_core::Money::new(d.amount, d.currency))).collect())).collect();
    Ok(bagholder_engine::input::Debts { owed, assets, history, repayments, asked: asked.into_values().collect() })
}

/// What each broker states about each account (`docs/plans/stage-3b-wealthsimple.md`,
/// "The book's new tables"), read from the book: each day's value and net
/// deposits (in CAD), the newest cash and when it was stated, the newest units
/// and the day they are as of, and when the account's activity was last read in
/// full. An account the broker has stated nothing about has none.
pub fn brokers(book: &Book) -> Result<BTreeMap<bagholder_core::AccountId, bagholder_engine::input::BrokerAccount>, String> {
    let mut out = BTreeMap::new();
    for a in book.accounts().map_err(err)? {
        if let Some(b) = broker_account(&book.stated(a.id).map_err(err)?) {
            out.insert(a.id, b);
        }
    }
    Ok(out)
}

/// What the broker states about one account, as the engine reads it, from what
/// the book holds of it (`Book::stated`, or `Book::stated_latest` without its days).
/// An account the broker has stated nothing about has none.
pub fn broker_account(s: &bagholder_book::statements::Stated) -> Option<bagholder_engine::input::BrokerAccount> {
    if s.days.is_empty() && s.days_held.is_empty() && s.cash.is_none() && s.units.is_none() && s.buying_power.is_none() && s.net_value_now.is_none() && s.margin_rates.is_none() {
        return None;
    }
    let mut b = bagholder_engine::input::BrokerAccount::default();
    // in whatever currency the broker states them (an account held in US dollars
    // states its value in US dollars): the figures put them in CAD at the rate
    // of the day they are for
    for (day, (value, deposits)) in &s.days {
        b.net_value.insert(*day, *value);
        b.net_deposits.insert(*day, *deposits);
    }
    // the account's value now, as the broker states it with the accounts
    // (its daily values are a day behind, and are the equity curve's)
    b.net_value_now = s.net_value_now.map(|(_, v)| v);
    if let Some((at, cash)) = &s.cash {
        b.as_of = Some(*at);
        b.cash = cash.clone();
    }
    // the holdings as the broker states them, units, worth and book value: the
    // newest positions read (today's, read with the balances)
    if let Some((day, units)) = &s.units {
        b.held_as_of = Some(*day);
        b.held = units.clone();
        b.held_value = s.unit_values.clone();
        b.held_book = s.unit_books.clone();
    }
    // each past day's holdings and cash, as the positions read as of it states them
    b.days_held = s
        .days_held
        .iter()
        .map(|(d, h)| (*d, bagholder_engine::input::HeldDay { held: h.units.clone(), book: h.books.iter().filter_map(|(i, m)| m.map(|m| (*i, m))).collect(), cash: h.cash.clone() }))
        .collect();
    b.buying_power = s.buying_power.as_ref().map(|(_, p)| p.clone());
    b.margin_rates = s.margin_rates.as_ref().map(|(_, r)| r.clone()).unwrap_or_default();
    Some(b)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use bagholder_book::statements::UnitsLine;
    use bagholder_core::account::{AccountRef, AccountStatus};
    use bagholder_core::instrument::{InstrumentKind, RefScheme, Reference};
    use bagholder_core::{Broker, Dec, Money};

    /// A book holding the recorded withdrawals that are a bill payment or a
    /// pre-authorized debit, stored as a pull stores a row (`{"activity": row}`).
    /// The recorded replies name each payee by stand-in, so the debts here are
    /// made to those names.
    pub(crate) fn with_payments() -> (tempfile::TempDir, Book, bagholder_core::ConnectionId) {
        use bagholder_broker::BrokerAdapter;
        let dir = tempfile::tempdir().unwrap();
        let now: bagholder_core::jiff::Timestamp = "2025-11-19T20:00:00Z".parse().unwrap();
        let (book, _) = Book::open_in(dir.path(), "test", now).unwrap();
        let ws = Broker::named("wealthsimple");
        let conn = book.add_connection(&ws, "Wealthsimple", now).unwrap();
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../wealthsimple/tests/replies/wealthsimple/edited-activity-one-per-kind.json");
        let reply = bagholder_core::json::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
        let edges = bagholder_sources::reply::Node::root(&reply).obj("data").and_then(|d| d.obj("activityFeedItems")).and_then(|f| f.list("edges")).unwrap();
        let adapter = bagholder_wealthsimple::adapter::Wealthsimple::new(bagholder_wealthsimple::replay::Replay::read(&std::env::temp_dir()).unwrap());
        let mut stored = 0;
        for e in edges {
            let row = e.obj("node").unwrap();
            if row.text("type").ok() != Some("WITHDRAWAL") || !matches!(row.text("subType").ok(), Some("BILL_PAY" | "AFT")) {
                continue;
            }
            let account = row.text("accountId").unwrap();
            if book.account_by_ref(&AccountRef::new(ws.clone(), account)).unwrap().is_none() {
                book.add_account(conn, &[AccountRef::new(ws.clone(), account)], &bagholder_wealthsimple::account_types::account_type("CASH"), AccountStatus::Open, None, now).unwrap();
            }
            let payload = bagholder_core::json::Value::Object(std::collections::BTreeMap::from([("activity".to_string(), row.value().clone())])).canonical();
            let key = row.text("canonicalId").unwrap();
            book.store(adapter.mapping(), &bagholder_book::records::Incoming { connection: Some(conn), source_key: key, payload: &payload, refs: vec![] }, now).unwrap();
            stored += 1;
        }
        assert_eq!(stored, 4, "three bill payments, two to one payee, and a loan debit in the recorded replies");
        (dir, book, conn)
    }

    fn debt(id: &str, kind: &str, institution: &str) -> bagholder_book::debts::OutsideDebt {
        bagholder_book::debts::OutsideDebt { entity_id: id.into(), kind: kind.into(), name: id.into(), institution: institution.into(), last4: None, balance: Dec::parse("1000").unwrap(), currency: Currency::CAD, as_of: None }
    }

    /// Each withdrawal is matched to the outside debt it paid: the loan debit to
    /// the one debt at its lender, the bill payments to the card at their payee;
    /// two debts at one payee wait for the person's answer, which then settles
    /// every payment to that payee (`docs/plans/debts-held-outside-wealthsimple.md`).
    #[test]
    fn each_payment_is_matched_to_the_outside_debt_it_paid() {
        let (_d, book, conn) = with_payments();
        assert!(debts(&book).unwrap().repayments.is_empty(), "no outside debt read: nothing repaid");
        let at: bagholder_core::jiff::Timestamp = "2025-11-20T00:00:00Z".parse().unwrap();
        book.keep_outside_debts(conn, at, &[debt("loan", "PersonalLoan", "anon-personal-229"), debt("card", "CreditCard", "anon-personal-117"), debt("chequing", "Chequing", "anon-personal-117")]).unwrap();
        let d = debts(&book).unwrap();
        let by: std::collections::BTreeMap<&str, usize> = d.repayments.values().fold(Default::default(), |mut m, v| {
            *m.entry(v.as_str()).or_default() += 1;
            m
        });
        // the payee's second bill payment expired: it moved no money and repaid nothing
        assert_eq!(by, std::collections::BTreeMap::from([("card", 1), ("loan", 1)]), "a chequing account is no debt, so the card is the payee's one");
        assert_eq!(d.owed.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["card", "loan"], "only debts are owed");
        // a second debt at the card's payee: asked, until answered
        let later: bagholder_core::jiff::Timestamp = "2025-11-21T00:00:00Z".parse().unwrap();
        book.keep_outside_debts(conn, later, &[debt("loan", "PersonalLoan", "anon-personal-229"), debt("card", "CreditCard", "anon-personal-117"), debt("loc", "LineOfCredit", "anon-personal-117")]).unwrap();
        let waiting = debts(&book).unwrap();
        assert_eq!(waiting.repayments.len(), 1, "the bill payment waits for an answer");
        assert_eq!(waiting.asked.iter().map(|a| (a.payee.as_str(), a.payments, a.candidates.clone())).collect::<Vec<_>>(), [("anon-personal-117", 1, vec!["card".to_string(), "loc".to_string()])], "asked once, with the debts it may be");
        let key = bagholder_wealthsimple::repayments::Payment::BillPay { company: "anon-personal-117".into(), account: Some("anon-personal-119".into()) }.payee_key();
        book.answer_repayment(&key, Some("loc"), later).unwrap();
        assert_eq!(debts(&book).unwrap().repayments.values().filter(|v| v.as_str() == "loc").count(), 1, "the answer settles it");
    }

    /// A read stating what the newest one stated keeps nothing and leaves the
    /// fingerprint as it was, so a pull that finds the debts as they were costs
    /// no reading of the withdrawals; a debt that moved, or an answer, moves it.
    #[test]
    fn the_debts_fingerprint_moves_only_with_what_moves_them() {
        let (_d, book, conn) = with_payments();
        let at = |s: &str| -> bagholder_core::jiff::Timestamp { s.parse().unwrap() };
        book.keep_outside_debts(conn, at("2025-11-20T00:00:00Z"), &[debt("loan", "PersonalLoan", "anon-personal-229")]).unwrap();
        let first = book.debts_print().unwrap();
        book.keep_outside_debts(conn, at("2025-11-21T00:00:00Z"), &[debt("loan", "PersonalLoan", "anon-personal-229")]).unwrap();
        assert_eq!(book.debts_print().unwrap(), first, "the same debts read again: nothing kept");
        let mut moved = debt("loan", "PersonalLoan", "anon-personal-229");
        moved.balance = Dec::parse("900").unwrap();
        book.keep_outside_debts(conn, at("2025-11-22T00:00:00Z"), &[moved]).unwrap();
        let second = book.debts_print().unwrap();
        assert_ne!(second, first, "a balance moved");
        book.answer_repayment("debit:x", None, at("2025-11-23T00:00:00Z")).unwrap();
        assert_ne!(book.debts_print().unwrap(), second, "an answer given");
    }

    /// The holdings are the newest positions read's: its units, and the worth and
    /// book value it states beside them where it states them (brief 79 §2.1).
    #[test]
    fn the_holdings_are_the_newest_positions_read_s_with_its_worth_and_book_value() {
        let dir = tempfile::tempdir().unwrap();
        let now: bagholder_core::jiff::Timestamp = "2026-09-27T12:00:00Z".parse().unwrap();
        let (book, _) = Book::open_in(dir.path(), "test", now).unwrap();
        let ws = Broker::named("wealthsimple");
        let conn = book.add_connection(&ws, "Wealthsimple", now).unwrap();
        let a = book.add_account(conn, &[AccountRef::new(ws.clone(), "crypto-1")], &bagholder_wealthsimple::account_types::account_type("CRYPTO"), AccountStatus::Open, None, now).unwrap();
        let draft = bagholder_book::mapping::InstrumentDraft { refs: vec![Reference::new(RefScheme::BrokerSecurity(ws), "sec-z-x")], kind: InstrumentKind::Crypto, currency: Currency::CAD, name: None, option: None, standing: None };
        let i = book.instrument_stated(&draft, &bagholder_core::SourceName::named("wealthsimple"), now).unwrap().unwrap();
        let read = book.broker_read(conn, "units", now).unwrap();
        let q = Dec::parse("0.0000005").unwrap();
        book.store_units(a, "2026-09-26".parse().unwrap(), &[UnitsLine { instrument: i, quantity: q, book_value: None, value: None }], &Default::default(), &read, now).unwrap();
        let b = &brokers(&book).unwrap()[&a];
        assert_eq!(b.held[&i], q, "kept before its worth was, its units stand");
        assert!(b.held_value.is_empty() && b.held_book.is_empty());
        let (worth, cost) = (Money::new(Dec::parse("0.0000014").unwrap(), Currency::CAD), Money::new(Dec::parse("0.0000011").unwrap(), Currency::CAD));
        book.store_units(a, "2026-09-27".parse().unwrap(), &[UnitsLine { instrument: i, quantity: q, book_value: Some(cost), value: Some(worth) }], &Default::default(), &read, now).unwrap();
        let b = &brokers(&book).unwrap()[&a];
        assert_eq!((b.held[&i], b.held_value[&i], b.held_book[&i]), (q, worth, cost));
    }
}
