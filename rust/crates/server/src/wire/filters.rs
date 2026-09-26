//! The filters the page sends (`SPEC.md` §5), read strictly into the engine's
//! (`bagholder_engine::scope::Filters`). Accounts and instruments are named by
//! their ids, never by a name or a symbol another could share; a value the
//! filters do not know is refused, naming it, never dropped.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use ts_rs::TS;

use bagholder_core::journal::Grade;
use bagholder_core::{AccountId, InstrumentId};
use bagholder_engine::ledger::Direction;
use bagholder_engine::scope::{self, Bound, Dates, Outcome, Preset};

/// A range filter: kept above or below `v`, or not set.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct Range {
    /// `>` or `<`.
    pub op: String,
    /// The bound as decimal text, or none.
    #[ts(type = "Dec | null")]
    pub v: Option<String>,
}

/// The filters, as the page keeps them.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, TS)]
#[serde(default, deny_unknown_fields)]
pub struct Filters {
    /// `account` (account ids), `symbol` (instrument ids), `grade`, `tag`, `kind`,
    /// `exchange`, `side`, `result`.
    pub lists: BTreeMap<String, Vec<String>>,
    /// `price`, `hold`, `pnl`, `qty`.
    pub ranges: BTreeMap<String, Range>,
    pub preset: String,
    pub years: Vec<String>,
    pub from: String,
    pub to: String,
    pub search: String,
    pub benchmark: String,
}

fn preset(p: &str) -> Result<Option<Preset>, String> {
    Ok(Some(match p {
        "" | "all" => return Ok(None),
        "1d" => Preset::Day,
        "1w" => Preset::Week,
        "1m" => Preset::Month,
        "3m" => Preset::Quarter,
        "6m" => Preset::HalfYear,
        "ytd" => Preset::YearToDate,
        "1y" => Preset::Year,
        "5y" => Preset::FiveYears,
        other => return Err(format!("no date preset {other:?}")),
    }))
}

fn day(s: &str) -> Result<Option<bagholder_core::jiff::civil::Date>, String> {
    if s.is_empty() {
        return Ok(None);
    }
    s.parse().map(Some).map_err(|e| format!("{s:?} is not a day: {e}"))
}

fn bound(key: &str, r: &Range) -> Result<Option<Bound>, String> {
    let Some(v) = r.v.as_deref() else { return Ok(None) };
    let v = bagholder_core::Dec::parse(v).map_err(|e| format!("the {key} filter's {v:?} is not a decimal: {e}"))?;
    match r.op.as_str() {
        ">" => Ok(Some(Bound::Above(v))),
        "<" => Ok(Some(Bound::Below(v))),
        op => Err(format!("the {key} filter's comparison {op:?} is neither > nor <")),
    }
}

impl Filters {
    /// The engine's filters, or what in these the filters do not know.
    pub fn to_engine(&self) -> Result<scope::Filters, String> {
        let mut f = scope::Filters { benchmark: if self.benchmark.is_empty() { "SP500".into() } else { self.benchmark.clone() }, search: self.search.trim().to_string(), ..Default::default() };
        let from = day(&self.from)?;
        let to = day(&self.to)?;
        f.dates = if from.is_some() || to.is_some() {
            Dates::Range { from, to }
        } else if !self.years.is_empty() {
            Dates::Years(self.years.iter().map(|y| y.parse::<i16>().map_err(|e| format!("{y:?} is not a year: {e}"))).collect::<Result<BTreeSet<_>, _>>()?)
        } else {
            match preset(&self.preset)? {
                Some(p) => Dates::Preset(p),
                None => Dates::All,
            }
        };
        for (key, values) in &self.lists {
            for v in values {
                match key.as_str() {
                    "account" => {
                        f.accounts.insert(AccountId::parse(v).map_err(|e| format!("account {v:?}: {e}"))?);
                    }
                    "symbol" => {
                        f.instruments.insert(InstrumentId::parse(v).map_err(|e| format!("instrument {v:?}: {e}"))?);
                    }
                    "grade" => {
                        f.grades.insert(match v.as_str() {
                            "Ungraded" => None,
                            g => Some(Grade::parse(g).map_err(|e| format!("grade {g:?}: {e}"))?),
                        });
                    }
                    "tag" => {
                        f.tags.insert(v.clone());
                    }
                    "kind" => {
                        let kind = super::build::KIND_WORDS.iter().find(|(_, w)| w == v).map(|(k, _)| *k).ok_or_else(|| format!("no kind {v:?}"))?;
                        f.kinds.insert(kind);
                    }
                    "exchange" => {
                        f.venues.insert(v.clone());
                    }
                    "side" => {
                        f.sides.insert(match v.as_str() {
                            "SELL" => Direction::Long,
                            "COVER" => Direction::Short,
                            other => return Err(format!("no side {other:?}")),
                        });
                    }
                    "result" => {
                        f.outcomes.insert(match v.as_str() {
                            "Winners" => Outcome::Win,
                            "Losers" => Outcome::Loss,
                            "Breakeven" => Outcome::Breakeven,
                            other => return Err(format!("no result {other:?}")),
                        });
                    }
                    other => return Err(format!("no filter {other:?}")),
                }
            }
        }
        for (key, r) in &self.ranges {
            let b = bound(key, r)?;
            match key.as_str() {
                "price" => f.price = b,
                "hold" => f.hold = b,
                "pnl" => f.pnl = b,
                "qty" => f.qty = b,
                other => return Err(format!("no range filter {other:?}")),
            }
        }
        Ok(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_filters_do_not_know_is_refused_by_name() {
        let parse = |s: &str| serde_json::from_str::<Filters>(s).map_err(|e| e.to_string()).and_then(|f| f.to_engine());
        assert!(parse(r#"{"preset":"ytd"}"#).is_ok());
        assert!(parse(r#"{"lists":{"account":["TFSA"]}}"#).unwrap_err().contains("TFSA"), "a name is not an id");
        assert!(parse(r#"{"lists":{"colour":["red"]}}"#).unwrap_err().contains("colour"));
        assert!(parse(r#"{"preset":"2w"}"#).unwrap_err().contains("2w"));
        assert!(parse(r#"{"ranges":{"pnl":{"op":">","v":"1e3"}}}"#).unwrap_err().contains("1e3"));
        assert!(parse(r#"{"surprise":1}"#).is_err(), "an unknown field");
        let f = parse(r#"{"lists":{"account":["0192a000-0000-7000-8000-000000000001"],"grade":["A","Ungraded"],"kind":["Options"]},"ranges":{"pnl":{"op":"<","v":"-100.5"}},"years":["2025"]}"#).unwrap();
        assert_eq!(f.accounts.len(), 1);
        assert_eq!(f.grades.len(), 2);
        assert_eq!(f.pnl, Some(Bound::Below(bagholder_core::Dec::parse("-100.5").unwrap())));
        assert_eq!(f.dates, Dates::Years([2025].into()));
    }
}
