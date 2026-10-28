//! The kinds of account Wealthsimple's `unifiedAccountType` names, and the reads
//! Wealthsimple issues for each.

use bagholder_broker::Reads;
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

/// The reads Wealthsimple issues for each kind of account, as a person's book had
/// them answered (brief 79, amended 2026-10-28): a margin account its cash, its
/// holdings, its history, what it can borrow and the rates it pays; an account of
/// securities, coins or event contracts the same without borrowing; a spending
/// account its cash and history (the positions read states nothing held in one);
/// a card or a line of credit its balance (the card read) and history. A kind
/// Bagholder does not know gets the accounts read and its activity alone.
pub fn reads(t: &AccountType) -> Reads {
    use AccountKind::*;
    let AccountType::Known { kind, .. } = t else { return Reads::default() };
    let securities = Reads { cash: true, positions: true, history: true, margin: false };
    match kind {
        Margin => Reads { margin: true, ..securities },
        Cash | Crypto | EventContracts => securities,
        Spending | CreditCard | LineOfCredit => Reads { positions: false, ..securities },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table, kind by kind, as the owner's book had each read answered.
    #[test]
    fn each_kind_gets_the_reads_wealthsimple_issues_for_it() {
        let r = |w: &str| reads(&account_type(w));
        let all = |cash, positions, history, margin| Reads { cash, positions, history, margin };
        assert_eq!(r("SELF_DIRECTED_NON_REGISTERED_MARGIN"), all(true, true, true, true));
        for w in ["SELF_DIRECTED_TFSA", "SELF_DIRECTED_RRSP", "SELF_DIRECTED_NON_REGISTERED", "SELF_DIRECTED_LIRA", "MANAGED_TFSA", "SELF_DIRECTED_CRYPTO", "SELF_DIRECTED_NON_REGISTERED_PREDICTIONS"] {
            assert_eq!(r(w), all(true, true, true, false), "{w}");
        }
        for w in ["CASH", "CASH_USD", "CREDIT_CARD", "PORTFOLIO_LINE_OF_CREDIT"] {
            assert_eq!(r(w), all(true, false, true, false), "{w}: no holdings read");
        }
        assert_eq!(r("A_KIND_NOT_KNOWN"), Reads::default(), "the accounts read and the activity alone");
    }
}
