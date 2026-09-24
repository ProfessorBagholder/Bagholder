//! Each payer's own statement of its distributions and schedule
//! (`docs/plans/stage-3a-sources.md`, "Distributions and schedules, from the
//! payer itself"; research 2).
//!
//! One adapter per fund company, reading the publication research 2 found for
//! it: the fund's page, the data the page itself reads, or the company's own
//! release. Which company a fund is comes from the brand its name carries (the
//! broker's names are inconsistent, "MacKenzie Financial Corp. - …", "Tidal
//! Trust II - Yieldmax …", but each names its brand); the company's adapter then
//! finds the ticker in the company's own list, and answers "not carried" when it
//! is not there. Two companies' publications cannot be read at all (their sites
//! answer only a browser, and getting past a bot check is not something the app
//! does): for them, and only them, the exchange-side record stands in (the
//! owner's exception of 2026-09-24): Mackenzie through TMX, WisdomTree through
//! Yahoo.
//!
//! What a publication states is kept as stated, then checked here before
//! anything is written: a row repeated identically is one row; two different
//! rows for one ex-date, a record or pay date that cannot belong to its
//! distribution, or a date far past the fund's life are a meaning failure, and
//! nothing is written.

pub mod bmo;
pub mod evolve;
pub mod exchange;
pub mod fidelity;
pub mod globalx;
pub mod goldman;
pub mod hamilton;
pub mod harvest;
pub mod ishares_ca;
pub mod ishares_us;
pub mod newswire;
pub mod ninepoint;
pub mod purpose;
pub mod run;
pub mod us_pages;
pub mod vanguard_ca;
pub mod vanguard_us;

use bagholder_core::jiff::civil::Date;
use bagholder_core::jiff::{SignedDuration, Timestamp};
use bagholder_core::{Currency, Dec, SourceName};
use bagholder_net::Net;

use crate::contract::Market;
use crate::needs::PayerNeed;
use crate::outcome::Noted;

/// A distribution as its payer states it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Distribution {
    pub ex_date: Date,
    pub record_date: Option<Date>,
    pub pay_date: Option<Date>,
    /// The cash paid per unit.
    pub cash: Dec,
    /// The part paid in units, where the payer states one.
    pub reinvested: Option<Dec>,
    pub currency: Currency,
}

/// What one read of a payer's publication states.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub rows: Vec<Distribution>,
    /// Payments a year, where the publication states the schedule.
    pub per_year: Option<u32>,
}

/// One fund company's (or the exchange-side stand-in's) reader.
pub trait Payer: Send + Sync {
    fn source(&self) -> SourceName;
    fn host(&self) -> &'static str;
    /// The brand words a fund's name carries when it is this company's,
    /// lower-case.
    fn brands(&self) -> &'static [&'static str];
    /// The markets whose listings this company's publication covers (one brand
    /// can be two companies: Vanguard Canada and Vanguard in the US).
    fn markets(&self) -> &'static [Market];
    fn read(&self, net: &Net, need: &PayerNeed, now: Timestamp) -> Noted<Record>;
}

/// Canadian listings, on any Canadian venue.
pub const CANADA: &[Market] = &[Market::Canada, Market::CboeCanada];
pub const US: &[Market] = &[Market::UnitedStates];

/// Every payer adapter, in the order a fund's name is tried against them.
pub fn all() -> Vec<Box<dyn Payer>> {
    vec![
        Box::new(ninepoint::Ninepoint),
        Box::new(evolve::Evolve),
        Box::new(harvest::Harvest),
        Box::new(vanguard_ca::VanguardCanada),
        Box::new(hamilton::Hamilton),
        Box::new(purpose::Purpose),
        Box::new(globalx::GlobalX),
        Box::new(bmo::Bmo),
        Box::new(ishares_ca::ISharesCanada),
        Box::new(fidelity::FidelityCanada),
        Box::new(vanguard_us::VanguardUs),
        Box::new(us_pages::YieldMax),
        Box::new(us_pages::Defiance),
        Box::new(goldman::GoldmanSachs),
        Box::new(ishares_us::ISharesUs),
        Box::new(exchange::Mackenzie),
        Box::new(exchange::WisdomTree),
    ]
}

/// The adapter for a payer, by the brand its name carries and the market it
/// trades in.
pub fn adapter_for(need: &PayerNeed) -> Option<Box<dyn Payer>> {
    let name = need.name.as_deref()?.to_lowercase();
    let market = need.listing.market()?;
    all().into_iter().find(|p| p.markets().contains(&market) && p.brands().iter().any(|b| name.contains(b)))
}

/// A record checked, repeats folded: a meaning failure names what is wrong.
pub fn checked(mut record: Record, now: Timestamp) -> Result<Record, String> {
    record.rows.sort();
    record.rows.dedup();
    let far = now.to_zoned(bagholder_core::jiff::tz::TimeZone::UTC).date().checked_add(SignedDuration::from_hours(24 * 400)).unwrap_or(Date::MAX);
    for w in record.rows.windows(2) {
        if w[0].ex_date == w[1].ex_date {
            return Err(format!("two different distributions go ex {}: {} and {}", w[0].ex_date, w[0].cash, w[1].cash));
        }
    }
    for r in &record.rows {
        if r.ex_date > far {
            return Err(format!("a distribution goes ex {}, past the fund's life", r.ex_date));
        }
        let week = SignedDuration::from_hours(24 * 7);
        if let Some(rd) = r.record_date {
            if rd > r.ex_date.checked_add(week).unwrap_or(Date::MAX) || rd < r.ex_date.checked_sub(week).unwrap_or(Date::MIN) {
                return Err(format!("the distribution going ex {} is on record {rd}", r.ex_date));
            }
        }
        if let Some(pd) = r.pay_date {
            if pd < r.ex_date || pd > r.ex_date.checked_add(SignedDuration::from_hours(24 * 120)).unwrap_or(Date::MAX) {
                return Err(format!("the distribution going ex {} is paid {pd}", r.ex_date));
            }
        }
        if r.cash < Dec::ZERO || r.reinvested.is_some_and(|x| x < Dec::ZERO) {
            return Err(format!("the distribution going ex {} is {}", r.ex_date, r.cash));
        }
    }
    Ok(record)
}

/// "$0.13500" or "0.135" as a decimal, exactly as written.
pub fn money_text(s: &str) -> Option<Dec> {
    let t = s.trim().trim_start_matches('$').replace(',', "");
    Dec::parse(t.trim()).ok()
}
