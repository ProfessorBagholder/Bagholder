//! Wealthsimple's statement transaction codes, one table for the statement file
//! a person imports (`csv`) and the statements the sync reads (`statements`), so
//! a code is placed, or reported, the same way in both. A code enters only as a
//! statement shows it, with the row that showed it; one not here is a problem
//! naming it, never guessed.

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
        // each below as the owner's statements showed it (2023-09 to 2026-08), with a row's words
        // `Transfer out to ✉️ Bills` / `Transfer in`: between the person's own accounts
        "TRFOUTTF" => Kind::TransferOut,
        "TRFINTF" => Kind::TransferIn,
        // `Direct deposit from CANADA`, `Deposit`, `Interac e-Transfer® Received`, `Cash received`
        "AFT_IN" | "EFT" | "IFT" | "E_TRFIN" | "P2P_RECEIVED" => Kind::Deposit,
        // `Pre-authorized Debit to Tangerine`, `Withdrawal`, `Online bill payment for …`, `Interac e-Transfer® Out`, `Cash sent`
        "AFT_OUT" | "EFTOUT" | "OBP_OUT" | "E_TRFOUT" | "P2P_SENT" => Kind::Withdrawal,
        // `Cash back - Credit card`, `Giveaway received`: as the feed places a cashback and a promotion
        "CASHBACK" | "GIVEAWAY" => Kind::Cashback,
        // `Reimbursement received`: as the feed places a reimbursement of a fee
        "REIMB" => Kind::Fee,
        // `Margin Interest Charges for …`
        "INTCHARGED" => Kind::InterestCharge,
        // `Convert CAD (executed at …) - $1USD = $1.35CAD`: each currency's side in its own list
        "FXCONVERSION" => Kind::CurrencyConversion,
        // `BBAI 5.00 USD PUT …: Bought 26 contract`, `Sold 1 contract`: fills
        "BUYTOCLOSE" => Kind::Buy,
        "SELLTOOPEN" => Kind::Sell,
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
            ("WHTFED", Kind::WithholdingTax),
            ("SELL", Kind::Sell),
        ] {
            assert_eq!(kind(code), Some(want), "{code}");
        }
        for code in ["", "WHTPROV", "XYZ", "DEP"] {
            assert_eq!(kind(code), None, "{code} has not been seen on a statement, so it is not placed");
        }
    }
}
