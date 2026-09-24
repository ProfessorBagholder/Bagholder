//! Currencies and amounts of money.
//!
//! An amount always carries its currency, and two currencies are never added:
//! `Money` has no `+` or `-`, only `checked_add`, which refuses a second currency.
//! Converting between currencies takes a rate, which the engine records as a
//! fact of the figure it computes (stage 2).

use std::fmt;
use std::str::FromStr;

use crate::dec::{Dec, DecError};

/// An ISO 4217 currency code: three capital letters. Any currency, not only the
/// two the first broker uses; a coin is an instrument, not a currency.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Currency([u8; 3]);

impl Currency {
    pub const CAD: Currency = Currency(*b"CAD");
    pub const USD: Currency = Currency(*b"USD");

    pub fn parse(s: &str) -> Result<Currency, MoneyError> {
        let b = s.as_bytes();
        if b.len() != 3 || !b.iter().all(u8::is_ascii_uppercase) {
            return Err(MoneyError::Currency(s.to_string()));
        }
        Ok(Currency([b[0], b[1], b[2]]))
    }

    /// Places of the currency's minor unit, per ISO 4217 (list one): cash in it
    /// is stated to these places.
    pub fn minor_units(self) -> u32 {
        match &self.0 {
            b"BIF" | b"CLP" | b"DJF" | b"GNF" | b"ISK" | b"JPY" | b"KMF" | b"KRW" | b"PYG" | b"RWF" | b"UGX" | b"UYI" | b"VND" | b"VUV" | b"XAF" | b"XOF" | b"XPF" => 0,
            b"BHD" | b"IQD" | b"JOD" | b"KWD" | b"LYD" | b"OMR" | b"TND" => 3,
            b"CLF" | b"UYW" => 4,
            _ => 2,
        }
    }

    pub fn as_str(&self) -> &str {
        // three ASCII capitals, checked on the way in
        std::str::from_utf8(&self.0).unwrap_or("???")
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Currency {
    type Err = MoneyError;
    fn from_str(s: &str) -> Result<Currency, MoneyError> {
        Currency::parse(s)
    }
}

impl serde::Serialize for Currency {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for Currency {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Currency, D::Error> {
        let s = String::deserialize(d)?;
        Currency::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoneyError {
    /// Not an ISO 4217 code.
    Currency(String),
    /// Two currencies in one sum.
    Mismatch { left: Currency, right: Currency },
    Dec(DecError),
}

impl fmt::Display for MoneyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MoneyError::Currency(s) => write!(f, "not a currency code: {s:?}"),
            MoneyError::Mismatch { left, right } => write!(f, "{left} and {right} added together"),
            MoneyError::Dec(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for MoneyError {}

impl From<DecError> for MoneyError {
    fn from(e: DecError) -> Self {
        MoneyError::Dec(e)
    }
}

/// An amount in a currency, signed: into the account is positive.
///
/// ```compile_fail
/// use bagholder_core::{Currency, Dec, Money};
/// let a = Money::new(Dec::ONE, Currency::CAD);
/// let _ = a + a; // two amounts are added only with `checked_add`
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Money {
    pub amount: Dec,
    pub currency: Currency,
}

impl Money {
    pub fn new(amount: Dec, currency: Currency) -> Money {
        Money { amount, currency }
    }

    pub fn zero(currency: Currency) -> Money {
        Money { amount: Dec::ZERO, currency }
    }

    /// The sum, or an error when the currencies differ or the sum is not exact.
    pub fn checked_add(self, other: Money) -> Result<Money, MoneyError> {
        if self.currency != other.currency {
            return Err(MoneyError::Mismatch { left: self.currency, right: other.currency });
        }
        Ok(Money { amount: self.amount.checked_add(other.amount)?, currency: self.currency })
    }

    /// A term added to a total, as [`Dec::add_to_fit`]; an error when the
    /// currencies differ or the sum is too large to hold.
    pub fn add_to_fit(self, other: Money) -> Result<Money, MoneyError> {
        if self.currency != other.currency {
            return Err(MoneyError::Mismatch { left: self.currency, right: other.currency });
        }
        Ok(Money { amount: self.amount.add_to_fit(other.amount)?, currency: self.currency })
    }

    pub fn checked_sub(self, other: Money) -> Result<Money, MoneyError> {
        self.checked_add(other.neg())
    }

    /// Every amount in `items` summed in `currency`; one in another currency is
    /// an error, never converted and never skipped.
    pub fn sum(currency: Currency, items: impl IntoIterator<Item = Money>) -> Result<Money, MoneyError> {
        items.into_iter().try_fold(Money::zero(currency), Money::checked_add)
    }

    /// This amount times a number: a price times a quantity, a rate times a balance.
    pub fn times(self, n: Dec) -> Result<Money, MoneyError> {
        Ok(Money { amount: self.amount.checked_mul(n)?, currency: self.currency })
    }

    pub fn neg(self) -> Money {
        Money { amount: self.amount.neg(), currency: self.currency }
    }

    pub fn abs(self) -> Money {
        Money { amount: self.amount.abs(), currency: self.currency }
    }
}

impl fmt::Debug for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.amount, self.currency)
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.amount, self.currency)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cad(s: &str) -> Money {
        Money::new(Dec::parse(s).unwrap(), Currency::CAD)
    }

    #[test]
    fn a_currency_is_three_capitals() {
        assert_eq!(Currency::parse("EUR").unwrap().to_string(), "EUR");
        for bad in ["cad", "CA", "CADX", "C4D", "", "ÉUR"] {
            assert!(Currency::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn two_currencies_are_never_added() {
        let usd = Money::new(Dec::ONE, Currency::USD);
        assert_eq!(cad("1").checked_add(usd), Err(MoneyError::Mismatch { left: Currency::CAD, right: Currency::USD }));
        assert_eq!(Money::sum(Currency::CAD, [cad("1"), usd]), Err(MoneyError::Mismatch { left: Currency::CAD, right: Currency::USD }));
    }

    #[test]
    fn sums_are_exact() {
        assert_eq!(Money::sum(Currency::CAD, [cad("0.1"), cad("0.2"), cad("-0.3")]).unwrap(), cad("0"));
        assert_eq!(cad("1050").checked_sub(cad("0.01")).unwrap(), cad("1049.99"));
        assert_eq!(cad("0.3").times(Dec::parse("35").unwrap()).unwrap(), cad("10.5"));
    }

    #[test]
    fn serde_carries_both_parts_as_text() {
        let m = cad("-12.50");
        let text = serde_json::to_string(&m).unwrap();
        assert_eq!(text, r#"{"amount":"-12.5","currency":"CAD"}"#);
        assert_eq!(serde_json::from_str::<Money>(&text).unwrap(), m);
        assert!(serde_json::from_str::<Money>(r#"{"amount":"1","currency":"cad"}"#).is_err());
    }
}
