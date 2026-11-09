//! When a venue's regular session is open, from each exchange's own published hours
//! and holiday calendar (`docs/plans/stage-money.md`, open question 2): held as data
//! with its source, never worked out. A date past what the exchange has published is
//! not known, and the caller treats it as open: a bracket rests outside the session
//! only when the calendar says the session is closed.

use jiff::civil::{Date, Time};
use jiff::tz::TimeZone;
use jiff::Timestamp;

/// A calendar the app holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Venue {
    /// NYSE and Nasdaq listings (NYSE, "Hours & Calendars", read 2026-10-04: core
    /// session 9:30 a.m. to 4:00 p.m. ET; holidays and 1:00 p.m. early closes 2026-2028;
    /// holidays 2010 to 2025 from its archived tables, read 2026-11-09).
    UsEquities,
    /// US listed options (Cboe, "US Options Hours", read 2026-10-04: 9:30 a.m. to 4:15
    /// p.m. ET, the widest of its classes, so the session is never taken as closed
    /// while one trades; holidays and early closes as the US equity calendar's, with
    /// the early close held to 1:15 p.m. on the same reasoning).
    UsOptions,
    /// TSX and TSX Venture listings (TMX, "Trading Hours", read 2026-10-04: 9:30 AM to
    /// 4:00 PM ET; TMX "Calendar", read 2026-10-04 and again 2026-11-04: the 2026 and
    /// 2027 holidays and each year's 1:00 PM Christmas Eve close; holidays 2010 to 2025
    /// from its archived calendars, read 2026-11-09).
    Tsx,
    /// CSE listings (the CSE, "Trading Resources", Trading Hours and Holidays, read
    /// 2026-11-04: CSE-listed securities trade continuously 9:30 am ET to 4 pm ET; its
    /// holidays to New Year's Day 2027, with Sept. 30 and Nov. 11 "Open for Trading –
    /// Settlement Holiday only"; no early close listed).
    Cse,
    /// Cboe Canada listings (Cboe, "Hours & Holidays", Canadian Equities, read
    /// 2026-11-04: Cboe Canada listed securities, NEO-L, trade continuously 9:30am to
    /// 4:00pm; the 2026 and 2027 holidays, Sept. 30 and Nov. 11 settlement holidays
    /// only; a "Christmas Early Close" on Dec. 24 listed with no time, so that day is
    /// held as a whole session, never taken as closed while it may trade).
    CboeCanada,
}

struct Calendar {
    zone: &'static str,
    open: (i8, i8),
    close: (i8, i8),
    early_close: (i8, i8),
    /// The last day the exchange's published calendar covers.
    published_to: (i16, i8, i8),
    holidays: &'static [(i16, i8, i8)],
    early: &'static [(i16, i8, i8)],
}

