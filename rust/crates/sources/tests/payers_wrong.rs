//! Wrong copies of recorded releases, each refused with nothing taken from it.

mod common;

use bagholder_sources::payers::{globalx, newswire};

#[test]
fn a_release_with_a_date_that_is_no_day_or_no_date_at_all_is_a_mismatch() {
    let r = newswire::release(&common::read("newswire", "wrong-shape-release-global-x-date.html")).unwrap();
    assert!(globalx::rows_for(&r, "AGCC").unwrap_err().why.contains("29/09/2026"));
    assert!(newswire::release(&common::read("newswire", "wrong-shape-release-no-date.html")).unwrap_err().path.contains("date"));
}
