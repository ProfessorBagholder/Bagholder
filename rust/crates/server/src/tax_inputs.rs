//! The tax-cost ledger's inputs (`bagholder_engine::acb::TaxInputs`) from what the
//! book and the market cache hold (brief 82, PR 4a; `SPEC.md` tax-cost-of-an-etf-5,
//! tax-slips-1 to tax-slips-5): the person's own slips, read from their kept text
//! each time; each T3 breakdown's funds matched to the person's instruments by
//! their own cash; each fund's CDS form; and which years' slips are all in.
//! Derived on every read, nothing kept: the current read wins.
//!
//! What cannot be taken in is said as a [`Note`], a code the page words: a slip not
//! read, a fund on a slip no holding matches, a T3 that combines funds it does not
//! break down. The ledger then finds that fund's year without its slip.

use std::collections::{BTreeMap, BTreeSet};

use bagholder_core::jiff::civil::Date;
use bagholder_core::{AccountId, Dec, DecError, InstrumentId, Rounding};
use bagholder_engine::acb::{Listed, PerUnit, Slip, Stated, TaxInputs};
use bagholder_wealthsimple::documents::SlipKind;
use bagholder_wealthsimple::slips;

/// The T3 boxes that are income in actual amounts: eligible and other dividends
/// (49, 23), capital gains (21), other income (26), foreign non-business and
/// business income (25, 24). Box 34 is the foreign tax paid, box 42 the return of
/// capital (CRA, T4013 "T3 slip boxes").
const INCOME_BOXES: [u16; 6] = [49, 23, 21, 26, 25, 24];
const FOREIGN_TAX_BOX: u16 = 34;
const RETURN_OF_CAPITAL_BOX: u16 = 42;

/// Why a slip kept with no text is not read: what identifies the person could not
/// be taken out of it (`SPEC.md` tax-slips-5).
pub const NOT_KEPT: &str = "kept without its text";

/// A kept slip, its account the book's, as its reader reads its text.
#[derive(Clone, Debug)]
pub struct SlipRead {
    pub account: AccountId,
    pub kind: SlipKind,
    pub year: i16,
    /// The slip, or why it is not read (kept without text, or refused).
    pub read: Result<slips::Slip, String>,
}

/// A fund the person holds, as the ledger takes it.
#[derive(Clone, Debug)]
pub struct Fund {
    pub instrument: InstrumentId,
    /// Where it lists, where its venue is one the ledger knows.
    pub listed: Option<Listed>,
    /// Its name, which breaks a tie between two funds that paid alike.
    pub name: String,
}

/// A distribution paid to the person, as the book states it.
#[derive(Clone, Debug)]
pub struct Paid {
    pub account: AccountId,
    pub instrument: InstrumentId,
    pub day: Date,
    pub cash: Dec,
}

/// What the inputs are made from.
#[derive(Clone, Debug, Default)]
pub struct Sources {
    /// The accounts of the holding: non-registered, held alone.
    pub alone: BTreeSet<AccountId>,
    pub funds: Vec<Fund>,
    pub paid: Vec<Paid>,
    /// Each kept slip in force.
    pub slips: Vec<SlipRead>,
    /// Each fund's CDS figures, per record date.
    pub forms: BTreeMap<InstrumentId, Vec<bagholder_market::cds_tax::PerUnit>>,
    /// The day the slips list was last read, in Wealthsimple's filing zone.
    pub listed_on: Option<Date>,
}

/// Why something a slip states is not taken in.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Note {
    /// A slip of the holding's kept without its text, or one its reader refuses.
    SlipNotRead { account: AccountId, kind: SlipKind, year: i16, why: String },
    /// A fund on a T3's breakdown that none of the person's instruments matches by
    /// its cash.
    FundUnmatched { account: AccountId, year: i16, name: String },
    /// A fund on a T3's breakdown that more than one instrument matches alike.
    FundAmbiguous { account: AccountId, year: i16, name: String },
    /// A T3 with no breakdown, for an account several funds paid that year.
    Combines { account: AccountId, year: i16, funds: usize },
    /// A fund whose venue is neither Canadian nor American.
    VenueUnknown { instrument: InstrumentId },
}