const US_HOLIDAYS: &[(i16, i8, i8)] = &[
    // 2010 to 2025: the NYSE's own holiday tables as archived each year (web.archive.org
    // copies of nyse.com and nyx.com, read 2026-11-09), and its unscheduled closures from
    // its statements: Hurricane Sandy 2012-10-29 and 30, the days of mourning 2018-12-05
    // and 2025-01-09
    (2010, 1, 1), (2010, 1, 18), (2010, 2, 15), (2010, 4, 2), (2010, 5, 31), (2010, 7, 5), (2010, 9, 6), (2010, 11, 25), (2010, 12, 24),
    (2011, 1, 17), (2011, 2, 21), (2011, 4, 22), (2011, 5, 30), (2011, 7, 4), (2011, 9, 5), (2011, 11, 24), (2011, 12, 26),
    (2012, 1, 2), (2012, 1, 16), (2012, 2, 20), (2012, 4, 6), (2012, 5, 28), (2012, 7, 4), (2012, 9, 3), (2012, 10, 29), (2012, 10, 30), (2012, 11, 22), (2012, 12, 25),
    (2013, 1, 1), (2013, 1, 21), (2013, 2, 18), (2013, 3, 29), (2013, 5, 27), (2013, 7, 4), (2013, 9, 2), (2013, 11, 28), (2013, 12, 25),
    (2014, 1, 1), (2014, 1, 20), (2014, 2, 17), (2014, 4, 18), (2014, 5, 26), (2014, 7, 4), (2014, 9, 1), (2014, 11, 27), (2014, 12, 25),
    (2015, 1, 1), (2015, 1, 19), (2015, 2, 16), (2015, 4, 3), (2015, 5, 25), (2015, 7, 3), (2015, 9, 7), (2015, 11, 26), (2015, 12, 25),
    (2016, 1, 1), (2016, 1, 18), (2016, 2, 15), (2016, 3, 25), (2016, 5, 30), (2016, 7, 4), (2016, 9, 5), (2016, 11, 24), (2016, 12, 26),
    (2017, 1, 2), (2017, 1, 16), (2017, 2, 20), (2017, 4, 14), (2017, 5, 29), (2017, 7, 4), (2017, 9, 4), (2017, 11, 23), (2017, 12, 25),
    (2018, 1, 1), (2018, 1, 15), (2018, 2, 19), (2018, 3, 30), (2018, 5, 28), (2018, 7, 4), (2018, 9, 3), (2018, 11, 22), (2018, 12, 5), (2018, 12, 25),
    (2019, 1, 1), (2019, 1, 21), (2019, 2, 18), (2019, 4, 19), (2019, 5, 27), (2019, 7, 4), (2019, 9, 2), (2019, 11, 28), (2019, 12, 25),
    (2020, 1, 1), (2020, 1, 20), (2020, 2, 17), (2020, 4, 10), (2020, 5, 25), (2020, 7, 3), (2020, 9, 7), (2020, 11, 26), (2020, 12, 25),
    (2021, 1, 1), (2021, 1, 18), (2021, 2, 15), (2021, 4, 2), (2021, 5, 31), (2021, 7, 5), (2021, 9, 6), (2021, 11, 25), (2021, 12, 24),
    (2022, 1, 17), (2022, 2, 21), (2022, 4, 15), (2022, 5, 30), (2022, 6, 20), (2022, 7, 4), (2022, 9, 5), (2022, 11, 24), (2022, 12, 26),
    (2023, 1, 2), (2023, 1, 16), (2023, 2, 20), (2023, 4, 7), (2023, 5, 29), (2023, 6, 19), (2023, 7, 4), (2023, 9, 4), (2023, 11, 23), (2023, 12, 25),
    (2024, 1, 1), (2024, 1, 15), (2024, 2, 19), (2024, 3, 29), (2024, 5, 27), (2024, 6, 19), (2024, 7, 4), (2024, 9, 2), (2024, 11, 28), (2024, 12, 25),
    (2025, 1, 1), (2025, 1, 9), (2025, 1, 20), (2025, 2, 17), (2025, 4, 18), (2025, 5, 26), (2025, 6, 19), (2025, 7, 4), (2025, 9, 1), (2025, 11, 27), (2025, 12, 25),
    (2026, 1, 1), (2026, 1, 19), (2026, 2, 16), (2026, 4, 3), (2026, 5, 25), (2026, 6, 19), (2026, 7, 3), (2026, 9, 7), (2026, 11, 26), (2026, 12, 25),
    (2027, 1, 1), (2027, 1, 18), (2027, 2, 15), (2027, 3, 26), (2027, 5, 31), (2027, 6, 18), (2027, 7, 5), (2027, 9, 6), (2027, 11, 25), (2027, 12, 24),
    (2028, 1, 17), (2028, 2, 21), (2028, 4, 14), (2028, 5, 29), (2028, 6, 19), (2028, 7, 4), (2028, 9, 4), (2028, 11, 23), (2028, 12, 25),
];
const US_EARLY: &[(i16, i8, i8)] = &[(2026, 11, 27), (2026, 12, 24), (2027, 11, 26), (2028, 7, 3), (2028, 11, 24)];
const TSX_HOLIDAYS: &[(i16, i8, i8)] = &[
    // 2010 to 2025: TMX's own calendars as archived each year (web.archive.org copies of
    // tmx.com and tsx.com, read 2026-11-09); no unscheduled full-day closure in those years
    (2010, 1, 1), (2010, 2, 15), (2010, 4, 2), (2010, 5, 24), (2010, 7, 1), (2010, 8, 2), (2010, 9, 6), (2010, 10, 11), (2010, 12, 27), (2010, 12, 28),
    (2011, 1, 3), (2011, 2, 21), (2011, 4, 22), (2011, 5, 23), (2011, 7, 1), (2011, 8, 1), (2011, 9, 5), (2011, 10, 10), (2011, 12, 26), (2011, 12, 27),
    (2012, 1, 2), (2012, 2, 20), (2012, 4, 6), (2012, 5, 21), (2012, 7, 2), (2012, 8, 6), (2012, 9, 3), (2012, 10, 8), (2012, 12, 25), (2012, 12, 26),
    (2013, 1, 1), (2013, 2, 18), (2013, 3, 29), (2013, 5, 20), (2013, 7, 1), (2013, 8, 5), (2013, 9, 2), (2013, 10, 14), (2013, 12, 25), (2013, 12, 26),
    (2014, 1, 1), (2014, 2, 17), (2014, 4, 18), (2014, 5, 19), (2014, 7, 1), (2014, 8, 4), (2014, 9, 1), (2014, 10, 13), (2014, 12, 25), (2014, 12, 26),
    (2015, 1, 1), (2015, 2, 16), (2015, 4, 3), (2015, 5, 18), (2015, 7, 1), (2015, 8, 3), (2015, 9, 7), (2015, 10, 12), (2015, 12, 25), (2015, 12, 28),
    (2016, 1, 1), (2016, 2, 15), (2016, 3, 25), (2016, 5, 23), (2016, 7, 1), (2016, 8, 1), (2016, 9, 5), (2016, 10, 10), (2016, 12, 26), (2016, 12, 27),
    (2017, 1, 2), (2017, 2, 20), (2017, 4, 14), (2017, 5, 22), (2017, 7, 3), (2017, 8, 7), (2017, 9, 4), (2017, 10, 9), (2017, 12, 25), (2017, 12, 26),
    (2018, 1, 1), (2018, 2, 19), (2018, 3, 30), (2018, 5, 21), (2018, 7, 2), (2018, 8, 6), (2018, 9, 3), (2018, 10, 8), (2018, 12, 25), (2018, 12, 26),
    (2019, 1, 1), (2019, 2, 18), (2019, 4, 19), (2019, 5, 20), (2019, 7, 1), (2019, 8, 5), (2019, 9, 2), (2019, 10, 14), (2019, 12, 25), (2019, 12, 26),
    (2020, 1, 1), (2020, 2, 17), (2020, 4, 10), (2020, 5, 18), (2020, 7, 1), (2020, 8, 3), (2020, 9, 7), (2020, 10, 12), (2020, 12, 25), (2020, 12, 28),
    (2021, 1, 1), (2021, 2, 15), (2021, 4, 2), (2021, 5, 24), (2021, 7, 1), (2021, 8, 2), (2021, 9, 6), (2021, 10, 11), (2021, 12, 27), (2021, 12, 28),
    (2022, 1, 3), (2022, 2, 21), (2022, 4, 15), (2022, 5, 23), (2022, 7, 1), (2022, 8, 1), (2022, 9, 5), (2022, 10, 10), (2022, 12, 26), (2022, 12, 27),
    (2023, 1, 2), (2023, 2, 20), (2023, 4, 7), (2023, 5, 22), (2023, 7, 3), (2023, 8, 7), (2023, 9, 4), (2023, 10, 9), (2023, 12, 25), (2023, 12, 26),
    (2024, 1, 1), (2024, 2, 19), (2024, 3, 29), (2024, 5, 20), (2024, 7, 1), (2024, 8, 5), (2024, 9, 2), (2024, 10, 14), (2024, 12, 25), (2024, 12, 26),
    (2025, 1, 1), (2025, 2, 17), (2025, 4, 18), (2025, 5, 19), (2025, 7, 1), (2025, 8, 4), (2025, 9, 1), (2025, 10, 13), (2025, 12, 25), (2025, 12, 26),
    (2026, 1, 1), (2026, 2, 16), (2026, 4, 3), (2026, 5, 18), (2026, 7, 1), (2026, 8, 3), (2026, 9, 7), (2026, 10, 12), (2026, 12, 25), (2026, 12, 28),
    (2027, 1, 1), (2027, 2, 15), (2027, 3, 26), (2027, 5, 24), (2027, 7, 1), (2027, 8, 2), (2027, 9, 6), (2027, 10, 11), (2027, 12, 27), (2027, 12, 28),
];
/// Days the TSX trades and CDS does not settle: Remembrance Day, and from 2021 the
/// National Day for Truth and Reconciliation, on the Monday after where either falls
/// on a weekend. CDS's holiday processing schedules and TMX's settlement notices as
/// archived each year to 2025 (read 2026-11-09); Cboe Canada's and the CSE's 2026 and
/// 2027 calendars, which mark both "settlement holiday only" (read 2026-11-04).
const CANADA_SETTLEMENT_ONLY: &[(i16, i8, i8)] = &[
    (2010, 11, 11),
    (2011, 11, 11),
    (2012, 11, 12),
    (2013, 11, 11),
    (2014, 11, 11),
    (2015, 11, 11),
    (2016, 11, 11),
    (2017, 11, 13),
    (2018, 11, 12),
    (2019, 11, 11),
    (2020, 11, 11),
    (2021, 9, 30), (2021, 11, 11),
    (2022, 9, 30), (2022, 11, 11),
    (2023, 10, 2), (2023, 11, 13),
    (2024, 9, 30), (2024, 11, 11),
    (2025, 9, 30), (2025, 11, 11),
    (2026, 9, 30), (2026, 11, 11),
    (2027, 9, 30), (2027, 11, 11),
];
/// The last day the Canadian settlement calendar is held to.
const CANADA_SETTLEMENT_TO: (i16, i8, i8) = (2027, 12, 31);
/// Days the NYSE trades and DTC does not settle: Columbus Day and Veterans Day, when
/// the Federal Reserve is closed (none for a Saturday holiday). The Federal Reserve's
/// holiday tables as archived (read 2026-11-09), DTCC's notices for 2020 to 2023, and
/// its 2026 holiday schedule.
const US_SETTLEMENT_ONLY: &[(i16, i8, i8)] = &[
    (2010, 10, 11), (2010, 11, 11),
    (2011, 10, 10), (2011, 11, 11),
    (2012, 10, 8), (2012, 11, 12),
    (2013, 10, 14), (2013, 11, 11),
    (2014, 10, 13), (2014, 11, 11),
    (2015, 10, 12), (2015, 11, 11),
    (2016, 10, 10), (2016, 11, 11),
    (2017, 10, 9),
    (2018, 10, 8), (2018, 11, 12),
    (2019, 10, 14), (2019, 11, 11),
    (2020, 10, 12), (2020, 11, 11),
    (2021, 10, 11), (2021, 11, 11),
    (2022, 10, 10), (2022, 11, 11),
    (2023, 10, 9),
    (2024, 10, 14), (2024, 11, 11),
    (2025, 10, 13), (2025, 11, 11),
    (2026, 10, 12), (2026, 11, 11),
    (2027, 10, 11), (2027, 11, 11),
];
/// Days the NYSE closed unscheduled and DTC settled as on any business day (DTCC's
/// statements of each day, read 2026-11-09).
const US_SETTLES_THOUGH_CLOSED: &[(i16, i8, i8)] = &[(2012, 10, 29), (2012, 10, 30), (2018, 12, 5), (2025, 1, 9)];
/// The last day the US settlement calendar is held to: the Federal Reserve's published
/// holidays run to 2027.
const US_SETTLEMENT_TO: (i16, i8, i8) = (2027, 12, 31);
const TSX_EARLY: &[(i16, i8, i8)] = &[(2026, 12, 24), (2027, 12, 24)];
const CSE_HOLIDAYS: &[(i16, i8, i8)] = &[(2026, 1, 1), (2026, 2, 16), (2026, 4, 3), (2026, 5, 18), (2026, 7, 1), (2026, 8, 3), (2026, 9, 7), (2026, 10, 12), (2026, 12, 25), (2026, 12, 28), (2027, 1, 1)];
const CBOE_CANADA_HOLIDAYS: &[(i16, i8, i8)] = &[
    (2026, 1, 1), (2026, 2, 16), (2026, 4, 3), (2026, 5, 18), (2026, 7, 1), (2026, 8, 3), (2026, 9, 7), (2026, 10, 12), (2026, 12, 25), (2026, 12, 28),
    (2027, 1, 1), (2027, 2, 15), (2027, 3, 26), (2027, 5, 24), (2027, 7, 1), (2027, 8, 2), (2027, 9, 6), (2027, 10, 11), (2027, 12, 27), (2027, 12, 28),
];

