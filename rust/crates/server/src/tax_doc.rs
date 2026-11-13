//! The `tax` document (brief 82, PR 4a): each ETF the person holds outside
//! registered plans, its tax-cost ledger (`bagholder_engine::acb`) on the inputs the
//! book and the market cache hold (`tax_inputs`), which years are complete, what
//! keeps the others waiting, and each T5008 sale against the ledger's own. Every
//! reason is a code the page words. It is worked out only while a page or an agent
//! reads it; nothing of it is kept.
//!
//! No slip's text reaches it: only what the readers read from them (`SPEC.md`
//! tax-slips-5).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::Serialize;
use ts_rs::TS;

use bagholder_core::{AccountId, Dec, InstrumentId, TransactionId};
use bagholder_engine::acb::{self, Event, GainKind, Holding, Listed, Missing, Source};
use bagholder_wealthsimple::documents::SlipKind;
use bagholder_wealthsimple::slips;

use crate::app::App;
use crate::http::OkOr;
use crate::tax_inputs::{Note, Sources};
use crate::wire::Dec as WDec;

/// Why a year is not complete, as a code and what it names.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TaxWait {
    /// `slip`, `record-date`, `rate`, `slip-differs`, `moved-in-cost`, `moved-out`,
    /// `period-open`, `account-plan`, `corporate-event`, `units`, `earlier-year`,
    /// `calendar` or `arithmetic` (`acb::Missing`).
    pub code: String,
    pub day: Option<String>,
    pub transaction: Option<String>,
    pub account: Option<String>,
    pub currency: Option<String>,
    pub year: Option<i16>,
    /// The slip less the figures, for `slip-differs`.
    pub amount: Option<WDec>,
}

/// A gain the year reports: a sale, or the cost going below zero.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TaxGain {
    /// `sale` or `below-zero`.
    pub kind: String,
    pub day: String,
    pub transaction: Option<String>,
    pub units: WDec,
    pub proceeds: WDec,
    pub cost: WDec,
    pub outlays: WDec,
    pub gain: WDec,
    pub denied: WDec,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TaxYear {
    pub year: i16,
    pub complete: bool,
    pub waiting: Vec<TaxWait>,
    pub gains: Vec<TaxGain>,
}

/// One row of a holding's ledger.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TaxRow {
    pub day: String,
    /// `bought`, `sold`, `moved-in`, `return-of-capital`, `non-cash`, `below-zero`,
    /// `loss-added` or `split`.
    pub event: String,
    pub units: WDec,
    pub cad: WDec,
    pub units_after: WDec,
    pub cost_after: WDec,
    pub rate: Option<WDec>,
    /// `book`, `entered`, `fund`, `breakdown` or `slip`.
    pub source: String,
    /// The row's transaction, for `book` and `entered`; whose figure, for `fund`.
    pub source_detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TaxHolding {
    /// The book's id for the fund.
    pub fund: String,
    pub symbol: String,
    /// Its venue's market identifier code, where known.
    pub venue: Option<String>,
    pub name: Option<String>,
    /// `canada` or `united-states`.
    pub listed: String,
    pub units: WDec,
    pub cost: WDec,
    /// The cost per unit, to the fourth place, where units are held.
    pub per_unit: Option<WDec>,
    pub complete_through: Option<i16>,
    pub years: Vec<TaxYear>,
    pub rows: Vec<TaxRow>,
}

/// What a slip states that is not taken in (`tax_inputs::Note`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TaxNote {
    /// `slip-without-text`, `slip-refused`, `fund-unmatched`, `fund-ambiguous`,
    /// `combines` or `venue-unknown`.
    pub code: String,
    pub account: Option<String>,
    pub year: Option<i16>,
    /// The slip's kind (`t3`, `t5`, `t5008`).
    pub kind: Option<String>,
    /// The fund as the slip names it.
    pub name: Option<String>,
    /// How many funds a T3 combines.
    pub funds: Option<u32>,
    /// The book's id for the fund, for `venue-unknown`.
    pub fund: Option<String>,
}

/// One sale a T5008 states, against the ledger's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TaxSale {
    pub year: i16,
    pub account: String,
    /// The security as the slip names it.
    pub security: String,
    pub day: String,
    pub units: WDec,
    /// Box 21.
    pub proceeds: WDec,
    /// Box 20, where the slip states one.
    pub slip_cost: Option<WDec>,
    /// What box 20 must say: box 21 less the ledger's gain; none unless matched.
    pub enter: Option<WDec>,
    /// `right` (the slip's box 20 is what it must say), `change`, `unmatched` (no
    /// sale of the ledger's is it) or `ambiguous` (more than one could be).
    pub state: String,
    /// The fund of the ledger's sale it is.
    pub fund: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TaxDoc {
    pub holdings: Vec<TaxHolding>,
    /// The years complete for every holding that has them, oldest first.
    pub ready: Vec<i16>,
    pub notes: Vec<TaxNote>,
    pub sales: Vec<TaxSale>,
}

