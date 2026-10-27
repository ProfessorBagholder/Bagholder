//! A violation of each form the workspace refuses, compiled only with the
//! `violations` feature (a discarded failure) or the `unfinished` feature (unsafe
//! with no reason, code left unfinished); see `Cargo.toml`. Nothing uses this crate.

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

#[cfg(feature = "unfinished")]
pub fn unsound() -> u32 {
    // an unsafe block with no SAFETY comment
    unsafe { std::ptr::read(&1u32) }
}

#[cfg(feature = "unfinished")]
pub fn left_in() -> u32 {
    // a debugging print left in
    dbg!(1)
}

#[cfg(feature = "unfinished")]
pub fn unfinished() -> u32 {
    // a body still to write
    todo!()
}

#[cfg(feature = "unfinished")]
pub fn unwritten() -> u32 {
    // a body never written
    unimplemented!()
}
