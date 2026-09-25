//! What the page is sent of the figures (`docs/plans/stage-3c-switch.md`, §4):
//! built from the engine's figures, typed, and generated to the page's
//! `web/src/lib/generated/figures.ts`.
//!
//! - **Exact amounts as text.** A money amount, a quantity or a price is the
//!   exact decimal the engine holds, written as text (`"1247.41"`), which the
//!   page types as `Dec` and formats without ever making a float of it. A ratio,
//!   a count or a number of days is a number.
//! - **A figure that may not be stated** is a [`Fig`]: its value, or the gaps it
//!   waits on, each a word `SPEC.md` lists.
//! - **Every row carries its id**, and a list's rows are told apart by the field
//!   their type declares (`#[diff(key = …)]`), never by one guessed from the data.

pub mod build;
pub mod filters;
pub mod figures;

use serde::Serialize;
use ts_rs::TS;

use bagholder_model::patch::Diff;

/// An exact decimal, as text. The page types it `Dec` (`web/src/lib/dec.ts`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, TS)]
#[ts(rename = "Dec")]
pub struct Dec(#[ts(type = "string")] pub bagholder_core::Dec);

impl From<bagholder_core::Dec> for Dec {
    fn from(d: bagholder_core::Dec) -> Dec {
        Dec(d)
    }
}

impl Diff for Dec {
    fn diff(&self, new: &Self, path: &mut Vec<serde_json::Value>, ops: &mut Vec<serde_json::Value>) {
        bagholder_model::patch::leaf(self, new, path, ops)
    }
}

/// A figure: its value, or the gaps it waits on.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(untagged)]
pub enum Fig<T> {
    Stated(T),
    Waits {
        /// Each thing the figure waits on, as the word `SPEC.md` lists for it.
        gaps: Vec<String>,
    },
}

impl<T: Serialize> Diff for Fig<T> {
    fn diff(&self, new: &Self, path: &mut Vec<serde_json::Value>, ops: &mut Vec<serde_json::Value>) {
        bagholder_model::patch::leaf(self, new, path, ops)
    }
}

impl<T> Fig<T> {
    /// A figure from the engine's, its value made the wire's.
    pub fn of<U>(f: &bagholder_engine::gap::Fig<U>, value: impl FnOnce(&U) -> T) -> Fig<T> {
        match f {
            Ok(u) => Fig::Stated(value(u)),
            Err(g) => Fig::Waits { gaps: g.words().into_iter().map(String::from).collect() },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::figures::*;
    use bagholder_model::patch::Diff;

    fn key<T: Diff>() -> Option<&'static str> {
        T::KEY
    }

    /// Every list of rows on the wire is told apart by a field its row type
    /// declares; a list of rows without one would be matched by guessing.
    #[test]
    fn every_list_of_rows_declares_its_key() {
        let keyed: Vec<(&str, Option<&str>)> = vec![
            ("Trade", key::<Trade>()),
            ("Position", key::<Position>()),
            ("Fill", key::<Fill>()),
            ("Point", key::<Point>()),
            ("YearRow", key::<YearRow>()),
            ("MonthlyBar", key::<MonthlyBar>()),
            ("BySymbolRow", key::<BySymbolRow>()),
            ("GradeBucket", key::<GradeBucket>()),
            ("QueueRow", key::<QueueRow>()),
            ("Slice", key::<Slice>()),
            ("Account", key::<Account>()),
            ("CashflowTile", key::<CashflowTile>()),
            ("CashflowMonth", key::<CashflowMonth>()),
            ("CashflowHolding", key::<CashflowHolding>()),
            ("CashflowRow", key::<CashflowRow>()),
            ("AccountOption", key::<AccountOption>()),
            ("InstrumentOption", key::<InstrumentOption>()),
        ];
        for (name, k) in &keyed {
            assert!(k.is_some(), "{name} is a list's row and declares no key");
        }
        // every list of a struct on the wire is one of these
        let source = include_str!("figures.rs");
        let list = regex::Regex::new(r"Vec<([A-Z]\w*)>").unwrap();
        for c in list.captures_iter(source) {
            let row = &c[1];
            if row == "String" {
                continue;
            }
            assert!(keyed.iter().any(|(n, _)| *n == row), "Vec<{row}> in figures.rs: add its key and list it here");
        }
    }
}