impl bagholder_diff::Diff for TaxDoc {
    fn diff(&self, new: &Self, path: &mut Vec<serde_json::Value>, ops: &mut Vec<serde_json::Value>) {
        bagholder_diff::as_json(self, new, path, ops)
    }
}

fn w(d: Dec) -> WDec {
    WDec(d)
}

fn wait(m: &Missing) -> TaxWait {
    let code = |c: &str| TaxWait { code: c.into(), ..TaxWait::default() };
    match m {
        Missing::Slip => code("slip"),
        Missing::RecordDate(t) => TaxWait { transaction: Some(t.to_string()), ..code("record-date") },
        Missing::Rate { currency, day } => TaxWait { currency: Some(currency.as_str().to_string()), day: Some(day.to_string()), ..code("rate") },
        Missing::SlipDiffers(d) => TaxWait { amount: Some(w(*d)), ..code("slip-differs") },
        Missing::MovedInCost(t) => TaxWait { transaction: Some(t.to_string()), ..code("moved-in-cost") },
        Missing::MovedOut(t) => TaxWait { transaction: Some(t.to_string()), ..code("moved-out") },
        Missing::PeriodOpen(d) => TaxWait { day: Some(d.to_string()), ..code("period-open") },
        Missing::AccountPlan(a) => TaxWait { account: Some(a.to_string()), ..code("account-plan") },
        Missing::CorporateEvent(t) => TaxWait { transaction: Some(t.to_string()), ..code("corporate-event") },
        Missing::Units(t) => TaxWait { transaction: Some(t.to_string()), ..code("units") },
        Missing::EarlierYear(y) => TaxWait { year: Some(*y), ..code("earlier-year") },
        Missing::Calendar(y) => TaxWait { year: Some(*y), ..code("calendar") },
        Missing::Arithmetic => code("arithmetic"),
    }
}

fn event(e: &Event) -> &'static str {
    match e {
        Event::Bought => "bought",
        Event::Sold => "sold",
        Event::MovedIn => "moved-in",
        Event::ReturnOfCapital => "return-of-capital",
        Event::NonCash => "non-cash",
        Event::BelowZero => "below-zero",
        Event::LossAdded => "loss-added",
        Event::Split => "split",
    }
}

fn note(n: &Note) -> TaxNote {
    match n {
        Note::SlipNotRead { account, kind, year, why } => TaxNote {
            // the reader's own words stay in the log: the page is told only which
            code: if why == crate::tax_inputs::NOT_KEPT { "slip-without-text" } else { "slip-refused" }.into(),
            account: Some(account.to_string()),
            year: Some(*year),
            kind: Some(kind.name().into()),
            ..TaxNote::default()
        },
        Note::FundUnmatched { account, year, name } => TaxNote { code: "fund-unmatched".into(), account: Some(account.to_string()), year: Some(*year), name: Some(name.clone()), ..TaxNote::default() },
        Note::FundAmbiguous { account, year, name } => TaxNote { code: "fund-ambiguous".into(), account: Some(account.to_string()), year: Some(*year), name: Some(name.clone()), ..TaxNote::default() },
        Note::Combines { account, year, funds } => TaxNote { code: "combines".into(), account: Some(account.to_string()), year: Some(*year), funds: Some(u32::try_from(*funds).unwrap_or(u32::MAX)), ..TaxNote::default() },
        Note::VenueUnknown { instrument } => TaxNote { code: "venue-unknown".into(), fund: Some(instrument.to_string()), ..TaxNote::default() },
    }
}

