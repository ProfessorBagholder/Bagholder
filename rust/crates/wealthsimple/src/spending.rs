//! The company a row of the person's spending names, for agents reading their
//! spending: a card purchase's merchant, a bill payment's company, a
//! pre-authorized debit's originator. A person's name (an e-Transfer's, a P2P
//! payment's) is never one: it is someone else's, and stays in the book.

use bagholder_core::json::Value;

/// The fields that name a company, each on the kind of row that states it.
pub const COMPANY: [&str; 3] = ["spendMerchant", "billPayCompanyName", "aftOriginatorName"];

/// The company `row` names, as Wealthsimple wrote it; none where it names none.
pub fn company(row: &Value) -> Option<String> {
    let Value::Object(fields) = row else { return None };
    COMPANY.iter().find_map(|k| match fields.get(*k) {
        Some(Value::String(v)) => Some(v.trim().to_string()).filter(|v| !v.is_empty()),
        _ => None,
    })
}

/// The company a stored record names: the row under its `activity`, beside the
/// replies read for it; none for a payload that is not one.
pub fn company_of_record(payload: &str) -> Option<String> {
    let Ok(Value::Object(parts)) = bagholder_core::json::parse(payload) else { return None };
    parts.get("activity").and_then(company)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(fields: &[(&str, &str)]) -> Value {
        bagholder_core::json::parse(&format!("{{{}}}", fields.iter().map(|(k, v)| format!("\"{k}\":\"{v}\"")).collect::<Vec<_>>().join(","))).unwrap()
    }

    #[test]
    fn a_company_is_named_and_a_person_never_is() {
        assert_eq!(company(&row(&[("type", "CREDIT_CARD"), ("subType", "PURCHASE"), ("spendMerchant", " Corner Grocer ")])).as_deref(), Some("Corner Grocer"));
        assert_eq!(company(&row(&[("type", "WITHDRAWAL"), ("subType", "BILL_PAY"), ("billPayCompanyName", "City Hydro"), ("billPayPayeeNickname", "my hydro")])).as_deref(), Some("City Hydro"));
        assert_eq!(company(&row(&[("type", "WITHDRAWAL"), ("subType", "AFT"), ("aftOriginatorName", "Some Insurer")])).as_deref(), Some("Some Insurer"));
        // another person: an e-Transfer's and a P2P payment's names and messages are never read
        assert_eq!(company(&row(&[("type", "WITHDRAWAL"), ("subType", "E_TRANSFER"), ("eTransferName", "A Person"), ("eTransferEmail", "a@example.com")])), None);
        assert_eq!(company(&row(&[("type", "P2P_PAYMENT"), ("p2pHandle", "someone"), ("p2pMessage", "rent")])), None);
        assert_eq!(company(&row(&[("spendMerchant", "  ")])), None);
        assert_eq!(company_of_record(r#"{"activity":{"spendMerchant":"Corner Grocer"},"replies":{}}"#).as_deref(), Some("Corner Grocer"));
        assert_eq!(company_of_record("not json"), None);
    }
}
