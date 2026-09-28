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

/// What a fill row's description states it moved: the symbol and the units
/// (`Purchase of 2442.5025927700 DOGE (executed at …)`, `Sale of 5.300E-7
/// STORJ`, `QNC - Quantum Emotion Corp: Bought 1000.0000 shares at $2.79 per
/// share`). None for a contract's fill, or a description of another shape.
pub fn fill_of(description: &str) -> Option<(String, bagholder_core::Dec)> {
    let d = description.trim();
    let (number, symbol) = if let Some(rest) = d.strip_prefix("Purchase of ").or_else(|| d.strip_prefix("Sale of ")) {
        let mut words = rest.split_whitespace();
        (words.next()?, words.next()?.to_string())
    } else {
        let (symbol, rest) = d.split_once(" - ")?;
        let (_, what) = rest.split_once(": ")?;
        let what = what.strip_prefix("Bought ").or_else(|| what.strip_prefix("Sold "))?;
        let mut words = what.split_whitespace();
        let n = words.next()?;
        if words.next()? != "shares" {
            return None;
        }
        (n, symbol.trim().to_string())
    };
    if symbol.is_empty() || symbol.contains(' ') {
        return None;
    }
    Some((symbol, plain_number(number)?))
}

/// A number as a statement writes it, `2442.50`, or with an exponent, `5.300E-7`.
fn plain_number(text: &str) -> Option<bagholder_core::Dec> {
    let Some((mantissa, exp)) = text.split_once(['E', 'e']) else { return bagholder_core::Dec::parse(text).ok() };
    let exp: i32 = exp.parse().ok()?;
    let negative = mantissa.starts_with('-');
    let m = mantissa.trim_start_matches(['-', '+']);
    let (whole, frac) = m.split_once('.').unwrap_or((m, ""));
    if whole.is_empty() || !whole.chars().chain(frac.chars()).all(|c| c.is_ascii_digit()) {
        return None;
    }
    let digits = format!("{whole}{frac}");
    // the decimal point sits after `whole.len()` digits, moved by the exponent
    let point = i64::try_from(whole.len()).ok()? + i64::from(exp);
    let text = if point <= 0 {
        format!("0.{}{digits}", "0".repeat(usize::try_from(-point).ok()?))
    } else if point as usize >= digits.len() {
        format!("{digits}{}", "0".repeat(point as usize - digits.len()))
    } else {
        format!("{}.{}", &digits[..point as usize], &digits[point as usize..])
    };
    bagholder_core::Dec::parse(&format!("{}{text}", if negative { "-" } else { "" })).ok()
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
        // `Referral bonus (2024-03-04)`: as the feed places a promotion
        "REFER" => Kind::Cashback,
        // `Cash correction (executed at 2024-03-14)`: the broker's own charge or refund of a cent
        "CORRECTION" => Kind::Fee,
        // `Reimbursement received`: as the feed places a reimbursement of a fee
        "REIMB" => Kind::Fee,
        // `Margin Interest Charges for …`
        "INTCHARGED" => Kind::InterestCharge,
        // `Convert CAD (executed at …) - $1USD = $1.35CAD`: each currency's side in its own list
        "FXCONVERSION" => Kind::CurrencyConversion,
        // `BBAI 5.00 USD PUT …: Bought 26 contract`, `Sold 1 contract`: fills
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
    fn a_fill_row_states_its_symbol_and_units_exactly_in_each_form_a_statement_writes() {
        let d = |s: &str| bagholder_core::Dec::parse(s).unwrap();
        assert_eq!(fill_of("Purchase of 2442.5025927700 DOGE (executed at 2024-11-11), FX Rate: 1.3990"), Some(("DOGE".into(), d("2442.5025927700"))));
        assert_eq!(fill_of("Sale of 5.300E-7 STORJ (executed at 2026-08-26)"), Some(("STORJ".into(), d("0.0000005300"))));
        assert_eq!(fill_of("Sale of 1.5E2 XYZ"), Some(("XYZ".into(), d("150"))));
        assert_eq!(fill_of("QNC - Quantum Emotion Corp: Bought 1000.0000 shares at $2.79 per share (executed at 2026-06-11)"), Some(("QNC".into(), d("1000"))));
        assert_eq!(fill_of("QNC - Quantum Emotion Corp: Sold 12.5 shares at $3.23 per share"), Some(("QNC".into(), d("12.5"))));
        assert_eq!(fill_of("BBAI 5.00 USD PUT 2026-01-16 - BigBear.ai: Bought 26 contract"), None);
        assert_eq!(fill_of("Fee for purchase of 1 BTC"), None);
    }

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
        for code in ["", "WHTPROV", "XYZ", "DEP"] {
            assert_eq!(kind(code), None, "{code} has not been seen on a statement, so it is not placed");
        }
    }
}