fn holding(h: &Holding, inputs: &bagholder_engine::input::Inputs) -> TaxHolding {
    let name = inputs.ledger.instruments.get(&h.fund).and_then(|i| i.current_name());
    TaxHolding {
        fund: h.fund.to_string(),
        symbol: name.map(|n| n.symbol.clone()).unwrap_or_default(),
        venue: name.and_then(|n| n.venue_mic.clone()),
        name: name.and_then(|n| n.name.clone()),
        listed: match h.listed {
            Listed::Canada => "canada",
            Listed::UnitedStates => "united-states",
        }
        .into(),
        units: w(h.units),
        cost: w(h.cost),
        per_unit: if h.units.is_positive() { h.cost.div_rounded(h.units, 4, bagholder_core::Rounding::HalfEven).ok().map(w) } else { None },
        complete_through: h.complete_through(),
        years: h
            .years
            .iter()
            .map(|(y, v)| TaxYear {
                year: *y,
                complete: v.complete(),
                waiting: v.missing.iter().map(wait).collect(),
                gains: v
                    .gains
                    .iter()
                    .map(|g| TaxGain {
                        kind: match g.kind {
                            GainKind::Sale => "sale",
                            GainKind::BelowZero => "below-zero",
                        }
                        .into(),
                        day: g.day.to_string(),
                        transaction: g.transaction.as_ref().map(ToString::to_string),
                        units: w(g.units),
                        proceeds: w(g.proceeds),
                        cost: w(g.cost),
                        outlays: w(g.outlays),
                        gain: w(g.gain),
                        denied: w(g.denied),
                    })
                    .collect(),
            })
            .collect(),
        rows: h
            .rows
            .iter()
            .map(|r| {
                let (source, detail) = match &r.source {
                    Source::Book(t) => ("book", Some(t.to_string())),
                    Source::Entered(t) => ("entered", Some(t.to_string())),
                    Source::Fund(s) => ("fund", Some(s.clone())),
                    Source::Breakdown => ("breakdown", None),
                    Source::Slip => ("slip", None),
                };
                TaxRow {
                    day: r.day.to_string(),
                    event: event(&r.event).into(),
                    units: w(r.units),
                    cad: w(r.cad),
                    units_after: w(r.units_after),
                    cost_after: w(r.cost_after),
                    rate: r.rate.map(w),
                    source: source.into(),
                    source_detail: detail,
                }
            })
            .collect(),
    }
}

/// The longest a T5008's date can stand before the ledger's settlement of the
/// sale: the slip dates a sale by its trade or its settlement, and settlement has
/// been at most three business days after the trade (T+3 before 2017), a weekend
/// between them at most once.
const SETTLED_WITHIN_DAYS: i64 = 5;

/// Each T5008 sale of the holding's accounts against the ledger's sales: one sale
/// of the same account, units and proceeds to the cent, settled on the slip's day
/// or within the settlement window after it, is it; where several could be, the
/// slip's name for the security breaks the tie, never makes a match.
fn sales(s: &Sources, holdings: &[Holding], inputs: &bagholder_engine::input::Inputs) -> Vec<TaxSale> {
    let account_of: BTreeMap<&TransactionId, AccountId> = inputs.ledger.transactions.iter().map(|t| (&t.id, t.account)).collect();
    let folded = |x: &str| -> String { x.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect() };
    let mut out = Vec::new();
    for r in s.slips.iter().filter(|r| r.kind == SlipKind::T5008 && s.alone.contains(&r.account)) {
        let Ok(slips::Slip::T5008(t)) = &r.read else { continue };
        for sale in &t.sales {
            let proceeds = sale.proceeds.amount;
            let mut found: Vec<(InstrumentId, &acb::Gain)> = Vec::new();
            for h in holdings {
                for g in h.years.values().flat_map(|y| &y.gains).filter(|g| g.kind == GainKind::Sale) {
                    let here = g.transaction.as_ref().and_then(|t| account_of.get(t)) == Some(&r.account);
                    let within = g.day.since(sale.day).map(|d| i64::from(d.get_days())).is_ok_and(|d| (0..=SETTLED_WITHIN_DAYS).contains(&d));
                    if here && within && g.units == sale.quantity && g.proceeds == proceeds {
                        found.push((h.fund, g));
                    }
                }
            }
            if found.len() > 1 {
                let named = folded(&sale.security);
                found.retain(|(f, _)| inputs.ledger.instruments.get(f).and_then(|i| i.current_name()).and_then(|n| n.name.as_deref()).is_some_and(|n| named.contains(&folded(n))));
            }
            let base = TaxSale {
                year: t.year,
                account: r.account.to_string(),
                security: sale.security.clone(),
                day: sale.day.to_string(),
                units: w(sale.quantity),
                proceeds: w(proceeds),
                slip_cost: sale.cost.as_ref().map(|c| w(c.amount)),
                enter: None,
                state: "unmatched".into(),
                fund: None,
            };
            out.push(match found.as_slice() {
                [(fund, g)] => {
                    let enter = proceeds.checked_sub(g.gain).ok();
                    let right = enter.is_some() && sale.cost.as_ref().map(|c| c.amount) == enter;
                    TaxSale { enter: enter.map(w), state: if right { "right" } else { "change" }.into(), fund: Some(fund.to_string()), ..base }
                }
                [] => base,
                _ => TaxSale { state: "ambiguous".into(), ..base },
            });
        }
    }
    out
}

