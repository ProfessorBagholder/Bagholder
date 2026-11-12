//! Each security's tax form for a year, read from CDS for this copy alone (brief
//! 82, PR 2; `bagholder_market::cds_tax`): after a day's pull, on its own work, for
//! each Canadian listing that paid the person a distribution in a non-registered
//! account of theirs alone, each tax year it did, when `cds_tax::due` says so.
//! What is read is kept in the market cache (`stored::fund_tax`), never shared.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use bagholder_core::account::{AccountType, Registration};
use bagholder_core::jiff::civil::Date;
use bagholder_core::transaction::Kind;
use bagholder_market::cds_tax::{self, Filing, FormRef};
use bagholder_market::stored::fund_tax;

use crate::app::{log, App};

/// The first tax year CDS lists.
const FIRST_YEAR: i16 = 2019;

/// A listing the person was paid by, as its form is looked for.
struct Wanted {
    listing: String,
    name: String,
    symbol: String,
    years: BTreeSet<i16>,
}

fn wanted(inputs: &bagholder_engine::input::Inputs) -> Vec<Wanted> {
    let mut out: BTreeMap<String, Wanted> = BTreeMap::new();
    for t in inputs.ledger.transactions.iter().filter(|t| t.kind == Kind::Dividend) {
        let alone = matches!(inputs.ledger.accounts.get(&t.account).map(|a| &a.account.account_type), Some(AccountType::Known { registration: Registration::Unregistered, joint: false, .. }));
        let Some(i) = t.instrument.filter(|_| alone) else { continue };
        let Some(n) = inputs.ledger.instruments.get(&i).and_then(|info| info.current_name()) else { continue };
        let canadian = n.venue_mic.as_deref().and_then(|m| bagholder_core::sessions::venue_of(m, false)).is_some_and(|v| matches!(v, bagholder_core::sessions::Venue::Tsx | bagholder_core::sessions::Venue::Cse | bagholder_core::sessions::Venue::CboeCanada));
        if !canadian {
            continue;
        }
        let listing = format!("{}:{}", n.symbol, n.venue_mic.clone().unwrap_or_default());
        let w = out.entry(listing.clone()).or_insert_with(|| Wanted { listing, name: n.name.clone().unwrap_or_else(|| n.symbol.clone()), symbol: n.symbol.clone(), years: BTreeSet::new() });
        w.years.insert(t.trade_date.year());
    }
    out.into_values().collect()
}

/// One pass: every form due read and kept. A list or a form that cannot be read is
/// logged and read again on a later pass.
pub fn refresh(app: &Arc<App>, today: Date) -> Result<(), String> {
    let f = app.figures.get().ok_or("the figures are not open")?;
    let wanted = f.read(|e| wanted(e.inputs())).ok_or("the figures are not built yet")?;
    let c = crate::feeds::conn(app)?;
    let mut lists: BTreeMap<i16, Vec<Filing>> = BTreeMap::new();
    let mut session: Option<cds_tax::Ctbs> = None;
    for w in wanted {
        let kept = fund_tax::kept(&c, &w.listing).map_err(|e| e.to_string())?;
        let cusip = fund_tax::cusip_of(&c, &w.listing).map_err(|e| e.to_string())?;
        for &y in w.years.iter().filter(|y| **y >= FIRST_YEAR) {
            let last = kept.iter().find(|k| k.year == y).and_then(|k| k.read_at.get(..10)?.parse::<Date>().ok());
            if !cds_tax::due(y, last, today) {
                continue;
            }
            if !lists.contains_key(&y) {
                let read = if y >= cds_tax::CTBS_FROM {
                    cds_tax::ctbs_open().map(|(s, l)| {
                        session = Some(s);
                        l
                    })
                } else {
                    cds_tax::year_listing(y)
                };
                match read {
                    Ok(l) => {
                        lists.insert(y, l);
                    }
                    Err(e) => {
                        log(&format!("bagholder: CDS's list of {y} tax forms could not be read: {e}"));
                        continue;
                    }
                }
            }
            let rows = &lists[&y];
            let fetch = |fl: &Filing| -> Result<Vec<u8>, String> {
                match &fl.form {
                    FormRef::Ctbs(id) => session.as_ref().ok_or_else(|| "no breakdown service session".to_string())?.form(id),
                    FormRef::Xls(href) => cds_tax::workbook(href),
                }
            };
            match cds_tax::find(rows, &w.name, &w.symbol, y, cusip.as_deref(), &fetch) {
                Ok(found) => fund_tax::save(&c, &w.listing, y, &found, &crate::app::now_iso()).map_err(|e| e.to_string())?,
                Err(e) => log(&format!("bagholder: {}'s {y} tax form could not be read: {e}", w.symbol)),
            }
        }
    }
    Ok(())
}
