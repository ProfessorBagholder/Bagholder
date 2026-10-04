//! The engine's inputs read from the book (`docs/plans/stage-2-engine.md`, "The
//! entry point"): its ledger and the facts it keeps. The market's part comes from
//! wherever the market data lives (the comparison tool reads the old store for
//! it; stage 3's market cache replaces that).

use std::collections::BTreeMap;

use bagholder_book::Book;
use bagholder_core::names::SourceName;
use bagholder_core::Currency;
use bagholder_engine::input::{Adjustments, Read, AccountInfo, Declared, DeclaredRead, Facts, InstrumentInfo, Ledger, Rates, RecordInfo, Series, Sourced};

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The smallest coin order each broker takes, as it states it; none for a
/// broker that states none.
fn coin_minimum(broker: &bagholder_core::Broker) -> Option<bagholder_core::Money> {
    (*broker == bagholder_wealthsimple::mapping::broker()).then(bagholder_wealthsimple::mapping::coin_minimum)
}

/// The ledger: accounts, instruments with their names and terms, the live
/// transactions and their records, the trades, groups and journal.
pub fn ledger(book: &Book) -> Result<Ledger, String> {
    let brokers: BTreeMap<_, _> = book.connections().map_err(err)?.into_iter().map(|c| (c.id, (c.broker, c.label))).collect();
    let mut accounts = BTreeMap::new();
    for a in book.accounts().map_err(err)? {
        let (broker, broker_label) = brokers.get(&a.connection).cloned().ok_or_else(|| format!("account {} has no connection", a.id))?;
        let coin_minimum = coin_minimum(&broker);
        accounts.insert(a.id, AccountInfo { account: a, broker, broker_label, coin_minimum });
    }
    let mut instruments = BTreeMap::new();
    for i in book.instruments().map_err(err)? {
        let names = book.names(i.id).map_err(err)?;
        let terms = book.option_terms(i.id).map_err(err)?;
        instruments.insert(i.id, InstrumentInfo { instrument: i, names, terms });
    }
    let mut records: BTreeMap<_, RecordInfo> = book.live_record_keys().map_err(err)?.into_iter().map(|(id, key)| (id, RecordInfo { source_key: key, problems: vec![] })).collect();
    for (id, p) in book.problems().map_err(err)? {
        if let Some(r) = records.get_mut(&id) {
            r.problems.push(p);
        }
    }
    Ok(Ledger {
        accounts,
        instruments,
        transactions: book.transactions().map_err(err)?,
        records,
        // the two sides of each move of holdings, as the broker's rows state them
        transfer_links: book.transfer_links().map_err(err)?,
        trades: book.trades().map_err(err)?,
        groups: book.groups().map_err(err)?,
        journal: book.journal_entries().map_err(err)?.into_iter().collect(),
    })
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
    Ok(Facts { rates, declared, frequencies, adjustments })
}

/// The source name a stand-in fact read from the old store carries.
pub fn old_store_source() -> SourceName {
    SourceName::named("bagholder-old-store")
}

