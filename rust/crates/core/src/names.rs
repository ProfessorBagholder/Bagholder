//! Short names that are part of the vocabulary: a broker (`wealthsimple`), a
//! source of records (`wealthsimple`, `bagholder-import`, `csv`, `person`).
//! Lower-case letters, digits and hyphens, so they read the same in the book, in
//! a reference's scheme and in a message.

use std::fmt;

use crate::ids::IdError;

fn slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

macro_rules! slug_name {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn parse(s: &str) -> Result<Self, IdError> {
                if !slug(s) {
                    return Err(IdError(s.to_string()));
                }
                Ok($name(s.to_string()))
            }

            /// A name known when the program is written. Panics on a malformed
            /// one: a fault in the code, found by its tests.
            pub fn named(s: &'static str) -> Self {
                $name::parse(s).unwrap_or_else(|_| panic!("malformed name {s:?}"))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0)
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

slug_name!(
    /// A brokerage: `wealthsimple`.
    Broker
);
slug_name!(
    /// Where records come from: a broker, a file format, the import of an
    /// earlier database, or the person.
    SourceName
);

impl SourceName {
    /// What the person entered themselves.
    pub fn person() -> SourceName {
        SourceName::named("person")
    }
}

/// A mapping's source and version: which rules turned a record into transactions.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct MappingVersion {
    pub source: SourceName,
    pub version: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_slugs() {
        assert_eq!(SourceName::parse("bagholder-import").unwrap().as_str(), "bagholder-import");
        for bad in ["", "Wealthsimple", "ws_1", "-x", "x-", "a b", &"x".repeat(41)] {
            assert!(Broker::parse(bad).is_err(), "{bad:?}");
        }
    }
}
