//! Time zone rules, read from the copy built into jiff: the tests pin it
//! (`pinned-tzdb`), so a machine's own database never changes an answer.

use bagholder_model::clock;

#[test]
fn test_a_fill_after_albertas_change_is_read_on_central_time() {
    // Alberta keeps UTC−6 from 1 Nov 2026 (IANA 2026c); a database without that
    // change reads this fill an hour early, at 08:00
    assert_eq!(clock::when_parts("2026-12-01T15:00:00+00:00"), ("2026-12-01".to_string(), "09:00".to_string()));
    assert_eq!(clock::when_parts("2026-01-01T15:00:00+00:00"), ("2026-01-01".to_string(), "08:00".to_string()));
}

#[test]
fn test_a_wall_clock_time_takes_its_own_days_offset() {
    let wall = |y, m, d, hh: i64| bagholder_model::dates::to_days(y, m, d) * 86400 + hh * 3600;
    assert_eq!(clock::offset_for_wall("America/Toronto", wall(2026, 9, 2, 9)), Some(-4 * 3600));
    assert_eq!(clock::offset_for_wall("America/Toronto", wall(2026, 1, 15, 9)), Some(-5 * 3600));
    assert_eq!(clock::offset_for_wall("Nowhere/Invented", wall(2026, 1, 15, 9)), None);
}