fn calendar(v: Venue) -> Calendar {
    match v {
        Venue::UsEquities => Calendar { zone: "America/New_York", open: (9, 30), close: (16, 0), early_close: (13, 0), published_to: (2028, 12, 31), holidays: US_HOLIDAYS, early: US_EARLY },
        Venue::UsOptions => Calendar { zone: "America/New_York", open: (9, 30), close: (16, 15), early_close: (13, 15), published_to: (2028, 12, 31), holidays: US_HOLIDAYS, early: US_EARLY },
        Venue::Tsx => Calendar { zone: "America/Toronto", open: (9, 30), close: (16, 0), early_close: (13, 0), published_to: (2027, 12, 31), holidays: TSX_HOLIDAYS, early: TSX_EARLY },
        Venue::Cse => Calendar { zone: "America/Toronto", open: (9, 30), close: (16, 0), early_close: (16, 0), published_to: (2027, 1, 1), holidays: CSE_HOLIDAYS, early: &[] },
        Venue::CboeCanada => Calendar { zone: "America/Toronto", open: (9, 30), close: (16, 0), early_close: (16, 0), published_to: (2027, 12, 31), holidays: CBOE_CANADA_HOLIDAYS, early: &[] },
    }
}

/// The calendar a listing trades on, by its venue's ISO 10383 code; none for a venue
/// the app holds no calendar of.
pub fn venue_of(mic: &str, option: bool) -> Option<Venue> {
    if option {
        return Some(Venue::UsOptions);
    }
    match mic {
        "XNYS" | "XNAS" | "XASE" | "ARCX" | "BATS" => Some(Venue::UsEquities),
        "XTSE" | "XTSX" => Some(Venue::Tsx),
        "XCNQ" => Some(Venue::Cse),
        "NEOE" => Some(Venue::CboeCanada),
        _ => None,
    }
}