/// The years complete for every holding whose ledger has them.
fn ready(holdings: &[Holding]) -> Vec<i16> {
    let years: BTreeSet<i16> = holdings.iter().flat_map(|h| h.years.keys().copied()).collect();
    years.into_iter().filter(|y| holdings.iter().filter(|h| h.years.contains_key(y)).all(|h| h.complete_through().is_some_and(|c| c >= *y))).collect()
}

/// The document from what the inputs are made of.
pub fn doc_of(inputs: &bagholder_engine::input::Inputs, s: &Sources) -> Result<TaxDoc, String> {
    let (tax, notes) = crate::tax_inputs::build(s).map_err(|e| e.to_string())?;
    let holdings = acb::holdings(inputs, &tax);
    Ok(TaxDoc { ready: ready(&holdings), notes: notes.iter().map(note).collect(), sales: sales(s, &holdings, inputs), holdings: holdings.iter().map(|h| holding(h, inputs)).collect() })
}

/// The `tax` document as the book, the figures and the market cache stand now.
pub fn tax_doc(app: &Arc<App>) -> Result<TaxDoc, String> {
    let f = app.figures.get().ok_or("the figures are not open")?;
    let book = f.book()?;
    let market = crate::feeds::conn(app)?;
    let zone = bagholder_core::jiff::tz::TimeZone::get(bagholder_wealthsimple::mapping::ZONE).map_err(|e| e.to_string())?;
    f.read(|e| {
        let inputs = e.inputs();
        let s = crate::tax_inputs::gather(inputs, &book, &market, &zone)?;
        doc_of(inputs, &s)
    })
    .ok_or("the figures are not built yet")?
}

/// `tax`, the document a page watches, and `GET /api/tax`, which agents read.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(untagged)]
pub enum TaxAnswer {
    Ok {
        #[ts(type = "true")]
        ok: bool,
        tax: TaxDoc,
    },
    Refused(OkOr),
}

impl bagholder_diff::Diff for TaxAnswer {
    fn diff(&self, new: &Self, path: &mut Vec<serde_json::Value>, ops: &mut Vec<serde_json::Value>) {
        bagholder_diff::as_json(self, new, path, ops)
    }
}

/// The `tax` document as it stands, or why it cannot be worked out now.
pub fn answer(app: &Arc<App>) -> TaxAnswer {
    match tax_doc(app) {
        Ok(tax) => TaxAnswer::Ok { ok: true, tax },
        Err(e) => TaxAnswer::Refused(OkOr::err(e)),
    }
}