fn cents(d: Dec) -> Dec {
    d.round(2, Rounding::HalfUp)
}

/// A name with only its letters and digits, lower-cased: two spellings of one
/// fund's name compared.
fn folded(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn sum(boxes: &BTreeMap<u16, Dec>, which: &[u16]) -> Result<Dec, DecError> {
    which.iter().filter_map(|b| boxes.get(b)).try_fold(Dec::ZERO, |a, v| a.checked_add(*v))
}

/// The slip of one fund in one account, from the boxes it states.
fn slip_of(boxes: &BTreeMap<u16, Dec>) -> Result<Slip, DecError> {
    Ok(Slip { return_of_capital: cents(sum(boxes, &[RETURN_OF_CAPITAL_BOX])?), income: cents(sum(boxes, &INCOME_BOXES)?), foreign_tax: cents(sum(boxes, &[FOREIGN_TAX_BOX])?) })
}

/// The ledger's inputs, and what could not be taken in; an error only for a sum
/// too large to state.
pub fn build(s: &Sources) -> Result<(TaxInputs, BTreeSet<Note>), DecError> {
    let mut out = TaxInputs::default();
    let mut notes = BTreeSet::new();
    let funds: BTreeMap<InstrumentId, &Fund> = s.funds.iter().map(|f| (f.instrument, f)).collect();
    for f in &s.funds {
        match f.listed {
            Some(l) => {
                out.funds.insert(f.instrument, l);
            }
            None => {
                notes.insert(Note::VenueUnknown { instrument: f.instrument });
            }
        }
    }
    // the person's distributions, by account, day and cash to the cent
    let mut paid_on: BTreeMap<(AccountId, Date, Dec), BTreeSet<InstrumentId>> = BTreeMap::new();
    for p in s.paid.iter().filter(|p| funds.contains_key(&p.instrument)) {
        paid_on.entry((p.account, p.day, cents(p.cash))).or_default().insert(p.instrument);
    }
    for r in s.slips.iter().filter(|r| r.kind == SlipKind::T3 && s.alone.contains(&r.account)) {
        let t3 = match &r.read {
            Ok(slips::Slip::T3(t)) => t,
            Ok(_) => continue,
            Err(why) => {
                notes.insert(Note::SlipNotRead { account: r.account, kind: r.kind, year: r.year, why: why.clone() });
                continue;
            }
        };
        if t3.funds.is_empty() {
            // no breakdown: the slip is one fund's only where one fund paid the account
            // that year (its payments to the end of January after it count, a December
            // record date being paid then)
            let from = bagholder_core::jiff::civil::date(r.year, 1, 1);
            let to = bagholder_core::jiff::civil::date(r.year + 1, 1, 31);
            let paid: BTreeSet<InstrumentId> = s.paid.iter().filter(|p| p.account == r.account && p.day >= from && p.day <= to && funds.contains_key(&p.instrument)).map(|p| p.instrument).collect();
            match paid.iter().collect::<Vec<_>>().as_slice() {
                [only] => out.slips.entry((**only, r.year)).or_default().push(slip_of(&t3.boxes)?),
                [] => {}
                several => {
                    notes.insert(Note::Combines { account: r.account, year: r.year, funds: several.len() });
                }
            }
            continue;
        }
        for fund in &t3.funds {
            // every one of its payments is one of the person's, to the cent, all one instrument's
            let mut candidates: Option<BTreeSet<InstrumentId>> = None;
            for d in &fund.distributions {
                let here = paid_on.get(&(r.account, d.paid, cents(d.cash))).cloned().unwrap_or_default();
                candidates = Some(match candidates {
                    None => here,
                    Some(c) => c.intersection(&here).copied().collect(),
                });
            }
            let mut candidates: Vec<InstrumentId> = candidates.unwrap_or_default().into_iter().collect();
            if candidates.len() > 1 {
                // the name breaks a tie, never makes a match
                let name = folded(&fund.name);
                candidates.retain(|i| funds.get(i).is_some_and(|f| folded(&f.name) == name));
            }
            let instrument = match candidates.as_slice() {
                [only] => *only,
                [] if fund.distributions.is_empty() => continue,
                [] => {
                    notes.insert(Note::FundUnmatched { account: r.account, year: r.year, name: fund.name.clone() });
                    continue;
                }
                _ => {
                    notes.insert(Note::FundAmbiguous { account: r.account, year: r.year, name: fund.name.clone() });
                    continue;
                }
            };
            let mut boxes: BTreeMap<u16, Dec> = BTreeMap::new();
            for d in &fund.distributions {
                for (b, v) in &d.boxes {
                    let e = boxes.entry(*b).or_insert(Dec::ZERO);
                    *e = e.checked_add(*v)?;
                }
                let at = out.stated.entry(instrument).or_default().entry(d.record).or_insert(Stated { return_of_capital: Dec::ZERO, non_cash: Dec::ZERO });
                at.return_of_capital = at.return_of_capital.checked_add(cents(d.boxes.get(&RETURN_OF_CAPITAL_BOX).copied().unwrap_or(Dec::ZERO)))?;
                at.non_cash = at.non_cash.checked_add(cents(d.non_cash))?;
            }
            out.slips.entry((instrument, r.year)).or_default().push(slip_of(&boxes)?);
        }
    }
    for (instrument, rows) in &s.forms {
        let m = out.per_unit.entry(*instrument).or_default();
        for r in rows {
            m.insert(r.record, PerUnit { return_of_capital: r.return_of_capital, non_cash: r.non_cash, source: "cds".into() });
        }
    }
    // a year's slips are all in once the list was read on or after 1 April after it
    // (CRA's last day to issue a T3 is 31 March: `SPEC.md` tax-slips-1), from the
    // first year the broker's documents hold a slip of: a year before it was never
    // the broker's to issue
    if let (Some(on), Some(first)) = (s.listed_on, s.slips.iter().map(|r| r.year).min()) {
        let (m, d) = bagholder_wealthsimple::documents::ALL_IN_FROM;
        let last = if on >= bagholder_core::jiff::civil::date(on.year(), m, d) { on.year() - 1 } else { on.year() - 2 };
        out.slips_final = (first..=last).collect();
    }
    Ok((out, notes))
}

/// Where a fund lists, by its venue: the ledger knows Canada's and the United
/// States' alone.
fn listed_on_venue(mic: Option<&str>) -> Option<Listed> {
    use bagholder_core::sessions::{venue_of, Venue};
    match mic.and_then(|m| venue_of(m, false))? {
        Venue::Tsx | Venue::Cse | Venue::CboeCanada => Some(Listed::Canada),
        Venue::UsEquities => Some(Listed::UnitedStates),
        _ => None,
    }
}

/// What the figures' own inputs give: the holding's accounts, the funds and the
/// distributions paid.
pub fn of_inputs(inputs: &bagholder_engine::input::Inputs) -> Sources {
    use bagholder_core::account::{AccountType, Registration};
    use bagholder_core::transaction::Kind;
    let mut s = Sources::default();
    for (id, a) in &inputs.ledger.accounts {
        if matches!(a.account.account_type, AccountType::Known { registration: Registration::Unregistered, joint: false, .. }) {
            s.alone.insert(*id);
        }
    }
    for (id, info) in inputs.ledger.instruments.iter().filter(|(_, i)| i.instrument.fund == Some(true)) {
        let name = info.current_name();
        s.funds.push(Fund {
            instrument: *id,
            listed: listed_on_venue(name.and_then(|n| n.venue_mic.as_deref())),
            name: name.map(|n| n.name.clone().unwrap_or_else(|| n.symbol.clone())).unwrap_or_default(),
        });
    }
    for t in inputs.ledger.transactions.iter().filter(|t| t.kind == Kind::Dividend) {
        let (Some(instrument), Some(cash)) = (t.instrument, t.cash.as_ref()) else { continue };
        if cash.currency == bagholder_core::Currency::CAD {
            s.paid.push(Paid { account: t.account, instrument, day: t.trade_date, cash: cash.amount });
        }
    }
    s
}

/// The sources as the figures, the book and the market cache hold them now. The
/// slips' days are in `zone`, Wealthsimple's filing zone.
pub fn gather(inputs: &bagholder_engine::input::Inputs, book: &bagholder_book::Book, market: &rusqlite::Connection, zone: &bagholder_core::jiff::tz::TimeZone) -> Result<Sources, String> {
    use bagholder_core::account::AccountRef;
    let mut s = of_inputs(inputs);
    for f in &s.funds {
        // its CDS form, each year this copy read one, by the listing's own key
        let Some(n) = inputs.ledger.instruments.get(&f.instrument).and_then(|i| i.current_name()).filter(|_| f.listed == Some(Listed::Canada)) else { continue };
        let Ok(key) = bagholder_core::ListingKey::on(&n.symbol, n.venue_mic.as_deref().unwrap_or("")) else { continue };
        let kept = bagholder_market::stored::fund_tax::kept(market, &key.to_string()).map_err(|e| e.to_string())?;
        let rows: Vec<_> = kept
            .into_iter()
            .flat_map(|k| match k.found {
                bagholder_market::cds_tax::Found::Read { per_unit, .. } => per_unit,
                _ => Vec::new(),
            })
            .collect();
        if !rows.is_empty() {
            s.forms.insert(f.instrument, rows);
        }
    }
    let day = |t: bagholder_core::jiff::Timestamp| t.to_zoned(zone.clone()).date();
    for c in book.connections().map_err(|e| e.to_string())? {
        if let Some(at) = book.last_read(c.id, bagholder_wealthsimple::documents::READ_PART).map_err(|e| e.to_string())? {
            s.listed_on = Some(s.listed_on.map_or(day(at), |d| d.max(day(at))));
        }
        for slip in book.tax_slips(c.id).map_err(|e| e.to_string())? {
            let Some(kind) = SlipKind::of(&slip.kind) else { continue };
            let Some(account) = book.account_by_ref(&AccountRef::new(c.broker.clone(), slip.account_key.clone())).map_err(|e| e.to_string())? else { continue };
            let read = match &slip.text {
                Some(text) => slips::read(kind, text),
                None => Err(NOT_KEPT.into()),
            };
            s.slips.push(SlipRead { account, kind, year: slip.year, read });
        }
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bagholder_core::jiff::civil::date;

    fn account(n: u32) -> AccountId {
        AccountId::parse(&format!("00000000-0000-0000-0000-{n:012}")).unwrap()
    }
    fn fund_id(n: u32) -> InstrumentId {
        InstrumentId::parse(&format!("10000000-0000-0000-0000-{n:012}")).unwrap()
    }
    fn dec(s: &str) -> Dec {
        Dec::parse(s).unwrap()
    }
    fn fund(n: u32, name: &str) -> Fund {
        Fund { instrument: fund_id(n), listed: Some(Listed::Canada), name: name.into() }
    }
    fn paid(a: u32, f: u32, day: Date, cash: &str) -> Paid {
        Paid { account: account(a), instrument: fund_id(f), day, cash: dec(cash) }
    }
    /// A breakdown row: its record and payment days, its cash, its return of capital and non-cash.
    fn row(record: Date, paid: Date, cash: &str, roc: &str, non_cash: &str) -> slips::Distribution {
        slips::Distribution { record, paid, boxes: BTreeMap::from([(42, dec(roc)), (23, dec("1.0000"))]), cash: dec(cash), non_cash: dec(non_cash) }
    }
    fn t3(a: u32, year: i16, funds: Vec<(&str, Vec<slips::Distribution>)>, boxes: &[(u16, &str)]) -> SlipRead {
        let funds = funds.into_iter().map(|(name, distributions)| slips::TrustFund { name: name.into(), distributions }).collect();
        SlipRead { account: account(a), kind: SlipKind::T3, year, read: Ok(slips::Slip::T3(slips::T3 { year, boxes: boxes.iter().map(|(b, v)| (*b, dec(v))).collect(), funds })) }
    }

    /// Each fund on a breakdown is the one instrument every payment of it is the
    /// person's, to the cent; a name only breaks a tie; its slip and its record
    /// dates' figures are summed over the holding's accounts, each to the cent.
    #[test]
    fn a_breakdown_fund_is_the_instrument_its_every_payment_is() {
        let (mar, jun) = (date(2024, 3, 28), date(2024, 6, 27));
        let (rmar, rjun) = (date(2024, 3, 20), date(2024, 6, 20));
        let s = Sources {
            alone: BTreeSet::from([account(1), account(2)]),
            funds: vec![fund(1, "Alpha Covered Call ETF"), fund(2, "Beta Yield ETF")],
            paid: vec![paid(1, 1, mar, "100.00"), paid(1, 1, jun, "100.00"), paid(1, 2, mar, "100.00"), paid(2, 1, mar, "50.00")],
            slips: vec![
                t3(1, 2024, vec![("ALPHA COVERED CALL ETF", vec![row(rmar, mar, "100.0000", "30.1234", "0"), row(rjun, jun, "100.0000", "31.0000", "5.5000")]), ("Beta Yield ETF", vec![row(rmar, mar, "100.0000", "12.0000", "0")])], &[]),
                t3(2, 2024, vec![("Alpha Covered Call ETF", vec![row(rmar, mar, "50.0000", "15.0000", "0")])], &[]),
            ],
            ..Sources::default()
        };
        let (t, notes) = build(&s).unwrap();
        assert_eq!(notes, BTreeSet::new());
        let alpha = &t.slips[&(fund_id(1), 2024)];
        assert_eq!(alpha.iter().map(|x| x.return_of_capital).collect::<Vec<_>>(), vec![dec("61.12"), dec("15.00")]);
        assert_eq!(alpha[0].income, dec("2.00"));
        assert_eq!(t.slips[&(fund_id(2), 2024)][0].return_of_capital, dec("12.00"));
        assert_eq!(t.stated[&fund_id(1)][&rmar], Stated { return_of_capital: dec("45.12"), non_cash: dec("0") });
        assert_eq!(t.stated[&fund_id(1)][&rjun], Stated { return_of_capital: dec("31.00"), non_cash: dec("5.50") });
    }

    /// Over every way a set of funds can pay alike (each of three funds paying, or
    /// not, each of four days, at one of two amounts), a breakdown made from one
    /// fund's own payments is that fund where no other paid every one of them alike,
    /// is told apart by its name where one did, and is never another fund.
    #[test]
    fn a_breakdown_fund_is_matched_whatever_the_others_paid() {
        let days: Vec<Date> = (0..4).map(|i| date(2024, 3 + i, 15)).collect();
        let amounts = ["10.00", "20.00"];
        // each fund's payment on each day: none, or one of the two amounts (3^4 ways a fund pays)
        let ways: Vec<Vec<Option<&str>>> = (0..81).map(|mut w: usize| (0..4).map(|_| { let c = w % 3; w /= 3; if c == 0 { None } else { Some(amounts[c - 1]) } }).collect()).collect();
        let mut tried = 0;
        for (i, a) in ways.iter().enumerate().step_by(7) {
            for b in ways.iter().step_by(11) {
                if a.iter().all(Option::is_none) {
                    continue;
                }
                for named in [false, true] {
                    tried += 1;
                    let pays = |f: u32, w: &[Option<&str>]| w.iter().zip(&days).filter_map(move |(c, d)| c.map(|c| paid(1, f, *d, c))).collect::<Vec<_>>();
                    let mut p = pays(1, a);
                    p.extend(pays(2, b));
                    let rows = a.iter().zip(&days).filter_map(|(c, d)| c.map(|c| row(*d, *d, c, "1.0000", "0"))).collect();
                    let s = Sources {
                        alone: BTreeSet::from([account(1)]),
                        funds: vec![fund(1, "First Fund"), fund(2, if named { "Second Fund" } else { "First Fund" })],
                        paid: p,
                        slips: vec![t3(1, 2024, vec![("First Fund", rows)], &[])],
                        ..Sources::default()
                    };
                    let (t, notes) = build(&s).unwrap();
                    // fund 2 paid every one of fund 1's payments alike
                    let alike = a.iter().zip(b).all(|(x, y)| x.is_none() || x == y);
                    let matched: Vec<_> = t.slips.keys().map(|(f, _)| *f).collect();
                    if !alike || named {
                        assert_eq!(matched, vec![fund_id(1)], "way {i}: {a:?} against {b:?}, named {named}");
                    } else {
                        assert!(matched.is_empty() && notes.iter().any(|n| matches!(n, Note::FundAmbiguous { .. })), "way {i}: {a:?} against {b:?}: {notes:?}");
                    }
                }
            }
        }
        assert!(tried > 100, "{tried}");
    }

    /// A breakdown fund the person was not paid by alike is said, never matched; a
    /// T3 with no breakdown is one fund's where one fund paid the account that year,
    /// and said to combine them where several did; a refused slip, a slip of an
    /// account outside the holding and a fund on another venue are said or passed over.
    #[test]
    fn what_cannot_be_taken_in_is_said() {
        let mar = date(2023, 3, 28);
        let s = Sources {
            alone: BTreeSet::from([account(1), account(2), account(3)]),
            funds: vec![fund(1, "One"), fund(2, "Two"), Fund { instrument: fund_id(3), listed: None, name: "Three".into() }],
            // account 9 is outside the holding (a joint account), though the fund paid it too
            paid: vec![paid(1, 1, mar, "10.00"), paid(2, 1, mar, "10.00"), paid(2, 2, mar, "10.00"), paid(3, 1, date(2024, 3, 28), "10.00"), paid(9, 1, mar, "10.00")],
            slips: vec![
                t3(1, 2023, vec![], &[(42, "4.00"), (49, "1.00"), (24, "2.00"), (34, "0.50")]),
                t3(2, 2023, vec![], &[(42, "4.00")]),
                t3(3, 2024, vec![("Somebody Else's Fund", vec![row(date(2024, 3, 20), date(2024, 3, 28), "99.0000", "1.0000", "0")])], &[]),
                SlipRead { account: account(3), kind: SlipKind::T3, year: 2025, read: Err("kept without its text".into()) },
                t3(9, 2023, vec![], &[(42, "4.00")]),
            ],
            ..Sources::default()
        };
        let (t, notes) = build(&s).unwrap();
        assert_eq!(t.slips.keys().copied().collect::<Vec<_>>(), vec![(fund_id(1), 2023)]);
        assert_eq!(t.slips[&(fund_id(1), 2023)], vec![Slip { return_of_capital: dec("4.00"), income: dec("3.00"), foreign_tax: dec("0.50") }]);
        assert!(!t.funds.contains_key(&fund_id(3)));
        assert_eq!(
            notes,
            BTreeSet::from([
                Note::Combines { account: account(2), year: 2023, funds: 2 },
                Note::FundUnmatched { account: account(3), year: 2024, name: "Somebody Else's Fund".into() },
                Note::SlipNotRead { account: account(3), kind: SlipKind::T3, year: 2025, why: "kept without its text".into() },
                Note::VenueUnknown { instrument: fund_id(3) },
            ])
        );
    }

    /// A year's slips are all in once the list was read on or after 1 April after
    /// it, from the first year a slip is kept for; never before any list was read.
    #[test]
    fn a_year_s_slips_are_all_in_from_the_read_on_or_after_1_april_after_it() {
        let slips = vec![t3(1, 2023, vec![], &[]), t3(1, 2024, vec![], &[])];
        for (on, last) in [(Some(date(2026, 3, 31)), Some(2024)), (Some(date(2026, 4, 1)), Some(2025)), (Some(date(2025, 12, 31)), Some(2024)), (None, None)] {
            let (t, _) = build(&Sources { alone: BTreeSet::from([account(1)]), slips: slips.clone(), listed_on: on, ..Sources::default() }).unwrap();
            assert_eq!(t.slips_final.iter().copied().max(), last, "{on:?}");
            assert_eq!(t.slips_final.iter().copied().min(), last.map(|_| 2023), "{on:?}");
        }
    }
}
