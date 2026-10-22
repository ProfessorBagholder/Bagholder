//! The kinds of account Wealthsimple's `unifiedAccountType` names.

use bagholder_core::account::{AccountKind, AccountType, Registration};

/// What Wealthsimple's `unifiedAccountType` words say an account is, in
/// Bagholder's vocabulary; words not listed here are kept as they are.
pub fn account_type(words: &str) -> AccountType {
    use AccountKind::*;
    use Registration::*;
    let words = words.to_string();
    let known = |kind, registration, managed, joint| AccountType::Known { kind, registration, managed, joint };
    match words.as_str() {
        "SELF_DIRECTED_NON_REGISTERED" => known(Cash, Unregistered, false, false),
        "SELF_DIRECTED_NON_REGISTERED_MARGIN" => known(Margin, Unregistered, false, false),
        "SELF_DIRECTED_JOINT_NON_REGISTERED" => known(Cash, Unregistered, false, true),
        "SELF_DIRECTED_JOINT_NON_REGISTERED_MARGIN" => known(Margin, Unregistered, false, true),
        "SELF_DIRECTED_TFSA" => known(Cash, Tfsa, false, false),
        "SELF_DIRECTED_FHSA" => known(Cash, Fhsa, false, false),
        "SELF_DIRECTED_RRSP" => known(Cash, Rrsp, false, false),
        "SELF_DIRECTED_RRIF" => known(Cash, Rrif, false, false),
        "SELF_DIRECTED_LIRA" => known(Cash, Lira, false, false),
        "SELF_DIRECTED_RESP_FAMILY" => known(Cash, Resp, false, false),
        "SELF_DIRECTED_JOINT_RESP_FAMILY" => known(Cash, Resp, false, true),
        "SELF_DIRECTED_CRYPTO" => known(Crypto, Unregistered, false, false),
        "SELF_DIRECTED_NON_REGISTERED_PREDICTIONS" => known(EventContracts, Unregistered, false, false),
        "MANAGED_NON_REGISTERED" | "MANAGED_PORTFOLIO_NON_REGISTERED" => known(Cash, Unregistered, true, false),
        "MANAGED_JOINT" => known(Cash, Unregistered, true, true),
        "MANAGED_TFSA" => known(Cash, Tfsa, true, false),
        "MANAGED_FHSA" => known(Cash, Fhsa, true, false),
        "MANAGED_RRSP" => known(Cash, Rrsp, true, false),
        "MANAGED_LIRA" => known(Cash, Lira, true, false),
        "MANAGED_RESP_FAMILY" => known(Cash, Resp, true, false),
        "MANAGED_GROUP_RRSP" => known(Cash, GroupRrsp, true, false),
        "CASH" | "CASH_USD" | "YOUTH_CASH" => known(Spending, Unregistered, false, false),
        "CREDIT_CARD" => known(CreditCard, Unregistered, false, false),
        "PORTFOLIO_LINE_OF_CREDIT" => known(LineOfCredit, Unregistered, false, false),
        _ => AccountType::Unrecognised(if words.is_empty() { "no type given".into() } else { words }),
    }
}
