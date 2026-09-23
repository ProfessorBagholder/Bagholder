//! Instruments, their names, their references and their issuers (`docs/architecture.md` §5).
//!
//! An instrument is identified by Bagholder's id, never by what it is called.
//! What it is called is dated (`Name`); what each source calls it is a
//! `Reference`. Only a strong reference (or, within one connection, a scoped one)
//! says two things are the same instrument; nothing is ever matched on a bare
//! symbol.

use std::fmt;

use crate::dec::Dec;
use crate::ids::{ConnectionId, IdError, InstrumentId, IssuerId};
use crate::money::Currency;
use crate::names::{Broker, SourceName};

text_enum! {
    /// What an instrument is.
    InstrumentKind "kind of instrument" {
        /// A listed share, ETF, fund or warrant.
        Security = "security",
        OptionContract = "option",
        Crypto = "crypto",
        /// A prediction market's contract.
        EventContract = "event-contract",
        Index = "index",
        Future = "future",
        Rate = "rate",
        CurrencyPair = "currency-pair",
    }
}

text_enum! {
    OptionRight "option right" {
        Call = "call",
        Put = "put",
    }
}

/// An instrument, as the book holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instrument {
    pub id: InstrumentId,
    pub kind: InstrumentKind,
    /// The currency it is priced in.
    pub currency: Currency,
    pub issuer: Option<IssuerId>,
}

/// What an instrument was called: the first and last days its records used the
/// name, on the days the naming source files things under. One instrument's
/// names never overlap. (When a name began or ended is known only to within the
/// days between its sightings; a source that states the change's date, a
/// corporate event, says it exactly.)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Name {
    pub symbol: String,
    /// The venue's market identifier code (ISO 10383, `XTSX`), where known.
    pub venue_mic: Option<String>,
    /// The venue as the source names it (`TSX-V`), where given.
    pub venue_name: Option<String>,
    pub name: Option<String>,
    pub first_seen: jiff::civil::Date,
    pub last_seen: jiff::civil::Date,
    pub source: SourceName,
}

/// An option contract's terms, as the contract states them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptionTerms {
    pub underlying: InstrumentId,
    pub expiry: jiff::civil::Date,
    pub strike: Dec,
    pub right: OptionRight,
    /// Shares per contract, once a source states it. Empty until then, and a
    /// figure that needs it waits for it: it is never taken to be 100.
    pub multiplier: Option<Dec>,
    pub source: SourceName,
}

/// How sure a reference is of what it names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strength {
    /// Names one instrument everywhere.
    Strong,
    /// Names one instrument within one connection's records only.
    Scoped,
    /// Only says how to ask a source for an instrument; never identifies one.
    Routing,
}

/// A kind of identifier an outside party uses for an instrument.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RefScheme {
    /// A broker's own security id.
    BrokerSecurity(Broker),
    Isin,
    Cusip,
    Figi,
    /// The OCC option symbol.
    Occ,
    /// A symbol and currency as one connection's records name them, where they
    /// carry no id (a CSV without security ids): `<symbol>|<currency>`.
    ConnectionSymbol(ConnectionId),
    Yahoo,
    TmxForm,
    SecCik,
    SedarProfile,
}

impl RefScheme {
    pub fn strength(&self) -> Strength {
        match self {
            RefScheme::BrokerSecurity(_) | RefScheme::Isin | RefScheme::Cusip | RefScheme::Figi | RefScheme::Occ => Strength::Strong,
            RefScheme::ConnectionSymbol(_) => Strength::Scoped,
            RefScheme::Yahoo | RefScheme::TmxForm | RefScheme::SecCik | RefScheme::SedarProfile => Strength::Routing,
        }
    }

