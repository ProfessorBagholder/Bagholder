//! Enums that are stored and sent as a fixed word each (`buy`, `tfsa`), read back
//! strictly: a word that is not one of them is an error, never a default.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownWord {
    pub what: &'static str,
    pub word: String,
}

impl std::fmt::Display for UnknownWord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "not a {}: {:?}", self.what, self.word)
    }
}

impl std::error::Error for UnknownWord {}

/// `text_enum! { Name "what it is" { Variant = "word", … } }`: the enum, `as_str`,
/// `parse`, `ALL`, `Display` and serde as the word.
#[macro_export]
macro_rules! text_enum {
    ($(#[$doc:meta])* $name:ident $what:literal { $($(#[$vdoc:meta])* $variant:ident = $word:literal),+ $(,)? }) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $($(#[$vdoc])* $variant),+
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $word),+
                }
            }

            pub fn parse(s: &str) -> Result<$name, $crate::text_enum::UnknownWord> {
                match s {
                    $($word => Ok($name::$variant),)+
                    _ => Err($crate::text_enum::UnknownWord { what: $what, word: s.to_string() }),
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<$name, D::Error> {
                let s = String::deserialize(d)?;
                $name::parse(&s).map_err(serde::de::Error::custom)
            }
        }
    };
}
