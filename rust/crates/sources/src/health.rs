//! A source's health, a pure function of its own recorded outcomes
//! (`docs/plans/stage-3a-sources.md`, "The contract", Health).
//!
//! The window is the source's last ten outcomes or its last day of them,
//! whichever holds more. "Not carried" is the source answering that it has no
//! such thing, which says nothing about its health, so it is not counted. Of the
//! rest, the newest decides: a refusal is `refusing`; a failure (unreachable, a
//! mismatch, a meaning failure) is `failing`; an answer is `working`, or
//! `shape-changed` while any answer in the window came in a shape the recorded
//! replies do not have. A source with nothing counted in the window has not been
//! asked anything that says how it is.

use std::collections::BTreeMap;
use std::fmt;

use bagholder_core::jiff::{SignedDuration, Timestamp};

use crate::cache::OutcomeRow;
use crate::outcome::OutcomeKind;

/// How many outcomes the window holds at least.
pub const WINDOW_COUNT: usize = 10;
/// How far back the window reaches at least.
pub const WINDOW_SPAN: SignedDuration = SignedDuration::from_hours(24);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Working,
    Refusing,
    Failing,
    ShapeChanged,
    /// Nothing in the window says how the source is.
    Unasked,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Working => "working",
            State::Refusing => "refusing",
            State::Failing => "failing",
            State::ShapeChanged => "shape-changed",
            State::Unasked => "unasked",
        }
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The outcomes the window holds, from `outcomes` newest first.
pub fn window(outcomes: &[OutcomeRow], now: Timestamp) -> &[OutcomeRow] {
    let since = now - WINDOW_SPAN;
    let in_day = outcomes.iter().take_while(|o| o.at >= since).count();
    &outcomes[..in_day.max(WINDOW_COUNT.min(outcomes.len()))]
}

/// A source's state from its outcomes, newest first, at `now`.
pub fn state(outcomes: &[OutcomeRow], now: Timestamp) -> State {
    let w = window(outcomes, now);
    let Some(newest) = w.iter().find(|o| o.outcome != OutcomeKind::NotCarried) else {
        return State::Unasked;
    };
    match newest.outcome {
        OutcomeKind::Refused => State::Refusing,
        k if k.is_failure() => State::Failing,
        _ if w.iter().any(|o| o.outcome == OutcomeKind::Answered && o.shape_change.is_some()) => State::ShapeChanged,
        _ => State::Working,
    }
}

/// The newest outcome of each kind a source has recorded.
pub fn last_of_each(outcomes: &[OutcomeRow]) -> BTreeMap<OutcomeKind, &OutcomeRow> {
    let mut out = BTreeMap::new();
    for o in outcomes {
        out.entry(o.outcome).or_insert(o);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::DataKind;
    use bagholder_core::SourceName;

    fn at(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    fn row(kind: OutcomeKind, when: &str, shape: bool) -> OutcomeRow {
        OutcomeRow {
            source: SourceName::named("tmx"),
            host: "app-money.tmx.com".into(),
            kind: DataKind::Quote,
            instrument: None,
            outcome: kind,
            detail: String::new(),
            shape_change: shape.then(|| "new: data.x".to_string()),
            at: at(when),
        }
    }

    const NOW: &str = "2026-09-24T12:00:00Z";

    #[test]
    fn the_newest_counted_outcome_decides() {
        use OutcomeKind::*;
        let now = at(NOW);
        assert_eq!(state(&[row(Answered, "2026-09-24T11:00:00Z", false), row(Unreachable, "2026-09-24T10:00:00Z", false)], now), State::Working);
        assert_eq!(state(&[row(Refused, "2026-09-24T11:00:00Z", false), row(Answered, "2026-09-24T10:00:00Z", false)], now), State::Refusing);
        for failure in [Unreachable, Mismatch, Meaning] {
            assert_eq!(state(&[row(failure, "2026-09-24T11:00:00Z", false), row(Answered, "2026-09-24T10:00:00Z", false)], now), State::Failing);
        }
    }

    #[test]
    fn not_carried_says_nothing_of_health() {
        use OutcomeKind::*;
        let now = at(NOW);
        assert_eq!(state(&[row(NotCarried, "2026-09-24T11:00:00Z", false), row(Mismatch, "2026-09-24T10:00:00Z", false)], now), State::Failing);
        assert_eq!(state(&[row(NotCarried, "2026-09-24T11:00:00Z", false)], now), State::Unasked);
        assert_eq!(state(&[], now), State::Unasked);
    }

    #[test]
    fn an_answer_in_a_new_shape_marks_the_window() {
        use OutcomeKind::*;
        let now = at(NOW);
        let rows = [row(Answered, "2026-09-24T11:00:00Z", false), row(Answered, "2026-09-24T10:00:00Z", true)];
        assert_eq!(state(&rows, now), State::ShapeChanged);
        // two days on, and past ten newer outcomes, it has left the window
        let mut later: Vec<OutcomeRow> = (0..10).map(|h| row(Answered, &format!("2026-09-26T{:02}:00:00Z", 11 - h), false)).collect();
        later.extend(rows);
        assert_eq!(state(&later, at("2026-09-26T12:00:00Z")), State::Working);
    }

    #[test]
    fn the_window_is_ten_or_a_day_whichever_holds_more() {
        use OutcomeKind::*;
        let now = at(NOW);
        let busy: Vec<OutcomeRow> = (0..30).map(|m| row(Answered, &format!("2026-09-24T11:{:02}:00Z", 59 - m), false)).collect();
        assert_eq!(window(&busy, now).len(), 30);
        let quiet: Vec<OutcomeRow> = (0..15).map(|d| row(Answered, &format!("2026-08-{:02}T11:00:00Z", 28 - d), false)).collect();
        assert_eq!(window(&quiet, now).len(), 10);
    }
}
