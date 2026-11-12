//! The calendars the app holds are extended before they are needed (`SPEC.md`
//! `tax-cost-of-an-etf-2`): a test here reads the clock, which the crate's own
//! sources may not (`boundaries.rs`).

use bagholder_core::jiff;
use bagholder_core::sessions::{settlement_held_to, trading_held_to, Venue};

/// Every calendar runs at least a year past the day this runs, so a calendar is
/// extended a year before a date in its last year is needed: from the year one
/// runs out, every row of that year with no settlement stated would wait, and
/// nothing else would say so. The CSE publishes one year at a time (through New
/// Year's Day of the next, its page read 2026-11-09), so it is held only to the day
/// this runs; every Canadian venue settles on the TSX's calendar.
#[test]
fn every_calendar_runs_a_year_ahead() {
    let today = jiff::Zoned::now().date();
    let ahead = today.checked_add(jiff::Span::new().months(12)).unwrap();
    let held = [
        ("the NYSE's holidays (US equities and options)", trading_held_to(Venue::UsEquities), "the NYSE's Markets, Hours & Calendars page", ahead),
        ("TMX's holidays (the TSX, and every Canadian venue's settlement)", trading_held_to(Venue::Tsx), "TMX's Calendars and Trading Hours page", ahead),
        ("Cboe Canada's holidays", trading_held_to(Venue::CboeCanada), "Cboe's Canadian Equities Hours & Holidays page", ahead),
        ("the CSE's holidays", trading_held_to(Venue::Cse), "the CSE's Trading Resources page, Holidays", today),
        ("Canada's settlement-only days (CANADA_SETTLEMENT_TO, CANADA_SETTLEMENT_ONLY)", settlement_held_to(Venue::Tsx), "CDS's holiday processing schedule and TMX's settlement schedule notices", ahead),
        ("the US's settlement-only days (US_SETTLEMENT_TO, US_SETTLEMENT_ONLY)", settlement_held_to(Venue::UsEquities), "the Federal Reserve's holiday schedule and DTCC's yearly holiday schedule", ahead),
    ];
    let short: Vec<String> = held
        .iter()
        .filter(|(_, ends, _, need)| ends.is_none_or(|e| e < *need))
        .map(|(what, ends, source, need)| format!("{what} end {}, before {need}: extend them in core/src/sessions.rs from {source}", ends.map_or("unread".into(), |e| e.to_string())))
        .collect();
    assert!(short.is_empty(), "a calendar runs out within the year:\n{}", short.join("\n"));
}
