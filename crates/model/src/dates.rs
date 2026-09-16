//! Calendar arithmetic on ISO dates. Only whole days are ever needed, so this
//! is Howard Hinnant's civil-days algorithm rather than a date library.

pub const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// `YYYY-MM-DD` at the head of the string, or `None` when it is not a date.
/// Python's `date.fromisoformat` is strict about the shape and about the day
/// existing, and callers here depend on the failure.
pub fn parse_iso(s: &str) -> Option<(i64, u32, u32)> {
    let b = s.as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' { return None; }
    if !b[..4].iter().all(|c| c.is_ascii_digit()) { return None; }
    if !b[5..7].iter().all(|c| c.is_ascii_digit()) { return None; }
    if !b[8..10].iter().all(|c| c.is_ascii_digit()) { return None; }
    let y: i64 = s[..4].parse().ok()?;
    let m: u32 = s[5..7].parse().ok()?;
    let d: u32 = s[8..10].parse().ok()?;
    if m < 1 || m > 12 || d < 1 || d > days_in_month(y, m) { return None; }
    Some((y, m, d))
}

pub fn is_leap(y: i64) -> bool { (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 }

pub fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if is_leap(y) { 29 } else { 28 },
        _ => 0,
    }
}

/// Days since 1970-01-01.
pub fn to_days(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn fmt(y: i64, m: u32, d: u32) -> String { format!("{:04}-{:02}-{:02}", y, m, d) }

/// `model.days_between`: never negative, and an unreadable date is zero days
/// rather than an error.
pub fn days_between(a: &str, b: &str) -> i64 {
    let (da, db) = (parse_iso(&head10(a)), parse_iso(&head10(b)));
    match (da, db) {
        (Some((y1, m1, d1)), Some((y2, m2, d2))) => (to_days(y2, m2, d2) - to_days(y1, m1, d1)).max(0),
        _ => 0,
    }
}

/// `model.shift_date`: the date moved by whole days, or the leading ten
/// characters unchanged when it cannot be read.
pub fn shift_date(iso: &str, days: i64) -> String {
    let h = head10(iso);
    match parse_iso(&h) {
        Some((y, m, d)) => { let (y2, m2, d2) = from_days(to_days(y, m, d) + days); fmt(y2, m2, d2) }
        None => h,
    }
}

pub fn head10(s: &str) -> String { s.chars().take(10).collect() }

/// `model.option_expiry`: `LUNR 29AUG25 11.50 CALL` -> `2025-08-29`.
pub fn option_expiry(symbol: &str) -> String {
    let u = crate::value::fold_spaces_upper(symbol);
    let mut parts = u.splitn(3, ' ');
    let _root = match parts.next() { Some(r) if !r.is_empty() => r, _ => return String::new() };
    let tok = match parts.next() { Some(t) => t, None => return String::new() };
    // the regex requires a trailing space, so a two-token symbol never matches
    if parts.next().is_none() { return String::new(); }
    let b = tok.as_bytes();
    if b.len() != 7 { return String::new(); }
    if !b[..2].iter().all(|c| c.is_ascii_digit()) { return String::new(); }
    if !b[2..5].iter().all(|c| c.is_ascii_uppercase()) { return String::new(); }
    if !b[5..].iter().all(|c| c.is_ascii_digit()) { return String::new(); }
    let title = format!("{}{}", &tok[2..3], tok[3..5].to_lowercase());
    let month = match MONTHS.iter().position(|m| *m == title) { Some(i) => i + 1, None => return String::new() };
    format!("20{}-{:02}-{}", &tok[5..7], month, &tok[..2])
}