#[cfg(test)]
#[path = "../../engine/tests/common/mod.rs"]
#[allow(dead_code, unused_imports, clippy::all)]
mod case_common;

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use bagholder_core::jiff::civil::date;
    use bagholder_core::{Currency, Dec, Money};
    use bagholder_wealthsimple::documents::SlipKind;
    use bagholder_wealthsimple::slips;

    use crate::tax_inputs::{of_inputs, SlipRead};

    fn dec(s: &str) -> Dec {
        Dec::parse(s).unwrap()
    }
    fn cad(s: &str) -> Money {
        Money::new(dec(s), Currency::CAD)
    }

    /// The document is the ledger on the slips' inputs, worked by hand: a made-up
    /// fund bought, paid twice and part sold in 2024, its T3's breakdown placing
    /// each record date's return of capital; the year complete and ready; the
    /// T5008's sale of it found as the ledger's and told what box 20 must say, a
    /// sale of something else left unmatched; and no slip's text anywhere in it.
    #[test]
    fn the_tax_document_is_the_ledger_on_the_slips_inputs() {
        let paid = |id: &str, day: &str| serde_json::json!({"id": id, "kind": "dividend", "account": "cash", "instrument": "etf", "day": day, "cash": "100.00"});
        let case = serde_json::json!({
            "today": "2025-04-15",
            "accounts": [{"id": "cash"}],
            "instruments": [{"id": "etf", "symbol": "EEE", "mic": "XTSE", "name": "Made-up Covered Call ETF", "fund": true}],
            "declared": {"etf": [
                {"ex": "2024-03-20", "record": "2024-03-20", "pay": "2024-03-28", "amount": "0.10"},
                {"ex": "2024-06-20", "record": "2024-06-20", "pay": "2024-06-27", "amount": "0.10"}
            ]},
            "transactions": [
                {"id": "b", "account": "cash", "day": "2024-01-08", "kind": "buy", "instrument": "etf", "qty": "1000", "cash": "-10000.00"},
                paid("d1", "2024-03-28"), paid("d2", "2024-06-27"),
                {"id": "s", "account": "cash", "day": "2024-09-09", "kind": "sell", "instrument": "etf", "qty": "-400", "cash": "4090.01", "fee": "9.99"}
            ]
        });
        let mut built = super::case_common::build(&case);
        let (account, etf) = (built.ids.account("cash"), built.ids.instrument("etf"));
        let mut s = of_inputs(&built.inputs);
        assert_eq!((s.alone.clone(), s.funds.len(), s.paid.len()), (BTreeSet::from([account]), 1, 2));
        let row = |record: (i8, i8), paid: (i8, i8)| slips::Distribution {
            record: date(2024, record.0, record.1),
            paid: date(2024, paid.0, paid.1),
            boxes: std::collections::BTreeMap::from([(42, dec("30.0000")), (23, dec("70.0000"))]),
            cash: dec("100.0000"),
            non_cash: Dec::ZERO,
        };
        let t3 = slips::T3 { year: 2024, boxes: std::collections::BTreeMap::new(), funds: vec![slips::TrustFund { name: "MADE-UP COVERED CALL ETF".into(), distributions: vec![row((3, 20), (3, 28)), row((6, 20), (6, 27))] }] };
        let sale = |day: (i8, i8), units: &str, security: &str, cost: &str, proceeds: &str| slips::Sale { day: date(2024, day.0, day.1), type_code: "SEL".into(), quantity: dec(units), security: security.into(), cost: Some(cad(cost)), proceeds: cad(proceeds) };
        // the fund's sale; a sale of something else; and one like the fund's, dated a month
        // before the ledger's settled, which is not it
        let t5008 = slips::T5008 {
            year: 2024,
            sales: vec![sale((9, 9), "400", "Made-up Capital Inc. - Made-up Covered Call ETF", "4000.00", "4100.00"), sale((10, 1), "5", "Some Made-up Stock", "50.00", "60.00"), sale((8, 9), "400", "Made-up Capital Inc. - Made-up Covered Call ETF", "4000.00", "4100.00")],
        };
        s.slips = vec![SlipRead { account, kind: SlipKind::T3, year: 2024, read: Ok(slips::Slip::T3(t3)) }, SlipRead { account, kind: SlipKind::T5008, year: 2024, read: Ok(slips::Slip::T5008(t5008)) }];
        let doc = super::doc_of(&built.inputs, &s).unwrap();
        // 10,000.00 - 30.00 - 30.00 = 9,940.00; the 400 sold cost 9,940.00 x 400/1,000 = 3,976.00,
        // proceeds 4,090.01 + 9.99 commission = 4,100.00, outlays 9.99: a gain of
        // 4,100.00 - 3,976.00 - 9.99 = 114.01; 600 units left at 5,964.00
        let h = &doc.holdings[0];
        assert_eq!((h.symbol.as_str(), h.units.0, h.cost.0, h.complete_through), ("EEE", dec("600"), dec("5964.00"), Some(2024)));
        assert_eq!(h.rows.iter().map(|r| (r.event.as_str(), r.cad.0, r.source.as_str())).collect::<Vec<_>>(), vec![("bought", dec("10000.00"), "book"), ("return-of-capital", dec("-30.00"), "breakdown"), ("return-of-capital", dec("-30.00"), "breakdown"), ("sold", dec("-3976.00"), "book")]);
        assert_eq!(h.years[0].gains[0].gain.0, dec("114.01"));
        assert_eq!(doc.ready, vec![2024]);
        assert!(doc.notes.is_empty(), "{:?}", doc.notes);
        // box 20 must say box 21 less the ledger's gain: 4,100.00 - 114.01 = 3,985.99
        assert_eq!(doc.sales.iter().map(|x| (x.state.as_str(), x.enter.as_ref().map(|e| e.0))).collect::<Vec<_>>(), vec![("change", Some(dec("3985.99"))), ("unmatched", None), ("unmatched", None)]);
        assert_eq!(doc.sales[0].fund.as_deref(), Some(etf.to_string().as_str()));
        let text = serde_json::to_string(&doc).unwrap();
        assert!(!text.contains("Recipient") && !text.contains("Statement of"), "no slip's text");
    }
}
