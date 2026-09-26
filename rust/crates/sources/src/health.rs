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
use bagholder_core::SourceName;

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

/// Each source by the name a person knows it by, as the header says it.
pub const LABELS: [(&str, &str); 26] = [
    (crate::adapters::boc::DAILY, "The Bank of Canada"),
    (crate::adapters::boc::NOON_SOURCE, "The Bank of Canada's noon archive"),
    (crate::adapters::holidays::SOURCE, "The Bank of Canada's holiday page"),
    (crate::adapters::statcan::SOURCE, "Statistics Canada"),
    (crate::adapters::tmx::SOURCE, "TMX Money"),
    (crate::adapters::yahoo::SOURCE, "Yahoo Finance"),
    (crate::adapters::cboe_ca::SOURCE, "Cboe Canada"),
    (crate::adapters::cboe_options::SOURCE, "Cboe's option chains"),
    (crate::adapters::coinbase::SPOT_SOURCE, "Coinbase"),
    (crate::adapters::coinbase::EXCHANGE_SOURCE, "Coinbase Exchange"),
    (crate::payers::companies::SOURCE, "Newswire.ca"),
    (crate::payers::bmo::SOURCE, "BMO ETFs"),
    (crate::payers::evolve::SOURCE, "Evolve ETFs"),
    (crate::payers::fidelity::SOURCE, "Fidelity Canada"),
    (crate::payers::globalx::SOURCE, "Global X"),
    (crate::payers::goldman::SOURCE, "Goldman Sachs Asset Management"),
    (crate::payers::hamilton::SOURCE, "Hamilton ETFs"),
    (crate::payers::harvest::SOURCE, "Harvest ETFs"),
    (crate::payers::ishares_ca::SOURCE, "iShares Canada"),
    (crate::payers::ishares_us::SOURCE, "iShares"),
    (crate::payers::ninepoint::SOURCE, "Ninepoint"),
    (crate::payers::purpose::SOURCE, "Purpose Investments"),
    (crate::payers::vanguard_ca::SOURCE, "Vanguard Canada"),
    (crate::payers::vanguard_us::SOURCE, "Vanguard"),
    (crate::payers::us_pages::YIELDMAX, "YieldMax"),
    (crate::payers::us_pages::DEFIANCE, "Defiance ETFs"),
];

/// A source by the name a person knows it by; its own name where none is
/// listed (a test holds every source this crate asks to a listed one).
pub fn label(source: &SourceName) -> &str {
    LABELS.iter().find(|(s, _)| *s == source.as_str()).map_or(source.as_str(), |(_, l)| l)
}

/// Each source failing now, in plain words, one sentence each, in the order
/// given: a source whose newest counted outcome (`MarketCache::newest_counted`)
/// is a refusal or a failure. It stays failing until that source next answers.
pub fn failures(newest: &[OutcomeRow]) -> Vec<String> {
    newest.iter().filter_map(failure).collect()
}

/// A failed outcome as one sentence naming the source and what failed, in the
/// words the trade chart uses for a failed read
/// (`bagholder_market::http::describe_failure`): `TMX Money could not be
/// reached.` None for an outcome that is not a failure of its source.
pub fn failure(o: &OutcomeRow) -> Option<String> {
    let what = match o.outcome {
        // a refusal's status is written into its detail by `Outcome::detail`
        OutcomeKind::Refused if o.detail.contains("status 429") => "refused the request (too many)",
        OutcomeKind::Refused => "refused the request",
        OutcomeKind::Unreachable => "could not be reached",
        OutcomeKind::Mismatch => "answered in a form Bagholder cannot read",
        OutcomeKind::Meaning => "answered with data that cannot be right",
        OutcomeKind::Answered | OutcomeKind::NotCarried => return None,
    };
    Some(format!("{} {what}.", label(&o.source)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::DataKind;

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

    #[test]
    fn every_source_asked_has_a_name_a_person_knows() {
        use crate::adapters::*;
        let mut asked = vec![boc::daily_source(), boc::noon_source(), holidays::source(), statcan::source(), tmx::source(), yahoo::source(), cboe_ca::source(), cboe_options::source(), coinbase::spot_source(), coinbase::exchange_source()];
        asked.extend(crate::payers::all().iter().map(|p| p.source()));
        for s in &asked {
            assert!(LABELS.iter().any(|(n, _)| *n == s.as_str()), "{s} has no name in LABELS");
        }
        let names: std::collections::BTreeSet<&str> = LABELS.iter().map(|(n, _)| *n).collect();
        let labels: std::collections::BTreeSet<&str> = LABELS.iter().map(|(_, l)| *l).collect();
        assert_eq!((names.len(), labels.len()), (LABELS.len(), LABELS.len()), "each source and each name listed once");
    }

    fn outcome_row<A>(source: &'static str, o: &crate::outcome::Outcome<A>) -> OutcomeRow {
        OutcomeRow { source: SourceName::named(source), outcome: o.kind(), detail: o.detail(), ..row(OutcomeKind::Answered, NOW, false) }
    }

    #[test]
    fn a_failure_is_said_by_its_source_and_what_failed() {
        use crate::outcome::Outcome;
        use std::time::Duration;
        for (name, _) in LABELS {
            let refused = |status| outcome_row::<()>(name, &Outcome::Refused { status, retry_after: Some(Duration::from_secs(30)) });
            let source = SourceName::named(name);
            let label = label(&source);
            assert_eq!(failure(&refused(Some(429))), Some(format!("{label} refused the request (too many).")));
            assert_eq!(failure(&refused(Some(503))), Some(format!("{label} refused the request.")));
            assert_eq!(failure(&outcome_row::<()>(name, &Outcome::Refused { status: None, retry_after: None })), Some(format!("{label} refused the request.")));
            assert_eq!(failure(&outcome_row::<()>(name, &Outcome::Unreachable("timed out".into()))), Some(format!("{label} could not be reached.")));
            assert_eq!(failure(&outcome_row::<()>(name, &Outcome::Meaning("a negative rate".into()))), Some(format!("{label} answered with data that cannot be right.")));
            assert!(failure(&outcome_row(name, &Outcome::Answered(()))).is_none());
            assert!(failure(&outcome_row::<()>(name, &Outcome::NotCarried("no such series".into()))).is_none());
        }
        // one sentence per failing source, in the order given; an answering one says nothing
        let (a, b, c) = (LABELS[0].0, LABELS[1].0, LABELS[2].0);
        let newest = [outcome_row(a, &Outcome::<()>::Unreachable("reset".into())), outcome_row(b, &Outcome::Answered(())), outcome_row::<()>(c, &Outcome::Refused { status: Some(429), retry_after: None })];
        assert_eq!(failures(&newest), vec![format!("{} could not be reached.", LABELS[0].1), format!("{} refused the request (too many).", LABELS[2].1)]);
    }
}
