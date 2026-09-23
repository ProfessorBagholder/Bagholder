//! The engine's inputs read from the book (`docs/plans/stage-2-engine.md`, "The
//! entry point"): its ledger and the facts it keeps. The market's part comes from
//! wherever the market data lives (the comparison tool reads the old store for
//! it; stage 3's market cache replaces that).

use std::collections::BTreeMap;

use bagholder_book::facts::DistributionKind as BookKind;
use bagholder_book::Book;
use bagholder_core::names::SourceName;
use bagholder_engine::input::{Read, AccountInfo, Declared, DeclaredRead, DistributionKind, Facts, InstrumentInfo, Ledger, Rates, RecordInfo, Sourced};

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The ledger: accounts, instruments with their names and terms, the live
/// transactions and their records, the trades, groups and journal.
pub fn ledger(book: &Book) -> Result<Ledger, String> {
    let brokers: BTreeMap<_, _> = book.connections().map_err(err)?.into_iter().map(|c| (c.id, c.broker)).collect();
    let mut accounts = BTreeMap::new();
    for a in book.accounts().map_err(err)? {
        let broker = brokers.get(&a.connection).cloned().ok_or_else(|| format!("account {} has no connection", a.id))?;
        accounts.insert(a.id, AccountInfo { account: a, broker });
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
        // the linked transfers' writer is stage 3's broker adapter
        transfer_links: vec![],
        trades: book.trades().map_err(err)?,
        groups: book.groups().map_err(err)?,
        journal: book.journal_entries().map_err(err)?.into_iter().collect(),
    })
}

/// The facts the book keeps.
pub fn facts(book: &Book) -> Result<Facts, String> {
    let rates = Rates { by_currency: book.rates().map_err(err)?, published: book.rate_series().map_err(err)?, holidays: book.bank_holidays().map_err(err)?, covered: book.rate_reads().map_err(err)?.into_iter().map(|(c, reads)| (c, reads.into_iter().map(|(first, last, at)| Read { first, last, at }).collect())).collect() };
    let declared = book
        .declared()
        .map_err(err)?
        .into_iter()
        .map(|(i, r)| {
            let items = r
                .items
                .into_iter()
                .map(|d| Declared {
                    ex_date: d.ex_date,
                    record_date: d.record_date,
                    pay_date: d.pay_date,
                    amount: d.amount,
                    kind: match d.kind {
                        BookKind::Regular => DistributionKind::Regular,
                        BookKind::Special => DistributionKind::Special,
                        BookKind::NonCash => DistributionKind::NonCash,
                    },
                })
                .collect();
            (i, DeclaredRead { read_at: r.read_at, source: r.source, items })
        })
        .collect();
    let frequencies = book.frequencies().map_err(err)?.into_iter().map(|(i, f)| (i, Sourced { value: f.per_year, source: f.source })).collect();
    let adjustments = book.adjustments().map_err(err)?.into_iter().map(|a| (a.applies_to.clone(), a)).collect();
    Ok(Facts { rates, declared, frequencies, adjustments })
}

/// The source name a stand-in fact read from the old store carries.
pub fn old_store_source() -> SourceName {
    SourceName::named("bagholder-old-store")
}
