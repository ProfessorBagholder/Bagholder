//! The company a row of the person's money names, for agents reading it: a card
//! purchase's merchant, a bill payment's company, a pre-authorized debit's
//! originator; and what is on the row's other side (`other_side`). A person's name
//! (an e-Transfer's, a P2P payment's) is never one: it is someone else's, and stays
//! in the book.

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
    of_record(payload).0
}

/// What is on the other side of a row's money, as Wealthsimple states it: one of
/// the person's own accounts, outside Wealthsimple, or nothing the row says. Read
/// from what the row states alone, never from an amount or a description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OtherSide {
    /// One of the person's own accounts, by Wealthsimple's id for it
    /// (`opposingAccountId`): the reader names it by the book's own account.
    Own(String),
    /// Outside Wealthsimple, with the institution's name where the row states one.
    Outside(Option<String>),
    /// The row says neither.
    Unstated,
}

/// The rows that move money to or from outside Wealthsimple, by type and subtype,
/// and whether the row's `institutionName` names the institution on the other side,
/// as the recorded feed shows each (`tests/replies/wealthsimple/edited-activity-one-per-kind.json`):
/// a bank transfer, a pre-authorized debit or credit, a bill payment, an e-Transfer,
/// a deposit by card, and a transfer from or to another institution. An
/// e-Transfer's name and email, a payee's nickname and an account number are never
/// read: they are a person's, or the person's own details at another bank.
pub const OUTSIDE: [(&str, &str, bool); 11] = [
    ("DEPOSIT", "EFT", false),
    ("WITHDRAWAL", "EFT", false),
    ("DEPOSIT", "AFT", false),
    ("WITHDRAWAL", "AFT", false),
    ("WITHDRAWAL", "BILL_PAY", false),
    ("DEPOSIT", "E_TRANSFER", false),
    ("WITHDRAWAL", "E_TRANSFER", false),
    ("DEPOSIT", "E_TRANSFER_FUNDING", false),
    ("DEPOSIT", "PAYMENT_CARD_TRANSACTION", false),
    ("INSTITUTIONAL_TRANSFER_INTENT", "TRANSFER_IN", true),
    ("INSTITUTIONAL_TRANSFER_INTENT", "TRANSFER_OUT", true),
];

/// What is on the other side of `row`'s money.
pub fn other_side(row: &Value) -> OtherSide {
    let Value::Object(fields) = row else { return OtherSide::Unstated };
    let text = |k: &str| match fields.get(k) {
        Some(Value::String(v)) => Some(v.trim().to_string()).filter(|v| !v.is_empty()),
        _ => None,
    };
    if let Some(id) = text("opposingAccountId").filter(|o| Some(o) != text("accountId").as_ref()) {
        return OtherSide::Own(id);
    }
    let (ty, sub) = (text("type"), text("subType"));
    match OUTSIDE.iter().find(|(t, s, _)| ty.as_deref() == Some(*t) && sub.as_deref() == Some(*s)) {
        Some((_, _, named)) => OtherSide::Outside(if *named { text("institutionName") } else { None }),
        None => OtherSide::Unstated,
    }
}

/// What is on the other side of a stored record's money: the row under its
/// `activity`; nothing the row says for a payload that is not one.
pub fn other_side_of_record(payload: &str) -> OtherSide {
    of_record(payload).1
}

