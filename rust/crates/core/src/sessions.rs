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
    /// 4:00 PM ET; TMX "Calendar", read 2026-10-04 and again 2026-11-04: the 2026 and
    /// 2027 holidays and each year's 1:00 PM Christmas Eve close).
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
    (2026, 1, 1), (2026, 1, 19), (2026, 2, 16), (2026, 4, 3), (2026, 5, 25), (2026, 6, 19), (2026, 7, 3), (2026, 9, 7), (2026, 11, 26), (2026, 12, 25),
    (2027, 1, 1), (2027, 1, 18), (2027, 2, 15), (2027, 3, 26), (2027, 5, 31), (2027, 6, 18), (2027, 7, 5), (2027, 9, 6), (2027, 11, 25), (2027, 12, 24),
    (2028, 1, 17), (2028, 2, 21), (2028, 4, 14), (2028, 5, 29), (2028, 6, 19), (2028, 7, 4), (2028, 9, 4), (2028, 11, 23), (2028, 12, 25),
];
const US_EARLY: &[(i16, i8, i8)] = &[(2026, 11, 27), (2026, 12, 24), (2027, 11, 26), (2028, 7, 3), (2028, 11, 24)];
const TSX_HOLIDAYS: &[(i16, i8, i8)] = &[
    (2026, 1, 1), (2026, 2, 16), (2026, 4, 3), (2026, 5, 18), (2026, 7, 1), (2026, 8, 3), (2026, 9, 7), (2026, 10, 12), (2026, 12, 25), (2026, 12, 28),
    (2027, 1, 1), (2027, 2, 15), (2027, 3, 26), (2027, 5, 24), (2027, 7, 1), (2027, 8, 2), (2027, 9, 6), (2027, 10, 11), (2027, 12, 27), (2027, 12, 28),
];
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

/// The venue's latest session to have opened at or before `at`, its day and its
/// open: the session a quote stamped `at` states the change of. None past what the
/// exchange has published.
pub fn session_opened_by(v: Venue, at: Timestamp) -> Option<(Date, Timestamp)> {
    let c = calendar(v);
    let zone = TimeZone::get(c.zone).ok()?;
    let mut date = at.to_zoned(zone.clone()).date();
    for _ in 0..LOOK {
        if let Some((open, _)) = session_on(&c, &zone, date)? {
            if open <= at {
                return Some((date, open));
            }
        }
        date = date.yesterday().ok()?;
    }
    None
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