fn day(d: (i16, i8, i8)) -> Option<Date> {
    Date::new(d.0, d.1, d.2).ok()
}

/// The venue's session on `date`, its open and its close: `Some(None)` on a day it
/// does not trade; none where the date is past what the exchange has published (or
/// the zone is not known).
fn session_on(c: &Calendar, zone: &TimeZone, date: Date) -> Option<Option<(Timestamp, Timestamp)>> {
    if date > day(c.published_to)? {
        return None;
    }
    use jiff::civil::Weekday;
    if matches!(date.weekday(), Weekday::Saturday | Weekday::Sunday) || c.holidays.iter().any(|h| day(*h) == Some(date)) {
        return Some(None);
    }
    let close = if c.early.iter().any(|e| day(*e) == Some(date)) { c.early_close } else { c.close };
    let at = |(h, m): (i8, i8)| -> Option<Timestamp> { date.to_datetime(Time::new(h, m, 0, 0).ok()?).to_zoned(zone.clone()).ok().map(|z| z.timestamp()) };
    Some(Some((at(c.open)?, at(close)?)))
}

/// How many days either way the next open and the last close are looked for: past
/// any run of weekend and holidays an exchange publishes.
const LOOK: i64 = 15;

/// Whether the venue's regular session is open at `at`; none where the date is past
/// what the exchange has published (or the zone is not known).
pub fn in_session(v: Venue, at: Timestamp) -> Option<bool> {
    let c = calendar(v);
    let zone = TimeZone::get(c.zone).ok()?;
    let date = at.to_zoned(zone.clone()).date();
    Some(session_on(&c, &zone, date)?.is_some_and(|(open, close)| at >= open && at < close))
}