/// A stored record's company and other side, its payload read once.
pub fn of_record(payload: &str) -> (Option<String>, OtherSide) {
    let Ok(Value::Object(parts)) = bagholder_core::json::parse(payload) else { return (None, OtherSide::Unstated) };
    match parts.get("activity") {
        Some(row) => (company(row), other_side(row)),
        None => (None, OtherSide::Unstated),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The activity rows of a recorded feed reply, each `node` of its edges.
    fn recorded_rows(file: &str) -> Vec<Value> {
        fn nodes(v: &Value, out: &mut Vec<Value>) {
            match v {
                Value::Object(m) => {
                    if let Some(n @ Value::Object(inner)) = m.get("node") {
                        if inner.contains_key("type") {
                            out.push(n.clone());
                        }
                    }
                    m.values().for_each(|v| nodes(v, out));
                }
                Value::Array(a) => a.iter().for_each(|v| nodes(v, out)),
                _ => {}
            }
        }
        let text = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/replies/wealthsimple").join(file)).unwrap();
        let mut out = Vec::new();
        nodes(&bagholder_core::json::parse(&text).unwrap(), &mut out);
        out
    }

    fn field<'a>(row: &'a Value, k: &str) -> Option<&'a str> {
        match row {
            Value::Object(m) => match m.get(k) {
                Some(Value::String(s)) => Some(s.as_str()),
                _ => None,
            },
            _ => None,
        }
    }

    #[test]
    fn a_move_between_the_persons_own_accounts_names_the_other_account_on_both_sides() {
        let rows = recorded_rows("edited-activity-one-per-kind.json");
        let moves: Vec<&Value> = rows.iter().filter(|r| field(r, "type") == Some("INTERNAL_TRANSFER")).collect();
        assert!(moves.len() >= 2, "the recorded feed holds a move's two rows");
        for r in &moves {
            assert_eq!(other_side(r), OtherSide::Own(field(r, "opposingAccountId").unwrap().to_string()), "{r:?}");
        }
        // each side names the other: a source's other account is a destination's own
        let pair = moves.iter().find(|a| moves.iter().any(|b| field(a, "accountId") == field(b, "opposingAccountId") && field(b, "accountId") == field(a, "opposingAccountId"))).expect("a recorded pair");
        let OtherSide::Own(other) = other_side(pair) else { panic!() };
        assert!(moves.iter().any(|b| field(b, "accountId") == Some(other.as_str()) && other_side(b) == OtherSide::Own(field(pair, "accountId").unwrap().to_string())));
    }

    #[test]
    fn money_from_outside_says_outside_and_names_an_institution_only_where_the_row_states_one() {
        let rows = recorded_rows("edited-activity-one-per-kind.json");
        let of = |ty: &str, sub: &str| rows.iter().find(|r| field(r, "type") == Some(ty) && field(r, "subType") == Some(sub)).unwrap_or_else(|| panic!("{ty} {sub} recorded"));
        let from_bank = of("INSTITUTIONAL_TRANSFER_INTENT", "TRANSFER_IN");
        assert_eq!(other_side(from_bank), OtherSide::Outside(field(from_bank, "institutionName").map(str::to_string)));
        assert!(matches!(other_side(from_bank), OtherSide::Outside(Some(_))));
        for (ty, sub) in [("DEPOSIT", "EFT"), ("WITHDRAWAL", "EFT"), ("DEPOSIT", "AFT"), ("WITHDRAWAL", "AFT"), ("WITHDRAWAL", "BILL_PAY"), ("WITHDRAWAL", "E_TRANSFER"), ("DEPOSIT", "E_TRANSFER_FUNDING"), ("DEPOSIT", "PAYMENT_CARD_TRANSACTION")] {
            assert_eq!(other_side(of(ty, sub)), OtherSide::Outside(None), "{ty} {sub}");
        }
        // a row that states neither: a grant, a tax withheld, a P2P payment, a card payment
        for (ty, sub) in [("RESP_GRANT", "CESG"), ("P2P_PAYMENT", "SEND")] {
            assert_eq!(other_side(of(ty, sub)), OtherSide::Unstated, "{ty} {sub}");
        }
        assert_eq!(other_side_of_record(r#"{"activity":{"type":"DEPOSIT","subType":"EFT"},"replies":{}}"#), OtherSide::Outside(None));
        assert_eq!(other_side_of_record("not json"), OtherSide::Unstated);
    }

    /// Nothing of another person or of the person's details elsewhere reaches what a
    /// row's other side or company says, over every recorded activity row: a name, an
    /// email, a handle, a message, a payee's nickname, an account number, or
    /// Wealthsimple's own account id outside the `Own` the reader resolves.
    #[test]
    fn no_persons_name_or_account_number_is_ever_read_for_the_other_side() {
        const PRIVATE: [&str; 8] = ["eTransferName", "eTransferEmail", "p2pHandle", "p2pMessage", "billPayPayeeNickname", "redactedExternalAccountNumber", "accountId", "identityId"];
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/replies/wealthsimple");
        let mut rows = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            if !name.contains("activity") || !name.ends_with(".json") {
                continue;
            }
            for r in recorded_rows(&name) {
                rows += 1;
                let said: Vec<String> = match other_side(&r) {
                    OtherSide::Outside(Some(i)) => vec![i],
                    _ => vec![],
                }
                .into_iter()
                .chain(company(&r))
                .collect();
                for k in PRIVATE {
                    if let Some(v) = field(&r, k).filter(|v| !v.trim().is_empty()) {
                        assert!(!said.iter().any(|s| s.contains(v.trim())), "{name}: {k} reached what is said: {said:?}");
                    }
                }
            }
        }
        assert!(rows > 100, "every recorded activity row is read: {rows}");
    }

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
