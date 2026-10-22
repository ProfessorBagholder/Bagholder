//! How a row Wealthsimple states with its activity type and sub-type is
//! booked: Bagholder's kind, what the instrument is, and which way its units and
//! its cash go. A file Wealthsimple exported states its rows this way (`csv`).

use bagholder_core::instrument::InstrumentKind;
use bagholder_core::transaction::Kind;
use bagholder_core::Dec;

/// How a row of one activity type and sub-type is booked.
pub struct Booking {
    pub kind: Kind,
    /// What the instrument is, for a row that concerns one.
    pub instrument: Option<InstrumentKind>,
    pub units: Units,
    pub cash: CashSign,
}

/// Which way a row's units go.
#[derive(Clone, Copy)]
pub enum Units {
    None,
    /// The row's quantity made positive.
    In,
    /// The row's quantity made negative.
    Out,
    /// The row's quantity with its own sign.
    AsSigned,
}

/// Which way a row's cash goes.
#[derive(Clone, Copy)]
pub enum CashSign {
    None,
    /// The row's cash with its own sign.
    AsSigned,
    /// Paid: made negative.
    Paid,
    /// Received: made positive.
    Received,
}

/// How a row of activity type `ty` and sub-type `sub` is booked: a transfer's
/// way is its cash's sign, or its `direction` where it moved no cash. None for a
/// row of a type not placed.
pub fn booking(ty: &str, sub: &str, cash: Option<Dec>, direction: &str) -> Option<Booking> {
    use InstrumentKind as I;
    let r = |kind, instrument, units, cash| Some(Booking { kind, instrument, units, cash });
    // a transfer's way: its cash's sign, or the row's direction where it moved
    // none; with neither, the way is not stated and the row is not placed
    let transfer = || match cash {
        Some(c) if c.is_positive() => Some(Kind::TransferIn),
        Some(c) if c.is_negative() => Some(Kind::TransferOut),
        _ => match direction {
            "CREDIT" => Some(Kind::TransferIn),
            "DEBIT" => Some(Kind::TransferOut),
            _ => None,
        },
    };
    match (ty, sub) {
        ("Trade", "BUY") => r(Kind::Buy, Some(I::Security), Units::In, CashSign::Paid),
        ("Trade", "SELL") => r(Kind::Sell, Some(I::Security), Units::Out, CashSign::Received),
        // whether an option fill opened or closed is not read from its sub-type
        ("OPTIONS_BUY", "BUYTOOPEN" | "BUYTOCLOSE") => r(Kind::Buy, Some(I::OptionContract), Units::In, CashSign::Paid),
        ("OPTIONS_SELL", "SELLTOOPEN" | "SELLTOCLOSE") => r(Kind::Sell, Some(I::OptionContract), Units::Out, CashSign::Received),
        ("EXPIR", "BUY") => r(Kind::OptionExpiry, Some(I::OptionContract), Units::In, CashSign::None),
        ("EXPIR", "SELL") => r(Kind::OptionExpiry, Some(I::OptionContract), Units::Out, CashSign::None),
        ("ASSIGN", _) => r(Kind::OptionAssignment, Some(I::OptionContract), Units::AsSigned, CashSign::AsSigned),
        ("EXERCISE", _) => r(Kind::OptionExercise, Some(I::OptionContract), Units::AsSigned, CashSign::AsSigned),
        ("STKDIS", "BUY" | "SELL") => r(Kind::CorporateEvent, Some(I::Security), Units::AsSigned, CashSign::None),
        ("Dividend", _) => r(Kind::Dividend, Some(I::Security), Units::None, CashSign::AsSigned),
        ("Interest", _) => r(Kind::Interest, None, Units::None, CashSign::AsSigned),
        ("INTEREST_CHARGE", _) => r(Kind::InterestCharge, None, Units::None, CashSign::AsSigned),
        ("WITHHOLDING_TAX", _) => r(Kind::WithholdingTax, None, Units::None, CashSign::AsSigned),
        ("FxExchange", _) => r(Kind::CurrencyConversion, None, Units::None, CashSign::AsSigned),
        ("Deposit", _) => r(Kind::Deposit, None, Units::None, CashSign::AsSigned),
        ("GROUP_CONTRIBUTION", "EMPLOYER_CONTRIBUTION") => r(Kind::EmployerDeposit, None, Units::None, CashSign::AsSigned),
        ("GROUP_CONTRIBUTION", "EMPLOYEE_CONTRIBUTION") => r(Kind::Deposit, None, Units::None, CashSign::AsSigned),
        ("RESP_GRANT", _) => r(Kind::GovernmentDeposit, None, Units::None, CashSign::AsSigned),
        ("Withdrawal", _) => r(Kind::Withdrawal, None, Units::None, CashSign::AsSigned),
        ("Transfer", _) | ("ASSET_MOVEMENT", _) => r(transfer()?, None, Units::None, CashSign::AsSigned),
        ("CRYPTO_BUY", _) => r(Kind::Buy, Some(I::Crypto), Units::In, CashSign::Paid),
        ("CRYPTO_SELL", _) => r(Kind::Sell, Some(I::Crypto), Units::Out, CashSign::Received),
        ("CRYPTO_TRANSFER", "TRANSFER_IN") => r(Kind::TransferIn, Some(I::Crypto), Units::In, CashSign::None),
        ("CRYPTO_TRANSFER", "TRANSFER_OUT") => r(Kind::TransferOut, Some(I::Crypto), Units::Out, CashSign::None),
        ("CRYPTO_STAKING_REWARD", _) => r(Kind::StakingReward, Some(I::Crypto), Units::In, CashSign::None),
        ("CRYPTO_STAKING_ACTION", _) => r(Kind::StakingMove, Some(I::Crypto), Units::None, CashSign::None),
        ("PREDICTIONS_BUY", _) => r(Kind::Buy, Some(I::EventContract), Units::In, CashSign::Paid),
        ("PREDICTIONS_RESOLUTION", _) => r(Kind::Resolution, Some(I::EventContract), Units::Out, CashSign::AsSigned),
        ("CREDIT_CARD", "PURCHASE") => r(Kind::CardPurchase, None, Units::None, CashSign::AsSigned),
        ("CREDIT_CARD", "REFUND") => r(Kind::CardRefund, None, Units::None, CashSign::AsSigned),
        ("CREDIT_CARD", "PAYMENT") => r(Kind::TransferIn, None, Units::None, CashSign::AsSigned),
        ("CREDIT_CARD_PAYMENT", _) => r(Kind::TransferOut, None, Units::None, CashSign::AsSigned),
        ("REIMBURSEMENT", _) => r(Kind::Cashback, None, Units::None, CashSign::AsSigned),
        _ => None,
    }
}
