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
        // 💰Cash, June 2025: `TRFOUTTF Transfer out -50.0`
        "TRFOUTTF" => Kind::TransferOut,
        // 💰Cash, June 2025: `AFT_IN Direct deposit from goPeer WD-P8WS7 57.81`
        "AFT_IN" => Kind::Deposit,
        // Retirement, June 2025 and January 2026: `WHTFED Federal withholding tax (executed at …) -15278.57`
        "WHTFED" => Kind::WithholdingTax,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_a_statement_showed_is_placed_and_any_other_is_not() {
        for (code, want) in [("WD", Kind::Withdrawal), ("TRFIN", Kind::TransferIn), ("trfouttf", Kind::TransferOut), ("AFT_IN", Kind::Deposit), ("WHTFED", Kind::WithholdingTax), ("SELL", Kind::Sell)] {
            assert_eq!(kind(code), Some(want), "{code}");
        }
        for code in ["", "WHTPROV", "XYZ", "DEP"] {
            assert_eq!(kind(code), None, "{code} has not been seen on a statement, so it is not placed");
        }
    }
}
