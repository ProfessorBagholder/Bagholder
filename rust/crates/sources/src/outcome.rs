//! What asking a source came to (`docs/plans/stage-3a-sources.md`, "The
//! contract"). Every request ends in exactly one of these, and every one is
//! recorded: a failure is never a silent empty answer.

use std::fmt;
use std::time::Duration;

use bagholder_net::NetError;

use crate::reply::{Mismatch, ShapeChange};

/// The result of one request to a source.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome<A> {
    Answered(A),
    /// The source answered that it does not carry what was asked (an unknown
    /// symbol, a series it does not publish). Not a failure of the source.
    NotCarried(String),
    /// The source refused (429, or 503 with a `Retry-After`), or is resting
    /// after a refusal and was not asked.
    Refused { status: Option<u16>, retry_after: Option<Duration> },
    /// The source could not be reached, or answered with a status that is no
    /// answer (a 5xx, a 403 challenge page).
    Unreachable(String),
    /// A field the adapter needs is absent, null or of another type.
    Mismatch(Mismatch),
    /// The reply is well formed and says something that cannot be so (another
    /// series than asked, a day outside the span, a negative rate).
    Meaning(String),
}

/// The kind of an outcome, as it is stored and counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OutcomeKind {
    Answered,
    NotCarried,
    Refused,
    Unreachable,
    Mismatch,
    Meaning,
}

impl OutcomeKind {
    pub const ALL: [OutcomeKind; 6] = [OutcomeKind::Answered, OutcomeKind::NotCarried, OutcomeKind::Refused, OutcomeKind::Unreachable, OutcomeKind::Mismatch, OutcomeKind::Meaning];

    pub fn as_str(self) -> &'static str {
        match self {
            OutcomeKind::Answered => "answered",
            OutcomeKind::NotCarried => "not-carried",
            OutcomeKind::Refused => "refused",
            OutcomeKind::Unreachable => "unreachable",
            OutcomeKind::Mismatch => "mismatch",
            OutcomeKind::Meaning => "meaning",
        }
    }

    pub fn parse(s: &str) -> Option<OutcomeKind> {
        OutcomeKind::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// Whether this outcome is a failure of the source.
    pub fn is_failure(self) -> bool {
        matches!(self, OutcomeKind::Unreachable | OutcomeKind::Mismatch | OutcomeKind::Meaning)
    }
}

impl fmt::Display for OutcomeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<A> Outcome<A> {
    pub fn kind(&self) -> OutcomeKind {
        match self {
            Outcome::Answered(_) => OutcomeKind::Answered,
            Outcome::NotCarried(_) => OutcomeKind::NotCarried,
            Outcome::Refused { .. } => OutcomeKind::Refused,
            Outcome::Unreachable(_) => OutcomeKind::Unreachable,
            Outcome::Mismatch(_) => OutcomeKind::Mismatch,
            Outcome::Meaning(_) => OutcomeKind::Meaning,
        }
    }

    /// What the outcome says beyond its kind, for the record.
    pub fn detail(&self) -> String {
        match self {
            Outcome::Answered(_) => String::new(),
            Outcome::NotCarried(why) | Outcome::Unreachable(why) | Outcome::Meaning(why) => why.clone(),
            Outcome::Refused { status, retry_after } => {
                let mut s = status.map_or_else(|| "resting after a refusal".to_string(), |c| format!("status {c}"));
                if let Some(d) = retry_after {
                    s.push_str(&format!(", retry after {}s", d.as_secs()));
                }
                s
            }
            Outcome::Mismatch(m) => m.to_string(),
        }
    }

    pub fn map<B>(self, f: impl FnOnce(A) -> B) -> Outcome<B> {
        match self {
            Outcome::Answered(a) => Outcome::Answered(f(a)),
            Outcome::NotCarried(w) => Outcome::NotCarried(w),
            Outcome::Refused { status, retry_after } => Outcome::Refused { status, retry_after },
            Outcome::Unreachable(w) => Outcome::Unreachable(w),
            Outcome::Mismatch(m) => Outcome::Mismatch(m),
            Outcome::Meaning(w) => Outcome::Meaning(w),
        }
    }

    /// The same outcome carrying another answer type, for one that is not answered.
    pub fn failed<B>(self) -> Option<Outcome<B>> {
        match self {
            Outcome::Answered(_) => None,
            other => Some(other.map(|_| unreachable!())),
        }
    }
}

impl<A> From<Mismatch> for Outcome<A> {
    fn from(m: Mismatch) -> Self {
        Outcome::Mismatch(m)
    }
}

impl<A> From<NetError> for Outcome<A> {
    fn from(e: NetError) -> Self {
        match e {
            NetError::Resting { .. } => Outcome::Refused { status: None, retry_after: None },
            NetError::Unreachable(why) => Outcome::Unreachable(why),
        }
    }
}

/// An outcome with the shape change noticed beside it, as it is recorded.
#[derive(Clone, Debug, PartialEq)]
pub struct Noted<A> {
    pub outcome: Outcome<A>,
    pub shape_change: Option<ShapeChange>,
}