/// Whether the venue trades on `date`, by the calendar the app holds: none before
/// the first year its holidays are held for, or past what the exchange has published.
pub fn trades_on(v: Venue, date: Date) -> Option<bool> {
    let c = calendar(v);
    let first = c.holidays.first().and_then(|h| day((h.0, 1, 1)))?;
    if date < first {
        return None;
    }
    let zone = TimeZone::get(c.zone).ok()?;
    Some(session_on(&c, &zone, date)?.is_some())
}

/// Whether trades in the venue's market settle on `date`: a day its listing exchange
/// trades (the TSX's for every Canadian venue, through CDS; the NYSE's for the US,
/// through DTC) other than a day the market trades and does not settle, and in the US
/// also a day the NYSE closed unscheduled while DTC settled. None outside the years the
/// app holds that market's settlement calendar for (2010 to 2027).
pub fn settles_on(v: Venue, date: Date) -> Option<bool> {
    let (exchange, settlement_only, though_closed, to) = match v {
        Venue::Tsx | Venue::Cse | Venue::CboeCanada => (Venue::Tsx, CANADA_SETTLEMENT_ONLY, &[][..], CANADA_SETTLEMENT_TO),
        Venue::UsEquities | Venue::UsOptions => (Venue::UsEquities, US_SETTLEMENT_ONLY, US_SETTLES_THOUGH_CLOSED, US_SETTLEMENT_TO),
    };
    if date > day(to)? {
        return None;
    }
    let on = |list: &[(i16, i8, i8)]| list.iter().any(|h| day(*h) == Some(date));
    if on(though_closed) {
        return Some(true);
    }
    Some(trades_on(exchange, date)? && !on(settlement_only))
}