    /// The stored form: `broker-security:wealthsimple`, `isin`, `connection-symbol:<id>`, …
    pub fn to_text(&self) -> String {
        match self {
            RefScheme::BrokerSecurity(b) => format!("broker-security:{b}"),
            RefScheme::Isin => "isin".into(),
            RefScheme::Cusip => "cusip".into(),
            RefScheme::Figi => "figi".into(),
            RefScheme::Occ => "occ".into(),
            RefScheme::ConnectionSymbol(c) => format!("connection-symbol:{c}"),
            RefScheme::Yahoo => "yahoo".into(),
            RefScheme::TmxForm => "tmx-form".into(),
            RefScheme::SecCik => "sec-cik".into(),
            RefScheme::SedarProfile => "sedar-profile".into(),
        }
    }

    pub fn parse(s: &str) -> Result<RefScheme, IdError> {
        if let Some(b) = s.strip_prefix("broker-security:") {
            return Ok(RefScheme::BrokerSecurity(Broker::parse(b)?));
        }
        if let Some(c) = s.strip_prefix("connection-symbol:") {
            return Ok(RefScheme::ConnectionSymbol(ConnectionId::parse(c)?));
        }
        Ok(match s {
            "isin" => RefScheme::Isin,
            "cusip" => RefScheme::Cusip,
            "figi" => RefScheme::Figi,
            "occ" => RefScheme::Occ,
            "yahoo" => RefScheme::Yahoo,
            "tmx-form" => RefScheme::TmxForm,
            "sec-cik" => RefScheme::SecCik,
            "sedar-profile" => RefScheme::SedarProfile,
            _ => return Err(IdError(s.to_string())),
        })
    }
}

impl fmt::Display for RefScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_text())
    }
}

/// One outside party's identifier for an instrument.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Reference {
    pub scheme: RefScheme,
    pub value: String,
}

impl Reference {
    pub fn new(scheme: RefScheme, value: impl Into<String>) -> Reference {
        Reference { scheme, value: value.into() }
    }

    /// The scoped reference for a symbol and currency in one connection's records.
    pub fn connection_symbol(connection: ConnectionId, symbol: &str, currency: Currency) -> Reference {
        Reference::new(RefScheme::ConnectionSymbol(connection), format!("{symbol}|{currency}"))
    }

    /// Whether this reference may identify an instrument (strong or scoped).
    pub fn identifies(&self) -> bool {
        self.scheme.strength() != Strength::Routing
    }
}

/// A company or fund, above its listings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issuer {
    pub id: IssuerId,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemes_read_back_as_written() {
        let c = ConnectionId::parse("01923e6a-7b1c-7def-8123-456789abcdef").unwrap();
        for s in [
            RefScheme::BrokerSecurity(Broker::named("wealthsimple")),
            RefScheme::Isin,
            RefScheme::Cusip,
            RefScheme::Figi,
            RefScheme::Occ,
            RefScheme::ConnectionSymbol(c),
            RefScheme::Yahoo,
            RefScheme::TmxForm,
            RefScheme::SecCik,
            RefScheme::SedarProfile,
        ] {
            assert_eq!(RefScheme::parse(&s.to_text()).unwrap(), s);
        }
        assert!(RefScheme::parse("ticker").is_err());
        assert!(RefScheme::parse("broker-security:").is_err());
    }

    #[test]
    fn only_strong_and_scoped_references_identify() {
        assert!(Reference::new(RefScheme::Isin, "CA0000000000").identifies());
        let c = ConnectionId::parse("01923e6a-7b1c-7def-8123-456789abcdef").unwrap();
        assert!(Reference::connection_symbol(c, "QNC", Currency::CAD).identifies());
        assert!(!Reference::new(RefScheme::Yahoo, "QNC.V").identifies());
        assert_eq!(Reference::connection_symbol(c, "QNC", Currency::CAD).value, "QNC|CAD");
    }

    #[test]
    fn kinds_are_words() {
        for k in InstrumentKind::ALL {
            assert_eq!(InstrumentKind::parse(k.as_str()).unwrap(), *k);
        }
        assert!(InstrumentKind::parse("stock").is_err());
    }
}
