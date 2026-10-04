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
    /// session 9:30 a.m. to 4:00 p.m. ET; holidays and 1:00 p.m. early closes 2026-2028).
    UsEquities,
    /// US listed options (Cboe, "US Options Hours", read 2026-10-04: 9:30 a.m. to 4:15
    /// p.m. ET, the widest of its classes, so the session is never taken as closed
    /// while one trades; holidays and early closes as the US equity calendar's, with
    /// the early close held to 1:15 p.m. on the same reasoning).
    UsOptions,
    /// TSX and TSX Venture listings (TMX, "Trading Hours", read 2026-10-04: 9:30 AM to
    /// 4:00 PM ET; TMX "Calendar", read 2026-10-04: 2026 holidays and the 1:00 PM
    /// Christmas Eve close; 2027 not published yet).
    Tsx,
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
    (2026, 1, 1), (2026, 1, 19), (2026, 2, 16), (2026, 4, 3), (2026, 5, 25), (2026, 6, 19), (2026, 7, 3), (2026, 9, 7), (2026, 11, 26), (2026, 12, 25),
    (2027, 1, 1), (2027, 1, 18), (2027, 2, 15), (2027, 3, 26), (2027, 5, 31), (2027, 6, 18), (2027, 7, 5), (2027, 9, 6), (2027, 11, 25), (2027, 12, 24),
    (2028, 1, 17), (2028, 2, 21), (2028, 4, 14), (2028, 5, 29), (2028, 6, 19), (2028, 7, 4), (2028, 9, 4), (2028, 11, 23), (2028, 12, 25),
];
const US_EARLY: &[(i16, i8, i8)] = &[(2026, 11, 27), (2026, 12, 24), (2027, 11, 26), (2028, 7, 3), (2028, 11, 24)];
const TSX_HOLIDAYS: &[(i16, i8, i8)] = &[(2026, 1, 1), (2026, 2, 16), (2026, 4, 3), (2026, 5, 18), (2026, 7, 1), (2026, 8, 3), (2026, 9, 7), (2026, 10, 12), (2026, 12, 25), (2026, 12, 28)];
const TSX_EARLY: &[(i16, i8, i8)] = &[(2026, 12, 24)];

fn calendar(v: Venue) -> Calendar {
    match v {
        Venue::UsEquities => Calendar { zone: "America/New_York", open: (9, 30), close: (16, 0), early_close: (13, 0), published_to: (2028, 12, 31), holidays: US_HOLIDAYS, early: US_EARLY },
        Venue::UsOptions => Calendar { zone: "America/New_York", open: (9, 30), close: (16, 15), early_close: (13, 15), published_to: (2028, 12, 31), holidays: US_HOLIDAYS, early: US_EARLY },
        Venue::Tsx => Calendar { zone: "America/Toronto", open: (9, 30), close: (16, 0), early_close: (13, 0), published_to: (2026, 12, 31), holidays: TSX_HOLIDAYS, early: TSX_EARLY },
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
        _ => None,
    }
}

fn day(d: (i16, i8, i8)) -> Option<Date> {
    Date::new(d.0, d.1, d.2).ok()
}

/// Whether the venue's regular session is open at `at`; none where the date is past
/// what the exchange has published (or the zone is not known).
pub fn in_session(v: Venue, at: Timestamp) -> Option<bool> {
    let c = calendar(v);
    let zone = TimeZone::get(c.zone).ok()?;
    let local = at.to_zoned(zone);
    let date = local.date();
    if date > day(c.published_to)? {
        return None;
    }
    use jiff::civil::Weekday;
    if matches!(date.weekday(), Weekday::Saturday | Weekday::Sunday) || c.holidays.iter().any(|h| day(*h) == Some(date)) {
        return Some(false);
    }
    let close = if c.early.iter().any(|e| day(*e) == Some(date)) { c.early_close } else { c.close };
    let (open, close) = (Time::new(c.open.0, c.open.1, 0, 0).ok()?, Time::new(close.0, close.1, 0, 0).ok()?);
    let t = local.time();
    Some(t >= open && t < close)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> Timestamp {
        s.parse().unwrap()
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
    fn a_date_past_what_the_exchange_published_is_not_known() {
        assert_eq!(in_session(Venue::Tsx, at("2027-01-04T15:00:00Z")), None);
        assert_eq!(in_session(Venue::UsEquities, at("2029-01-02T15:00:00Z")), None);
        assert_eq!(venue_of("XLON", false), None, "a venue with no calendar held");
    }
}
