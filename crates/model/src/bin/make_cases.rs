//! Writes the shared model cases to `tests/cases`: the rows of every case in
//! `bagholder_model::cases`, and what this model makes of them. Run after an
//! intended model change and review the diff of `tests/cases`.
//!
//!     cargo run -p bagholder-model --bin make-cases

use std::path::PathBuf;

use bagholder_model::cases::{case_doc, cases, to_text};

fn main() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/cases");
    for (name, case) in cases() {
        let path = dir.join(format!("{}.json", name));
        std::fs::write(&path, to_text(&case_doc(&case))).unwrap();
        println!("wrote tests/cases/{}.json", name);
    }
}