/// What each broker states about each account (`docs/plans/stage-3b-wealthsimple.md`,
/// "The book's new tables"), read from the book: each day's value and net
/// deposits (in CAD), the newest cash and when it was stated, the newest units
/// and the day they are as of, and when the account's activity was last read in
/// full. An account the broker has stated nothing about has none.
pub fn brokers(book: &Book) -> Result<BTreeMap<bagholder_core::AccountId, bagholder_engine::input::BrokerAccount>, String> {
    let mut out = BTreeMap::new();
    for a in book.accounts().map_err(err)? {
        let s = book.stated(a.id).map_err(err)?;
        if s.days.is_empty() && s.cash.is_none() && s.units.is_none() && s.activity_read_at.is_none() && s.buying_power.is_none() && s.net_value_now.is_none() {
            continue;
        }
        let mut b = bagholder_engine::input::BrokerAccount::default();
        for (day, (value, deposits)) in &s.days {
            if value.currency != Currency::CAD || deposits.currency != Currency::CAD {
                return Err(format!("account {} states its value on {day} in {}, not CAD", a.id, value.currency));
            }
            b.net_value.insert(*day, value.amount);
            b.net_deposits.insert(*day, deposits.amount);
        }
        // the account's value now, as the broker states it with the accounts
        // (its daily values are a day behind, and are the equity curve's)
        b.net_value_now = match s.net_value_now {
            Some((_, v)) if v.currency == Currency::CAD => Some(v.amount),
            Some((_, v)) => return Err(format!("account {} states what it is worth in {}, not CAD", a.id, v.currency)),
            None => None,
        };
        if let Some((at, cash)) = s.cash {
            b.as_of = Some(at);
            b.cash = cash;
        }
        // a statement of units kept before the broker's worth of each was
        // (migration 021) is not compared: the next read states both
        if let Some((day, units)) = s.units.filter(|(_, u)| u.is_empty() || !s.unit_values.is_empty()) {
            b.held_as_of = Some(day);
            b.held = units;
            b.held_value = s.unit_values;
        }
        b.activity_read_at = s.activity_read_at;
        b.cash_read = s.cash_read.map(|(_, c)| c);
        b.cash_read_holds = s.cash_read_holds;
        b.buying_power = match s.buying_power {
            None => None,
            Some((_, Ok(m))) if m.currency == Currency::CAD => Some(Ok(m.amount)),
            Some((_, Ok(m))) => return Err(format!("account {} states what it can borrow in {}, not CAD", a.id, m.currency)),
            Some((_, Err(why))) => Some(Err(why)),
        };
        out.insert(a.id, b);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bagholder_book::statements::UnitsLine;
    use bagholder_core::account::{AccountRef, AccountStatus};
    use bagholder_core::instrument::{InstrumentKind, RefScheme, Reference};
    use bagholder_core::{Broker, Dec, Money};

    #[test]
    fn a_statement_of_units_kept_before_their_worth_was_is_not_compared_and_one_with_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let now: bagholder_core::jiff::Timestamp = "2026-09-27T12:00:00Z".parse().unwrap();
        let (book, _) = Book::open_in(dir.path(), "test", now).unwrap();
        let ws = Broker::named("wealthsimple");
        let conn = book.add_connection(&ws, "Wealthsimple", now).unwrap();
        let a = book.add_account(conn, &[AccountRef::new(ws.clone(), "crypto-1")], &bagholder_book::import::wealthsimple_account_type("CRYPTO"), AccountStatus::Open, None, now).unwrap();
        let draft = bagholder_book::mapping::InstrumentDraft { refs: vec![Reference::new(RefScheme::BrokerSecurity(ws), "sec-z-x")], kind: InstrumentKind::Crypto, currency: Currency::CAD, name: None, option: None, standing: None };
        let i = book.instrument_stated(&draft, &SourceName::named("wealthsimple"), now).unwrap().unwrap();
        let read = book.broker_read(conn, "units", now).unwrap();
        let q = Dec::parse("0.0000005").unwrap();
        book.store_units(a, "2026-09-26".parse().unwrap(), &[UnitsLine { instrument: i, quantity: q, book_value: None, value: None }], &read, now).unwrap();
        assert!(brokers(&book).unwrap()[&a].held.is_empty(), "kept before the worth was: not compared");
        let worth = Money::new(Dec::parse("0.0000014").unwrap(), Currency::CAD);
        book.store_units(a, "2026-09-27".parse().unwrap(), &[UnitsLine { instrument: i, quantity: q, book_value: None, value: Some(worth) }], &read, now).unwrap();
        let b = &brokers(&book).unwrap()[&a];
        assert_eq!((b.held[&i], b.held_value[&i]), (q, worth));
    }
}
