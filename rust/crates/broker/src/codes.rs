//! Wealthsimple's statement transaction codes, for the statement file a person
//! imports (`csv`). A code enters only as a statement shows it, with the row
//! that showed it; one not here is a problem naming it, never guessed.

use bagholder_core::transaction::Kind;

/// `(executed at YYYY-MM-DD)` in a row's description: the day a fill was made,
/// where the statement files it under its settlement. None where the row
/// states none; the text where it states one that is not a day.
pub fn executed_at(description: &str) -> Result<Option<jiff::civil::Date>, String> {
    let Some((_, after)) = description.split_once("(executed at ") else { return Ok(None) };
    let v = after.split(')').next().unwrap_or("").trim();
    v.parse().map(Some).map_err(|_| format!("an executed-at day that is not a day: {v:?}"))
}

/// What a statement code is, where the table places it.
pub fn kind(code: &str) -> Option<Kind> {
    Some(match code.trim().to_uppercase().as_str() {
        "BUY" => Kind::Buy,
        "SELL" => Kind::Sell,
        "DIV" => Kind::Dividend,
        "INT" => Kind::Interest,
        "FEE" => Kind::Fee,
        "CONT" => Kind::Deposit,
        "WD" => Kind::Withdrawal,
        "TRFIN" => Kind::TransferIn,
        "TRFOUT" => Kind::TransferOut,
        // each below as recorded statements showed it (2023-09 to 2026-10), with a row's form
        // (the recordings went with the statements' reading, brief 79; the codes stay
        // for a statement file a person imports)
        // `Transfer out to …` / `Transfer in`: between the person's own accounts
        "TRFOUTTF" => Kind::TransferOut,
        "TRFINTF" => Kind::TransferIn,
        // `Direct deposit from …`, `Deposit`, `Interac e-Transfer® Received`, `Cash received`
        "AFT_IN" | "EFT" | "IFT" | "E_TRFIN" | "P2P_RECEIVED" => Kind::Deposit,
        // `Pre-authorized Debit to …`, `Withdrawal`, `Online bill payment for …`, `Interac e-Transfer® Out`, `Cash sent`
        "AFT_OUT" | "EFTOUT" | "OBP_OUT" | "E_TRFOUT" | "P2P_SENT" => Kind::Withdrawal,
        // `Cash back - …`, `Giveaway received`: as the feed places a cashback and a promotion
        "CASHBACK" | "GIVEAWAY" => Kind::Cashback,
        // `Referral bonus (…)`: as the feed places a promotion
        "REFER" => Kind::Cashback,
        // `Cash correction (executed at …)`: the broker's own charge or refund of a cent
        "CORRECTION" => Kind::Fee,
        // `Reimbursement received`: as the feed places a reimbursement of a fee
        "REIMB" => Kind::Fee,
        // `Margin Interest Charges for …`
        "INTCHARGED" => Kind::InterestCharge,
        // `Convert CAD (executed at …) - $…USD = $…CAD` in the CAD list, `Convert USD …` in the USD list: each currency's side in its own list
        "FXCONVERSION" => Kind::CurrencyConversion,
        // `XQM 72.00 USD CALL 2034-06-20: Bought 1 contract (executed at …)`, with `, Fee: $…` or
        // without, and `Sold`: fills. A contract's row names no security after its terms
        "BUYTOCLOSE" | "BUYTOOPEN" => Kind::Buy,
        "SELLTOOPEN" | "SELLTOCLOSE" => Kind::Sell,
        // `Stock lending monthly interest payment`
        "FPLINT" => Kind::Interest,
        // `Non-resident tax (executed at …)`
        "NRT" => Kind::WithholdingTax,
        // `Federal withholding tax (executed at …)`
        "WHTFED" => Kind::WithholdingTax,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_a_statement_showed_is_placed_and_any_other_is_not() {
        for (code, want) in [
            ("WD", Kind::Withdrawal),
            ("TRFIN", Kind::TransferIn),
            ("trfouttf", Kind::TransferOut),
            ("TRFINTF", Kind::TransferIn),
            ("AFT_IN", Kind::Deposit),
            ("E_TRFIN", Kind::Deposit),
            ("OBP_OUT", Kind::Withdrawal),
            ("P2P_SENT", Kind::Withdrawal),
            ("CASHBACK", Kind::Cashback),
            ("GIVEAWAY", Kind::Cashback),
            ("REIMB", Kind::Fee),
            ("INTCHARGED", Kind::InterestCharge),
            ("FXCONVERSION", Kind::CurrencyConversion),
            ("BUYTOCLOSE", Kind::Buy),
            ("SELLTOOPEN", Kind::Sell),
            ("BUYTOOPEN", Kind::Buy),
            ("SELLTOCLOSE", Kind::Sell),
            ("FPLINT", Kind::Interest),
            ("REFER", Kind::Cashback),
            ("CORRECTION", Kind::Fee),
            ("NRT", Kind::WithholdingTax),
            ("WHTFED", Kind::WithholdingTax),
            ("SELL", Kind::Sell),
        ] {
            assert_eq!(kind(code), Some(want), "{code}");
        }
        // the rest of the codes the recorded statements show, each placed
        for (code, want) in [
            ("BUY", Kind::Buy),
            ("DIV", Kind::Dividend),
            ("INT", Kind::Interest),
            ("FEE", Kind::Fee),
            ("CONT", Kind::Deposit),
            ("TRFOUT", Kind::TransferOut),
            ("EFT", Kind::Deposit),
            ("IFT", Kind::Deposit),
            ("P2P_RECEIVED", Kind::Deposit),
            ("AFT_OUT", Kind::Withdrawal),
            ("EFTOUT", Kind::Withdrawal),
            ("E_TRFOUT", Kind::Withdrawal),
        ] {
            assert_eq!(kind(code), Some(want), "{code}");
        }
        // shown on the recorded statements and not placed: an option's assignment or
        // expiry, a coin's reward, staking, a share's loan and recall, a stock
        // distribution or reorganization. Their rows are no movement the walk books
        for code in ["ASSIGN", "EXPIRY", "CRYPTORWD", "STAKE", "UNSTAKE", "LOAN", "RECALL", "STKDIS", "STKREORG"] {
            assert_eq!(kind(code), None, "{code} is recorded but not placed");
        }
        for code in ["", "WHTPROV", "XYZ", "DEP"] {
            assert_eq!(kind(code), None, "{code} has not been seen on a statement, so it is not placed");
        }
    }
}
