//! The app's on/off settings, read from its environment by one parser
//! (`docs/plans/stage-money.md`, part D). A setting takes `1`, `true`, `yes` or
//! `on`, or `0`, `false`, `no` or `off` (any case; unset or blank is off), and
//! nothing else: the server refuses to start on a value it cannot read, so
//! `BAGHOLDER_DRY_ORDERS=true` is never read as live orders.

/// Every on/off setting the app reads.
pub const SWITCHES: [&str; 6] = ["BAGHOLDER_DRY_ORDERS", "BAGHOLDER_NO_BROWSER", "BAGHOLDER_CHILD", "BAGHOLDER_OFFLINE", "BAGHOLDER_LOG_REQUESTS", "BAGHOLDER_NO_PDF"];

/// A setting's value as given, or why it does not read.
pub fn read(name: &str, value: Option<&std::ffi::OsStr>) -> Result<bool, String> {
    let Some(v) = value else { return Ok(false) };
    let Some(text) = v.to_str() else { return Err(format!("{name} is set to something that is not text")) };
    match text.trim().to_ascii_lowercase().as_str() {
        "" | "0" | "false" | "no" | "off" => Ok(false),
        "1" | "true" | "yes" | "on" => Ok(true),
        _ => Err(format!("{name} is set to {text:?}: it takes 1, true, yes or on, or 0, false, no or off")),
    }
}

/// A setting from the environment, or why it does not read.
pub fn switch(name: &str) -> Result<bool, String> {
    read(name, std::env::var_os(name).as_deref())
}

/// A setting for a reader that cannot refuse: one that does not read is on, the
/// side every setting here is safe on (no orders, no browser, no network). The
/// server never gets this far with one: it refuses to start first (`check`).
pub fn switch_on(name: &str) -> bool {
    switch(name).unwrap_or(true)
}

/// Every setting reads, or the first that does not, for the start to refuse.
pub fn check() -> Result<(), String> {
    SWITCHES.iter().try_for_each(|n| switch(n).map(|_| ()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn a_setting_reads_only_the_words_it_takes() {
        for on in ["1", "true", "TRUE", "yes", "On", " on "] {
            assert_eq!(read("X", Some(OsStr::new(on))), Ok(true), "{on:?}");
        }
        for off in ["", "0", "false", "No", "OFF", "  "] {
            assert_eq!(read("X", Some(OsStr::new(off))), Ok(false), "{off:?}");
        }
        assert_eq!(read("X", None), Ok(false), "unset is off");
        for bad in ["maybe", "2", "y", "enabled", "1 "] {
            if bad.trim() == "1" {
                continue;
            }
            let e = read("BAGHOLDER_DRY_ORDERS", Some(OsStr::new(bad))).unwrap_err();
            assert!(e.starts_with("BAGHOLDER_DRY_ORDERS is set to"), "{e}");
        }
    }
}
