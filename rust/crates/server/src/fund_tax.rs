//! Each security's tax form for a year, read from CDS for this copy alone (brief
//! 82, PR 2; `bagholder_market::cds_tax`): after a day's pull, on its own work, for
//! each Canadian listing its broker states is a fund (an ETF) that paid the person
//! a distribution in a non-registered account of theirs alone, each tax year it
//! did, when `cds_tax::due` says so.
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
        // a fund as its broker states it (an ETF): the only listings whose form is read
        let Some(info) = inputs.ledger.instruments.get(&i).filter(|info| info.instrument.fund == Some(true)) else { continue };
        let Some(n) = info.current_name() else { continue };
        let canadian = n.venue_mic.as_deref().and_then(|m| bagholder_core::sessions::venue_of(m, false)).is_some_and(|v| matches!(v, bagholder_core::sessions::Venue::Tsx | bagholder_core::sessions::Venue::Cse | bagholder_core::sessions::Venue::CboeCanada));
        if !canadian {
            continue;
        }
        // kept under the listing's own key, as every market-cache table is
        let Ok(key) = bagholder_core::ListingKey::on(&n.symbol, n.venue_mic.as_deref().unwrap_or("")) else { continue };
        let listing = key.to_string();
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

// the engine's own case reader, to build the inputs in the test below
#[cfg(test)]
#[path = "../../engine/tests/common/mod.rs"]
#[allow(dead_code, unused_imports, clippy::all)]
mod case_common;

#[cfg(test)]
mod tests {
    /// The forms looked for (brief 82): a Canadian listing's, for each year it paid
    /// a distribution into a non-registered account the person holds alone, where
    /// its broker states it is a fund. A listing stated not to be one, or of which
    /// no source says, is never looked for, whatever its name.
    #[test]
    fn a_form_is_looked_for_only_where_the_broker_states_a_fund() {
        let paid = |id: &str, account: &str, instrument: &str, day: &str| serde_json::json!({"id": id, "kind": "dividend", "account": account, "instrument": instrument, "day": day, "cash": "12.30"});
        let case = serde_json::json!({
            "today": "2026-02-02",
            "accounts": [{"id": "cash"}, {"id": "tfsa", "registration": "tfsa"}, {"id": "joint", "joint": true}],
            "instruments": [
                {"id": "etf", "symbol": "EEE", "mic": "XTSE", "name": "Made-up Index ETF", "fund": true},
                {"id": "stock", "symbol": "SSS", "mic": "XTSE", "name": "Made-up Fund Corp", "fund": false},
                {"id": "unsaid", "symbol": "UUU.UN", "mic": "XTSE", "name": "Made-up Income Fund"},
                {"id": "us", "symbol": "FFF", "mic": "ARCX", "currency": "USD", "fund": true}
            ],
            "transactions": [
                paid("e1", "cash", "etf", "2024-03-28"), paid("e2", "cash", "etf", "2025-03-27"), paid("e3", "tfsa", "etf", "2023-03-30"), paid("e4", "joint", "etf", "2022-03-31"),
                paid("s1", "cash", "stock", "2025-03-01"), paid("u1", "cash", "unsaid", "2025-03-01"), paid("f1", "cash", "us", "2025-03-01")
            ]
        });
        let built = super::case_common::build(&case);
        let got: Vec<(String, Vec<i16>)> = super::wanted(&built.inputs).into_iter().map(|w| (w.symbol, w.years.into_iter().collect())).collect();
        assert_eq!(got, vec![("EEE".to_string(), vec![2024, 2025])]);
    }
}
