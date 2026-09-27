//! A violation of each form the workspace refuses, compiled only with the
//! `violations` feature (see `Cargo.toml`). Nothing uses this crate.

#[cfg(feature = "violations")]
pub fn violations() -> u32 {
    let read = || "1".parse::<u32>();
    // a failure dropped with `let _ =`
    let _ = read();
    // a failure turned into nothing
    read().ok();
    // a failure read as a default
    read().unwrap_or_default()
}