/// When the venue's session next opens after `at`; none where that is past what the
/// exchange has published.
pub fn next_open(v: Venue, at: Timestamp) -> Option<Timestamp> {
    let c = calendar(v);
    let zone = TimeZone::get(c.zone).ok()?;
    let mut date = at.to_zoned(zone.clone()).date();
    for _ in 0..LOOK {
        if let Some((open, _)) = session_on(&c, &zone, date)? {
            if open > at {
                return Some(open);
            }
        }
        date = date.tomorrow().ok()?;
    }
    None
}

/// The day of the venue's last session to have closed at or before `at`; none where
/// the date is past what the exchange has published.
pub fn last_closed_day(v: Venue, at: Timestamp) -> Option<Date> {
    let c = calendar(v);
    let zone = TimeZone::get(c.zone).ok()?;
    let mut date = at.to_zoned(zone.clone()).date();
    for _ in 0..LOOK {
        if let Some((_, close)) = session_on(&c, &zone, date)? {
            if close <= at {
                return Some(date);
            }
        }
        date = date.yesterday().ok()?;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    /// Every calendar runs at least a year past the day this runs, so a calendar
    /// is extended a year before a date in its last year is needed: from 2028 every
    /// row of a year no calendar holds would wait, and nothing else would say so. The
    /// CSE publishes one year at a time (through New Year's Day of the next, its page
    /// read 2026-11-09), so it is held only to the day this runs.
    #[test]
    fn every_calendar_runs_a_year_ahead() {
        let today = jiff::Zoned::now().date();
        let ahead = today.checked_add(jiff::Span::new().months(12)).unwrap();
        let to = |d: (i16, i8, i8)| day(d).unwrap();
        let held = [
            ("the NYSE's holidays (US equities and options)", to(calendar(Venue::UsEquities).published_to), "the NYSE's Markets, Hours & Calendars page", ahead),
            ("TMX's holidays (the TSX, and every Canadian venue's settlement)", to(calendar(Venue::Tsx).published_to), "TMX's Calendars and Trading Hours page", ahead),
            ("Cboe Canada's holidays", to(calendar(Venue::CboeCanada).published_to), "Cboe's Canadian Equities Hours & Holidays page", ahead),
            ("the CSE's holidays", to(calendar(Venue::Cse).published_to), "the CSE's Trading Resources page, Holidays", today),
            ("Canada's settlement-only days (CANADA_SETTLEMENT_TO, CANADA_SETTLEMENT_ONLY)", to(CANADA_SETTLEMENT_TO), "CDS's holiday processing schedule and TMX's settlement schedule notices", ahead),
            ("the US's settlement-only days (US_SETTLEMENT_TO, US_SETTLEMENT_ONLY)", to(US_SETTLEMENT_TO), "the Federal Reserve's holiday schedule and DTCC's yearly holiday schedule", ahead),
        ];
        let short: Vec<String> = held
            .iter()
            .filter(|(_, ends, _, need)| ends < need)
            .map(|(what, ends, source, need)| format!("{what} end {ends}, before {need}: extend them from {source}"))
            .collect();
        assert!(short.is_empty(), "a calendar runs out within the year:\n{}", short.join("\n"));
    }

    /// Settlement days are the market's own: a day the exchange trades and the
    /// settlement system does not (Columbus Day, Remembrance Day) is none, an
    /// unscheduled NYSE closure DTC settled through is one, and a year the app holds no
    /// settlement calendar for is not known.
    #[test]
    fn settlement_days_are_each_markets_own() {
        let d = |s: &str| s.parse::<Date>().unwrap();
        // US Thanksgiving 2025 and Good Friday: neither trades nor settles
        assert_eq!(settles_on(Venue::UsEquities, d("2025-11-27")), Some(false));
        assert_eq!(settles_on(Venue::UsEquities, d("2025-04-18")), Some(false));
        assert_eq!(settles_on(Venue::Tsx, d("2025-11-27")), Some(true));
        // Columbus Day: the NYSE trades, DTC does not settle
        assert_eq!(trades_on(Venue::UsEquities, d("2025-10-13")), Some(true));
        assert_eq!(settles_on(Venue::UsEquities, d("2025-10-13")), Some(false));
        // the NYSE closed for President Carter's day of mourning; DTC settled
        assert_eq!(trades_on(Venue::UsEquities, d("2025-01-09")), Some(false));
        assert_eq!(settles_on(Venue::UsEquities, d("2025-01-09")), Some(true));
        // Remembrance Day: the TSX trades, CDS does not settle, on every Canadian venue
        assert_eq!(trades_on(Venue::Tsx, d("2025-11-11")), Some(true));
        assert_eq!(settles_on(Venue::Tsx, d("2025-11-11")), Some(false));
        assert_eq!(settles_on(Venue::CboeCanada, d("2025-11-11")), Some(false));
        assert_eq!(settles_on(Venue::Tsx, d("2017-11-13")), Some(false), "Nov 11 a Saturday: the Monday after");
        // years before and after what is held: not known
        assert_eq!(settles_on(Venue::Tsx, d("2009-06-01")), None);
        assert_eq!(settles_on(Venue::UsEquities, d("2028-03-01")), None);
    }

    #[test]
    fn a_session_is_open_from_the_open_to_the_close_on_a_trading_day_in_the_venues_own_zone() {
        // Monday 2026-10-05, EDT: 9:30 is 13:30Z
        assert_eq!(in_session(Venue::UsEquities, at("2026-10-05T13:29:59Z")), Some(false));
        assert_eq!(in_session(Venue::UsEquities, at("2026-10-05T13:30:00Z")), Some(true));
        assert_eq!(in_session(Venue::UsEquities, at("2026-10-05T19:59:59Z")), Some(true));
        assert_eq!(in_session(Venue::UsEquities, at("2026-10-05T20:00:00Z")), Some(false));
        // options to 4:15
        assert_eq!(in_session(Venue::UsOptions, at("2026-10-05T20:10:00Z")), Some(true));
        // winter, EST: 9:30 is 14:30Z
        assert_eq!(in_session(Venue::Tsx, at("2026-12-01T14:29:00Z")), Some(false));
        assert_eq!(in_session(Venue::Tsx, at("2026-12-01T14:30:00Z")), Some(true));
    }

    #[test]
    fn weekends_holidays_and_early_closes_are_the_exchanges_own() {
        assert_eq!(in_session(Venue::UsEquities, at("2026-10-03T15:00:00Z")), Some(false), "a Saturday");
        assert_eq!(in_session(Venue::UsEquities, at("2026-11-26T15:00:00Z")), Some(false), "US Thanksgiving");
        assert_eq!(in_session(Venue::Tsx, at("2026-11-26T15:00:00Z")), Some(true), "not a TSX holiday");
        assert_eq!(in_session(Venue::Tsx, at("2026-10-12T15:00:00Z")), Some(false), "Canadian Thanksgiving");
        assert_eq!(in_session(Venue::UsEquities, at("2026-10-12T15:00:00Z")), Some(true), "not an NYSE holiday");
        // Christmas Eve 2026 closes at 1 PM ET (18:00Z)
        assert_eq!(in_session(Venue::UsEquities, at("2026-12-24T17:59:00Z")), Some(true));
        assert_eq!(in_session(Venue::UsEquities, at("2026-12-24T18:00:00Z")), Some(false));
        assert_eq!(in_session(Venue::Tsx, at("2026-12-24T18:00:00Z")), Some(false));
    }

    #[test]
    fn the_cse_and_cboe_canada_keep_their_own_published_hours_and_holidays() {
        for v in [Venue::Cse, Venue::CboeCanada] {
            // Monday 2026-11-09, EST: 9:30 is 14:30Z, 4:00 PM is 21:00Z
            assert_eq!(in_session(v, at("2026-11-09T14:29:00Z")), Some(false), "{v:?}");
            assert_eq!(in_session(v, at("2026-11-09T14:30:00Z")), Some(true), "{v:?}");
            assert_eq!(in_session(v, at("2026-11-09T20:59:00Z")), Some(true), "{v:?}");
            assert_eq!(in_session(v, at("2026-11-09T21:00:00Z")), Some(false), "{v:?}");
            assert_eq!(in_session(v, at("2026-11-11T15:00:00Z")), Some(true), "{v:?}: a settlement holiday trades");
            assert_eq!(in_session(v, at("2026-09-30T15:00:00Z")), Some(true), "{v:?}: a settlement holiday trades");
            assert_eq!(in_session(v, at("2026-12-28T15:00:00Z")), Some(false), "{v:?}: Boxing Day, observed");
            // no early close is published for either: Christmas Eve trades to 4:00 PM
            assert_eq!(in_session(v, at("2026-12-24T20:00:00Z")), Some(true), "{v:?}");
        }
        assert_eq!(in_session(Venue::CboeCanada, at("2027-03-26T15:00:00Z")), Some(false), "Good Friday 2027");
        assert_eq!(in_session(Venue::Tsx, at("2027-12-24T18:00:00Z")), Some(false), "TMX's 2027 Christmas Eve closes at 1:00 PM");
        assert_eq!(in_session(Venue::Tsx, at("2027-12-24T17:59:00Z")), Some(true));
        assert_eq!(in_session(Venue::Tsx, at("2027-02-15T15:00:00Z")), Some(false), "Family Day 2027");
        assert_eq!(venue_of("XCNQ", false), Some(Venue::Cse));
        assert_eq!(venue_of("NEOE", false), Some(Venue::CboeCanada));
    }

    #[test]
    fn the_next_open_and_the_last_close_step_over_weekends_and_holidays() {
        for v in [Venue::Tsx, Venue::Cse, Venue::CboeCanada] {
            // 02:00 Sunday 2026-11-08 Toronto: the last session closed Friday, the next opens Monday 9:30
            let sunday = at("2026-11-08T07:00:00Z");
            assert_eq!(last_closed_day(v, sunday), Some(Date::new(2026, 11, 6).unwrap()), "{v:?}");
            assert_eq!(next_open(v, sunday), Some(at("2026-11-09T14:30:00Z")), "{v:?}");
            // before the open on a trading day: that day's own open; the last close is the day before's
            let early = at("2026-11-10T12:00:00Z");
            assert_eq!(next_open(v, early), Some(at("2026-11-10T14:30:00Z")), "{v:?}");
            assert_eq!(last_closed_day(v, early), Some(Date::new(2026, 11, 9).unwrap()), "{v:?}");
            // during the session: the next open is tomorrow's, the last close yesterday's
            let open = at("2026-11-10T16:00:00Z");
            assert_eq!(next_open(v, open), Some(at("2026-11-11T14:30:00Z")), "{v:?}");
            assert_eq!(last_closed_day(v, open), Some(Date::new(2026, 11, 9).unwrap()), "{v:?}");
            // over Christmas and Boxing Day observed: Thursday 24th to Tuesday 29th
            assert_eq!(next_open(v, at("2026-12-25T15:00:00Z")), Some(at("2026-12-29T14:30:00Z")), "{v:?}");
            assert_eq!(last_closed_day(v, at("2026-12-28T15:00:00Z")), Some(Date::new(2026, 12, 24).unwrap()), "{v:?}");
        }
        // a US holiday is the US calendar's alone
        assert_eq!(next_open(Venue::UsEquities, at("2026-11-26T15:00:00Z")), Some(at("2026-11-27T14:30:00Z")));
        assert_eq!(next_open(Venue::Tsx, at("2026-11-26T22:00:00Z")), Some(at("2026-11-27T14:30:00Z")));
    }

    #[test]
    fn a_date_past_what_the_exchange_published_is_not_known() {
        assert_eq!(in_session(Venue::Tsx, at("2028-01-04T15:00:00Z")), None);
        assert_eq!(in_session(Venue::Cse, at("2027-01-04T15:00:00Z")), None, "the CSE has published to New Year's Day 2027");
        assert_eq!(next_open(Venue::Cse, at("2026-12-31T22:00:00Z")), None, "the next open is past what the CSE has published");
        assert_eq!(in_session(Venue::UsEquities, at("2029-01-02T15:00:00Z")), None);
        assert_eq!(venue_of("XLON", false), None, "a venue with no calendar held");
    }
}
