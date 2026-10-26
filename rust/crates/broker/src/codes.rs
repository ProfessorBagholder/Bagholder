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
/// (`Purchase of 7.2177302134 XQD (executed at …), FX Rate: …`, `Sale of 3.0E-8
/// XQX (executed at …), FX Rate: …, Fee charged $…`, `XQJ - Examplia
/// Wobbleworks Plc.: Bought 4.8200 shares at $3.00 per share (executed at …)`,
/// the recorded forms in `wealthsimple/tests/replies/wealthsimple-statements`).
/// None for a contract's fill, or a description of another shape.
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
    let exp: i64 = exp.parse().ok()?;
    // past the cap no digit lands where a `Dec` holds one: refused before a string that long is built
    if exp.abs() > bagholder_core::Dec::EXPONENT_CAP {
        return None;
    }
    let negative = mantissa.starts_with('-');
    let m = mantissa.trim_start_matches(['-', '+']);
    let (whole, frac) = m.split_once('.').unwrap_or((m, ""));
    if whole.is_empty() || !whole.chars().chain(frac.chars()).all(|c| c.is_ascii_digit()) {
        return None;
    }
    let digits = format!("{whole}{frac}");
    // the decimal point sits after `whole.len()` digits, moved by the exponent
    let point = i64::try_from(whole.len()).ok()? + exp;
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
        // each below as recorded statements showed it (2023-09 to 2026-10), with a row's form
        // (`wealthsimple/tests/replies/wealthsimple-statements`)
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
    fn a_fill_row_states_its_symbol_and_units_exactly_in_each_form_a_statement_writes() {
        let d = |s: &str| bagholder_core::Dec::parse(s).unwrap();
        // the recorded forms (`wealthsimple/tests/replies/wealthsimple-statements`), made-up figures
        assert_eq!(fill_of("Purchase of 7.2177302134 XQD (executed at 2034-03-03), FX Rate: 1.8536"), Some(("XQD".into(), d("7.2177302134"))));
        assert_eq!(fill_of("Purchase of 7.2177302134 XQF (executed at 2034-03-07), FX Rate: 1.8536, Fee charged $3.73"), Some(("XQF".into(), d("7.2177302134"))));
        assert_eq!(fill_of("Sale of 3.0E-8 XQX (executed at 2034-04-27), FX Rate: 1.8536, Fee charged $0.01"), Some(("XQX".into(), d("0.00000003"))));
        assert_eq!(fill_of("Sale of 1.5E2 XQY"), Some(("XQY".into(), d("150"))));
        assert_eq!(fill_of("XQJ - Examplia Wobbleworks Plc.: Bought 4.8200 shares at $3.00 per share (executed at 2034-03-06)"), Some(("XQJ".into(), d("4.82"))));
        assert_eq!(fill_of("XQL - Examplia Wobbleworks Plc.: Sold 4.8200 shares (executed at 2034-03-30)"), Some(("XQL".into(), d("4.82"))));
        assert_eq!(fill_of("XQN - Examplia Wobbleworks Plc.: Sold 4.8200 shares (executed at 2034-04-19), FX Rate: 1.8536"), Some(("XQN".into(), d("4.82"))));
        // a contract's fill, and rows that move no fill
        assert_eq!(fill_of("XQM 72.00 USD CALL 2034-06-20: Bought 1 contract (executed at 2034-03-02)"), None);
        assert_eq!(fill_of("XQB 72.00 USD CALL 2034-06-20: Assigned -1 contract (executed at 2034-03-03)"), None);
        assert_eq!(fill_of("Fee for purchase of 7.2177302134 XQP (executed at 2034-03-15), FX Rate: 1.8536"), None);
        assert_eq!(fill_of("XQF - Examplia Wobbleworks Plc.: 4.8200 Shares on loan (executed at 2034-03-03)"), None);
    }

    #[test]
    fn an_exponent_past_the_cap_is_refused_before_it_is_expanded() {
        let d = |s: &str| bagholder_core::Dec::parse(s).unwrap();
        let cap = bagholder_core::Dec::EXPONENT_CAP;
        // expanded, the largest would be more zeros than any memory holds
        for exp in [cap + 1, i64::from(i32::MAX), i64::MAX] {
            assert_eq!(plain_number(&format!("3.0E{exp}")), None, "E{exp}");
            assert_eq!(plain_number(&format!("3.0E-{exp}")), None, "E-{exp}");
        }
        assert_eq!(plain_number("3.0E-8"), Some(d("0.00000003")));
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
