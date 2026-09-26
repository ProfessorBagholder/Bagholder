//! The ids Bagholder assigns. Each is a UUID v7, made by the book when a thing
//! is first stored and never changed; each kind of thing has its own type, so an
//! account's id can never be passed where an instrument's is meant.
//!
//! A transaction is the exception: its id is its record's id and the name of its
//! leg (`TransactionId`), so deriving a record's transactions again gives them the
//! same ids.

use std::fmt;
use std::str::FromStr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdError(pub String);

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not an id: {:?}", self.0)
    }
}

impl std::error::Error for IdError {}

macro_rules! id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(uuid::Uuid);

        impl $name {
            pub fn from_uuid(u: uuid::Uuid) -> Self {
                $name(u)
            }

            pub fn uuid(&self) -> uuid::Uuid {
                self.0
            }

            /// The hyphenated lower-case form, and nothing else.
            pub fn parse(s: &str) -> Result<Self, IdError> {
                let u = uuid::Uuid::try_parse(s).map_err(|_| IdError(s.to_string()))?;
                if u.hyphenated().to_string() != s {
                    return Err(IdError(s.to_string()));
                }
                Ok($name(u))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0.hyphenated())
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0.hyphenated())
            }
        }

        impl FromStr for $name {
            type Err = IdError;
            fn from_str(s: &str) -> Result<Self, IdError> {
                $name::parse(s)
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                $name::parse(&s).map_err(serde::de::Error::custom)
            }
        }
    };
}

id!(
    /// An instrument: a listing, an option contract, a coin, an event contract,
    /// an index, a future, a rate or a currency pair.
    InstrumentId
);
id!(
    /// A company or fund, above its listings.
    IssuerId
);
id!(
    /// One login at one broker.
    ConnectionId
);
id!(
    /// An account, whatever currencies it holds.
    AccountId
);
id!(
    /// One thing a source reported, across its revisions.
    RecordId
);
id!(
    /// A round trip, from its opening transaction.
    TradeId
);
id!(
    /// A group of trades the person made.
    GroupId
);
id!(
    /// A link between records: one superseding others.
    LinkId
);

/// What a leg of a record is (`trade`, `out`, `in`, `fee`): the mapping's name for
/// it, never its position, so a mapping that adds a leg does not move the others.
/// Lower-case letters and hyphens, at most 24.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Leg(String);

impl Leg {
    pub fn parse(s: &str) -> Result<Leg, IdError> {
        let ok = !s.is_empty()
            && s.len() <= 24
            && s.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
            && !s.starts_with('-')
            && !s.ends_with('-');
        if !ok {
            return Err(IdError(s.to_string()));
        }
        Ok(Leg(s.to_string()))
    }

    /// A leg name known when the program is written (`Leg::named("trade")`).
    /// Panics on a malformed name: a fault in the code, found by its tests.
    pub fn named(s: &'static str) -> Leg {
        Leg::parse(s).unwrap_or_else(|_| panic!("malformed leg name {s:?}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Leg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Leg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Leg({})", self.0)
    }
}

impl serde::Serialize for Leg {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> serde::Deserialize<'de> for Leg {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Leg, D::Error> {
        let s = String::deserialize(d)?;
        Leg::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// A transaction: its record and its leg. Written `<record id>/<leg>`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TransactionId {
    pub record: RecordId,
    pub leg: Leg,
}

impl TransactionId {
    pub fn new(record: RecordId, leg: Leg) -> TransactionId {
        TransactionId { record, leg }
    }

    pub fn parse(s: &str) -> Result<TransactionId, IdError> {
        let (record, leg) = s.split_once('/').ok_or_else(|| IdError(s.to_string()))?;
        Ok(TransactionId { record: RecordId::parse(record)?, leg: Leg::parse(leg)? })
    }
}

impl fmt::Display for TransactionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.record, self.leg)
    }
}

impl fmt::Debug for TransactionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TransactionId({self})")
    }
}

impl serde::Serialize for TransactionId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for TransactionId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<TransactionId, D::Error> {
        let s = String::deserialize(d)?;
        TransactionId::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const U: &str = "01923e6a-7b1c-7def-8123-456789abcdef";

    #[test]
    fn ids_read_only_their_canonical_form() {
        assert_eq!(RecordId::parse(U).unwrap().to_string(), U);
        for bad in ["01923E6A-7B1C-7DEF-8123-456789ABCDEF", "01923e6a7b1c7def8123456789abcdef", "{01923e6a-7b1c-7def-8123-456789abcdef}", "", "x"] {
            assert!(RecordId::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_transaction_id_is_its_record_and_leg() {
        let t = TransactionId::parse(&format!("{U}/trade")).unwrap();
        assert_eq!(t.leg, Leg::named("trade"));
        assert_eq!(t.to_string(), format!("{U}/trade"));
        assert_eq!(serde_json::to_string(&t).unwrap(), format!("\"{U}/trade\""));
        for bad in [U.to_string(), format!("{U}/"), format!("{U}/Trade"), format!("{U}/-x"), format!("{U}/{}", "a".repeat(25))] {
            assert!(TransactionId::parse(&bad).is_err(), "{bad:?}");
        }
    }
}
